use nx_catalog::Catalog;
use nx_hlx::Preset;

/// One row of the signal path as HX Edit draws it: path 0 on top, path 1 below once a split has blocks.
#[derive(Debug, Clone, PartialEq)]
pub struct Lane {
    pub path: u8,
    pub tiles: Vec<Tile>,
    /// The jack this lane ends in, as the pedal labels it.
    pub output: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub block_key: String,
    /// The block's slot on the pedal; empty slots leave gaps in the drawing.
    pub position: u8,
    pub label: String,
    /// HX Edit's category of the model (Amp, Cab, Delay, ...), when the catalog knows it.
    pub category: Option<String>,
    pub category_color: Option<u32>,
    pub enabled: bool,
}

pub fn lanes(preset: &Preset, catalog: Option<&Catalog>) -> Vec<Lane> {
    let mut lanes = vec![Lane { path: 0, tiles: Vec::new(), output: "Main L/R" }];
    for block in preset.blocks() {
        let category = catalog.and_then(|c| c.categories().iter().find(|cat| cat.model_ids.contains(&block.model)));
        let tile = Tile {
            label: catalog
                .and_then(|c| c.model(&block.model))
                .and_then(|m| m.name.clone())
                .unwrap_or_else(|| block.model.clone()),
            category: category.map(|c| c.name.clone()),
            category_color: category.map(|c| c.color),
            enabled: block.enabled,
            position: block.position,
            block_key: block.key,
        };
        if block.path == 0 {
            lanes[0].tiles.push(tile);
        } else {
            if lanes.len() < 2 {
                lanes.push(Lane { path: 1, tiles: Vec::new(), output: "Main L/R" });
            }
            lanes[1].tiles.push(tile);
        }
    }
    // The .hlx view carries no output routing; a send block on path B stands for the Send output.
    if let Some(below) = lanes.get_mut(1)
        && preset.blocks().iter().any(|b| b.path == 1 && b.model.starts_with("HD2_Send"))
    {
        below.output = "Send";
    }
    lanes
}

/// Where path 1 leaves path 0 and comes back, in slot positions: the branch starts before slot
/// `split` and rejoins before slot `join`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Branch {
    pub split: u8,
    pub join: u8,
}

pub fn branch(preset: &Preset) -> Option<Branch> {
    let blocks = preset.blocks();
    let below = blocks.iter().filter(|b| b.path == 1);
    let first = below.clone().map(|b| b.position).min()?;
    let after_last = blocks.iter().map(|b| b.position + 1).max().unwrap_or(first + 1);
    Some(Branch {
        split: preset.split().map_or(first, |s| s.position),
        join: preset.join().map_or(after_last, |j| j.position),
    })
}

/// Slot columns the chain needs between its input and output: through the last block, and through
/// the join when path B rejoins after it.
pub fn slot_columns(lanes: &[Lane], branch: Option<Branch>) -> u8 {
    let blocks = lanes.iter().flat_map(|l| &l.tiles).map(|t| t.position + 1).max().unwrap_or(0);
    blocks.max(branch.map_or(0, |b| b.join))
}

/// Pixels of vertical drag that sweep a knob through its whole range.
pub const KNOB_TRAVEL: f32 = 200.0;

/// HX Edit's parameter value types (`valueType` in its model files).
const INTEGER: u8 = 0;
const FLOAT: u8 = 1;
const SWITCH: u8 = 2;

/// A parameter's value after dragging its knob `up` pixels, clamped to its range: a float rounded
/// to hundredths of the range, the steps HX Edit sends, an integer to whole steps. `None` for a
/// switch, text or a parameter without a range.
pub fn dragged(value: f64, def: &nx_catalog::ParamDef, up: f32) -> Option<f64> {
    let (lo, hi) = (def.min?, def.max?);
    if !matches!(def.value_type, INTEGER | FLOAT) || hi <= lo {
        return None;
    }
    let span = hi - lo;
    let raw = (value + f64::from(up / KNOB_TRAVEL) * span).clamp(lo, hi);
    if def.value_type == INTEGER {
        return Some(raw.round());
    }
    Some(lo + ((raw - lo) / span * 100.0).round() / 100.0 * span)
}

