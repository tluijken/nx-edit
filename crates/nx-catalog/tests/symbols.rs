//! `Helix.sym`: entry n is the device's model number n, with its parameters in the order the device
//! stores their values. The real file is Line 6's and is not in the repo; the test that needs it
//! runs when `NX_HXEDIT_RES` (or nx-edit's import directory) holds one.

use std::path::PathBuf;

use nx_catalog::{ModelSymbols, default_res_dir, shared_id};

const SAMPLE: &str = r#"[
    { "symbol": "HD2_AmpCali400Ch1", "parameters": [ "Drive", "Bass" ] },
    { "symbol": "HD2_DistScream808Mono", "parameters": [ "Gain", "Tone", "Level" ] },
    { "symbol": "HD2_ReverbHallStereo", "parameters": [ ] }
]"#;

#[test]
fn index_is_the_model_number() {
    let syms = ModelSymbols::parse(SAMPLE.as_bytes()).unwrap();
    assert_eq!(syms.len(), 3);
    assert_eq!(syms.get(1).unwrap().symbol, "HD2_DistScream808Mono");
    assert_eq!(syms.get(1).unwrap().parameters, ["Gain", "Tone", "Level"]);
    assert_eq!(syms.number("HD2_ReverbHallStereo"), Some(2));
    assert!(syms.get(3).is_none());
}

#[test]
fn shared_id_drops_the_channel_suffix() {
    assert_eq!(shared_id("HD2_DistScream808Mono"), "HD2_DistScream808");
    assert_eq!(shared_id("HD2_ReverbHallStereo"), "HD2_ReverbHall");
    assert_eq!(shared_id("HD2_SendMono1"), "HD2_SendMono1");
    assert_eq!(shared_id("HD2_AmpRevvGenRed"), "HD2_AmpRevvGenRed");
}

#[test]
fn not_json_is_an_error() {
    assert!(ModelSymbols::parse(b"{").is_err());
}

fn real_helix_sym() -> Option<PathBuf> {
    let dir = std::env::var_os("NX_HXEDIT_RES").map(PathBuf::from).unwrap_or_else(default_res_dir);
    let path = dir.join("Helix.sym");
    if path.exists() {
        Some(path)
    } else {
        eprintln!("skipped: no Helix.sym in {} (set NX_HXEDIT_RES)", dir.display());
        None
    }
}

#[test]
fn real_helix_sym_numbers_revv_dm_models() {
    let Some(path) = real_helix_sym() else { return };
    let syms = ModelSymbols::load(&path).unwrap();
    assert_eq!(syms.len(), 833, "HX Edit 3.82");
    // The model numbers in capture 02's preset document, against Revv_DM.hlx's @model.
    for (n, id) in [
        (101, "HD2_DistScream808"),
        (131, "HD2_EQParametric"),
        (545, "HD2_AmpRevvGenRed"),
        (77, "HD2_DelayDuckedDelay"),
        (243, "HD2_ReverbHall"),
        (693, "HD2_CabMicIr_4x12CaliV30"),
        (250, "HD2_SendMono1"),
        (257, "HD2_AppDSPFlowSplitY"),
        (151, "HD2_AppDSPFlowJoin"),
    ] {
        assert_eq!(shared_id(&syms.get(n).unwrap().symbol), id, "model {n}");
    }
    assert_eq!(syms.get(545).unwrap().parameters[0], "Drive");
}
