//! Putting a library preset on a place of the pedal: choosing the place and saying what is replaced.
//! Sending it waits for a capture of how HX Edit imports a preset (capture-session step 19).

use std::path::PathBuf;

use nx_proto::{PresetEntry, slot_label};

/// A library preset on its way to a place on the pedal.
pub struct Assign {
    pub file: PathBuf,
    /// The preset's own name, as it will be called on the pedal.
    pub name: String,
    pub place: Option<u64>,
    pub filter: String,
}

/// The places whose label ("03B") or current name contains `filter`, ignoring case.
pub fn places<'a>(presets: &'a [PresetEntry], filter: &str) -> Vec<&'a PresetEntry> {
    let needle = filter.trim().to_lowercase();
    presets
        .iter()
        .filter(|p| needle.is_empty() || slot_label(p.index).to_lowercase().contains(&needle) || p.name.to_lowercase().contains(&needle))
        .collect()
}

/// What happens to the place, in words.
pub fn replacing(place: &PresetEntry, name: &str) -> String {
    format!("{} \"{}\" on the HX Stomp is replaced by \"{name}\". The preset now on {0} is gone unless it is in the library.", slot_label(place.index), place.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> Vec<PresetEntry> {
        ["Recto DM", "5150 DM", "Revv DM", "Mark Lead", "Revv Home", "New Preset", "New Preset"]
            .iter()
            .enumerate()
            .map(|(i, n)| PresetEntry { index: i as u64, name: (*n).into() })
            .collect()
    }

    fn labels(found: Vec<&PresetEntry>) -> Vec<String> {
        found.into_iter().map(|p| slot_label(p.index)).collect()
    }

    #[test]
    fn no_filter_lists_every_place() {
        assert_eq!(places(&list(), "  ").len(), 7);
    }

    #[test]
    fn places_are_found_by_label_or_name_ignoring_case() {
        assert_eq!(labels(places(&list(), "02b")), ["02B"]);
        assert_eq!(labels(places(&list(), "revv")), ["01C", "02B"]);
        assert_eq!(labels(places(&list(), "new preset")), ["02C", "03A"]);
        assert!(places(&list(), "Fear Factory").is_empty());
    }

    #[test]
    fn the_confirmation_names_the_place_and_both_presets() {
        let place = PresetEntry { index: 7, name: "New Preset".into() };
        assert_eq!(
            replacing(&place, "BE100 CH1+2"),
            "03B \"New Preset\" on the HX Stomp is replaced by \"BE100 CH1+2\". The preset now on 03B is gone unless it is in the library."
        );
    }
}