/// A parameter value as HX Edit shows it: 0–1 knobs on a 0–10 scale, switches as On/Off, and a
/// list's options by name, `options` naming them from the parameter's minimum on.
pub fn display_value(value: &serde_json::Value, def: Option<&nx_catalog::ParamDef>, options: Option<&[String]>) -> String {
    match value {
        serde_json::Value::Bool(on) => (if *on { "On" } else { "Off" }).to_owned(),
        serde_json::Value::Number(n) => {
            let v = n.as_f64().unwrap_or_default();
            let unit_range = def.is_some_and(|d| d.min == Some(0.0) && d.max == Some(1.0) && d.value_type == FLOAT);
            let option = def.and_then(|d| d.min).and_then(|lo| options?.get(usize::try_from((v - lo) as i64).ok()?));
            if let Some(name) = option {
                name.clone()
            } else if unit_range {
                format!("{:.1}", v * 10.0)
            } else if v.fract() == 0.0 {
                format!("{v:.0}")
            } else {
                format!("{v:.1}")
            }
        }
        other => other.to_string(),
    }
}

/// `v` as the `.hlx` holds a parameter of this kind: an integer, a switch or a float. The `.hlx`
/// writes whole floats without a fraction, so the kind comes from the model, not from the value.
pub fn typed_value(def: Option<&nx_catalog::ParamDef>, v: f64) -> serde_json::Value {
    match def.map(|d| d.value_type) {
        Some(INTEGER) => serde_json::json!(v.round() as i64),
        Some(SWITCH) => serde_json::json!(v != 0.0),
        _ => serde_json::json!(v),
    }
}

/// A `.hlx` value as op 30 carries it: floats as f32, as HX Edit sends them.
pub fn wire_value(value: &serde_json::Value) -> Option<rmpv::Value> {
    match value {
        serde_json::Value::Bool(b) => Some(rmpv::Value::Boolean(*b)),
        serde_json::Value::Number(n) if n.is_f64() => Some(rmpv::Value::F32(n.as_f64()? as f32)),
        serde_json::Value::Number(n) => n.as_i64().map(rmpv::Value::from),
        _ => None,
    }
}

/// How a parameter is set in the editor.
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    /// Dragged up and down.
    Knob,
    /// A row of buttons, one per option: switches, and the short lists HX Edit shows side by side.
    Buttons(Vec<String>),
    /// A drop-down of named options.
    List(Vec<String>),
}

/// The editor's control for a parameter: switches and HX Edit's segmented lists as buttons, other
/// named lists as a drop-down, anything else as a knob.
pub fn control_for(def: &nx_catalog::ParamDef, catalog: Option<&Catalog>) -> Control {
    let display = def.display_type.as_deref().unwrap_or_default();
    let named = catalog.and_then(|c| c.options(display)).map(<[String]>::to_vec);
    match def.value_type {
        SWITCH => Control::Buttons(named.filter(|n| n.len() == 2).unwrap_or_else(|| vec!["Off".into(), "On".into()])),
        INTEGER => match named {
            Some(names) if catalog.is_some_and(|c| c.segmented(display)) => Control::Buttons(names),
            Some(names) => Control::List(names),
            None => Control::Knob,
        },
        _ => Control::Knob,
    }
}

/// Which option a value is, counted from the parameter's minimum; a switch's off is 0.
pub fn chosen(value: &serde_json::Value, def: &nx_catalog::ParamDef) -> Option<usize> {
    let v = value.as_bool().map_or_else(|| value.as_f64(), |on| Some(f64::from(u8::from(on))))?;
    usize::try_from((v - def.min.unwrap_or_default()).round() as i64).ok()
}

