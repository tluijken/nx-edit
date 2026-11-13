use serde_json::{Map, Value};

/// `data.device` of an HX Stomp preset.
pub const HX_STOMP_DEVICE: u32 = 2162694;

#[derive(Debug, thiserror::Error)]
pub enum HlxError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not a Line 6 preset: {0}")]
    Shape(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub key: String,
    pub model: String,
    pub path: u8,
    pub position: u8,
    pub enabled: bool,
    /// Model parameters in file order; `@`-prefixed metadata is left out.
    pub params: Vec<(String, Value)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SplitInfo {
    pub model: String,
    pub position: u8,
}

impl Preset {
    pub fn from_slice(bytes: &[u8]) -> Result<Self, HlxError> {
        let raw: Value = serde_json::from_slice(bytes)?;
        let preset = Self { raw };
        preset.dsp0().ok_or(HlxError::Shape("missing data.tone.dsp0"))?;
        Ok(preset)
    }

    pub fn to_vec_pretty(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(&self.raw).expect("a JSON value always serialises")
    }

    pub fn name(&self) -> &str {
        self.raw["data"]["meta"]["name"].as_str().unwrap_or_default()
    }

    pub fn device(&self) -> u32 {
        self.raw["data"]["device"].as_u64().unwrap_or_default() as u32
    }

    pub fn device_version(&self) -> u32 {
        self.raw["data"]["device_version"].as_u64().unwrap_or_default() as u32
    }

    pub fn blocks(&self) -> Vec<Block> {
        let Some(dsp0) = self.dsp0() else { return Vec::new() };
        let mut blocks: Vec<Block> = dsp0
            .iter()
            .filter(|(key, _)| key.starts_with("block"))
            .filter_map(|(key, value)| {
                let obj = value.as_object()?;
                Some(Block {
                    key: key.clone(),
                    model: obj.get("@model")?.as_str()?.to_owned(),
                    path: small(obj.get("@path")),
                    position: small(obj.get("@position")),
                    enabled: obj.get("@enabled").and_then(Value::as_bool).unwrap_or(true),
                    params: obj
                        .iter()
                        .filter(|(k, _)| !k.starts_with('@'))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                })
            })
            .collect();
        blocks.sort_by_key(|b| (b.position, b.path));
        blocks
    }

    pub fn split(&self) -> Option<SplitInfo> {
        self.junction("split")
    }

    pub fn join(&self) -> Option<SplitInfo> {
        self.junction("join")
    }

    fn junction(&self, key: &str) -> Option<SplitInfo> {
        let junction = self.dsp0()?.get(key)?.as_object()?;
        Some(SplitInfo {
            model: junction.get("@model")?.as_str()?.to_owned(),
            position: small(junction.get("@position")),
        })
    }

    pub fn snapshot_names(&self) -> Vec<String> {
        let Some(tone) = self.raw["data"]["tone"].as_object() else { return Vec::new() };
        (0..)
            .map_while(|i| tone.get(&format!("snapshot{i}")))
            .map(|s| s["@name"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    /// Index of the active snapshot (`@current_snapshot` in `data.tone.global`), 0 when absent.
    pub fn current_snapshot(&self) -> usize {
        self.raw["data"]["tone"]["global"]["@current_snapshot"].as_u64().unwrap_or_default() as usize
    }

    /// Replaces an existing model parameter of a block; false if the block or parameter is absent.
    /// A value the current snapshot controls is kept in that snapshot too.
    pub fn set_param(&mut self, block_key: &str, name: &str, value: Value) -> bool {
        if name.starts_with('@') {
            return false;
        }
        let slot = self.raw.pointer_mut("/data/tone/dsp0").and_then(|d| d.get_mut(block_key)).and_then(|b| b.get_mut(name));
        let Some(v) = slot else { return false };
        *v = value.clone();
        let snapshot = format!("/data/tone/snapshot{}/controllers/dsp0/{block_key}/{name}", self.current_snapshot());
        if let Some(controlled) = self.raw.pointer_mut(&snapshot).and_then(Value::as_object_mut) {
            controlled.insert("@value".into(), value);
        }
        true
    }

    pub fn set_name(&mut self, name: &str) {
        if let Some(meta) = self.raw.pointer_mut("/data/meta").and_then(Value::as_object_mut) {
            meta.insert("name".into(), Value::from(name));
        }
    }

    /// Switches a block on or off, in the current snapshot as well; false if there is no such block.
    pub fn set_enabled(&mut self, block_key: &str, enabled: bool) -> bool {
        let block = self.raw.pointer_mut("/data/tone/dsp0").and_then(|d| d.get_mut(block_key)).and_then(Value::as_object_mut);
        let Some(block) = block else { return false };
        block.insert("@enabled".into(), Value::Bool(enabled));
        let snapshot = format!("/data/tone/snapshot{}/blocks/dsp0", self.current_snapshot());
        if let Some(states) = self.raw.pointer_mut(&snapshot).and_then(Value::as_object_mut)
            && states.contains_key(block_key)
        {
            states.insert(block_key.into(), Value::Bool(enabled));
        }
        true
    }

    /// Makes snapshot `index` current and loads what it keeps: which blocks are on and the values
    /// it controls. False, and nothing changed, if the preset has no such snapshot.
    pub fn select_snapshot(&mut self, index: usize) -> bool {
        let Some(snapshot) = self.raw.pointer(&format!("/data/tone/snapshot{index}")).cloned() else { return false };
        self.set_current_snapshot(index);
        let Some(dsp0) = self.raw.pointer_mut("/data/tone/dsp0").and_then(Value::as_object_mut) else { return true };
        for (key, on) in snapshot.pointer("/blocks/dsp0").and_then(Value::as_object).into_iter().flatten() {
            if let (Some(block), Some(on)) = (dsp0.get_mut(key).and_then(Value::as_object_mut), on.as_bool()) {
                block.insert("@enabled".into(), Value::Bool(on));
            }
        }
        for (key, params) in snapshot.pointer("/controllers/dsp0").and_then(Value::as_object).into_iter().flatten() {
            let Some(block) = dsp0.get_mut(key).and_then(Value::as_object_mut) else { continue };
            for (name, controlled) in params.as_object().into_iter().flatten() {
                if let (Some(value), Some(slot)) = (controlled.get("@value"), block.get_mut(name)) {
                    *slot = value.clone();
                }
            }
        }
        true
    }

    pub fn set_current_snapshot(&mut self, index: usize) {
        if let Some(global) = self.raw.pointer_mut("/data/tone/global").and_then(Value::as_object_mut) {
            global.insert("@current_snapshot".into(), Value::from(index));
        }
    }

    fn dsp0(&self) -> Option<&Map<String, Value>> {
        self.raw.get("data")?.get("tone")?.get("dsp0")?.as_object()
    }
}

fn small(value: Option<&Value>) -> u8 {
    value.and_then(Value::as_u64).unwrap_or_default() as u8
}
