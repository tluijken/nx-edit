use std::path::PathBuf;

use nx_catalog::Catalog;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/res")
}

#[test]
fn loads_fixture_models() {
    let catalog = Catalog::load(&fixture_dir()).unwrap();
    let revv = catalog.model("HD2_AmpRevvGenRed").unwrap();
    assert_eq!(revv.name.as_deref(), Some("Revv Gen Red"));
    let drive = revv.params.iter().find(|p| p.id == "Drive").unwrap();
    assert_eq!(drive.min, Some(0.0));
    assert_eq!(drive.max, Some(1.0));
    assert!(catalog.model("HD2_AmpPVPanama").is_some());
}

#[test]
fn missing_optional_keys_are_none() {
    let catalog = Catalog::load(&fixture_dir()).unwrap();
    let bare = catalog.model("HD2_TestBare").unwrap();
    assert_eq!(bare.name, None);
    assert_eq!(bare.category, None);
    assert_eq!(bare.load, None);
    assert_eq!(bare.params[0].display_type, None);
}

#[test]
fn categories_flatten_subcategories() {
    let catalog = Catalog::load(&fixture_dir()).unwrap();
    let dist = &catalog.categories()[1];
    assert_eq!(dist.name, "Distortion");
    assert_eq!(dist.color, 0xf5901e);
    assert!(dist.model_ids.iter().any(|id| id == "HD2_DistKinkyBoost"));
    assert_eq!(catalog.categories()[0].model_ids, vec!["None"]);
}

#[test]
fn boolean_ranges_become_zero_and_one() {
    let catalog = Catalog::load(&fixture_dir()).unwrap();
    let boost = catalog.model("HD2_DistKinkyBoost").unwrap();
    let switch = boost.params.iter().find(|p| p.id == "Boost").unwrap();
    assert_eq!((switch.min, switch.max), (Some(0.0), Some(1.0)));
}

