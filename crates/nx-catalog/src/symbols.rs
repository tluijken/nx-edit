use std::path::Path;

use serde::Deserialize;

use crate::CatalogError;

/// One entry of `Helix.sym`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ModelSymbol {
    pub symbol: String,
    /// In the order the preset document stores the values.
    #[serde(default)]
    pub parameters: Vec<String>,
}

/// HX Edit's `Helix.sym`: the entry at index n is the model the device numbers n.
#[derive(Debug, Clone, Default)]
pub struct ModelSymbols {
    entries: Vec<ModelSymbol>,
}

impl ModelSymbols {
    pub fn parse(json: &[u8]) -> Result<Self, serde_json::Error> {
        Ok(Self { entries: serde_json::from_slice(json)? })
    }

    pub fn load(path: &Path) -> Result<Self, CatalogError> {
        let bytes = std::fs::read(path).map_err(|source| CatalogError::Io { path: path.to_owned(), source })?;
        Self::parse(&bytes).map_err(|source| CatalogError::Json { path: path.to_owned(), source })
    }

    pub fn get(&self, model: u32) -> Option<&ModelSymbol> {
        self.entries.get(model as usize)
    }

    pub fn number(&self, symbol: &str) -> Option<u32> {
        self.entries.iter().position(|e| e.symbol == symbol).map(|i| i as u32)
    }

    /// The number of a catalog model (a shared id such as `HD2_DistKinkyBoost`), in its mono or
    /// stereo variant where it has both. Some models exist in one variant only, such as the legacy
    /// reverbs, which are all stereo; a mono `.hlx` block of those gets that one.
    pub fn number_for(&self, id: &str, stereo: bool) -> Option<u32> {
        let (wanted, other) = if stereo { ("Stereo", "Mono") } else { ("Mono", "Stereo") };
        self.number(&format!("{id}{wanted}")).or_else(|| self.number(id)).or_else(|| self.number(&format!("{id}{other}")))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The id `.models` files and `.hlx` presets use: `Helix.sym` names mono and stereo variants
/// separately (`HD2_ReverbHallStereo`), the others share one id (`HD2_ReverbHall`).
pub fn shared_id(symbol: &str) -> &str {
    symbol.strip_suffix("Mono").or_else(|| symbol.strip_suffix("Stereo")).unwrap_or(symbol)
}
