use std::fs;
use std::path::PathBuf;

use nx_hlx::{HX_STOMP_DEVICE, Preset};

fn fixture(name: &str) -> Vec<u8> {
    fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

fn fixtures() -> Vec<(String, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut out: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "hlx"))
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), fs::read(&p).unwrap()))
        .collect();
    out.sort();
    out
}

fn dsp0_keys(bytes: &[u8]) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    v["data"]["tone"]["dsp0"].as_object().unwrap().keys().cloned().collect()
}

#[test]
fn every_fixture_round_trips() {
    let all = fixtures();
    assert!(!all.is_empty());
    for (name, bytes) in all {
        let preset = Preset::from_slice(&bytes).unwrap();
        let written = preset.to_vec_pretty();
        let original: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let reread: serde_json::Value = serde_json::from_slice(&written).unwrap();
        assert_eq!(original, reread, "{name}");
        assert_eq!(dsp0_keys(&bytes), dsp0_keys(&written), "{name}");
    }
}

#[test]
fn unknown_keys_survive() {
    let mut v: serde_json::Value = serde_json::from_slice(&fixture("Revv_DM.hlx")).unwrap();
    v["data"]["tone"]["dsp0"]["block0"]["ZZ_unknown"] = serde_json::json!(1);
    let preset = Preset::from_slice(&serde_json::to_vec(&v).unwrap()).unwrap();
    let reread: serde_json::Value = serde_json::from_slice(&preset.to_vec_pretty()).unwrap();
    assert_eq!(reread["data"]["tone"]["dsp0"]["block0"]["ZZ_unknown"], 1);
}

#[test]
fn revv_dm_blocks() {
    let preset = Preset::from_slice(&fixture("Revv_DM.hlx")).unwrap();
    let blocks = preset.blocks();
    assert_eq!(blocks[2].model, "HD2_AmpRevvGenRed");
    let cab = blocks.iter().find(|b| b.model == "HD2_CabMicIr_4x12CaliV30").unwrap();
    let send = blocks.iter().find(|b| b.model == "HD2_SendMono1").unwrap();
    assert_eq!(cab.path, 1);
    assert_eq!(send.path, 1);
    assert_eq!(preset.split().unwrap().position, 6);
    assert_eq!(preset.join().unwrap().position, 8);
    let amp = &blocks[2];
    assert!(amp.enabled);
    assert!(amp.params.iter().any(|(k, _)| k == "Drive"));
    assert!(amp.params.iter().all(|(k, _)| !k.starts_with('@')));
}

#[test]
fn blocks_are_sorted_by_position_then_path() {
    let preset = Preset::from_slice(&fixture("Revv_DM.hlx")).unwrap();
    let order: Vec<(u8, u8)> = preset.blocks().iter().map(|b| (b.position, b.path)).collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted);
}

#[test]
fn snapshot_names_of_revv_dm() {
    let preset = Preset::from_slice(&fixture("Revv_DM.hlx")).unwrap();
    assert_eq!(preset.snapshot_names(), vec!["808", "33", "Lead"]);
}

#[test]
fn device_is_hx_stomp() {
    let preset = Preset::from_slice(&fixture("Revv_DM.hlx")).unwrap();
    assert_eq!(preset.device(), HX_STOMP_DEVICE);
    assert_eq!(preset.device_version(), 0x0370_0000);
    assert_eq!(preset.name(), "Revv DM");
}

#[test]
fn missing_tone_is_a_shape_error() {
    let err = Preset::from_slice(br#"{"data":{}}"#).unwrap_err();
    assert!(matches!(err, nx_hlx::HlxError::Shape(_)));
}

#[test]
fn current_snapshot_comes_from_global() {
    let preset = Preset::from_slice(&fixture("Revv_DM.hlx")).unwrap();
    assert_eq!(preset.current_snapshot(), 0);
    let mut v: serde_json::Value = serde_json::from_slice(&fixture("Revv_DM.hlx")).unwrap();
    v["data"]["tone"]["global"]["@current_snapshot"] = serde_json::json!(2);
    assert_eq!(Preset::from_slice(&serde_json::to_vec(&v).unwrap()).unwrap().current_snapshot(), 2);
}

#[test]
fn set_param_changes_one_block_parameter() {
    let mut preset = Preset::from_slice(&fixture("Revv_DM.hlx")).unwrap();
    assert!(preset.set_param("block2", "Drive", serde_json::json!(0.5)));
    let amp = preset.blocks().into_iter().find(|b| b.key == "block2").unwrap();
    assert_eq!(amp.params.iter().find(|(k, _)| k == "Drive").unwrap().1, serde_json::json!(0.5));
}

#[test]
fn set_param_ignores_unknown_blocks_parameters_and_metadata() {
    let mut preset = Preset::from_slice(&fixture("Revv_DM.hlx")).unwrap();
    let before = preset.clone();
    assert!(!preset.set_param("block99", "Drive", serde_json::json!(0.5)));
    assert!(!preset.set_param("block2", "Nonexistent", serde_json::json!(0.5)));
    assert!(!preset.set_param("block2", "@model", serde_json::json!("x")));
    assert_eq!(preset, before);
}

#[test]
fn set_enabled_switches_a_block() {
    let mut preset = Preset::from_slice(&fixture("Revv_DM.hlx")).unwrap();
    let enabled = |p: &Preset| p.blocks().into_iter().find(|b| b.key == "block4").unwrap().enabled;
    let before = enabled(&preset);
    assert!(preset.set_enabled("block4", !before));
    assert_eq!(enabled(&preset), !before);
    assert!(!preset.set_enabled("block99", true));
}

#[test]
fn set_current_snapshot_moves_the_active_snapshot() {
    let mut preset = Preset::from_slice(&fixture("Revv_DM.hlx")).unwrap();
    preset.set_current_snapshot(2);
    assert_eq!(preset.current_snapshot(), 2);
}

#[test]
fn set_name_renames_the_preset() {
    let mut preset = Preset::from_slice(&fixture("Revv_DM.hlx")).unwrap();
    preset.set_name("Revv Live");
    assert_eq!(Preset::from_slice(&preset.to_vec_pretty()).unwrap().name(), "Revv Live");
}