#[test]
fn text_parameters_have_no_range() {
    let dir = std::env::temp_dir().join(format!("nx-catalog-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(fixture_dir().join("HX_ModelCatalog.json"), dir.join("HX_ModelCatalog.json")).unwrap();
    std::fs::write(
        dir.join("fixed.models"),
        r#"[{"symbolicID":"@global","params":[{"symbolicID":"@topology0","name":"DSP 0 Topology","valueType":3,"min":"","max":"","default":""}]}]"#,
    )
    .unwrap();
    let catalog = Catalog::load(&dir).unwrap();
    let topo = &catalog.model("@global").unwrap().params[0];
    assert_eq!((topo.min, topo.max), (None, None));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn params_missing_their_fields_still_load() {
    let dir = std::env::temp_dir().join(format!("nx-catalog-sparse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(fixture_dir().join("HX_ModelCatalog.json"), dir.join("HX_ModelCatalog.json")).unwrap();
    std::fs::write(dir.join("future.models"), r#"[{"symbolicID":"HD2_Future","params":[{"symbolicID":"Mystery"}]}]"#).unwrap();
    let catalog = Catalog::load(&dir).unwrap();
    let param = &catalog.model("HD2_Future").unwrap().params[0];
    assert_eq!(param.id, "Mystery");
    assert_eq!(param.name, "Mystery");
    assert_eq!((param.min, param.max), (None, None));
    assert_eq!(param.default, serde_json::Value::Null);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn categories_keep_hx_edit_groups() {
    let catalog = Catalog::load(&fixture_dir()).unwrap();
    let dist = &catalog.categories()[1];
    let names: Vec<&str> = dist.groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, vec!["Mono", "Stereo", "Legacy"]);
    assert!(dist.groups[0].model_ids.iter().any(|id| id == "HD2_DistKinkyBoost"));
    assert!(catalog.categories()[0].groups.is_empty());
}

#[test]
fn missing_directory_is_an_error() {
    assert!(Catalog::load(&fixture_dir().join("does-not-exist")).is_err());
}

#[test]
#[ignore = "needs NX_HXEDIT_RES pointing at a real HX Edit res directory"]
fn real_res_dir_loads_all_models() {
    let dir = PathBuf::from(std::env::var("NX_HXEDIT_RES").unwrap());
    let catalog = Catalog::load(&dir).unwrap();
    assert!(catalog.len() >= 681, "{}", catalog.len());
    assert!(catalog.model("HD2_AmpPVPanama").is_some());
    assert!(catalog.categories().len() > 10);
}

#[test]
#[ignore = "needs NX_HXEDIT_INSTALLER pointing at the HX Edit Windows installer, and 7z"]
fn imports_from_installer() {
    let installer = PathBuf::from(std::env::var("NX_HXEDIT_INSTALLER").unwrap());
    let dest = std::env::temp_dir().join(format!("nx-catalog-import-{}", std::process::id()));
    nx_catalog::import_installer(&installer, &dest).unwrap();
    let catalog = Catalog::load(&dest).unwrap();
    assert!(catalog.len() >= 681);
    std::fs::remove_dir_all(&dest).unwrap();
}

#[test]
fn discrete_controls_name_their_options() {
    let catalog = Catalog::load(&fixture_dir()).unwrap();
    assert_eq!(catalog.options("test_mics"), Some(&["Mic One".to_owned(), "Mic Two".into(), "Mic Three".into()][..]));
    assert_eq!(catalog.options("generic_knob"), None);
    assert_eq!(catalog.options("unknown"), None);
}

#[test]
fn a_res_folder_without_controls_still_loads() {
    let dir = std::env::temp_dir().join(format!("nx-catalog-{}-no-controls", std::process::id()));
    _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["amp.models", "HX_ModelCatalog.json"] {
        std::fs::copy(fixture_dir().join(name), dir.join(name)).unwrap();
    }
    let catalog = Catalog::load(&dir).unwrap();
    assert!(catalog.model("HD2_AmpRevvGenRed").is_some());
    assert_eq!(catalog.options("off_on"), None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn segmented_controls_are_told_apart_from_lists() {
    let catalog = Catalog::load(&fixture_dir()).unwrap();
    assert!(catalog.segmented("off_on"));
    assert!(!catalog.segmented("test_mics"));
    assert!(!catalog.segmented("unknown"));
}

#[test]
fn a_block_engine_follows_its_model_category() {
    use nx_catalog::engine;
    assert_eq!(engine(Some(3), "HD2_DistScream808Mono"), Some(1));
    assert_eq!(engine(Some(9), "HD2_DelaySimpleDelayMono"), Some(8));
    assert_eq!(engine(Some(10), "VIC_DynPlateMono"), Some(8));
    assert_eq!(engine(Some(1), "HD2_AmpRevvGenRed"), Some(17));
    assert_eq!(engine(Some(13), "HD2_PreampTweedBluesBrt"), Some(17));
    assert_eq!(engine(Some(19), "HD2_CabMicIr_4x12CaliV30"), Some(31));
    assert_eq!(engine(Some(16), "HD2_ImpulseResponse1024Mono"), Some(19));
    assert_eq!(engine(Some(15), "HD2_LooperOneSwitchMono"), Some(22));
    assert_eq!(engine(Some(12), "HD2_SendMono1"), Some(25));
    assert_eq!(engine(Some(12), "HD2_SendMono2"), Some(26));
}

#[test]
fn an_engine_not_seen_yet_is_none() {
    use nx_catalog::engine;
    assert_eq!(engine(Some(12), "HD2_ReturnMono1"), None, "returns were not captured");
    assert_eq!(engine(Some(2), "HD2_CabSomething"), None);
    assert_eq!(engine(None, "HD2_DistScream808Mono"), None);
}

#[test]
fn a_catalog_model_finds_its_mono_or_stereo_number() {
    let syms = nx_catalog::ModelSymbols::parse(
        br#"[{"symbol": "HD2_AmpRevvGenRed"}, {"symbol": "HD2_DistKinkyBoostMono"}, {"symbol": "HD2_DistKinkyBoostStereo"}, {"symbol": "HD2_ReverbHallStereo"}]"#,
    )
    .unwrap();
    assert_eq!(syms.number_for("HD2_ReverbHall", false), Some(3), "only stereo exists");
    assert_eq!(syms.number_for("HD2_AmpRevvGenRed", false), Some(0));
    assert_eq!(syms.number_for("HD2_DistKinkyBoost", false), Some(1));
    assert_eq!(syms.number_for("HD2_DistKinkyBoost", true), Some(2));
    assert_eq!(syms.number_for("HD2_Nothing", false), None);
}
