//! The preset document: the result of RPC opcode 22, as captured from an HX Stomp on firmware 3.71.
//! See docs/protocol.md, "Preset document".
//!
//! ```text
//! str    "l6-helix\0"
//! str16  section table: 12 u32 LE byte offsets
//! map    the preset, keyed by small integers
//! ```
//!
//! [`Document`] keeps the whole preset as a lossless MessagePack tree, so unknown fields survive and
//! an unedited document encodes back to the captured bytes. [`Preset`] is a typed, read-only view of
//! the parts that are understood.

mod tree;

pub use tree::{IntForm, Item, LenForm};

use tree::Reader;

const MAGIC: &[u8] = b"l6-helix\0";

/// Top-level keys in section-table order; the table starts with the map's own offset and ends with
/// the total length, twice.
const SECTION_KEYS: [i64; 9] = [0, 1, 3, 4, 2, 5, 6, 7, 10];
const TABLE_WORDS: usize = 12;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DocError {
    #[error("not a preset document (no \"l6-helix\" magic)")]
    Magic,
    #[error("document truncated at offset {offset}")]
    Truncated { offset: usize },
    #[error("unsupported {what} at offset {offset}")]
    Unsupported { offset: usize, what: &'static str },
    #[error("section table of {0} bytes; only the 48-byte form has been seen")]
    Table(usize),
    #[error("section table {read:?} does not match the document, which places its sections at {computed:?}")]
    TableMismatch { read: Vec<u32>, computed: Vec<u32> },
    #[error("{0} bytes after the preset map")]
    Trailing(usize),
    #[error("unexpected document shape: {0}")]
    Shape(String),
    #[error("invalid edit: {0}")]
    Value(String),
}

fn shape(what: impl Into<String>) -> DocError {
    DocError::Shape(what.into())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    magic_form: LenForm,
    table_form: LenForm,
    tone: Item,
}

impl Document {
    /// Reads a document and checks that its section table is the one [`Document::encode`] would
    /// write, so that encoding an unedited document can never quietly change the table.
    pub fn decode(bytes: &[u8]) -> Result<Self, DocError> {
        let mut r = Reader::new(bytes);
        let magic_form = match r.item() {
            Ok(Item::Str(s, form)) if s == MAGIC => form,
            _ => return Err(DocError::Magic),
        };
        let Item::Str(table, table_form) = r.item()? else { return Err(shape("section table is not a string")) };
        if table.len() != TABLE_WORDS * 4 {
            return Err(DocError::Table(table.len()));
        }
        let read: Vec<u32> = table.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let tone = r.item()?;
        if tone.as_map().is_none() {
            return Err(shape("preset is not a map"));
        }
        if !r.at_end() {
            return Err(DocError::Trailing(bytes.len() - r.pos));
        }
        let doc = Self { magic_form, table_form, tone };
        let computed = doc.layout()?.1;
        if computed != read {
            return Err(DocError::TableMismatch { read, computed });
        }
        Ok(doc)
    }

    /// Writes the document, with the section table recomputed from where the sections land.
    pub fn encode(&self) -> Result<Vec<u8>, DocError> {
        Ok(self.layout()?.0)
    }

    /// The section table [`Document::encode`] writes: offsets of the map and of top-level keys
    /// 0 1 3 4 2 5 6 7 10, then the total length twice. For a document that was not edited it is the
    /// table that was read.
    pub fn sections(&self) -> Result<Vec<u32>, DocError> {
        Ok(self.layout()?.1)
    }

    fn layout(&self) -> Result<(Vec<u8>, Vec<u32>), DocError> {
        let mut out = Vec::new();
        Item::Str(MAGIC.to_vec(), self.magic_form).write(&mut out);
        Item::Str(vec![0; TABLE_WORDS * 4], self.table_form).write(&mut out);
        let table_at = out.len() - TABLE_WORDS * 4;
        let Item::Map(entries, form) = &self.tone else { return Err(shape("preset is not a map")) };
        let map_at = out.len();
        tree::write_map_head(&mut out, entries.len(), *form);
        let mut key_at = Vec::new();
        for (k, v) in entries {
            key_at.push((k.as_int(), out.len()));
            k.write(&mut out);
            v.write(&mut out);
        }
        let mut words = vec![map_at as u32];
        for key in SECTION_KEYS {
            let at =
                key_at.iter().find(|(k, _)| *k == Some(key)).ok_or_else(|| shape(format!("no top-level key {key}")))?;
            words.push(at.1 as u32);
        }
        words.extend([out.len() as u32; 2]);
        for (i, w) in words.iter().enumerate() {
            out[table_at + i * 4..table_at + i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        Ok((out, words))
    }

    /// The whole preset map, including everything [`Preset`] does not interpret.
    pub fn tone(&self) -> &Item {
        &self.tone
    }

    pub fn preset(&self) -> Result<Preset, DocError> {
        Preset::from_tone(&self.tone)
    }

    /// Sets value `index` of the block, input, output, split or join in DSP 0 slot `slot`.
    pub fn set_param(&mut self, slot: usize, index: usize, value: ParamValue) -> Result<(), DocError> {
        let (kind, body) = self.slot_body_mut(slot)?;
        let values = match kind {
            KIND_BLOCK => body.get_mut(11),
            KIND_INPUT | KIND_OUTPUT => body.get_mut(7),
            KIND_SPLIT => body.get_mut(15).and_then(|j| j.get_mut(7)),
            KIND_JOIN => body.get_mut(17).and_then(|j| j.get_mut(7)),
            _ => None,
        };
        let target = values
            .and_then(|v| v.get_mut(4))
            .and_then(|v| v.index_mut(index))
            .ok_or_else(|| shape(format!("slot {slot} has no value {index}")))?;
        *target = value.convert_to(target).ok_or_else(|| {
            DocError::Value(format!("slot {slot} value {index} is {target:?} and cannot hold {value:?}"))
        })?;
        Ok(())
    }

    /// Turns the block, split or join in DSP 0 slot `slot` on or off in the current state.
    pub fn set_enabled(&mut self, slot: usize, enabled: bool) -> Result<(), DocError> {
        let (kind, body) = self.slot_body_mut(slot)?;
        let flag = match kind {
            KIND_BLOCK => body.get_mut(10),
            KIND_SPLIT => body.get_mut(15).and_then(|j| j.get_mut(10)),
            KIND_JOIN => body.get_mut(17).and_then(|j| j.get_mut(10)),
            _ => None,
        };
        *flag.ok_or_else(|| shape(format!("slot {slot} cannot be enabled")))? = Item::Bool(enabled);
        Ok(())
    }

    pub fn set_cursor(&mut self, slot: usize, path: usize) -> Result<(), DocError> {
        let cursor = self.tone.get_mut(6).ok_or_else(|| shape("no cursor (key 6)"))?;
        set_int(cursor, 98, slot as i64)?;
        set_int(cursor, 26, path as i64)
    }

    pub fn set_current_snapshot(&mut self, index: usize) -> Result<(), DocError> {
        let snaps = self.tone.get_mut(10).ok_or_else(|| shape("no snapshots (key 10)"))?;
        set_int(snaps, 6, index as i64)
    }

    pub fn set_snapshot_name(&mut self, index: usize, name: &str) -> Result<(), DocError> {
        let snap = self
            .tone
            .get_mut(10)
            .and_then(|s| s.get_mut(10))
            .and_then(|l| l.index_mut(index))
            .ok_or_else(|| shape(format!("no snapshot {index}")))?;
        let slot = snap.get_mut(4).ok_or_else(|| shape("snapshot without a name"))?;
        let form = match slot {
            Item::Str(_, form) => *form,
            _ => LenForm::Fix,
        };
        let mut bytes = name.as_bytes().to_vec();
        bytes.push(0);
        *slot = Item::Str(bytes, form);
        Ok(())
    }

    fn slot_body_mut(&mut self, slot: usize) -> Result<(i64, &mut Item), DocError> {
        let entry = self
            .tone
            .get_mut(0)
            .and_then(|d| d.get_mut(22))
            .and_then(|s| s.index_mut(slot))
            .ok_or_else(|| shape(format!("no slot {slot}")))?;
        let kind = entry.get(19).and_then(Item::as_int).ok_or_else(|| shape(format!("slot {slot} has no kind")))?;
        let body = entry.get_mut(20).ok_or_else(|| shape(format!("slot {slot} has no body")))?;
        Ok((kind, body))
    }
}

fn set_int(map: &mut Item, key: i64, value: i64) -> Result<(), DocError> {
    let slot = map.get_mut(key).ok_or_else(|| shape(format!("no key {key}")))?;
    *slot = match slot {
        Item::Int(_, form) => Item::Int(value, *form),
        _ => Item::int(value),
    };
    Ok(())
}

const KIND_INPUT: i64 = 0;
const KIND_OUTPUT: i64 = 1;
const KIND_SPLIT: i64 = 2;
const KIND_JOIN: i64 = 3;
const KIND_BLOCK: i64 = 6;
const KIND_EMPTY: i64 = 8;

/// A parameter value. Floats are in the model's own units (Drive 0..1, a frequency in Hz).
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    Float(f32),
    Int(i64),
    Bool(bool),
    Other(Item),
}

impl ParamValue {
    fn from_item(item: &Item) -> Self {
        match item {
            Item::F32(f) => Self::Float(*f),
            Item::Int(i, _) => Self::Int(*i),
            Item::Bool(b) => Self::Bool(*b),
            other => Self::Other(other.clone()),
        }
    }

    /// This value in the type and encoding of `stored`, or `None` if that type cannot hold it
    /// exactly. The device's own documents decide a value's type; an edit must not change it.
    fn convert_to(&self, stored: &Item) -> Option<Item> {
        let exact_int = |f: f64| (f.fract() == 0.0 && f >= i64::MIN as f64 && f < i64::MAX as f64).then_some(f as i64);
        match (stored, self) {
            (Item::F32(_), Self::Float(f)) => Some(Item::F32(*f)),
            (Item::F32(_), Self::Int(i)) => (*i != i64::MAX && (*i as f32) as i64 == *i).then_some(Item::F32(*i as f32)),
            (Item::F64(_), Self::Float(f)) => Some(Item::F64(f64::from(*f))),
            (Item::F64(_), Self::Int(i)) => (*i != i64::MAX && (*i as f64) as i64 == *i).then_some(Item::F64(*i as f64)),
            (Item::Int(_, form), Self::Int(i)) => tree::fits(*i, *form).then_some(Item::Int(*i, *form)),
            (Item::Int(_, form), Self::Float(f)) => {
                exact_int(f64::from(*f)).filter(|i| tree::fits(*i, *form)).map(|i| Item::Int(i, *form))
            }
            (Item::Bool(_), Self::Bool(b)) => Some(Item::Bool(*b)),
            (Item::Int(..) | Item::F32(_) | Item::F64(_) | Item::Bool(_), _) => None,
            (stored, Self::Other(item)) => {
                (std::mem::discriminant(stored) == std::mem::discriminant(item)).then(|| item.clone())
            }
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f as f64),
            Self::Int(i) => Some(*i as f64),
            Self::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Self::Other(_) => None,
        }
    }
}

/// `{2: count, 3: params, 4: [values]}`. `params` is one less than `count` on delays and reverbs,
/// whose last value is `@trails`.
#[derive(Debug, Clone, PartialEq)]
pub struct Values {
    pub count: i64,
    pub params: i64,
    pub values: Vec<ParamValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Firmware {
    pub service: String,
    /// BCD, `0x03710000` for 3.71.
    pub version: u32,
    pub build: String,
}

/// An input or output (key 5 or 6 is the routing, the hlx `@input` / `@output`).
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    pub route: i64,
    pub values: Values,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Junction {
    pub model: u32,
    /// The path-0 slot the junction sits in front of.
    pub before_slot: i64,
    pub enabled: bool,
    pub values: Values,
    /// The second path's input (split) or output (join).
    pub endpoint: Endpoint,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Index into HX Edit's `Helix.sym`.
    pub model: u32,
    /// Key 9; amp 17, cab 31, delay and reverb 8, send 25, others 1 in our captures.
    pub engine: i64,
    /// On or off in the current state, which follows the active snapshot.
    pub enabled: bool,
    pub values: Values,
    /// Key 12, empty in every captured block.
    pub extra: Values,
    /// Key 24 as read: `{23: false, 25: model, 26: -1}`; 23 and 26 are not understood.
    pub header: Item,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SlotKind {
    Input(Endpoint),
    Output(Endpoint),
    Split(Junction),
    Join(Junction),
    Block(Block),
    Empty,
    Unknown { kind: i64, body: Item },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dsp {
    pub slots: Vec<SlotKind>,
}

/// A block with where it sits: `path` 0 runs between input and output, path 1 between split and
/// join; `position` counts from the first slot after the input (path 0) or after the split (path 1),
/// which matches the hlx `@path` and `@position` of Revv DM.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed<'a> {
    pub dsp: usize,
    pub slot: usize,
    pub path: usize,
    pub position: usize,
    pub block: &'a Block,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    /// Key 0; snapshots refer to the assignment by this number.
    pub id: i64,
    /// 9 is the snapshot controller, as in the hlx.
    pub controller: i64,
    pub slot: i64,
    /// Index into the model's parameter list.
    pub param: i64,
    pub min: ParamValue,
    pub max: ParamValue,
    /// Key 1 as read, including the fields not understood (1, 4, 6.28, 6.41, 7, 13).
    pub raw: Item,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlotState {
    /// First element, false everywhere in our captures.
    pub flag: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControllerValue {
    pub assignment: i64,
    /// First element, false in our captures; the hlx has `@fs_enabled: false` for these.
    pub flag: bool,
    pub value: ParamValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub valid: bool,
    pub name: String,
    pub custom_name: bool,
    pub tempo: f32,
    pub led_color: i64,
    /// One per slot, in slot order.
    pub slots: Vec<SlotState>,
    /// The used entries of the 64-entry controller table.
    pub controllers: Vec<ControllerValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshots {
    pub current: i64,
    pub list: Vec<Snapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cursor {
    pub slot: i64,
    pub path: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub firmware: Firmware,
    /// Keys 0 and 1; an HX Stomp leaves the second DSP nil.
    pub dsps: Vec<Option<Dsp>>,
    pub controllers: Vec<Assignment>,
    pub snapshots: Snapshots,
    /// The block selected in the editor (key 6).
    pub cursor: Cursor,
    pub tempo: Option<f32>,
}

/// An unused entry in a snapshot's controller table names this assignment.
const NO_ASSIGNMENT: i64 = 64;

fn field<'a>(item: &'a Item, key: i64, what: &str) -> Result<&'a Item, DocError> {
    item.get(key).ok_or_else(|| shape(format!("{what}: no key {key}")))
}

fn int_at(item: &Item, key: i64, what: &str) -> Result<i64, DocError> {
    field(item, key, what)?.as_int().ok_or_else(|| shape(format!("{what}.{key} is not an integer")))
}

fn bool_at(item: &Item, key: i64, what: &str) -> Result<bool, DocError> {
    field(item, key, what)?.as_bool().ok_or_else(|| shape(format!("{what}.{key} is not a boolean")))
}

fn f32_at(item: &Item, key: i64, what: &str) -> Result<f32, DocError> {
    match field(item, key, what)? {
        Item::F32(f) => Ok(*f),
        _ => Err(shape(format!("{what}.{key} is not a float"))),
    }
}

fn array<'a>(item: &'a Item, what: &str) -> Result<&'a [Item], DocError> {
    item.as_array().ok_or_else(|| shape(format!("{what} is not an array")))
}

fn text(item: &Item, what: &str) -> Result<String, DocError> {
    let bytes = item.as_str_bytes().ok_or_else(|| shape(format!("{what} is not a string")))?;
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

fn values(item: &Item, what: &str) -> Result<Values, DocError> {
    Ok(Values {
        count: int_at(item, 2, what)?,
        params: int_at(item, 3, what)?,
        values: array(field(item, 4, what)?, what)?.iter().map(ParamValue::from_item).collect(),
    })
}

fn endpoint(body: &Item, what: &str) -> Result<Endpoint, DocError> {
    let route = body.get(5).or_else(|| body.get(6)).and_then(Item::as_int);
    Ok(Endpoint {
        route: route.ok_or_else(|| shape(format!("{what}: no route")))?,
        values: values(field(body, 7, what)?, what)?,
    })
}

fn junction(body: &Item, endpoint_key: i64, inner_key: i64, what: &str) -> Result<Junction, DocError> {
    let inner = field(body, inner_key, what)?;
    Ok(Junction {
        model: int_at(inner, 8, what)? as u32,
        before_slot: int_at(inner, 13, what)?,
        enabled: bool_at(inner, 10, what)?,
        values: values(field(inner, 7, what)?, what)?,
        endpoint: endpoint(field(body, endpoint_key, what)?, what)?,
    })
}

fn slot(item: &Item, index: usize) -> Result<SlotKind, DocError> {
    let what = format!("slot {index}");
    let kind = int_at(item, 19, &what)?;
    let body = field(item, 20, &what)?;
    Ok(match kind {
        KIND_INPUT => SlotKind::Input(endpoint(body, &what)?),
        KIND_OUTPUT => SlotKind::Output(endpoint(body, &what)?),
        KIND_SPLIT => SlotKind::Split(junction(body, 14, 15, &what)?),
        KIND_JOIN => SlotKind::Join(junction(body, 16, 17, &what)?),
        KIND_BLOCK => {
            let header = field(body, 24, &what)?;
            SlotKind::Block(Block {
                model: int_at(header, 25, &what)? as u32,
                engine: int_at(body, 9, &what)?,
                enabled: bool_at(body, 10, &what)?,
                values: values(field(body, 11, &what)?, &what)?,
                extra: values(field(body, 12, &what)?, &what)?,
                header: header.clone(),
            })
        }
        KIND_EMPTY => SlotKind::Empty,
        kind => SlotKind::Unknown { kind, body: body.clone() },
    })
}

fn assignment(item: &Item) -> Result<Assignment, DocError> {
    let what = "controller assignment";
    let raw = field(item, 1, what)?;
    Ok(Assignment {
        id: int_at(item, 0, what)?,
        controller: int_at(raw, 0, what)?,
        slot: int_at(raw, 5, what)?,
        param: int_at(field(raw, 6, what)?, 29, what)?,
        min: ParamValue::from_item(field(raw, 2, what)?),
        max: ParamValue::from_item(field(raw, 3, what)?),
        raw: raw.clone(),
    })
}

fn snapshot(item: &Item, index: usize) -> Result<Snapshot, DocError> {
    let what = format!("snapshot {index}");
    let slots = array(field(item, 3, &what)?, &what)?
        .iter()
        .map(|s| match s.as_array() {
            Some([Item::Bool(flag), Item::Bool(enabled)]) => Ok(SlotState { flag: *flag, enabled: *enabled }),
            _ => Err(shape(format!("{what}: slot state is not [bool, bool]"))),
        })
        .collect::<Result<_, _>>()?;
    let mut controllers = Vec::new();
    for entry in array(field(item, 2, &what)?, &what)? {
        let Some([Item::Bool(flag), assignment, value]) = entry.as_array() else {
            return Err(shape(format!("{what}: controller value is not [bool, int, value]")));
        };
        let assignment = assignment.as_int().ok_or_else(|| shape(format!("{what}: assignment is not an integer")))?;
        if assignment != NO_ASSIGNMENT {
            controllers.push(ControllerValue { assignment, flag: *flag, value: ParamValue::from_item(value) });
        }
    }
    Ok(Snapshot {
        valid: bool_at(item, 0, &what)?,
        name: text(field(item, 4, &what)?, &what)?,
        custom_name: bool_at(item, 14, &what)?,
        tempo: f32_at(item, 5, &what)?,
        led_color: int_at(item, 12, &what)?,
        slots,
        controllers,
    })
}

impl Preset {
    fn from_tone(tone: &Item) -> Result<Self, DocError> {
        let header = field(tone, 7, "preset")?;
        let firmware = Firmware {
            service: text(field(header, 36, "header")?, "service")?,
            version: int_at(header, 35, "header")? as u32,
            build: text(field(header, 37, "header")?, "build")?,
        };
        let mut dsps = Vec::new();
        for key in [0, 1] {
            dsps.push(match field(tone, key, "preset")? {
                Item::Nil => None,
                dsp => {
                    let slots = array(field(dsp, 22, "dsp")?, "dsp slots")?;
                    Some(Dsp { slots: slots.iter().enumerate().map(|(i, s)| slot(s, i)).collect::<Result<_, _>>()? })
                }
            });
        }
        let mut controllers = Vec::new();
        for entry in array(field(tone, 4, "preset")?, "controllers")? {
            if let Some(list) = entry.as_array() {
                controllers.extend(list.iter().map(assignment).collect::<Result<Vec<_>, _>>()?);
            }
        }
        let snaps = field(tone, 10, "preset")?;
        let list = array(field(snaps, 10, "snapshots")?, "snapshots")?;
        let snapshots = Snapshots {
            current: int_at(snaps, 6, "snapshots")?,
            list: list.iter().enumerate().map(|(i, s)| snapshot(s, i)).collect::<Result<_, _>>()?,
        };
        let cursor_item = field(tone, 6, "preset")?;
        let cursor = Cursor { slot: int_at(cursor_item, 98, "cursor")?, path: int_at(cursor_item, 26, "cursor")? };
        let tempo = match tone.get(5).and_then(|g| g.get(16)) {
            Some(Item::F32(t)) => Some(*t),
            _ => None,
        };
        Ok(Self { firmware, dsps, controllers, snapshots, cursor, tempo })
    }

    /// Every block with its path and position. Blocks outside both input→output and split→join
    /// ranges are left out rather than given a guessed place.
    pub fn blocks(&self) -> impl Iterator<Item = Placed<'_>> {
        self.dsps.iter().enumerate().filter_map(|(d, dsp)| dsp.as_ref().map(|dsp| (d, dsp))).flat_map(|(d, dsp)| {
            let find = |f: fn(&SlotKind) -> bool| dsp.slots.iter().position(f);
            let input = find(|s| matches!(s, SlotKind::Input(_)));
            let output = find(|s| matches!(s, SlotKind::Output(_)));
            let split = find(|s| matches!(s, SlotKind::Split(_)));
            let join = find(|s| matches!(s, SlotKind::Join(_)));
            let ranges = [(0, input, output), (1, split, join)];
            dsp.slots.iter().enumerate().filter_map(move |(slot, kind)| {
                let SlotKind::Block(block) = kind else { return None };
                ranges.iter().find_map(|&(path, start, end)| {
                    let (start, end) = (start?, end?);
                    (start < slot && slot < end).then_some(Placed {
                        dsp: d,
                        slot,
                        path,
                        position: slot - start - 1,
                        block,
                    })
                })
            })
        })
    }
}
