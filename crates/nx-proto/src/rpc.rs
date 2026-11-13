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
    SetBlockEnabled { block: u64, enabled: bool },
    SelectBlock { block: u64, path: u64 },
    SelectSnapshot { index: u64 },
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
            Self::SetParam { .. } => 30,
            Self::SetBlockEnabled { .. } => 41,
            Self::SelectBlock { .. } => 78,
            Self::SelectSnapshot { .. } => 88,
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
            // 29 is always true in HX Edit's captures; its meaning is unknown.
            Self::SetParam { block, path, param, value } => map(vec![
                (98, int(*block)),
                (29, Value::Boolean(true)),
                (26, int(*path)),
                (28, int(*param)),
                (119, value.clone()),
            ]),
            Self::SetBlockEnabled { block, enabled } => map(vec![(98, int(*block)), (59, Value::Boolean(*enabled))]),
            Self::SelectBlock { block, path } => map(vec![(98, int(*block)), (26, int(*path))]),
            Self::SelectSnapshot { index } => map(vec![(92, int(*index))]),
            Self::ControlStart => map(Vec::new()),
            Self::ListSetlists => Value::Nil,
            Self::ListPresets { setlist } => map(vec![(107, int(*setlist)), (101, int(2))]),
            Self::Other { args, .. } => args.clone(),
        }
    }

    pub fn to_value(&self, txn: u64) -> Value {
        map(vec![(TXN, int(txn)), (OPCODE, int(self.opcode())), (ARGS, self.args())])
    }

    pub fn encode(&self, txn: u64) -> Vec<u8> {
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
            30 => Some(Self::SetParam { block: u(98)?, path: u(26)?, param: u(28)?, value: get(a, 119)?.clone() }),
            41 => Some(Self::SetBlockEnabled { block: u(98)?, enabled: get(a, 59)?.as_bool()? }),
            78 => Some(Self::SelectBlock { block: u(98)?, path: u(26)? }),
            88 => Some(Self::SelectSnapshot { index: u(92)? }),
            254 => Some(Self::ControlStart),
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
