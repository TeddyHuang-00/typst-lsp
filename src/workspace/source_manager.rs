use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::mem;

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

    pub fn is_cached(&self) -> bool {
        matches!(self, Self::Open(_) | Self::ClosedUnmodified(_))
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

    /// Caches the file if it is not cached, unless there is an error
    pub async fn cache(&mut self, id: SourceId) -> FileResult<()> {
        if let Self::ClosedModified(uri) = self {
            let cached = Self::new_cached(id, uri.clone()).await?;
            mem::replace(self, cached);
        }
        Ok(())
    }
}

/// Owns, tracks, and caches Typst source files. Each file has state that affects how it can be
/// accessed. In general, using methods which make the most restrictive assumptions will be faster,
/// put less requirements on the caller, and leave a cleaner API.
///
/// Known/unknown:
/// - If a file is ever added to the `SourceManager`, it will know about that file from then on
/// - Otherwise, it is unknown
///
/// Open/closed for known files:
/// - If a known file is open in the editor, it is considered open.
/// - Otherwise, it is closed.
///
/// Cached/not cached for known files:
/// - If a known file is open, or is closed but unchanged since it was last cached or opened, it is
///     cached.
/// - Otherwise, it is not cached.
///
/// All files open in the editor are guaranteed to be open, which implies that they are known and
/// cached.
///
/// All files with a `SourceId` are guaranteed to be known.
#[derive(Debug, Default)]
pub struct SourceManager {
    ids: HashMap<Url, SourceId>,
    sources: Vec<CachedSource>,
}

impl SourceManager {
    /// Get a `CachedSource` by its id
    fn get_inner_source_by_id(&self, id: SourceId) -> &CachedSource {
        &self.sources[id.0 as usize]
    }

    /// Get a `CachedSource` by its id
    fn get_mut_inner_source_by_id(&mut self, id: SourceId) -> &mut CachedSource {
        &mut self.sources[id.0 as usize]
    }

    /// Iterator over the URIs files currently open in the editor
    pub fn open_uri_iter(&self) -> impl Iterator<Item = &Url> {
        self.ids
            .iter()
            .filter(|(_uri, id)| self.get_inner_source_by_id(**id).is_open())
            .map(|(uri, _id)| uri)
    }

    /// Get the id of a known file by its URI, returning `None` if no known file has the given URI
    fn get_id_by_uri(&self, uri: &Url) -> Option<SourceId> {
        self.ids.get(uri).copied()
    }

    /// Get the id of a known file by its URI, panicking if it is not known
    pub fn get_known_id_by_uri(&self, uri: &Url) -> SourceId {
        self.get_id_by_uri(uri).expect("file should be known")
    }

    /// Get the id of a file by its URI. If the file is known, its existing id is retrieved and its
    /// state remains unchanged. If the file is unknown, it is added and cached, possibly resulting
    /// in a `FileError`.
    pub async fn get_or_init_id_by_uri(&mut self, uri: Url) -> FileResult<SourceId> {
        match self.get_id_by_uri(&uri) {
            Some(id) => Ok(id),
            None => self.insert_closed(uri).await,
        }
    }

    /// Determine if a file is cached
    pub fn is_cached_by_id(&self, id: SourceId) -> bool {
        self.get_inner_source_by_id(id).is_cached()
    }

    /// Cache a file if it is not cached
    pub async fn cache_source_by_id(&mut self, id: SourceId) -> FileResult<()> {
        self.get_mut_inner_source_by_id(id).cache(id).await
    }

    /// Get the `Source` of a file if it is cached
    pub fn try_get_cached_source_by_id(&self, id: SourceId) -> Option<&Source> {
        self.get_inner_source_by_id(id).get_cached_source()
    }

    /// Get the `Source` of a file if it is cached
    fn try_get_mut_cached_source_by_id(&mut self, id: SourceId) -> Option<&mut Source> {
        self.get_mut_inner_source_by_id(id).get_mut_cached_source()
    }

    /// Get the `Source` of a cached file, panicking if it is not cached
    pub fn get_cached_source_by_id(&self, id: SourceId) -> &Source {
        self.try_get_cached_source_by_id(id)
            .expect("source should be cached")
    }

    /// Get the `Source` of a cached file, panicking if it is not cached
    pub fn get_mut_cached_source_by_id(&mut self, id: SourceId) -> &mut Source {
        self.try_get_mut_cached_source_by_id(id)
            .expect("source should be cached")
    }

    /// Get a `Source` by its `uri` if it exists in the cache
    pub fn get_source_by_uri(&self, uri: &Url) -> Option<&Source> {
        self.get_id_by_uri(uri)
            .map(|id| self.get_cached_source_by_id(id))
    }

    fn get_next_id(&self) -> SourceId {
        SourceId(self.sources.len() as u16)
    }

    /// Open a `Source`
    pub fn open(&mut self, uri: Url, text: String) -> SourceId {
        // Can't do this later, since `entry` borrows `&mut self`
        let next_id = self.get_next_id();

        match self.ids.entry(uri.clone()) {
            Entry::Occupied(entry) => {
                let existing_id = *entry.get();
                let source = Source::new(existing_id, uri, text);
                *self.get_mut_inner_source_by_id(existing_id) = CachedSource::Open(source);
                existing_id
            }
            Entry::Vacant(entry) => {
                entry.insert(next_id);
                let source = Source::new(next_id, uri, text);
                self.sources.push(CachedSource::Open(source));
                next_id
            }
        }
    }

    async fn insert_closed(&mut self, uri: Url) -> FileResult<SourceId> {
        let id = self.get_next_id();
        let source = Source::new_from_file(id, uri).await?;
        self.sources.push(CachedSource::ClosedUnmodified(source));

        Ok(id)
    }
}
