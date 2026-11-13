//! The editor window after the "Preset editor — Scythe style" design: a top bar, the signal chain
//! card, and below it the block editor beside a snapshots and models panel. Only what works is
//! drawn; knobs are live while connected to the pedal.

use std::collections::HashMap;

use egui::{Align2, Color32, CornerRadius, FontId, Frame, Margin, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use nx_catalog::Catalog;
use nx_hlx::Preset;

use crate::device::BlockAddress;
use crate::layout::{Branch, Lane, Tile, branch, display_value, dragged, lanes, slot_columns};
use crate::live::Live;
use crate::theme;

const GAP: f32 = 10.0;
const CARD_RADIUS: u8 = 18;
const ROW_H: f32 = 128.0;
const BLOCK_H: f32 = 104.0;
/// A block column narrower than this makes the chain scroll sideways instead.
const MIN_COLUMN: f32 = 104.0;
const SIDE_W: f32 = 340.0;
const KNOB_CARD: egui::Vec2 = vec2(116.0, 140.0);

/// The open session with the pedal and where each block of the shown preset lives on it.
pub struct Connection {
    pub live: Live,
    pub addresses: HashMap<String, BlockAddress>,
    /// The pedal's presets, read once on connecting.
    pub presets: Vec<nx_proto::PresetEntry>,
    /// The place of the preset shown, as the pedal last reported it.
    pub current: Option<u64>,
    /// Model names, to show a preset read after a switch.
    pub syms: nx_catalog::ModelSymbols,
}

/// A knob being dragged: its block and parameter, the value it started at, and the pixels moved up.
struct Drag {
    block: String,
    param: String,
    start: f64,
    up: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    Presets,
    Library,
    Snapshots,
    Models,
}

pub struct EditorApp {
    preset: Option<Preset>,
    catalog: Option<Catalog>,
    selected_block: Option<String>,
    selected_category: usize,
    status: String,
    connection: Option<Connection>,
    drag: Option<Drag>,
    panel: Panel,
    search: String,
    library: crate::library::Scan,
    /// The library file shown instead of the pedal's preset; its knobs never reach the pedal.
    showing_file: Option<std::path::PathBuf>,
    /// The pedal's preset, kept while a library file is shown.
    pedal_preset: Option<Preset>,
}

impl EditorApp {
    pub fn new(preset: Option<Preset>, catalog: Option<Catalog>, status: String, connection: Option<Connection>) -> Self {
        let selected_block = preset.as_ref().and_then(|p| p.blocks().first().map(|b| b.key.clone()));
        let panel = match (&connection, &preset) {
            (Some(_), _) => Panel::Presets,
            (None, None) => Panel::Library,
            (None, Some(_)) => Panel::Snapshots,
        };
        Self {
            preset,
            catalog,
            selected_block,
            selected_category: 1,
            status,
            connection,
            drag: None,
            panel,
            search: String::new(),
            library: crate::library::scan(&crate::library::default_dir()),
            showing_file: None,
            pedal_preset: None,
        }
    }

    fn top_bar(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let (logo, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
            ui.painter().rect_filled(logo, CornerRadius::same(12), theme::ACCENT);
            pulse_icon(ui.painter(), logo.center(), theme::BG);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(2.0);
                ui.label(RichText::new("nx-edit").font(theme::semibold(15.0)));
                let firmware = self.preset.as_ref().map(|p| p.device_version()).filter(|v| *v != 0);
                let sub = firmware.map_or_else(|| "HX Stomp".to_owned(), |v| format!("HX Stomp · firmware {}.{:02x}", v >> 24, (v >> 16) & 0xff));
                ui.label(RichText::new(sub).font(theme::regular(12.0)).color(theme::TEXT_DIM));
            });
            if let Some(preset) = &self.preset {
                pill(ui, |ui| {
                    ui.label(RichText::new(preset.name()).font(theme::semibold(15.0)));
                });
                let current = preset.current_snapshot();
                if let Some(name) = preset.snapshot_names().get(current) {
                    pill(ui, |ui| {
                        dot(ui, theme::ACCENT, 8.0);
                        ui.label(RichText::new(format!("Snapshot {} · {name}", current + 1)).font(theme::semibold(13.0)));
                    });
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                pill(ui, |ui| {
                    dot(ui, if self.connection.is_some() { theme::GREEN } else { theme::ARM_OFF }, 8.0);
                    ui.label(RichText::new(&self.status).font(theme::regular(12.0)).color(theme::TEXT_DIM));
                });
            });
        });
    }

    fn chain_card(&mut self, ui: &mut Ui, lanes: &[Lane]) {
        let Some(preset) = &self.preset else { return };
        let branch = branch(preset);
        let rejoins = lanes.get(1).is_some_and(|l| l.output == lanes[0].output);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Signal chain").font(theme::semibold(16.0)));
                if lanes.len() > 1 {
                    chip(ui, "Path A and B");
                }
            });
            ui.add_space(6.0);
            let columns = usize::from(slot_columns(lanes, branch)) + 2;
            let column = (ui.available_width() / columns as f32).max(MIN_COLUMN);
            let size = vec2(column * columns as f32, ROW_H * lanes.len() as f32);
            let mut clicked = None;
            egui::ScrollArea::horizontal().show(ui, |ui| {
                let (area, _) = ui.allocate_exact_size(size, Sense::hover());
                let grid = Grid { origin: area.min, column, columns };
                clicked = self.paint_chain(ui, &grid, lanes, branch, rejoins);
            });
            if let Some(key) = clicked {
                self.selected_block = Some(key);
            }
        });
    }

    fn paint_chain(&self, ui: &Ui, grid: &Grid, lanes: &[Lane], branch: Option<Branch>, rejoins: bool) -> Option<String> {
        let painter = ui.painter();
        let wire = Stroke::new(4.0, theme::WIRE);
        let (top, bottom) = (grid.row(0), grid.row(1));
        painter.line_segment([pos2(grid.center(0), top), pos2(grid.center(grid.columns - 1), top)], wire);
        let below = lanes.get(1).filter(|l| !l.tiles.is_empty());
        let nodes = match (branch, below) {
            (Some(b), Some(_)) => {
                let split = grid.boundary(b.split);
                bend(painter, pos2(split, top), pos2(split + 48.0, bottom), wire);
                let merge = grid.boundary(b.join);
                if rejoins {
                    painter.line_segment([pos2(split + 48.0, bottom), pos2(merge - 48.0, bottom)], wire);
                    bend(painter, pos2(merge, top), pos2(merge - 48.0, bottom), wire);
                    vec![split, merge]
                } else {
                    painter.line_segment([pos2(split + 48.0, bottom), pos2(grid.center(grid.columns - 1), bottom)], wire);
                    vec![split]
                }
            }
            _ => Vec::new(),
        };
        io_pill(painter, grid.cell(0, 0), "Input", "In");
        for (row, lane) in lanes.iter().enumerate() {
            if row == 0 || !rejoins {
                io_pill(painter, grid.cell(grid.columns - 1, row), "Output", lane.output);
            }
        }
        let mut clicked = None;
        for (row, lane) in lanes.iter().enumerate() {
            for tile in &lane.tiles {
                let inset = ((grid.column - 96.0) / 2.0).clamp(4.0, 16.0);
                let rect = grid.cell(usize::from(tile.position) + 1, row).shrink2(vec2(inset, (ROW_H - BLOCK_H) / 2.0));
                if ui.interact(rect, ui.scope_id().with(("block", &tile.block_key)), Sense::click()).clicked() {
                    clicked = Some(tile.block_key.clone());
                }
                self.paint_block(painter, rect, tile);
            }
        }
        for x in nodes {
            painter.circle_filled(pos2(x, top), 16.0, theme::PANEL);
            painter.circle_stroke(pos2(x, top), 10.0, Stroke::new(2.0, theme::SWITCH_OFF));
            painter.circle_filled(pos2(x, top), 7.0, theme::TEXT_DIM);
        }
        clicked
    }

    fn paint_block(&self, painter: &egui::Painter, rect: Rect, tile: &Tile) {
        let selected = self.selected_block.as_deref() == Some(tile.block_key.as_str());
        let color = tile.category_color.map_or(theme::TEXT_MID, theme::from_rgb_u32);
        let radius = CornerRadius::same(14);
        let fill = match (tile.enabled, selected) {
            (false, _) => theme::LANE,
            (true, true) => theme::RAISED_HI,
            (true, false) => theme::RAISED,
        };
        painter.rect_filled(rect, radius, fill);
        if !tile.enabled {
            painter.rect_stroke(rect, radius, Stroke::new(1.0, theme::RAISED_HI), StrokeKind::Inside);
        }
        if selected {
            painter.rect_stroke(rect, radius, Stroke::new(2.0, theme::ACCENT), StrokeKind::Inside);
        }
        let head = rect.left_top() + vec2(12.0, 22.0);
        painter.circle_filled(head + vec2(4.0, 0.0), 4.0, if tile.enabled { color } else { theme::SWITCH_OFF });
        let power = pos2(rect.right() - 20.0, head.y);
        if let Some(category) = &tile.category {
            let room = power.x - 16.0 - (head.x + 14.0);
            let galley = one_line(painter, category, theme::semibold(11.0), theme::TEXT_DIM, room);
            painter.galley(head + vec2(14.0, -galley.size().y / 2.0), galley, theme::TEXT_DIM);
        }
        if tile.enabled {
            painter.circle_filled(power, 13.0, theme::with_alpha(color, 0x33));
        }
        power_icon(painter, power, if tile.enabled { color } else { theme::POWER_OFF });
        let name_color = if tile.enabled { theme::TEXT } else { theme::TEXT_DIM };
        let size = if rect.width() >= 120.0 { 15.0 } else { 13.0 };
        let galley = painter.layout(tile.label.clone(), theme::semibold(size), name_color, rect.width() - 20.0);
        painter.galley(rect.left_top() + vec2(10.0, 44.0), galley, name_color);
    }

    fn editor_card(&mut self, ui: &mut Ui, lanes: &[Lane]) {
        let (Some(preset), Some(key)) = (&self.preset, self.selected_block.clone()) else { return };
        let Some(block) = preset.blocks().into_iter().find(|b| b.key == key) else { return };
        let tile = lanes.iter().flat_map(|l| &l.tiles).find(|t| t.block_key == key).cloned();
        let model = self.catalog.as_ref().and_then(|c| c.model(&block.model)).cloned();
        let color = tile.as_ref().and_then(|t| t.category_color).map_or(theme::TEXT_MID, theme::from_rgb_u32);
        let address = self.connection.as_ref().filter(|_| self.showing_file.is_none()).and_then(|c| c.addresses.get(&key)).cloned();
        let mut turned = None;
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                let (icon, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
                ui.painter().rect_filled(icon, CornerRadius::same(14), theme::with_alpha(color, 0x2e));
                ui.painter().circle_filled(icon.center(), 8.0, color);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add_space(4.0);
                    let title = model.as_ref().and_then(|m| m.name.clone()).unwrap_or_else(|| block.model.clone());
                    ui.label(RichText::new(title).font(theme::semibold(22.0)));
                    let category = tile.as_ref().and_then(|t| t.category.clone()).unwrap_or_else(|| "Block".to_owned());
                    let path = if block.path == 0 { "A" } else { "B" };
                    let mut sub = format!("{category} · Path {path}");
                    if let Some(address) = &address {
                        sub.push_str(&format!(" · slot {}", address.slot));
                    }
                    ui.label(RichText::new(sub).font(theme::regular(13.0)).color(theme::TEXT_DIM));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    pill(ui, |ui| {
                        dot(ui, if block.enabled { theme::ACCENT } else { theme::SWITCH_OFF }, 8.0);
                        let state = if block.enabled { "On" } else { "Bypassed" };
                        ui.label(RichText::new(state).font(theme::semibold(13.0)).color(if block.enabled { theme::TEXT } else { theme::TEXT_DIM }));
                    });
                });
            });
            ui.add_space(8.0);
            let per_row = (((ui.available_width() + GAP) / (KNOB_CARD.x + GAP)).floor() as usize).max(1);
            let width = (ui.available_width() - GAP * (per_row - 1) as f32) / per_row as f32;
            for row in block.params.chunks(per_row) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for (name, value) in row {
                        let def = model.as_ref().and_then(|m| m.params.iter().find(|p| &p.id == name));
                        let current = value.as_f64().filter(|_| value.is_f64());
                        let live = match (def, current) {
                            (Some(d), Some(v)) if address.as_ref().and_then(|a| a.param(name)).is_some() => dragged(v, d, 0.0).map(|_| (d, v)),
                            _ => None,
                        };
                        let label = def.map_or(name.as_str(), |d| d.name.as_str());
                        let response = knob_card(ui, width, label, &display_value(value, def), knob_fraction(value, def), color, live.is_some());
                        let Some((def, v)) = live else { continue };
                        if response.drag_started() {
                            self.drag = Some(Drag { block: key.clone(), param: name.clone(), start: v, up: 0.0 });
                        }
                        if let Some(drag) = self.drag.as_mut().filter(|d| d.block == key && &d.param == name)
                            && response.dragged()
                        {
                            drag.up -= response.drag_delta().y;
                            if let Some(next) = dragged(drag.start, def, drag.up).filter(|n| (n - v).abs() > 1e-9) {
                                turned = Some((name.clone(), next));
                            }
                        }
                        if response.drag_stopped() {
                            self.drag = None;
                        }
                    }
                });
                ui.add_space(GAP);
            }
        });
        let Some((name, value)) = turned else { return };
        if let Some(preset) = self.preset.as_mut() {
            preset.set_param(&key, &name, serde_json::json!(value));
        }
        if let Some(connection) = &self.connection
            && let Some(address) = connection.addresses.get(&key)
            && let Some(param) = address.param(&name)
        {
            connection.live.set(address.slot, address.sub, param, value as f32);
        }
    }

    fn side_card(&mut self, ui: &mut Ui) {
        card(ui, |ui| {
            let mut tabs = vec![(Panel::Library, "Library"), (Panel::Snapshots, "Snapshots"), (Panel::Models, "Models")];
            if self.connection.is_some() {
                tabs.insert(0, (Panel::Presets, "Presets"));
            }
            segmented(ui, &mut self.panel, &tabs);
            ui.add_space(12.0);
            match self.panel {
                Panel::Presets => self.presets(ui),
                Panel::Snapshots => self.snapshots(ui),
                Panel::Models => self.models(ui),
                Panel::Library => self.library(ui),
            }
        });
    }

    fn presets(&mut self, ui: &mut Ui) {
        let Some(connection) = &self.connection else { return };
        let mut pick = None;
        egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
            for entry in &connection.presets {
                let current = connection.current == Some(entry.index);
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                let painter = ui.painter();
                if current {
                    painter.rect_filled(rect, CornerRadius::same(10), theme::RAISED_HI);
                    painter.rect_stroke(rect, CornerRadius::same(10), Stroke::new(2.0, theme::ACCENT), StrokeKind::Inside);
                } else if response.hovered() {
                    painter.rect_filled(rect, CornerRadius::same(10), theme::RAISED);
                }
                let label_color = if current { theme::ACCENT } else { theme::TEXT_DIM };
                painter.text(rect.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, nx_proto::slot_label(entry.index), mono(13.0), label_color);
                let font = if current { theme::semibold(14.0) } else { theme::regular(14.0) };
                painter.text(rect.left_center() + vec2(60.0, 0.0), Align2::LEFT_CENTER, &entry.name, font, if current { theme::TEXT } else { theme::TEXT_MID });
                if response.clicked() {
                    pick = Some((entry.index, current));
                }
            }
        });
        match (pick, &self.connection) {
            (Some((_, true)), _) => self.show_pedal(),
            (Some((index, false)), Some(connection)) => {
                connection.live.select(0, index);
                self.status = format!("Switching to {}…", nx_proto::slot_label(index));
            }
            _ => {}
        }
    }

    fn library(&mut self, ui: &mut Ui) {
        if ui.button("Rescan").clicked() {
            self.library = crate::library::scan(&crate::library::default_dir());
        }
        if self.library.entries.is_empty() {
            ui.label(RichText::new(format!("No presets in {}", crate::library::default_dir().display())).color(theme::TEXT_DIM));
        }
        let mut open = None;
        egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
            let mut folder = None;
            for entry in &self.library.entries {
                if folder != Some(&entry.folder) {
                    folder = Some(&entry.folder);
                    ui.add_space(4.0);
                    ui.label(RichText::new(&entry.folder).font(theme::semibold(12.0)).color(theme::TEXT_DIM));
                }
                let shown = self.showing_file.as_ref() == Some(&entry.path);
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                if shown {
                    ui.painter().rect_filled(rect, CornerRadius::same(10), theme::RAISED_HI);
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(10), theme::RAISED);
                }
                let font = if shown { theme::semibold(14.0) } else { theme::regular(14.0) };
                ui.painter().text(rect.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, &entry.name, font, if shown { theme::TEXT } else { theme::TEXT_MID });
                if response.clicked() {
                    open = Some(entry.path.clone());
                }
            }
        });
        if !self.library.skipped.is_empty() {
            ui.label(RichText::new(format!("{} files could not be read", self.library.skipped.len())).font(theme::regular(12.0)).color(theme::TEXT_DIM));
        }
        if let Some(path) = open {
            self.open_file(path);
        }
    }

    fn open_file(&mut self, path: std::path::PathBuf) {
        match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| Preset::from_slice(&b).map_err(|e| e.to_string())) {
            Ok(preset) => {
                if self.showing_file.is_none() && self.connection.is_some() {
                    self.pedal_preset = self.preset.take();
                }
                self.selected_block = preset.blocks().first().map(|b| b.key.clone());
                self.status = if self.connection.is_some() {
                    format!("Library: {} · not on the pedal", preset.name())
                } else {
                    format!("Library: {}", preset.name())
                };
                self.preset = Some(preset);
                self.showing_file = Some(path);
                self.drag = None;
            }
            Err(e) => self.status = format!("{}: {e}", path.display()),
        }
    }

    /// Back from a library file to the pedal's preset.
    fn show_pedal(&mut self) {
        let Some(preset) = self.pedal_preset.take() else { return };
        self.selected_block = preset.blocks().first().map(|b| b.key.clone());
        self.preset = Some(preset);
        self.showing_file = None;
        self.drag = None;
        self.status = "Connected · changes not saved".to_owned();
    }

    /// Shows a preset the pedal switched to, by a click here or a footswitch.
    fn take_loaded(&mut self) {
        let Some(connection) = self.connection.as_mut() else { return };
        let Some(loaded) = connection.live.loaded() else { return };
        let shown = loaded.to_hlx(&connection.syms).and_then(|hlx| Ok((hlx, loaded.addresses(&connection.syms)?)));
        match shown {
            Ok((hlx, addresses)) => {
                connection.addresses = addresses;
                connection.current = loaded.index;
                if let Some(entry) = connection.presets.iter_mut().find(|p| Some(p.index) == loaded.index) {
                    entry.name = loaded.name.clone();
                }
                self.selected_block = hlx.blocks().first().map(|b| b.key.clone());
                self.preset = Some(hlx);
                self.showing_file = None;
                self.pedal_preset = None;
                self.drag = None;
                self.status = "Connected · changes not saved".to_owned();
            }
            Err(e) => self.status = format!("HX Stomp: {e}"),
        }
    }

    fn snapshots(&self, ui: &mut Ui) {
        let Some(preset) = &self.preset else { return };
        let current = preset.current_snapshot();
        for (i, name) in preset.snapshot_names().iter().enumerate() {
            let active = i == current;
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, CornerRadius::same(12), if active { theme::RAISED_HI } else { theme::RAISED });
            if active {
                painter.rect_stroke(rect, CornerRadius::same(12), Stroke::new(2.0, theme::ACCENT), StrokeKind::Inside);
            }
            let badge = Rect::from_center_size(pos2(rect.left() + 27.0, rect.center().y), vec2(30.0, 30.0));
            painter.rect_filled(badge, CornerRadius::same(9), if active { theme::ACCENT } else { theme::BG });
            painter.text(badge.center(), Align2::CENTER_CENTER, (i + 1).to_string(), mono(13.0), if active { theme::BG } else { theme::TEXT_DIM });
            painter.text(pos2(rect.left() + 54.0, rect.center().y), Align2::LEFT_CENTER, name, theme::semibold(14.0), theme::TEXT);
            ui.add_space(6.0);
        }
    }

    fn models(&mut self, ui: &mut Ui) {
        let Some(catalog) = &self.catalog else {
            ui.label(RichText::new("No HX Edit data imported: nx-edit --import-hxedit <installer>").color(theme::TEXT_DIM));
            return;
        };
        Frame::new().fill(theme::BG).corner_radius(CornerRadius::same(255)).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search models").frame(Frame::NONE).desired_width(f32::INFINITY));
        });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (i, cat) in catalog.categories().iter().enumerate().skip(1) {
                let active = i == self.selected_category;
                let response = Frame::new()
                    .fill(if active { theme::RAISED_HI } else { theme::RAISED })
                    .corner_radius(CornerRadius::same(255))
                    .inner_margin(Margin::symmetric(12, 7))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            dot(ui, theme::from_rgb_u32(cat.color), 7.0);
                            ui.label(RichText::new(&cat.name).font(theme::semibold(12.0)).color(if active { theme::TEXT } else { theme::TEXT_DIM }));
                        });
                    })
                    .response
                    .interact(Sense::click());
                if response.clicked() {
                    self.selected_category = i;
                }
            }
        });
        ui.add_space(10.0);
        let current = self.selected_block.as_ref().and_then(|key| {
            self.preset.as_ref().and_then(|p| p.blocks().into_iter().find(|b| &b.key == key)).map(|b| b.model)
        });
        let needle = self.search.trim().to_lowercase();
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            let name_of = |id: &String| catalog.model(id).and_then(|m| m.name.clone()).unwrap_or_else(|| id.clone());
            if !needle.is_empty() {
                for cat in catalog.categories().iter().skip(1) {
                    for id in cat.model_ids.iter().filter(|id| name_of(id).to_lowercase().contains(&needle)) {
                        model_row(ui, &name_of(id), current.as_ref() == Some(id));
                    }
                }
                return;
            }
            let Some(cat) = catalog.categories().get(self.selected_category) else { return };
            let ungrouped = [nx_catalog::CategoryGroup { name: String::new(), model_ids: cat.model_ids.clone() }];
            let groups = if cat.groups.is_empty() { &ungrouped[..] } else { &cat.groups[..] };
            for group in groups {
                if !group.name.is_empty() {
                    ui.add_space(4.0);
                    ui.label(RichText::new(&group.name).font(theme::semibold(12.0)).color(theme::TEXT_DIM));
                }
                for id in &group.model_ids {
                    model_row(ui, &name_of(id), current.as_ref() == Some(id));
                }
            }
        });
    }
}

