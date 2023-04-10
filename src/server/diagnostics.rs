use std::collections::HashMap;

use futures::future::join_all;
use tower_lsp::lsp_types::Url;

use crate::lsp_typst_boundary::LspDiagnostic;

use super::TypstServer;

impl TypstServer {
    pub async fn update_all_diagnostics(&self, mut diagnostics: HashMap<Url, Vec<LspDiagnostic>>) {
        let sources = self.workspace.sources.read().await;

        // Clear the previous diagnostics (could be done with the refresh notification when implemented by tower-lsp)
        for uri in sources.open_uris().await {
            diagnostics.entry(uri).or_insert_with(Vec::new);
        }

        let diagnostic_futures = diagnostics.into_iter().map(|(url, file_diagnostics)| {
            self.client.publish_diagnostics(url, file_diagnostics, None)
        });
        join_all(diagnostic_futures).await;
    }
}
