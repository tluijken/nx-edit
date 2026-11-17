//! An `.hlx` preset as the document HX Edit writes to a place with librarian op 8 [seen, 19]. The
//! pedal normalises that form when it stores the preset; see docs/protocol.md, "A document as HX Edit
//! writes it", for what differs from the form it reads back.

use nx_catalog::{Catalog, ModelSymbols, shared_id};
use nx_proto::{Document, Firmware, IntForm, Item, LenForm};
use serde_json::{Map, Value};

const SLOTS: usize = 20;
const PATH_POSITIONS: usize = 8;
const OUTPUT_SLOT: usize = 9;
const SPLIT_SLOT: usize = 10;
const JOIN_SLOT: usize = 19;
const CONTROLLERS: usize = 10;
const CONTROLLER_VALUES: usize = 64;
const NO_ASSIGNMENT: i64 = 64;

const KIND_INPUT: i64 = 0;
const KIND_OUTPUT: i64 = 1;
const KIND_SPLIT: i64 = 2;
const KIND_JOIN: i64 = 3;
const KIND_BLOCK: i64 = 6;
const KIND_LOOPER: i64 = 7;
const KIND_EMPTY: i64 = 8;
const LOOPER_CATEGORY: u32 = 15;

/// Builds the document for `hlx`. `firmware` is the pedal's own (key 7 of a document it sent), which
/// HX Edit copies except for the build, where it writes the one the file names.
pub fn hlx_document(hlx: &nx_hlx::Preset, syms: &ModelSymbols, catalog: &Catalog, firmware: &Firmware) -> Result<Vec<u8>, String> {
    let tone = hlx.tone();
    let dsp0 = tone["dsp0"].as_object().ok_or("the preset has no dsp0")?;
    let model = Models { syms, catalog };

    let mut slots: Vec<Item> = (0..SLOTS).map(|_| slot(KIND_EMPTY, Item::Nil)).collect();
    let mut placed: Vec<(&str, usize, Vec<String>)> = Vec::new();
    for (key, block) in dsp0.iter().filter(|(k, _)| k.starts_with("block")) {
        let (at, body, kind, params) = model.block(key, block)?;
        if slots[at] != slot(KIND_EMPTY, Item::Nil) {
            return Err(format!("{key} shares its place with another block"));
        }
        slots[at] = slot(kind, body);
        placed.push((key, at, params));
    }
    slots[0] = slot(KIND_INPUT, model.endpoint(&dsp0["inputA"], "@input", 5, "inputA")?);
    slots[OUTPUT_SLOT] = slot(KIND_OUTPUT, model.endpoint(&dsp0["outputA"], "@output", 6, "outputA")?);
    slots[SPLIT_SLOT] = slot(KIND_SPLIT, model.junction(dsp0, "split", "inputB", "@input", [14, 15, 5])?);
    slots[JOIN_SLOT] = slot(KIND_JOIN, model.junction(dsp0, "join", "outputB", "@output", [16, 17, 6])?);
    let slot_of = |key: &str| placed.iter().find(|(k, ..)| *k == key).map(|(_, at, params)| (*at, params));

    // HX Edit numbers assignments in the order of its own maps, by block and then parameter name
    // [seen, 19: Level 0, MidFreq 1, MidGain 2].
    let mut assigned: Vec<(&str, &str, &Map<String, Value>)> = Vec::new();
    for (key, params) in object(&tone["controller"]["dsp0"]) {
        for (name, assignment) in object(params) {
            assigned.push((key, name, assignment.as_object().ok_or_else(|| format!("controller of {key}.{name}"))?));
        }
    }
    assigned.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    let mut controllers: Vec<Vec<Item>> = vec![Vec::new(); CONTROLLERS];
    for (id, (key, name, assignment)) in assigned.iter().enumerate() {
        let (at, params) = slot_of(key).ok_or_else(|| format!("a controller moves {key}, which is not in the preset"))?;
        let index = params.iter().position(|p| p == name).ok_or_else(|| format!("a controller moves {key}.{name}, which nx-edit cannot assign yet"))?;
        let controller = assignment.get("@controller").and_then(Value::as_i64).ok_or_else(|| format!("{key}.{name} has no @controller"))?;
        let list = usize::try_from(controller).ok().and_then(|c| controllers.get_mut(c)).ok_or_else(|| format!("{key}.{name}: controller {controller}"))?;
        let at = at as i64;
        list.push(map(vec![
            (0, Item::int(id as i64)),
            (
                1,
                map(vec![
                    (0, Item::int(controller)),
                    (1, Item::int(at)),
                    (2, value(&assignment["@min"])?),
                    (3, value(&assignment["@max"])?),
                    (4, Item::int(0)),
                    (5, Item::int(at)),
                    (6, map(vec![(28, Item::int(0)), (29, Item::int(index as i64)), (41, Item::Bool(false))])),
                    (7, Item::int(0)),
                    (13, Item::Bool(false)),
                ]),
            ),
        ]));
    }
    let controllers = controllers.into_iter().map(|list| if list.is_empty() { Item::Nil } else { array(list) }).collect();

    let global = &tone["global"];
    let mut snapshots = Vec::new();
    for snapshot in (0..).map_while(|i| tone.get(format!("snapshot{i}"))) {
        let mut states: Vec<Item> = (0..SLOTS)
            .map(|at| {
                // What HX Edit writes for the slots that are not blocks [seen, 19].
                let on = !matches!(at, 0 | OUTPUT_SLOT | JOIN_SLOT);
                array(vec![Item::Nil, Item::Bool(on)])
            })
            .collect();
        for (key, at, _) in &placed {
            let on = snapshot["blocks"]["dsp0"][key].as_bool().or_else(|| dsp0[*key]["@enabled"].as_bool()).unwrap_or(true);
            states[*at] = array(vec![Item::Nil, Item::Bool(on)]);
        }
        let mut values: Vec<Item> =
            (0..CONTROLLER_VALUES).map(|_| array(vec![Item::Bool(false), Item::int(NO_ASSIGNMENT), Item::Nil])).collect();
        for (id, (key, name, _)) in assigned.iter().enumerate() {
            let held = &snapshot["controllers"]["dsp0"][key][name];
            if held.is_null() {
                continue;
            }
            let entry = values.get_mut(id).ok_or("more controller assignments than a snapshot holds")?;
            let fs = held["@fs_enabled"].as_bool().unwrap_or(false);
            *entry = array(vec![Item::Bool(fs), Item::int(id as i64), value(&held["@value"])?]);
        }
        snapshots.push(map(vec![
            (0, Item::Bool(snapshot["@valid"].as_bool().unwrap_or(true))),
            (1, array((0..13).map(|_| array(vec![Item::int(13), Item::Bool(false), zeros(7, IntForm::Fix)])).collect())),
            (2, array(values)),
            (3, array(states)),
            (4, text(snapshot["@name"].as_str().unwrap_or_default())),
            (5, Item::F32(snapshot["@tempo"].as_f64().or_else(|| global["@tempo"].as_f64()).unwrap_or(120.0) as f32)),
            (12, Item::int(snapshot["@ledcolor"].as_i64().unwrap_or(0))),
            (14, Item::Bool(snapshot["@custom_name"].as_bool().unwrap_or(false))),
        ]));
    }

    let cursor_path = global["@cursor_path"].as_i64().unwrap_or(0);
    let cursor_position = global["@cursor_position"].as_i64().unwrap_or(0);
    let cursor_slot = if cursor_path == 1 { SPLIT_SLOT as i64 + 1 + cursor_position } else { 1 + cursor_position };
    let variax = &tone["variax"];
    let int_of = |v: &Value| Item::int(v.as_i64().unwrap_or(0));
    let float_of = |v: &Value, default: f64| Item::F32(v.as_f64().unwrap_or(default) as f32);
    let bool_of = |v: &Value| Item::Bool(v.as_bool().unwrap_or(false));
    let mut globals = vec![
        (16, float_of(&global["@tempo"], 120.0)),
        (45, int_of(&variax["@variax_model"])),
        (46, float_of(&variax["@variax_toneknob"], -0.1)),
        (47, float_of(&variax["@variax_volumeknob"], -0.1)),
        (48, int_of(&variax["@variax_lockctrls"])),
        (49, bool_of(&variax["@variax_customtuning"])),
    ];
    globals.extend((1..=6).map(|s| (49 + s, int_of(&variax[format!("@variax_str{s}tuning")]))));
    globals.push((56, Item::Bool(variax["@variax_magmode"].as_bool().unwrap_or(true))));
    // Values HX Edit adds whose meaning is unknown; the pedal drops 111-116, 380, 381, 252 and 253.
    globals.push((30, Item::int(0)));
    globals.extend((111..=116).map(|k| (k, Item::F32(1.0))));
    globals.extend([(380, Item::int(2)), (381, Item::int(2)), (252, Item::int(0)), (253, Item::int(0)), (134, Item::int(0))]);

    let build = hlx.build_sha().map_or_else(|| firmware.build.clone(), str::to_owned);
    let tone = map(vec![
        (7, map(vec![(36, text(&firmware.service)), (35, Item::Int(i64::from(firmware.version), IntForm::U32)), (37, text(&build))])),
        (0, map(vec![(21, Item::int(0)), (22, array(slots))])),
        (1, Item::Nil),
        (3, map(vec![(7, int_of(&global["@pedalstate"])), (8, array((0..5).map(|_| Item::Nil).collect()))])),
        (4, array(controllers)),
        (2, map(vec![(0, zeros(13, IntForm::I16)), (1, array((0..13).map(|_| zeros(7, IntForm::U8)).collect()))])),
        (5, map(globals)),
        (6, map(vec![(98, Item::int(cursor_slot)), (26, Item::int(cursor_path))])),
        (
            10,
            map(vec![
                (6, Item::int(global["@current_snapshot"].as_i64().unwrap_or(0))),
                (7, Item::int(0)),
                (8, Item::int(snapshots.len() as i64)),
                (9, Item::int(SLOTS as i64)),
                (10, array(snapshots)),
                (13, array((0..SLOTS).map(|_| Item::Bool(true)).collect())),
            ]),
        ),
    ]);
    Document::new(tone).encode().map_err(|e| e.to_string())
}

