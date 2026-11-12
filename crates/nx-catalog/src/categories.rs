use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::CatalogError;

#[derive(Debug, Clone, PartialEq)]
pub struct Category {
    pub id: u32,
    pub name: String,
    pub color: u32,
    /// Models of the category and all its subcategories, in HX Edit's order.
    pub model_ids: Vec<String>,
}

#[derive(Deserialize)]
struct RawCatalog {
    categories: Vec<RawCategory>,
}

#[derive(Deserialize)]
struct RawCategory {
    id: u32,
    name: String,
    #[serde(default)]
    color: String,
    #[serde(default)]
    models: Vec<RawModel>,
    #[serde(default)]
    subcategories: Vec<RawSubcategory>,
}

#[derive(Deserialize)]
struct RawSubcategory {
    #[serde(default)]
    models: Vec<RawModel>,
}

#[derive(Deserialize)]
struct RawModel {
    id: String,
}

pub(crate) fn read_categories(path: &Path) -> Result<Vec<Category>, CatalogError> {
    let bytes = fs::read(path).map_err(|source| CatalogError::Io { path: path.to_owned(), source })?;
    let raw: RawCatalog =
        serde_json::from_slice(&bytes).map_err(|source| CatalogError::Json { path: path.to_owned(), source })?;
    Ok(raw
        .categories
        .into_iter()
        .map(|c| Category {
            id: c.id,
            color: u32::from_str_radix(c.color.trim_start_matches("0x"), 16).unwrap_or(0xffffff),
            model_ids: c
                .models
                .iter()
                .chain(c.subcategories.iter().flat_map(|s| &s.models))
                .map(|m| m.id.clone())
                .collect(),
            name: c.name,
        })
        .collect())
}
