//! Editing a preset file the way the pedal treats snapshots: the blocks' own `@enabled` and the
//! snapshot-controlled values are those of the current snapshot, which keeps its own copy of both.

use std::path::PathBuf;

use nx_hlx::Preset;
use serde_json::{Value, json};

fn revv_dm() -> Preset {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Revv_DM.hlx");
    Preset::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn raw(preset: &Preset) -> Value {
    serde_json::from_slice(&preset.to_vec_pretty()).unwrap()
}

fn param(preset: &Preset, block: &str, name: &str) -> Value {
    preset.blocks().into_iter().find(|b| b.key == block).unwrap().params.into_iter().find(|(k, _)| k == name).unwrap().1
}

fn enabled(preset: &Preset, block: &str) -> bool {
    preset.blocks().into_iter().find(|b| b.key == block).unwrap().enabled
}

#[test]
fn selecting_a_snapshot_loads_its_block_states_and_values() {
    let mut preset = revv_dm();
    assert!(!enabled(&preset, "block4"));
    assert!(preset.select_snapshot(2));
    assert_eq!(preset.current_snapshot(), 2);
    assert!(enabled(&preset, "block4") && enabled(&preset, "block5") && !enabled(&preset, "block1"));
    assert_eq!(param(&preset, "block3", "MidFreq"), json!(800.0));
    assert_eq!(param(&preset, "block3", "Level"), json!(5.0));
    assert!(preset.select_snapshot(1));
    assert!(!enabled(&preset, "block0") && enabled(&preset, "block1"));
}

#[test]
fn a_snapshot_that_does_not_exist_is_not_selected() {
    let mut preset = revv_dm();
    assert!(!preset.select_snapshot(3));
    assert_eq!(preset.current_snapshot(), 0);
    assert_eq!(raw(&preset), raw(&revv_dm()));
}

#[test]
fn bypassing_a_block_is_kept_in_the_current_snapshot_only() {
    let mut preset = revv_dm();
    preset.select_snapshot(2);
    assert!(preset.set_enabled("block4", false));
    assert_eq!(raw(&preset)["data"]["tone"]["snapshot2"]["blocks"]["dsp0"]["block4"], json!(false));
    preset.select_snapshot(0);
    preset.select_snapshot(2);
    assert!(!enabled(&preset, "block4"));
    assert_eq!(raw(&preset)["data"]["tone"]["snapshot0"]["blocks"]["dsp0"]["block4"], json!(false));
    assert!(enabled(&preset, "block5"));
}

#[test]
fn a_snapshot_controlled_value_is_kept_in_the_current_snapshot_only() {
    let mut preset = revv_dm();
    preset.select_snapshot(2);
    assert!(preset.set_param("block3", "MidFreq", json!(1200.0)));
    preset.select_snapshot(0);
    assert_eq!(param(&preset, "block3", "MidFreq"), json!(400.0));
    preset.select_snapshot(2);
    assert_eq!(param(&preset, "block3", "MidFreq"), json!(1200.0));
}

#[test]
fn a_value_no_snapshot_controls_is_the_same_in_every_snapshot() {
    let mut preset = revv_dm();
    let before = raw(&preset)["data"]["tone"]["snapshot1"].clone();
    assert!(preset.set_param("block2", "Drive", json!(0.7)));
    preset.select_snapshot(1);
    assert_eq!(param(&preset, "block2", "Drive"), json!(0.7));
    assert_eq!(raw(&preset)["data"]["tone"]["snapshot1"], before);
}