/// Where the chain's cells are: column 0 is the input, the last column the outputs, and slot `n`
/// sits in column `n + 1`.
struct Grid {
    origin: Pos2,
    column: f32,
    columns: usize,
}

impl Grid {
    fn center(&self, column: usize) -> f32 {
        self.origin.x + self.column * (column as f32 + 0.5)
    }

    /// The x between slot `slot - 1` and slot `slot`, where a split or merge sits.
    fn boundary(&self, slot: u8) -> f32 {
        self.origin.x + self.column * (f32::from(slot) + 1.0)
    }

    fn row(&self, row: usize) -> f32 {
        self.origin.y + ROW_H * (row as f32 + 0.5)
    }

    fn cell(&self, column: usize, row: usize) -> Rect {
        Rect::from_min_size(self.origin + vec2(self.column * column as f32, ROW_H * row as f32), vec2(self.column, ROW_H))
    }
}

fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    Frame::new().fill(theme::PANEL).corner_radius(CornerRadius::same(CARD_RADIUS)).inner_margin(Margin::same(16)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}

fn pill(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    Frame::new().fill(theme::PANEL).corner_radius(CornerRadius::same(255)).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            add(ui);
        });
    });
}

fn chip(ui: &mut Ui, text: &str) {
    Frame::new().fill(theme::RAISED).corner_radius(CornerRadius::same(255)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.label(RichText::new(text).font(theme::regular(12.0)).color(theme::TEXT_DIM));
    });
}

