//! Holds types relating to the LSP concept of a "workspace". That is, the directories a user has
//! open in their editor, the files in them, the files they're currently editing, and so on.

use comemo::Prehashed;
use tokio::sync::RwLock;
use tower_lsp::Client;
use typst::eval::Library;

use crate::lsp_typst_boundary::TypstSource;

use self::font_manager::FontManager;
use self::resource_manager::ResourceManager;
use self::source_manager::SourceManager;

pub mod font_manager;
pub mod resource;
pub mod resource_manager;
pub mod source;
pub mod source_manager;

pub struct Workspace {
    pub sources: RwLock<SourceManager>,
    pub resources: RwLock<ResourceManager>,

    pub client: Client,

    // Needed so that `Workspace` can implement Typst's `World` trait
    pub typst_stdlib: Prehashed<Library>,
    pub fonts: FontManager,
    pub detached_source: TypstSource,
}

impl Workspace {
    pub fn with_client(client: Client) -> Self {
        Self {
            sources: Default::default(),
            resources: Default::default(),
            client,
            typst_stdlib: Prehashed::new(typst_library::build()),
            fonts: FontManager::builder().with_system().with_embedded().build(),
            detached_source: TypstSource::detached(""),
        }
    }
}