/// The value of option `index`, as `set_param` takes it.
pub fn choice_value(index: usize, def: &nx_catalog::ParamDef) -> f64 {
    def.min.unwrap_or_default() + index as f64
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn preset(name: &str) -> Preset {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nx-hlx/tests/fixtures").join(name);
        Preset::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn catalog() -> Catalog {
        Catalog::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nx-catalog/tests/fixtures/res")).unwrap()
    }

    fn labels(lane: &Lane) -> Vec<&str> {
        lane.tiles.iter().map(|t| t.label.as_str()).collect()
    }

    #[test]
    fn revv_dm_has_two_lanes() {
        let lanes = lanes(&preset("Revv_DM.hlx"), Some(&catalog()));
        assert_eq!(lanes.len(), 2);
        assert_eq!(labels(&lanes[1]), vec!["HD2_CabMicIr_4x12CaliV30", "HD2_SendMono1"]);
        assert_eq!(lanes[1].path, 1);
    }

    #[test]
    fn each_lane_names_the_output_it_feeds() {
        let dm = lanes(&preset("Revv_DM.hlx"), None);
        assert_eq!(dm.iter().map(|l| l.output).collect::<Vec<_>>(), vec!["Main L/R", "Send"]);
        assert_eq!(lanes(&preset("Revv_Home.hlx"), None)[0].output, "Main L/R");
    }

    #[test]
    fn revv_home_has_one_lane() {
        let lanes = lanes(&preset("Revv_Home.hlx"), Some(&catalog()));
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].tiles.len(), 7);
    }

    #[test]
    fn catalog_names_replace_model_ids() {
        let lanes = lanes(&preset("Revv_DM.hlx"), Some(&catalog()));
        assert!(labels(&lanes[0]).contains(&"Revv Gen Red"));
        let amp = lanes[0].tiles.iter().find(|t| t.label == "Revv Gen Red").unwrap();
        assert_eq!(amp.block_key, "block2");
        assert!(amp.enabled);
    }

    #[test]
    fn model_missing_from_a_loaded_catalog_shows_raw_id() {
        let lanes = lanes(&preset("Revv_DM.hlx"), Some(&catalog()));
        let send = lanes[1].tiles.iter().find(|t| t.label == "HD2_SendMono1").unwrap();
        assert_eq!(send.category_color, None);
    }

    #[test]
    fn unknown_model_shows_raw_id() {
        let lanes = lanes(&preset("Revv_DM.hlx"), None);
        assert!(labels(&lanes[0]).contains(&"HD2_AmpRevvGenRed"));
    }

    #[test]
    fn tiles_keep_their_slot_positions() {
        let mut v: serde_json::Value =
            serde_json::from_slice(&preset("Revv_DM.hlx").to_vec_pretty()).unwrap();
        let dsp0 = v["data"]["tone"]["dsp0"].as_object_mut().unwrap();
        dsp0.retain(|k, _| !matches!(k.as_str(), "block1" | "block3" | "block4" | "block5"));
        dsp0["block2"]["@position"] = serde_json::json!(4);
        let preset = Preset::from_slice(&serde_json::to_vec(&v).unwrap()).unwrap();
        let lanes = lanes(&preset, None);
        let top: Vec<u8> = lanes[0].tiles.iter().map(|t| t.position).collect();
        let bottom: Vec<u8> = lanes[1].tiles.iter().map(|t| t.position).collect();
        assert_eq!(top, vec![0, 4]);
        assert_eq!(bottom, vec![6, 7]);
    }

    #[test]
    fn revv_dm_branches_at_its_split_and_rejoins_at_its_join() {
        assert_eq!(branch(&preset("Revv_DM.hlx")), Some(Branch { split: 6, join: 8 }));
    }

    #[test]
    fn the_chain_has_a_column_per_slot_up_to_the_last_block_or_the_join() {
        let dm = preset("Revv_DM.hlx");
        assert_eq!(slot_columns(&lanes(&dm, None), branch(&dm)), 8);
        let home = preset("Revv_Home.hlx");
        assert_eq!(slot_columns(&lanes(&home, None), branch(&home)), 7);
        let branch_past_blocks = Some(Branch { split: 6, join: 9 });
        assert_eq!(slot_columns(&lanes(&dm, None), branch_past_blocks), 9);
    }

    #[test]
    fn a_split_without_blocks_below_draws_no_branch() {
        assert_eq!(branch(&preset("Revv_Home.hlx")), None);
    }

    #[test]
    fn a_branch_without_join_rejoins_after_the_last_block() {
        let mut v: serde_json::Value = serde_json::from_slice(&preset("Revv_DM.hlx").to_vec_pretty()).unwrap();
        v["data"]["tone"]["dsp0"].as_object_mut().unwrap().remove("join");
        let preset = Preset::from_slice(&serde_json::to_vec(&v).unwrap()).unwrap();
        assert_eq!(branch(&preset), Some(Branch { split: 6, join: 8 }));
    }

    #[test]
    fn unit_range_values_show_on_a_ten_scale() {
        let cat = catalog();
        let drive = cat.model("HD2_AmpRevvGenRed").unwrap().params.iter().find(|p| p.id == "Drive").unwrap();
        assert_eq!(display_value(&serde_json::json!(0.45), Some(drive), None), "4.5");
    }

    fn param<'a>(cat: &'a Catalog, model: &str, id: &str) -> &'a nx_catalog::ParamDef {
        cat.model(model).unwrap().params.iter().find(|p| p.id == id).unwrap()
    }

    #[test]
    fn dragging_up_a_fifth_of_the_travel_adds_a_fifth_of_the_range() {
        let cat = catalog();
        let drive = param(&cat, "HD2_AmpRevvGenRed", "Drive");
        let got = dragged(0.45, drive, 40.0).unwrap();
        assert!((got - 0.65).abs() < 1e-9, "{got}");
    }

    #[test]
    fn a_dragged_value_stays_in_range() {
        let cat = catalog();
        let drive = param(&cat, "HD2_AmpRevvGenRed", "Drive");
        assert_eq!(dragged(0.9, drive, 500.0), Some(1.0));
        assert_eq!(dragged(0.1, drive, -500.0), Some(0.0));
    }

    #[test]
    fn a_dragged_value_lands_on_hundredths_of_the_range() {
        let cat = catalog();
        let drive = param(&cat, "HD2_AmpRevvGenRed", "Drive");
        let got = dragged(0.45, drive, 1.2).unwrap();
        assert!((got - 0.46).abs() < 1e-9, "{got}");
    }

    #[test]
    fn switches_and_unranged_parameters_are_not_dragged() {
        let mut def = param(&catalog(), "HD2_AmpRevvGenRed", "Drive").clone();
        def.value_type = 2;
        assert_eq!(dragged(1.0, &def, 10.0), None);
        def.value_type = 1;
        def.max = None;
        assert_eq!(dragged(0.5, &def, 10.0), None);
    }

    fn list(min: f64, max: f64) -> nx_catalog::ParamDef {
        let mut def = param(&catalog(), "HD2_AmpRevvGenRed", "Drive").clone();
        (def.value_type, def.min, def.max) = (0, Some(min), Some(max));
        def
    }

    #[test]
    fn an_integer_parameter_drags_in_whole_steps() {
        let mic = list(0.0, 11.0);
        assert_eq!(dragged(3.0, &mic, KNOB_TRAVEL * 2.0 / 11.0), Some(5.0));
        assert_eq!(dragged(3.0, &mic, 1.0), Some(3.0));
        assert_eq!(dragged(3.0, &mic, -KNOB_TRAVEL), Some(0.0));
    }

    #[test]
    fn a_list_shows_the_name_of_its_option() {
        let mic = list(0.0, 11.0);
        let names: Vec<String> = (0..12).map(|i| format!("Mic {i}")).collect();
        assert_eq!(display_value(&serde_json::json!(8), Some(&mic), Some(&names)), "Mic 8");
        let sync = list(1.0, 3.0);
        let notes = ["1/1".to_owned(), "1/2".into(), "1/4".into()];
        assert_eq!(display_value(&serde_json::json!(3), Some(&sync), Some(&notes)), "1/4");
        assert_eq!(display_value(&serde_json::json!(9), Some(&sync), Some(&notes)), "9");
    }

    #[test]
    fn a_value_is_written_with_its_parameters_type() {
        let mut def = list(0.0, 11.0);
        assert_eq!(typed_value(Some(&def), 9.0), serde_json::json!(9));
        def.value_type = 2;
        assert_eq!(typed_value(Some(&def), 1.0), serde_json::json!(true));
        def.value_type = 1;
        assert!(typed_value(Some(&def), 0.0).is_f64(), "a float stays a float at a whole value");
        assert!(typed_value(None, 0.5).is_f64());
    }

    #[test]
    fn values_go_to_the_pedal_as_their_own_messagepack_type() {
        assert_eq!(wire_value(&serde_json::json!(9)), Some(rmpv::Value::from(9)));
        assert_eq!(wire_value(&serde_json::json!(true)), Some(rmpv::Value::Boolean(true)));
        assert_eq!(wire_value(&serde_json::json!(0.5)), Some(rmpv::Value::F32(0.5)));
        assert_eq!(wire_value(&serde_json::json!("x")), None);
    }

    #[test]
    fn switches_show_on_and_off() {
        assert_eq!(display_value(&serde_json::json!(true), None, None), "On");
        assert_eq!(display_value(&serde_json::json!(false), None, None), "Off");
    }

    #[test]
    fn other_numbers_show_as_stored() {
        assert_eq!(display_value(&serde_json::json!(8500.0), None, None), "8500");
        assert_eq!(display_value(&serde_json::json!(-1.5), None, None), "-1.5");
        assert_eq!(display_value(&serde_json::json!(2), None, None), "2");
    }

    #[test]
    fn category_colour_comes_from_the_catalog() {
        let lanes = lanes(&preset("Revv_DM.hlx"), Some(&catalog()));
        let ts = lanes[0].tiles.iter().find(|t| t.label == "Scream 808").unwrap();
        assert_eq!(ts.category_color, Some(0xf5901e));
        assert_eq!(ts.category.as_deref(), Some("Distortion"));
    }

    fn kind(value_type: u8, display: &str, min: f64, max: f64) -> nx_catalog::ParamDef {
        let mut def = list(min, max);
        (def.value_type, def.display_type) = (value_type, Some(display.to_owned()));
        def
    }

    #[test]
    fn a_switch_is_a_pair_of_buttons_named_as_hx_edit_names_them() {
        let cat = catalog();
        assert_eq!(control_for(&kind(2, "off_on", 0.0, 1.0), Some(&cat)), Control::Buttons(vec!["Off".into(), "On".into()]));
        assert_eq!(control_for(&kind(2, "unknown", 0.0, 1.0), Some(&cat)), Control::Buttons(vec!["Off".into(), "On".into()]));
        assert_eq!(control_for(&kind(2, "off_on", 0.0, 1.0), None), Control::Buttons(vec!["Off".into(), "On".into()]));
    }

    #[test]
    fn a_long_list_is_a_drop_down_and_a_plain_integer_a_knob() {
        let cat = catalog();
        let mics = vec!["Mic One".to_owned(), "Mic Two".into(), "Mic Three".into()];
        assert_eq!(control_for(&kind(0, "test_mics", 0.0, 2.0), Some(&cat)), Control::List(mics));
        assert_eq!(control_for(&kind(0, "integer_slider", 0.0, 2.0), Some(&cat)), Control::Knob);
        assert_eq!(control_for(&kind(1, "generic_knob", 0.0, 1.0), Some(&cat)), Control::Knob);
    }

    #[test]
    fn a_short_segmented_list_is_buttons() {
        let cat = catalog();
        assert_eq!(control_for(&kind(0, "off_on", 0.0, 1.0), Some(&cat)), Control::Buttons(vec!["Off".into(), "On".into()]));
    }

    #[test]
    fn a_choice_is_the_option_index_from_the_minimum() {
        let sync = kind(0, "sync_note", 1.0, 19.0);
        assert_eq!(chosen(&serde_json::json!(6), &sync), Some(5));
        assert_eq!(chosen(&serde_json::json!(true), &kind(2, "off_on", 0.0, 1.0)), Some(1));
        assert_eq!(choice_value(5, &sync), 6.0);
        assert_eq!(choice_value(1, &kind(2, "off_on", 0.0, 1.0)), 1.0);
    }
}