fn dot(ui: &mut Ui, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    ui.painter().circle_filled(rect.center(), size / 2.0, color);
}

fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// A row of buttons of which one is selected, as the design's tab switchers.
fn segmented<T: Copy + PartialEq>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) {
    Frame::new().fill(theme::BG).corner_radius(CornerRadius::same(255)).inner_margin(Margin::same(4)).show(ui, |ui| {
        let width = (ui.available_width() - 4.0 * (options.len() - 1) as f32) / options.len() as f32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (option, label) in options {
                let active = *value == *option;
                let (rect, response) = ui.allocate_exact_size(vec2(width, 34.0), Sense::click());
                if active {
                    ui.painter().rect_filled(rect, CornerRadius::same(255), theme::RAISED_HI);
                }
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, theme::semibold(13.0), if active { theme::TEXT } else { theme::TEXT_DIM });
                if response.clicked() {
                    *value = *option;
                }
            }
        });
    });
}

fn model_row(ui: &mut Ui, name: &str, current: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    if current {
        ui.painter().rect_filled(rect, CornerRadius::same(10), theme::RAISED_HI);
    }
    let font = if current { theme::semibold(14.0) } else { theme::regular(14.0) };
    ui.painter().text(rect.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, name, font, if current { theme::TEXT } else { theme::TEXT_MID });
    if current {
        let c = rect.right_center() - vec2(20.0, 0.0);
        let check = vec![c + vec2(-6.0, 0.0), c + vec2(-1.5, 4.5), c + vec2(6.0, -5.0)];
        ui.painter().add(Shape::line(check, Stroke::new(2.4, theme::ACCENT)));
    }
}

