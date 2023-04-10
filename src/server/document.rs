use tower_lsp::lsp_types::TextDocumentContentChangeEvent;

use crate::config::{Config, ExportPdfMode};
use crate::lsp_typst_boundary::LspRange;
use crate::workspace::source::Source;

use super::TypstServer;

impl TypstServer {
    /// Apply a single change event to a document
    pub fn apply_single_document_change(
        &self,
        source: &mut Source,
        change: TextDocumentContentChangeEvent,
    ) {
        let replacement = change.text;

        match change.range {
            Some(range) => {
                let range = LspRange::new(range, self.get_const_config().position_encoding);
                source.edit(&range, &replacement);
            }
            None => source.replace(replacement),
        }
    }

    pub async fn on_source_changed(&self, config: &Config, source: &Source) {
        match config.export_pdf {
            ExportPdfMode::OnType => self.run_diagnostics_and_export(source).await,
            _ => self.run_diagnostics(source).await,
        }
    }

    pub async fn run_export(&self, source: &Source) {
        let (document, _) = self.compile_source(source).await;

        if let Some(document) = document {
            self.export_pdf(source, &document).await;
        }
    }

    pub async fn run_diagnostics_and_export(&self, source: &Source) {
        let (document, diagnostics) = self.compile_source(source).await;

        self.update_all_diagnostics(diagnostics).await;
        if let Some(document) = document {
            self.export_pdf(source, &document).await;
        }
    }

    pub async fn run_diagnostics(&self, source: &Source) {
        let (_, diagnostics) = self.eval_source(source).await;

        self.update_all_diagnostics(diagnostics).await;
    }
}
