//! The engine number a block carries in the preset document (slot key 9). HX Edit's data files do
//! not hold it; capture 31 showed it following the model's category in its `*.models` files.

/// A model's engine by its `*.models` category and Helix.sym symbol; `None` for a kind of block whose
/// engine has not been seen, which must not be sent to the pedal.
pub fn engine(category: Option<u32>, symbol: &str) -> Option<u64> {
    match category? {
        3 | 4 | 6 | 7 | 8 | 11 | 14 | 17 => Some(1),
        9 | 10 => Some(8),
        1 | 13 => Some(17),
        19 => Some(31),
        16 => Some(19),
        15 => Some(22),
        12 => match symbol {
            "HD2_SendMono1" => Some(25),
            "HD2_SendMono2" => Some(26),
            _ => None,
        },
        _ => None,
    }
}

/// An amp's engine once it carries a cab (block key 24.23) [seen, capture 31].
pub const AMP_WITH_CAB: u64 = 33;
