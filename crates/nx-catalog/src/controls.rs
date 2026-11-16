use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::CatalogError;

#[derive(Deserialize)]
struct RawControl {
    #[serde(rename = "isDiscrete", default)]
    discrete: bool,
    #[serde(default)]
    format: Option<serde_json::Value>,
}

/// The option names of HX Edit's discrete controls, by display type (`HelixControls.json`). A
/// missing file gives none, so a `res` folder imported before nx-edit read it still loads.
pub(crate) fn read_options(path: &Path) -> Result<HashMap<String, Vec<String>>, CatalogError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(source) => return Err(CatalogError::Io { path: path.to_owned(), source }),
    };
    let raw: HashMap<String, RawControl> =
        serde_json::from_slice(&bytes).map_err(|source| CatalogError::Json { path: path.to_owned(), source })?;
    Ok(raw
        .into_iter()
        .filter(|(_, c)| c.discrete)
        .filter_map(|(kind, c)| {
            let names = c.format?.as_array()?.iter().map(|n| n.as_str().map(str::to_owned)).collect::<Option<Vec<_>>>()?;
            Some((kind, names))
        })
        .collect())
}
