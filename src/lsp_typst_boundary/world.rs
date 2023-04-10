use std::ops::Deref;
use std::sync::Arc;

use comemo::Prehashed;
use tokio::runtime::Handle;
use tokio::sync::{OwnedRwLockWriteGuard, RwLockReadGuard};
use tower_lsp::lsp_types::MessageType;
use typst::diag::FileResult;
use typst::eval::Library;
use typst::font::{Font, FontBook};
use typst::util::Buffer;
use typst::World;

use crate::workspace::source::Source;
use crate::workspace::source_manager::SourceManager;
use crate::workspace::Workspace;

use super::{typst_to_lsp, TypstPath, TypstSource, TypstSourceId};

pub struct WorkspaceWorld {
    workspace: Arc<Workspace>,
    sources: OwnedRwLockWriteGuard<SourceManager>,
}

impl WorkspaceWorld {
    pub fn new(workspace: Arc<Workspace>, sources: OwnedRwLockWriteGuard<SourceManager>) -> Self {
        Self { workspace, sources }
    }
}

impl World for WorkspaceWorld {
    fn library(&self) -> &Prehashed<Library> {
        &self.workspace.typst_stdlib
    }

    fn main(&self) -> &TypstSource {
        // The best `main` file depends on what the LSP is doing. For example, when providing
        // diagnostics, the file for which diagnostics are being produced is the best choice of
        // `main`. However, that means `main` needs to change between invocations of Typst
        // functions, but stay constant across each of them. This is very hard to do with the
        // `'static` requirement from `comemo`.
        //
        // The most obvious way would to store the current `main` in `Workspace`, setting it each
        // time we call a Typst function and using a synchronization object to maintain it. However,
        // this becomes difficult, and leads to storing state local to a single function call within
        // global `Workspace` state, which is a bad idea.
        //
        // Ideally, we would instead implement `World` for something like `(&Workspace, SourceId)`,
        // so that each caller who wants to use `Workspace` as a `World` must declare what `main`
        // should be via a `SourceId`. However, the `'static` requirement prevents this, and
        // `(Workspace, SourceId)` or even `(Rc<Workspace>, SourceId)` would increase complexity
        // substantially.
        //
        // So in order of theoretical niceness, the best solutions are:
        // - Relax the `'static` requirement from `comemo` (if that is even possible)
        // - Fork `typst` just to remove `main`, leading to tons of extra work
        // - Disallow calling `main` on `Workspace`
        //
        // To be clear, this is also a bad idea. However, at time of writing, `main` seems to be
        // called in only two places in the `typst` library (`compile` and `analyze_expr`), both of
        // which can be worked around as needed. Assuming this holds true into the future,
        // invocations of `main` should be easy to catch and avoid during development, so this is
        // good enough.
        panic!("should not invoke `World`'s `main` on a `Workspace` because there is no reasonable default context")
    }

    fn resolve(&self, typst_path: &TypstPath) -> FileResult<TypstSourceId> {
        let lsp_uri = typst_to_lsp::path_to_uri(typst_path).unwrap();

        Handle::current()
            .block_on(async {
                match self.sources.get_id(lsp_uri).await {
                    // Try caching the file here, because `source` doesn't allow us to return errors
                    Ok(id) => self.sources.cache_source(id).await.map(|()| id),
                    Err(error) => Err(error),
                }
            })
            .map(Into::into)
    }

    fn source<'a>(&'a self, typst_id: TypstSourceId) -> &'a TypstSource {
        let id = typst_id.into();

        Handle::current().block_on(async {
            match self.sources.get_source(id).await {
                Ok(source) => {
                    let a: RwLockReadGuard<'a, _> = source;
                    let b: &'a Source = a.deref();
                    // let c: &'a TypstSource = b.as_ref();
                    // c
                    b.as_ref()
                }
                Err(error) => {
                    // We cache in `resolve` to try avoiding this, since we can't return errors here
                    self.workspace.client.log_message(
                        MessageType::ERROR,
                        format!("unable to get source id {typst_id:?} because an error occurred: {error}")
                    ).await;
                    &self.workspace.detached_source
                }
            }
        })
    }

    fn book(&self) -> &Prehashed<FontBook> {
        self.workspace.fonts.book()
    }

    fn font(&self, id: usize) -> Option<Font> {
        let mut resources = self.workspace.resources.blocking_write();
        self.workspace.fonts.font(id, &mut resources)
    }

    fn file(&self, typst_path: &TypstPath) -> FileResult<Buffer> {
        let lsp_uri = typst_to_lsp::path_to_uri(typst_path).unwrap();
        let mut resources = self.workspace.resources.blocking_write();
        let lsp_resource = resources.get_or_insert_resource(lsp_uri)?;
        Ok(lsp_resource.into())
    }
}
