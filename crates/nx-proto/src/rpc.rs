//! RPC requests on the `0x1080` channel: `{102: txn, 100: opcode, 101: args}`. Key order follows
//! HX Edit so encoded requests match captured ones byte for byte. See docs/protocol.md.

use rmpv::Value;

/// Host endpoint id of the RPC channel.
pub const RPC_CHANNEL: u16 = 0x1080;

const TXN: u64 = 102;
const OPCODE: u64 = 100;
const ARGS: u64 = 101;

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    SelectPreset { setlist: u64, index: u64 },
    PresetDocument,
    PresetInfo,
    ReadGlobal { id: u64 },
    /// `block` counts slots from 1; `param` is the index in the model's parameter list.
    SetParam { block: u64, path: u64, param: u64, value: Value },
    /// Op 30 with `29: false`: `index` counts the values after the model's parameters, the first
    /// being a delay's or reverb's trails switch.
    SetExtra { block: u64, path: u64, index: u64, value: Value },
    SetBlockEnabled { block: u64, enabled: bool },
    /// Puts a model into an empty slot (op 39); the pedal answers with the whole slot, its values at
    /// the model's defaults. `cab` makes it an amp with a cab, by the cab's model number.
    InsertBlock { block: u64, model: u32, engine: u64, cab: Option<u32> },
    /// Gives a block another model (op 40); the pedal picks the engine and answers like op 39.
    ChangeModel { block: u64, model: u32, cab: Option<u32> },
    /// Empties a slot (op 28).
    RemoveBlock { block: u64 },
    /// Replaces the preset being edited with `document` (op 21); deferred, completed by notification
    /// 20 and followed by notification 21.
    WriteDocument { document: Vec<u8> },
    SelectBlock { block: u64, path: u64 },
    SelectSnapshot { index: u64 },
    /// A snapshot's footswitch LED colour (op 98), numbered as HX Edit's menu from Auto (0); the
    /// pedal confirms with notification 48.
    SetSnapshotColor { index: u64, color: u64 },
    /// Names a snapshot (op 89); the pedal confirms with notification 43 [seen, 25].
    RenameSnapshot { index: u64, name: String },
    /// Stores the preset being edited at a place under `name` (op 71); the pedal saves its own edit
    /// buffer, so only the place and the name travel.
    SavePreset { setlist: u64, index: u64, name: String },
    /// Librarian: stores `document` at a place under `name` (op 8), between [`Request::ControlBegin`]
    /// and [`Request::ControlStart`], as HX Edit imports a file [seen, 19]. The meaning of keys 123,
    /// 124 and 125 (false, false, 0) is unknown; the preset list holds the same three per place.
    WritePreset { setlist: u64, index: u64, name: String, document: Vec<u8> },
    /// Control channel, `{}`: HX Edit sends it before writing a place (op 8) and op 254 after.
    ControlBegin,
    /// Control channel, `{}`; HX Edit sends it before listing and the device answers nil. Its
    /// meaning is unknown.
    ControlStart,
    /// Control channel: the setlists, `[{index: name}]` (one, "PRESETS", on an HX Stomp).
    ListSetlists,
    /// Control channel: the presets of a setlist, `[{index: {109: name, 123, 124, 125}}]`. The
    /// `101: 2` inside the arguments is copied from HX Edit; its meaning is unknown.
    ListPresets { setlist: u64 },
    /// An opcode nx-edit does not model yet, kept verbatim.
    Other { opcode: u64, args: Value },
}

fn int(n: u64) -> Value {
    Value::from(n)
}

fn map(pairs: Vec<(u64, Value)>) -> Value {
    Value::Map(pairs.into_iter().map(|(k, v)| (int(k), v)).collect())
}

/// Op 30's arguments in HX Edit's key order; `29` says whether `28` counts the model's parameters.
fn set(block: u64, model_param: bool, path: u64, at: u64, value: &Value) -> Value {
    map(vec![(98, int(block)), (29, Value::Boolean(model_param)), (26, int(path)), (28, int(at)), (119, value.clone())])
}

/// A block's model as ops 39 and 40 carry it: `{23: amp with cab, 25: model, 26: cab or -1}`.
fn model_ref(model: u32, cab: Option<u32>) -> Value {
    let cab_value = cab.map_or(Value::from(-1), |c| int(u64::from(c)));
    map(vec![(23, Value::Boolean(cab.is_some())), (25, int(u64::from(model))), (26, cab_value)])
}

/// The model and cab of `{23, 25, 26}`, as ops 39 and 40 carry them.
fn parse_model_ref(v: &Value) -> Option<(u32, Option<u32>)> {
    let m = v.as_map()?;
    let model = u32::try_from(get(m, 25)?.as_u64()?).ok()?;
    let cab = if get(m, 23)?.as_bool()? { Some(u32::try_from(get(m, 26)?.as_u64()?).ok()?) } else { None };
    Some((model, cab))
}

