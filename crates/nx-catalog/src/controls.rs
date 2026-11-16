use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::CatalogError;

#[derive(Deserialize)]
struct RawControl {
    #[serde(rename = "isDiscrete", default)]
    discrete: bool,
    #[serde(rename = "controlType", default)]
    control_type: Option<String>,
    #[serde(default)]
    format: Option<serde_json::Value>,
}

/// A discrete control of HX Edit: its options' names, and whether HX Edit shows them side by side
/// (`"controlType": "segmented"`) rather than as a list.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Control {
    pub options: Vec<String>,
    pub segmented: bool,
}

/// HX Edit's discrete controls by display type (`HelixControls.json`). A missing file gives none, so
/// a `res` folder imported before nx-edit read it still loads.
pub(crate) fn read_controls(path: &Path) -> Result<HashMap<String, Control>, CatalogError> {
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
            let options = c.format?.as_array()?.iter().map(|n| n.as_str().map(str::to_owned)).collect::<Option<Vec<_>>>()?;
            Some((kind, Control { options, segmented: c.control_type.as_deref() == Some("segmented") }))
        })
        .collect())
}
