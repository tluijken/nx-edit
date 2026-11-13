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
    /// The block's slot on the pedal; empty slots leave gaps in the drawing.
    pub position: u8,
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
            position: block.position,
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