struct Models<'a> {
    syms: &'a ModelSymbols,
    catalog: &'a Catalog,
}

impl Models<'_> {
    /// A model's name as HX Edit shows it, or its id when the catalog does not know it.
    fn name(&self, id: &str) -> String {
        self.catalog.model(id).and_then(|m| m.name.clone()).unwrap_or_else(|| id.to_owned())
    }

    fn number(&self, id: &str, stereo: bool, what: &str) -> Result<u32, String> {
        self.syms.number_for(id, stereo).ok_or_else(|| format!("{what}: {id} is not in HX Edit's Helix.sym"))
    }

    /// A block's slot, body, slot kind and the parameter names its values follow.
    fn block(&self, key: &str, block: &Value) -> Result<(usize, Item, i64, Vec<String>), String> {
        let id = block["@model"].as_str().ok_or_else(|| format!("{key} has no @model"))?;
        let name = self.name(id);
        if block.get("@cab").is_some() {
            return Err(format!(
                "{name} with its cab cannot be sent to the pedal yet. In HX Edit, give it a cab block of its own, or pick another preset."
            ));
        }
        if id.contains("DualCab") || id.contains("CabMicIrDual") {
            return Err(format!("{name} is a dual cab, which nx-edit cannot send to the pedal yet."));
        }
        let position = usize::try_from(block["@position"].as_u64().unwrap_or(0)).unwrap_or(usize::MAX);
        let path = block["@path"].as_u64().unwrap_or(0);
        if position >= PATH_POSITIONS || path > 1 {
            return Err(format!("{name} sits outside the HX Stomp's chain (path {path}, position {position})."));
        }
        let at = if path == 1 { SPLIT_SLOT + 1 + position } else { 1 + position };
        let number = self.number(id, block["@stereo"].as_bool().unwrap_or(false), key)?;
        let symbol = &self.syms.get(number).expect("a number Helix.sym gave").symbol;
        let def = self.catalog.model(shared_id(symbol));
        let category = def.and_then(|d| d.category);
        let engine = nx_catalog::engine(category, symbol)
            .ok_or_else(|| format!("{name} is a kind of block nx-edit cannot put on the pedal yet."))?;
        // Helix.sym lists more than the document holds for some models (a cab's IrData), so the
        // values end with the last parameter the file has, as HX Edit's do [seen, 19].
        let mut params = self.params(number);
        let last = params.iter().rposition(|p| block.get(p.as_str()).is_some_and(|v| !v.is_null()));
        params.truncate(last.map_or(0, |l| l + 1));
        let mut values = Vec::new();
        for name in &params {
            let def = def.and_then(|d| d.params.iter().find(|p| &p.id == name));
            let given = block.get(name.as_str()).filter(|v| !v.is_null());
            let v = given.or(def.map(|p| &p.default)).ok_or_else(|| format!("{key} ({id}) has no {name}"))?;
            values.push(param(v, def.map(|p| p.value_type))?);
        }
        let count = values.len() as i64;
        if let Some(on) = block.get("@trails").and_then(Value::as_bool) {
            values.push(Item::Bool(on));
        }
        let held = self::values(values, count);
        let enabled = Item::Bool(block["@enabled"].as_bool().unwrap_or(true));
        let engine = Item::int(engine as i64);
        if category == Some(LOOPER_CATEGORY) {
            let body = map(vec![(8, Item::int(i64::from(number))), (9, engine), (10, enabled), (7, held)]);
            return Ok((at, body, KIND_LOOPER, params));
        }
        let header = map(vec![(23, Item::Bool(false)), (25, Item::int(i64::from(number))), (26, Item::int(-1))]);
        let body = map(vec![(24, header), (9, engine), (10, enabled), (11, held), (12, self::values(Vec::new(), 0))]);
        Ok((at, body, KIND_BLOCK, params))
    }

    fn params(&self, number: u32) -> Vec<String> {
        self.syms.get(number).map(|s| s.parameters.clone()).unwrap_or_default()
    }

    /// An input or output: its routing and the values the file holds, in the model's order. HX Edit
    /// stops at the first parameter the file leaves out (`select`, `gain`) [seen, 19].
    fn endpoint(&self, endpoint: &Value, route_key: &str, key: i64, what: &str) -> Result<Item, String> {
        let id = endpoint["@model"].as_str().ok_or_else(|| format!("{what} has no @model"))?;
        let params = self.params(self.number(id, false, what)?);
        let values = params.iter().map_while(|p| endpoint.get(p.as_str())).map(|v| param(v, None)).collect::<Result<Vec<_>, _>>()?;
        let count = values.len() as i64;
        Ok(map(vec![(key, Item::int(endpoint[route_key].as_i64().unwrap_or(0))), (7, self::values(values, count))]))
    }

    /// A split or join, with path B's input or output beside it. HX Edit leaves that endpoint's
    /// values out and places the junction in front of the slot after its `@position` [seen, 19].
    fn junction(&self, dsp0: &Map<String, Value>, key: &str, end: &str, route_key: &str, keys: [i64; 3]) -> Result<Item, String> {
        let junction = dsp0.get(key).ok_or_else(|| format!("the preset has no {key}"))?;
        let id = junction["@model"].as_str().ok_or_else(|| format!("{key} has no @model"))?;
        let number = self.number(id, false, key)?;
        let values = self.params(number).iter().map_while(|p| junction.get(p.as_str())).map(|v| param(v, None)).collect::<Result<Vec<_>, _>>()?;
        let count = values.len() as i64;
        let route = dsp0.get(end).and_then(|e| e[route_key].as_i64()).unwrap_or(0);
        Ok(map(vec![
            (keys[0], map(vec![(keys[2], Item::int(route)), (7, self::values(Vec::new(), 0))])),
            (
                keys[1],
                map(vec![
                    (8, Item::int(i64::from(number))),
                    (13, Item::int(junction["@position"].as_i64().unwrap_or(0) + 1)),
                    (10, Item::Bool(junction["@enabled"].as_bool().unwrap_or(true))),
                    (7, self::values(values, count)),
                ]),
            ),
            (18, Item::Bool(false)),
        ]))
    }
}