#[cfg(test)]
mod pedal_tests {
    use std::path::PathBuf;

    use nx_catalog::ModelSymbols;
    use nx_device::device::DevicePreset;

    use super::*;

    fn captured() -> DevicePreset {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nx-proto/tests/fixtures/02-select-preset-01C.doc");
        DevicePreset { name: "Revv DM".into(), index: Some(2), document: std::fs::read(path).unwrap(), modified: false }
    }

    fn fixture() -> Preset {
        Preset::from_slice(include_bytes!("../../nx-hlx/tests/fixtures/Revv_DM.hlx")).unwrap()
    }

    fn helix_sym() -> Option<ModelSymbols> {
        let dir = std::env::var_os("NX_HXEDIT_RES").map(PathBuf::from).unwrap_or_else(nx_catalog::default_res_dir);
        let path = dir.join("Helix.sym");
        if !path.exists() {
            eprintln!("skipped: no Helix.sym in {} (set NX_HXEDIT_RES)", dir.display());
            return None;
        }
        Some(ModelSymbols::load(&path).unwrap())
    }

    #[test]
    fn converted_capture_lays_out_like_the_hlx() {
        let Some(syms) = helix_sym() else { return };
        let converted = captured().to_hlx(&syms).unwrap();
        assert_eq!(lanes(&converted, None), lanes(&fixture(), None));
    }

    #[test]
    fn converted_snapshots_switch_like_the_hlx() {
        let Some(syms) = helix_sym() else { return };
        let mut converted = captured().to_hlx(&syms).unwrap();
        let mut want = fixture();
        converted.select_snapshot(2);
        want.select_snapshot(2);
        assert_eq!(lanes(&converted, None), lanes(&want, None));
    }
}
