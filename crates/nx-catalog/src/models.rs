use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer};

use crate::CatalogError;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ModelDef {
    #[serde(rename = "symbolicID")]
    pub id: String,
    pub name: Option<String>,
    pub category: Option<u32>,
    /// DSP cost as HX Edit accounts it.
    pub load: Option<f32>,
    #[serde(default)]
    pub params: Vec<ParamDef>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ParamDef {
    #[serde(rename = "symbolicID")]
    pub id: String,
    pub name: String,
    #[serde(rename = "valueType")]
    pub value_type: u8,
    #[serde(rename = "displayType")]
    pub display_type: Option<String>,
    /// Switch parameters store their range as booleans; those become 0.0 and 1.0. Text parameters
    /// (`value_type` 3) have no range.
    #[serde(deserialize_with = "range_bound")]
    pub min: Option<f64>,
    #[serde(deserialize_with = "range_bound")]
    pub max: Option<f64>,
    pub default: serde_json::Value,
}

pub(crate) fn model_files(res_dir: &Path) -> Result<Vec<PathBuf>, CatalogError> {
    let io = |source| CatalogError::Io { path: res_dir.to_owned(), source };
    let mut files: Vec<PathBuf> = fs::read_dir(res_dir)
        .map_err(io)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "models"))
        .collect();
    files.sort();
    Ok(files)
}

pub(crate) fn read_models(path: &Path) -> Result<Vec<ModelDef>, CatalogError> {
    let bytes = fs::read(path).map_err(|source| CatalogError::Io { path: path.to_owned(), source })?;
    serde_json::from_slice(&bytes).map_err(|source| CatalogError::Json { path: path.to_owned(), source })
}

fn range_bound<'de, D: Deserializer<'de>>(de: D) -> Result<Option<f64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Number(f64),
        Bool(bool),
        Text(#[allow(dead_code)] String),
    }
    Ok(match Raw::deserialize(de)? {
        Raw::Number(n) => Some(n),
        Raw::Bool(b) => Some(f64::from(u8::from(b))),
        Raw::Text(_) => None,
    })
}