fn get(m: &[(Value, Value)], key: u64) -> Option<&Value> {
    m.iter().find(|(k, _)| k.as_u64() == Some(key)).map(|(_, v)| v)
}

impl Request {
    pub fn opcode(&self) -> u64 {
        match self {
            Self::SelectPreset { .. } => 20,
            Self::PresetDocument => 22,
            Self::PresetInfo => 23,
            Self::ReadGlobal { .. } => 24,
            Self::SetParam { .. } | Self::SetExtra { .. } => 30,
            Self::SetBlockEnabled { .. } => 41,
            Self::InsertBlock { .. } => 39,
            Self::ChangeModel { .. } => 40,
            Self::RemoveBlock { .. } => 28,
            Self::WriteDocument { .. } => 21,
            Self::SelectBlock { .. } => 78,
            Self::SelectSnapshot { .. } => 88,
            Self::SetSnapshotColor { .. } => 98,
            Self::RenameSnapshot { .. } => 89,
            Self::SavePreset { .. } => 71,
            Self::WritePreset { .. } => 8,
            Self::ControlBegin => 255,
            Self::ControlStart => 254,
            Self::ListSetlists => 0,
            Self::ListPresets { .. } => 1,
            Self::Other { opcode, .. } => *opcode,
        }
    }

    fn args(&self) -> Value {
        match self {
            Self::SelectPreset { setlist, index } => map(vec![(107, int(*setlist)), (108, int(*index))]),
            Self::PresetDocument | Self::PresetInfo => Value::Nil,
            Self::ReadGlobal { id } => map(vec![(118, int(*id))]),
            Self::SetParam { block, path, param, value } => set(*block, true, *path, *param, value),
            Self::SetExtra { block, path, index, value } => set(*block, false, *path, *index, value),
            Self::SetBlockEnabled { block, enabled } => map(vec![(98, int(*block)), (59, Value::Boolean(*enabled))]),
            Self::InsertBlock { block, model, engine, cab } => {
                let body = map(vec![(24, model_ref(*model, *cab)), (9, int(*engine)), (10, Value::Boolean(true))]);
                map(vec![(98, int(*block)), (99, map(vec![(19, int(6)), (20, body)]))])
            }
            Self::ChangeModel { block, model, cab } => map(vec![(98, int(*block)), (100, model_ref(*model, *cab))]),
            Self::RemoveBlock { block } => map(vec![(98, int(*block))]),
            Self::WriteDocument { document } => map(vec![(110, raw_str(document))]),
            Self::WritePreset { setlist, index, name, document } => {
                let mut pairs = place(*setlist, *index, name);
                pairs.push((110, raw_str(document)));
                map(pairs)
            }
            Self::SelectBlock { block, path } => map(vec![(98, int(*block)), (26, int(*path))]),
            Self::SelectSnapshot { index } => map(vec![(92, int(*index))]),
            Self::SetSnapshotColor { index, color } => map(vec![(92, int(*index)), (66, int(*color))]),
            Self::RenameSnapshot { index, name } => map(vec![(92, int(*index)), (109, Value::from(format!("{name}\0")))]),
            Self::SavePreset { setlist, index, name } => {
                map(vec![(107, int(*setlist)), (108, int(*index)), (109, Value::from(format!("{name}\0")))])
            }
            Self::ControlStart | Self::ControlBegin => map(Vec::new()),
            Self::ListSetlists => Value::Nil,
            Self::ListPresets { setlist } => map(vec![(107, int(*setlist)), (101, int(2))]),
            Self::Other { args, .. } => args.clone(),
        }
    }

    pub fn to_value(&self, txn: u64) -> Value {
        map(vec![(TXN, int(txn)), (OPCODE, int(self.opcode())), (ARGS, self.args())])
    }

    pub fn encode(&self, txn: u64) -> Vec<u8> {
        match self {
            Self::WriteDocument { document } => return encode_write(txn, 21, Vec::new(), document),
            Self::WritePreset { setlist, index, name, document } => {
                return encode_write(txn, 8, place(*setlist, *index, name), document);
            }
            _ => {}
        }
        let mut out = Vec::new();
        rmpv::encode::write_value(&mut out, &self.to_value(txn)).expect("writing to a Vec cannot fail");
        out
    }

