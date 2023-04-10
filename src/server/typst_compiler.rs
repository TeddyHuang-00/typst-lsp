use comemo::Track;
use tokio::task::block_in_place;
use typst::doc::Document;
use typst::eval::{Module, Route, Tracer};
use typst::World;

use crate::lsp_typst_boundary::workaround::compile;
use crate::lsp_typst_boundary::{typst_to_lsp, LspDiagnostics};
use crate::workspace::source::Source;

use super::TypstServer;

impl TypstServer {
    pub async fn compile_source(&self, source: &Source) -> (Option<Document>, LspDiagnostics) {
        let world = self.workspace.get_world().await;
        let result = block_in_place(|| compile(&world, source.as_ref()));
        drop(world);

        let (document, errors) = match result {
            Ok(document) => (Some(document), Default::default()),
            Err(errors) => (Default::default(), errors),
        };

        let diagnostics = typst_to_lsp::source_errors_to_diagnostics(
            errors.as_ref(),
            &self.workspace,
            self.get_const_config(),
        )
        .await;

        // Garbage collect incremental cache. This evicts all memoized results that haven't been
        // used in the last 30 compilations.
        comemo::evict(30);

        (document, diagnostics)
    }

    pub async fn eval_source(&self, source: &Source) -> (Option<Module>, LspDiagnostics) {
        let world = self.workspace.get_world().await;

        let result = block_in_place(|| {
            let route = Route::default();
            let mut tracer = Tracer::default();
            typst::eval::eval(
                (&world as &dyn World).track(),
                route.track(),
                tracer.track_mut(),
                source.as_ref(),
            )
        });

        let (module, errors) = match result {
            Ok(module) => (Some(module), Default::default()),
            Err(errors) => (Default::default(), errors),
        };

        let diagnostics = typst_to_lsp::source_errors_to_diagnostics(
            errors.as_ref(),
            &self.workspace,
            self.get_const_config(),
        )
        .await;

        // Garbage collect incremental cache. This evicts all memoized results that haven't been
        // used in the last 30 compilations.
        comemo::evict(30);

        (module, diagnostics)
    }
}