/// An input or output of the chain, drawn as a pill centred in its cell; a narrow cell drops the
/// arrow icon so the name keeps its room.
fn io_pill(painter: &egui::Painter, cell: Rect, label: &str, name: &str) {
    let rect = Rect::from_center_size(cell.center(), vec2((cell.width() - 12.0).min(140.0), 52.0));
    painter.rect_filled(rect, CornerRadius::same(255), theme::RAISED);
    let text_left = if rect.width() >= 124.0 {
        let icon = pos2(rect.left() + 26.0, rect.center().y);
        painter.circle_filled(icon, 18.0, theme::BG);
        let arrow = Stroke::new(2.2, theme::TEXT_MID);
        painter.line_segment([icon - vec2(6.0, 0.0), icon + vec2(6.0, 0.0)], arrow);
        painter.add(Shape::line(vec![icon + vec2(1.0, -5.0), icon + vec2(6.0, 0.0), icon + vec2(1.0, 5.0)], arrow));
        rect.left() + 52.0
    } else {
        rect.left() + 16.0
    };
    let room = rect.right() - 12.0 - text_left;
    let label = one_line(painter, label, theme::regular(11.0), theme::TEXT_DIM, room);
    painter.galley(pos2(text_left, rect.center().y - 8.0 - label.size().y / 2.0), label, theme::TEXT_DIM);
    let name = one_line(painter, name, theme::semibold(14.0), theme::TEXT, room);
    painter.galley(pos2(text_left, rect.center().y + 8.0 - name.size().y / 2.0), name, theme::TEXT);
}

