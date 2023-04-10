use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::mem;

use append_only_vec::AppendOnlyVec;
use futures::{stream, StreamExt};
use tokio::sync::{RwLock, RwLockMappedWriteGuard, RwLockReadGuard, RwLockWriteGuard};
use tower_lsp::lsp_types::Url;
use typst::diag::FileResult;

use crate::lsp_typst_boundary::TypstSourceId;

use super::source::Source;

#[derive(Debug, Clone, Copy)]
pub struct SourceId(u16);

impl From<TypstSourceId> for SourceId {
    fn from(typst_id: TypstSourceId) -> Self {
        Self(typst_id.into_u16())
    }
}

impl From<SourceId> for TypstSourceId {
    fn from(lsp_id: SourceId) -> Self {
        Self::from_u16(lsp_id.0)
    }
}

/// Typst source while for which we have a cached `Source`, or we did until it was invalidated
#[derive(Debug)]
enum CachedSource {
    /// Currently open in the editor, in the sense that we received `textDocument/didOpen` but not
    /// `textDocument/didClose` for it
    Open(Source),
    /// Not open in the editor, but the FS watcher has not told us about changes since the `Source`
    /// was known to be correct
    ClosedUnmodified(Source),
    /// Not open in the editor, and the FS watcher told us that it changed since the last associated
    /// `Source` was known to be correct
    ClosedModified(Url),
}

impl CachedSource {
    pub async fn new_cached(id: SourceId, uri: Url) -> FileResult<Self> {
        Source::new_from_file(id, uri)
            .await
            .map(Self::ClosedUnmodified)
    }

    pub fn new_open(id: SourceId, uri: Url, text: String) -> Self {
        Self::Open(Source::new(id, uri, text))
    }

    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open(_))
    }

    pub fn get_cached_source(&self) -> Option<&Source> {
        match self {
            Self::Open(source) | Self::ClosedUnmodified(source) => Some(source),
            Self::ClosedModified(_) => None,
        }
    }

    pub fn get_mut_cached_source(&mut self) -> Option<&mut Source> {
        match self {
            Self::Open(source) | Self::ClosedUnmodified(source) => Some(source),
            Self::ClosedModified(_) => None,
        }
    }

    pub fn take_cached_source(self) -> Option<Source> {
        match self {
            Self::Open(source) | Self::ClosedUnmodified(source) => Some(source),
            Self::ClosedModified(_) => None,
        }
    }

    /// Caches the file if it is not cached, unless there is an error
    pub async fn cache(&mut self, id: SourceId) -> FileResult<()> {
        if let Self::ClosedModified(uri) = self {
            let cached = Self::new_cached(id, uri.clone()).await?;
            mem::replace(self, cached);
        }
        Ok(())
    }
}

/// Owns, tracks, and caches Typst source files and their ids
pub struct SourceManager {
    ids: RwLock<HashMap<Url, SourceId>>,
    sources: AppendOnlyVec<RwLock<CachedSource>>,
}

impl SourceManager {
    /// The URIs of the files currently open in the editor
    pub async fn open_uris(&self) -> Vec<Url> {
        let ids = self.ids.read().await;
        stream::iter(ids.iter())
            .filter(|(_uri, &id)| {
                Box::pin(async move { self.get_cached_source(id).await.is_open() })
            })
            .map(|(uri, _id)| uri.clone())
            .collect()
            .await
    }

    /// Get the id of a file by its URI. Loads and caches the file if it is not already cached,
    /// which may cause an error.
    pub async fn get_id(&self, uri: Url) -> FileResult<SourceId> {
        let mut ids = self.ids.write().await;

        // We hold an exclusive write lock on `ids`, so it can't change until after the id has been
        // committed.
        // We take this now because `entry` borrows `ids` mutably later.
        let next_id = SourceId(ids.len() as u16);

        match ids.entry(uri.clone()) {
            Entry::Vacant(entry) => {
                let source = Source::new_from_file(next_id, uri).await?;

                self.sources
                    .push(RwLock::new(CachedSource::ClosedUnmodified(source)));
                Ok(*entry.insert(next_id))
            }
            Entry::Occupied(entry) => Ok(*entry.get()),
        }
    }

    /// Get a `CachedSource` by its id
    async fn get_cached_source(&self, id: SourceId) -> RwLockWriteGuard<CachedSource> {
        self.sources[id.0 as usize].write().await
    }

    /// Get a file, unless there was an error
    pub async fn get_source<'a>(&'a self, id: SourceId) -> FileResult<RwLockReadGuard<'a, Source>> {
        let mut cached_source = self.get_cached_source(id).await;
        cached_source.cache(id).await?;
        // Since the source was just cached, we should always be able to get it
        RwLockWriteGuard::try_downgrade_map(cached_source, |source| source.get_cached_source())
            .map_err(|_| unreachable!())
    }

    /// Get a file, unless there was an error
    pub async fn get_mut_source(&self, id: SourceId) -> FileResult<RwLockMappedWriteGuard<Source>> {
        let mut cached_source = self.get_cached_source(id).await;
        cached_source.cache(id).await?;
        // Since the source was just cached, we should always be able to get it
        RwLockWriteGuard::try_map(cached_source, |source| source.get_mut_cached_source())
            .map_err(|_| unreachable!())
    }

    /// Cache a file so it may not need to be loaded later
    pub async fn cache_source(&self, id: SourceId) -> FileResult<()> {
        self.get_cached_source(id).await.cache(id).await
    }

    /// Get a `Source` by its `uri`
    pub async fn get_source_by_uri(&self, uri: Url) -> FileResult<RwLockReadGuard<Source>> {
        let id = self.get_id(uri).await?;
        self.get_source(id).await
    }

    /// Get a `Source` by its `uri`
    pub async fn get_mut_source_by_uri(
        &self,
        uri: Url,
    ) -> FileResult<RwLockMappedWriteGuard<Source>> {
        let id = self.get_id(uri).await?;
        self.get_mut_source(id).await
    }

    /// Open a file
    pub async fn open(&self, uri: Url, text: String) -> RwLockReadGuard<Source> {
        let mut ids = self.ids.write().await;

        // We hold an exclusive write lock on `ids`, so it can't change until after the id has been
        // committed.
        // We take this now because `entry` borrows `ids` mutably later.
        let next_id = SourceId(ids.len() as u16);

        match ids.entry(uri.clone()) {
            Entry::Vacant(entry) => {
                let source = CachedSource::new_open(next_id, uri, text);
                self.sources.push(RwLock::new(source));
                entry.insert(next_id);
                self.get_source(next_id).await.unwrap()
            }
            Entry::Occupied(entry) => {
                let id = *entry.get();
                let mut source = self.get_cached_source(id).await;
                *source = CachedSource::new_open(id, uri, text);
                RwLockWriteGuard::downgrade_map(source, |source| {
                    source.get_cached_source().unwrap()
                })
            }
        }
    }

    pub async fn close(&self, uri: Url) {
        let Ok(id) = self.get_id(uri.clone()).await else { return; };
        let mut cached_source = self.get_cached_source(id).await;
        let old = mem::replace(&mut *cached_source, CachedSource::ClosedModified(uri));
        let Some(new) = old.take_cached_source() else { return; };
        *cached_source = CachedSource::ClosedUnmodified(new);
    }
}

impl Default for SourceManager {
    fn default() -> Self {
        Self {
            ids: Default::default(),
            sources: AppendOnlyVec::new(),
        }
    }
}
