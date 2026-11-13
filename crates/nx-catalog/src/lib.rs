//! HX Edit's model definitions (`*.models`), category tree (`HX_ModelCatalog.json`) and model numbering
//! (`Helix.sym`), read from a local copy of HX Edit's `res` directory. The data is Line 6's and is never
//! bundled with nx-edit.

mod categories;
mod import;
mod models;
mod symbols;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub use categories::{Category, CategoryGroup};
pub use import::{default_res_dir, import_hxedit, import_installer};
pub use models::{ModelDef, ParamDef};
pub use symbols::{ModelSymbol, ModelSymbols, shared_id};

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{path}: {source}")]
    Json { path: PathBuf, source: serde_json::Error },
    #[error("extracting the HX Edit installer failed: {0}")]
    Extract(String),
    #[error("{path} is not HX Edit's res folder: {missing} is missing")]
    NotHxEditRes { path: PathBuf, missing: &'static str },
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    models: HashMap<String, ModelDef>,
    categories: Vec<Category>,
}

impl Catalog {
    pub fn load(res_dir: &Path) -> Result<Self, CatalogError> {
        let mut models = HashMap::new();
        for path in models::model_files(res_dir)? {
            for model in models::read_models(&path)? {
                models.insert(model.id.clone(), model);
            }
        }
        let categories = categories::read_categories(&res_dir.join("HX_ModelCatalog.json"))?;
        Ok(Self { models, categories })
    }

    pub fn model(&self, id: &str) -> Option<&ModelDef> {
        self.models.get(id)
    }

    pub fn categories(&self) -> &[Category] {
        &self.categories
    }

    pub fn len(&self) -> usize {
        self.models.len()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }
}