/// Text on one line, cut short with an ellipsis when it does not fit `width`.
fn one_line(painter: &egui::Painter, text: &str, font: FontId, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping { max_width: width.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    painter.layout_job(job)
}

/// A thick line from a junction on path A down into path B, leaving vertically and arriving level,
/// like the design's split and merge wires.
fn bend(painter: &egui::Painter, junction: Pos2, end: Pos2, stroke: Stroke) {
    let toward = (end.x - junction.x).signum();
    let points = [junction, pos2(junction.x, junction.y + (end.y - junction.y) * 0.72), pos2(junction.x + toward * 12.0, end.y), end];
    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(points, false, Color32::TRANSPARENT, stroke));
}

fn power_icon(painter: &egui::Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(2.0, color);
    let r = 5.5;
    let points: Vec<Pos2> = (0..=24)
        .map(|i| {
            let a = (-60.0 + 300.0 * i as f32 / 24.0 - 90.0).to_radians();
            center + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    painter.add(Shape::line(points, stroke));
    painter.line_segment([center - vec2(0.0, 7.0), center - vec2(0.0, 1.5)], stroke);
}

fn pulse_icon(painter: &egui::Painter, center: Pos2, color: Color32) {
    let p = |x: f32, y: f32| center + vec2(x - 12.0, y - 12.0) * 0.85;
    let points = vec![p(2.0, 12.0), p(6.0, 12.0), p(8.0, 7.0), p(12.0, 17.0), p(15.0, 10.0), p(17.0, 12.0), p(22.0, 12.0)];
    painter.add(Shape::line(points, Stroke::new(2.2, color)));
}

fn knob_fraction(value: &serde_json::Value, def: Option<&nx_catalog::ParamDef>) -> f32 {
    match value {
        serde_json::Value::Bool(on) => f32::from(u8::from(*on)),
        serde_json::Value::Number(n) => {
            let v = n.as_f64().unwrap_or_default();
            match def.and_then(|d| Some((d.min?, d.max?))) {
                Some((lo, hi)) if hi > lo => ((v - lo) / (hi - lo)).clamp(0.0, 1.0) as f32,
                _ => 0.5,
            }
        }
        _ => 0.0,
    }
}

/// A parameter as a card with an arc knob; a live one is dragged up and down to turn it.
fn knob_card(ui: &mut Ui, width: f32, label: &str, value: &str, fraction: f32, color: Color32, live: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, KNOB_CARD.y), if live { Sense::drag() } else { Sense::hover() });
    let response = if live { response.on_hover_cursor(egui::CursorIcon::ResizeVertical) } else { response };
    let painter = ui.painter();
    let hot = live && (response.hovered() || response.dragged());
    painter.rect_filled(rect, CornerRadius::same(14), if hot { theme::RAISED_HI } else { theme::RAISED });
    let center = rect.center_top() + vec2(0.0, 54.0);
    let arc = |to: f32| -> Vec<Pos2> {
        let steps = 48;
        (0..=steps)
            .map(|i| {
                let a = (135.0 + 270.0 * to * i as f32 / steps as f32).to_radians();
                center + vec2(a.cos(), a.sin()) * 31.0
            })
            .collect()
    };
    painter.add(Shape::line(arc(1.0), Stroke::new(6.0, theme::BG)));
    if fraction > 0.0 {
        painter.add(Shape::line(arc(fraction), Stroke::new(6.0, color)));
    }
    painter.circle_filled(center, 23.0, theme::RAISED_HI);
    let a = (135.0 + 270.0 * fraction).to_radians();
    painter.line_segment([center, center + vec2(a.cos(), a.sin()) * 17.0], Stroke::new(3.0, theme::TEXT));
    painter.text(rect.center_top() + vec2(0.0, 104.0), Align2::CENTER_CENTER, label, theme::regular(13.0), theme::TEXT_MID);
    painter.text(rect.center_top() + vec2(0.0, 123.0), Align2::CENTER_CENTER, value, mono(15.0), theme::TEXT);
    response
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.take_loaded();
        if let Some(connection) = &self.connection {
            if let Some(error) = connection.live.errors().pop() {
                self.status = format!("HX Stomp: {error}");
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        egui::Panel::top("top").frame(Frame::new().fill(theme::BG).inner_margin(Margin::same(10))).show(ui, |ui| self.top_bar(ui));
        egui::CentralPanel::default().frame(Frame::new().fill(theme::BG).inner_margin(Margin::same(10))).show(ui, |ui| {
            let lanes = self.preset.as_ref().map(|p| lanes(p, self.catalog.as_ref())).unwrap_or_default();
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = GAP;
                self.chain_card(ui, &lanes);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    let editor_w = (ui.available_width() - SIDE_W - GAP).max(320.0);
                    ui.allocate_ui_with_layout(vec2(editor_w, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(editor_w);
                        self.editor_card(ui, &lanes);
                    });
                    ui.allocate_ui_with_layout(vec2(SIDE_W, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(SIDE_W);
                        self.side_card(ui);
                    });
                });
            });
        });
    }
}
