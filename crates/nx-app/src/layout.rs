use nx_catalog::Catalog;
use nx_hlx::Preset;

/// One row of the signal path as HX Edit draws it: path 0 on top, path 1 below once a split has blocks.
#[derive(Debug, Clone, PartialEq)]
pub struct Lane {
    pub path: u8,
    pub tiles: Vec<Tile>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub block_key: String,
    pub label: String,
    pub category_color: Option<u32>,
    pub enabled: bool,
}

pub fn lanes(preset: &Preset, catalog: Option<&Catalog>) -> Vec<Lane> {
    let mut lanes = vec![Lane { path: 0, tiles: Vec::new() }];
    for block in preset.blocks() {
        let tile = Tile {
            label: catalog
                .and_then(|c| c.model(&block.model))
                .and_then(|m| m.name.clone())
                .unwrap_or_else(|| block.model.clone()),
            category_color: catalog.and_then(|c| {
                c.categories().iter().find(|cat| cat.model_ids.contains(&block.model)).map(|cat| cat.color)
            }),
            enabled: block.enabled,
            block_key: block.key,
        };
        if block.path == 0 {
            lanes[0].tiles.push(tile);
        } else {
            if lanes.len() < 2 {
                lanes.push(Lane { path: 1, tiles: Vec::new() });
            }
            lanes[1].tiles.push(tile);
        }
    }
    lanes
}

/// A parameter value as HX Edit shows it: 0–1 knobs on a 0–10 scale, switches as On/Off.
pub fn display_value(value: &serde_json::Value, def: Option<&nx_catalog::ParamDef>) -> String {
    match value {
        serde_json::Value::Bool(on) => (if *on { "On" } else { "Off" }).to_owned(),
        serde_json::Value::Number(n) => {
            let v = n.as_f64().unwrap_or_default();
            let unit_range = def.is_some_and(|d| d.min == Some(0.0) && d.max == Some(1.0) && d.value_type == 1);
            if unit_range {
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
    fn unknown_model_shows_raw_id() {
        let lanes = lanes(&preset("Revv_DM.hlx"), None);
        assert!(labels(&lanes[0]).contains(&"HD2_AmpRevvGenRed"));
    }

    #[test]
    fn unit_range_values_show_on_a_ten_scale() {
        let cat = catalog();
        let drive = cat.model("HD2_AmpRevvGenRed").unwrap().params.iter().find(|p| p.id == "Drive").unwrap();
        assert_eq!(display_value(&serde_json::json!(0.45), Some(drive)), "4.5");
    }

    #[test]
    fn switches_show_on_and_off() {
        assert_eq!(display_value(&serde_json::json!(true), None), "On");
        assert_eq!(display_value(&serde_json::json!(false), None), "Off");
    }

    #[test]
    fn other_numbers_show_as_stored() {
        assert_eq!(display_value(&serde_json::json!(8500.0), None), "8500");
        assert_eq!(display_value(&serde_json::json!(-1.5), None), "-1.5");
        assert_eq!(display_value(&serde_json::json!(2), None), "2");
    }

    #[test]
    fn category_colour_comes_from_the_catalog() {
        let lanes = lanes(&preset("Revv_DM.hlx"), Some(&catalog()));
        let ts = lanes[0].tiles.iter().find(|t| t.label == "Scream 808").unwrap();
        assert_eq!(ts.category_color, Some(0xf5901e));
    }
}