fn slot(kind: i64, body: Item) -> Item {
    map(vec![(19, Item::int(kind)), (20, body)])
}

/// `{2: count, 3: params, 4: values}`: a delay's or reverb's trails follows its parameters.
fn values(values: Vec<Item>, params: i64) -> Item {
    map(vec![(2, Item::int(values.len() as i64)), (3, Item::int(params)), (4, array(values))])
}

fn map(entries: Vec<(i64, Item)>) -> Item {
    Item::Map(entries.into_iter().map(|(k, v)| (Item::int(k), v)).collect(), LenForm::Fix)
}

fn array(items: Vec<Item>) -> Item {
    Item::Array(items, LenForm::Fix)
}

fn zeros(n: usize, form: IntForm) -> Item {
    array((0..n).map(|_| Item::Int(0, form)).collect())
}

fn text(s: &str) -> Item {
    Item::Str(format!("{s}\0").into_bytes(), LenForm::Fix)
}

fn object(v: &Value) -> impl Iterator<Item = (&str, &Value)> {
    v.as_object().into_iter().flatten().map(|(k, v)| (k.as_str(), v))
}

/// A number or switch as the document stores it: floats as f32.
fn value(v: &Value) -> Result<Item, String> {
    param(v, None)
}

/// `value_type` is the catalog's (0 int, 1 float, 2 switch); it turns a float parameter the file
/// wrote as a whole number into a float.
fn param(v: &Value, value_type: Option<u8>) -> Result<Item, String> {
    Ok(match v {
        Value::Bool(b) => Item::Bool(*b),
        Value::Number(n) if n.is_f64() || value_type == Some(1) => Item::F32(n.as_f64().unwrap_or_default() as f32),
        Value::Number(n) => Item::int(n.as_i64().ok_or_else(|| format!("{n} does not fit a parameter"))?),
        other => return Err(format!("{other} is not a parameter value")),
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn res() -> Option<(ModelSymbols, Catalog)> {
        let dir = std::env::var_os("NX_HXEDIT_RES").map(PathBuf::from).unwrap_or_else(nx_catalog::default_res_dir);
        let Ok(syms) = ModelSymbols::load(&dir.join("Helix.sym")) else {
            eprintln!("skipped: no Helix.sym in {} (set NX_HXEDIT_RES)", dir.display());
            return None;
        };
        Some((syms, Catalog::load(&dir).unwrap()))
    }

    fn hlx(name: &str) -> nx_hlx::Preset {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nx-hlx/tests/fixtures").join(format!("{name}.hlx"));
        nx_hlx::Preset::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn firmware() -> Firmware {
        Firmware { service: "P33".into(), version: 0x0371_0000, build: "pedal".into() }
    }

    #[test]
    fn revv_home_is_written_as_hx_edit_wrote_it_in_capture_19() {
        let Some((syms, catalog)) = res() else { return };
        let want = include_bytes!("../../nx-proto/tests/fixtures/19-import-to-03B.doc");
        let got = hlx_document(&hlx("Revv_Home"), &syms, &catalog, &firmware()).unwrap();
        if got != want {
            let at = got.iter().zip(want.iter()).position(|(a, b)| a != b).unwrap_or(got.len().min(want.len()));
            panic!("differs at byte {at} of {} (want {}): {:02x?} vs {:02x?}", got.len(), want.len(), &got[at..(at + 24).min(got.len())], &want[at..(at + 24).min(want.len())]);
        }
    }

    #[test]
    fn every_library_fixture_reads_back_as_the_same_chain() {
        let Some((syms, catalog)) = res() else { return };
        for name in ["5150_DM", "Mark_Lead", "Recto_DM", "Revv_DM", "Revv_Home"] {
            let file = hlx(name);
            let document = hlx_document(&file, &syms, &catalog, &firmware()).unwrap();
            let preset = Document::decode(&document).and_then(|d| d.preset()).unwrap_or_else(|e| panic!("{name}: {e}"));
            let read = crate::device::DevicePreset { name: name.into(), index: None, document, modified: false }.to_hlx(&syms).unwrap();
            let blocks = |p: &nx_hlx::Preset| p.blocks().into_iter().map(|b| (b.model, b.path, b.position, b.enabled, b.trails)).collect::<Vec<_>>();
            assert_eq!(blocks(&read), blocks(&file), "{name}");
            assert_eq!(read.snapshot_names(), file.snapshot_names(), "{name}");
            assert_eq!(preset.snapshots.list.len(), file.snapshot_names().len(), "{name}");
        }
    }

    #[test]
    fn an_amp_with_a_cab_is_refused_by_name() {
        let Some((syms, catalog)) = res() else { return };
        let mut raw: Value = serde_json::from_slice(&hlx("Revv_Home").to_vec_pretty()).unwrap();
        raw["data"]["tone"]["dsp0"]["block2"]["@cab"] = "cab0".into();
        let file = nx_hlx::Preset::from_slice(&serde_json::to_vec(&raw).unwrap()).unwrap();
        let e = hlx_document(&file, &syms, &catalog, &firmware()).unwrap_err();
        assert!(e.ends_with("with its cab cannot be sent to the pedal yet. In HX Edit, give it a cab block of its own, or pick another preset."), "{e}");
        assert!(!e.contains("HD2_") && !e.contains("block2"), "named as HX Edit names it: {e}");
    }
}
