use tokio::fs::read_to_string;
use tower_lsp::lsp_types::Url;
use typst::diag::{FileError, FileResult};

use crate::lsp_typst_boundary::{lsp_to_typst, LspRange, TypstSource};

use super::source_manager::SourceId;

/// Typst source file
#[derive(Debug)]
pub struct Source {
    uri: Url,
    inner: TypstSource,
}

impl Source {
    pub fn new(id: SourceId, uri: Url, text: String) -> Self {
        let typst_path = lsp_to_typst::uri_to_path(&uri);

        Self {
            uri,
            inner: TypstSource::new(id.into(), &typst_path, text),
        }
    }

    pub async fn new_from_file(id: SourceId, uri: Url) -> FileResult<Self> {
        // TODO: choose better `FileError`s based on the actual errors
        let path = uri.to_file_path().map_err(|_| FileError::Other)?;
        let text = read_to_string(path).await.map_err(|_| FileError::Other)?;
        Ok(Self::new(id, uri, text))
    }

    pub fn edit(&mut self, replace: &LspRange, with: &str) {
        let typst_replace = lsp_to_typst::range(replace, self);
        self.inner.edit(typst_replace, with);
    }

    pub fn replace(&mut self, text: String) {
        self.inner.replace(text);
    }
}

impl AsRef<TypstSource> for Source {
    fn as_ref(&self) -> &TypstSource {
        &self.inner
    }
}