    /// Parses a captured request into its transaction id and request; `None` if `value` is not one.
    pub fn from_value(value: &Value) -> Option<(u64, Request)> {
        let m = value.as_map()?;
        let txn = get(m, TXN)?.as_u64()?;
        let opcode = get(m, OPCODE)?.as_u64()?;
        let args = get(m, ARGS)?.clone();
        let a = args.as_map().map(Vec::as_slice).unwrap_or_default();
        let u = |k| get(a, k).and_then(Value::as_u64);
        let parsed = match opcode {
            20 => Some(Self::SelectPreset { setlist: u(107)?, index: u(108)? }),
            22 if args.is_nil() => Some(Self::PresetDocument),
            23 if args.is_nil() => Some(Self::PresetInfo),
            24 => Some(Self::ReadGlobal { id: u(118)? }),
            30 => {
                let (block, path, at, value) = (u(98)?, u(26)?, u(28)?, get(a, 119)?.clone());
                Some(match get(a, 29)?.as_bool()? {
                    true => Self::SetParam { block, path, param: at, value },
                    false => Self::SetExtra { block, path, index: at, value },
                })
            }
            39 => {
                let slot = get(get(a, 99)?.as_map()?, 20)?.as_map()?.to_vec();
                let (model, cab) = parse_model_ref(get(&slot, 24)?)?;
                Some(Self::InsertBlock { block: u(98)?, model, engine: get(&slot, 9)?.as_u64()?, cab })
            }
            40 => {
                let (model, cab) = parse_model_ref(get(a, 100)?)?;
                Some(Self::ChangeModel { block: u(98)?, model, cab })
            }
            28 => Some(Self::RemoveBlock { block: u(98)? }),
            21 => match get(a, 110)? {
                Value::String(s) => Some(Self::WriteDocument { document: s.as_bytes().to_vec() }),
                Value::Binary(b) => Some(Self::WriteDocument { document: b.clone() }),
                _ => None,
            },
            8 => Some(Self::WritePreset {
                setlist: u(107)?,
                index: u(108)?,
                name: get(a, 109)?.as_str()?.strip_suffix('\0')?.to_owned(),
                document: match get(a, 110)? {
                    Value::String(s) => s.as_bytes().to_vec(),
                    Value::Binary(b) => b.clone(),
                    _ => return None,
                },
            }),
            41 => Some(Self::SetBlockEnabled { block: u(98)?, enabled: get(a, 59)?.as_bool()? }),
            78 => Some(Self::SelectBlock { block: u(98)?, path: u(26)? }),
            88 => Some(Self::SelectSnapshot { index: u(92)? }),
            98 => Some(Self::SetSnapshotColor { index: u(92)?, color: u(66)? }),
            89 => Some(Self::RenameSnapshot { index: u(92)?, name: get(a, 109)?.as_str()?.strip_suffix('\0')?.to_owned() }),
            71 => Some(Self::SavePreset {
                setlist: u(107)?,
                index: u(108)?,
                name: get(a, 109)?.as_str()?.strip_suffix('\0')?.to_owned(),
            }),
            254 => Some(Self::ControlStart),
            255 => Some(Self::ControlBegin),
            0 if args.is_nil() => Some(Self::ListSetlists),
            1 => Some(Self::ListPresets { setlist: u(107)? }),
            _ => None,
        }
        // A known opcode whose arguments differ from the shape above is kept verbatim, so it still
        // round-trips instead of being re-encoded wrongly.
        .filter(|r| r.args() == args)
        .unwrap_or(Self::Other { opcode, args });
        Some((txn, parsed))
    }
}

/// A place's name and the three values after it, as the preset list and op 8 carry them.
fn place(setlist: u64, index: u64, name: &str) -> Vec<(u64, Value)> {
    vec![
        (107, int(setlist)),
        (108, int(index)),
        (109, Value::from(format!("{name}\0"))),
        (123, Value::Boolean(false)),
        (124, Value::Boolean(false)),
        (125, int(0)),
    ]
}

/// Op 21 or op 8 as HX Edit writes it: `{102: txn, 100: opcode, 101: {..., 110: document}}` with the
/// document as a str, although it is not UTF-8, which rmpv would only write as bin.
fn encode_write(txn: u64, opcode: u64, head: Vec<(u64, Value)>, document: &[u8]) -> Vec<u8> {
    let write = |out: &mut Vec<u8>, v: &Value| rmpv::encode::write_value(out, v).expect("writing to a Vec cannot fail");
    let mut out = vec![0x83];
    for v in [int(TXN), int(txn), int(OPCODE), int(opcode), int(ARGS)] {
        write(&mut out, &v);
    }
    out.push(0x80 | (head.len() as u8 + 1));
    for (k, v) in &head {
        write(&mut out, &int(*k));
        write(&mut out, v);
    }
    out.push(110);
    out.extend(str_head(document.len()));
    out.extend_from_slice(document);
    out
}

/// The document as rmpv reads it off the wire: a str that is not UTF-8. rmpv builds such a value
/// only by reading it (and writes it as bin), so `encode_write` puts it on the wire instead.
fn raw_str(document: &[u8]) -> Value {
    let wire = encode_write(0, 21, Vec::new(), document);
    let at = wire.len() - document.len() - str_head(document.len()).len();
    rmpv::decode::read_value(&mut &wire[at..]).expect("a str written by encode_write")
}

fn str_head(len: usize) -> Vec<u8> {
    match len {
        n if n < 0x100 => vec![0xd9, n as u8],
        n if n < 0x1_0000 => [vec![0xda], (n as u16).to_be_bytes().to_vec()].concat(),
        n => [vec![0xdb], (n as u32).to_be_bytes().to_vec()].concat(),
    }
}
