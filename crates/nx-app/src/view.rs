//! The editor window after the "Preset editor — Scythe style" design: a top bar, the signal chain
//! card, and below it the block editor beside a snapshots and models panel. Only what works is
//! drawn; knobs are live while connected to the pedal.

use std::collections::HashMap;

use egui::{Align2, Color32, CornerRadius, FontId, Frame, Margin, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use nx_catalog::Catalog;
use nx_hlx::Preset;

use crate::device::BlockAddress;
use crate::history::{History, Turn};
use crate::layout::{Branch, Lane, Tile, branch, display_value, dragged, lanes, slot_columns};
use crate::live::{Live, Loaded, ParamChange, Update};
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
    /// The worker's generation of the preset shown; edits carry it.
    pub generation: u64,
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

/// What the main area shows, picked in the rail.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Editor,
    CustomTone,
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
    /// A library preset being renamed, and the name typed so far.
    renaming: Option<(std::path::PathBuf, String)>,
    /// Knob turns on the pedal's preset, for undo and redo.
    history: History,
    /// The pedal's preset, kept while a library file is shown.
    pedal_preset: Option<Preset>,
    screen: Screen,
    customtone: crate::customtone::CustomToneView,
    /// Edits to the library file shown that are not written to it yet.
    unsaved: bool,
    /// Where to go once the unsaved edits are saved or discarded, while that is asked.
    leaving: Option<Leave>,
    /// The window may close: the unsaved edits were saved or discarded.
    closing: bool,
}

/// Leaving a library file with unsaved edits, for another file, the pedal's preset, another
/// preset on the pedal, or quitting.
#[derive(Clone)]
enum Leave {
    Open(std::path::PathBuf),
    Pedal,
    Select(u64),
    Quit,
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
            renaming: None,
            history: History::default(),
            pedal_preset: None,
            screen: Screen::Editor,
            customtone: Default::default(),
            unsaved: false,
            leaving: None,
            closing: false,
        }
    }

    /// Draws the top bar; true when its Save button was clicked.
    fn top_bar(&self, ui: &mut Ui) -> bool {
        let mut save = false;
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
                if self.unsaved {
                    let button = egui::Button::new(RichText::new("Save").font(theme::semibold(13.0)).color(theme::BG))
                        .fill(theme::ACCENT)
                        .corner_radius(CornerRadius::same(255))
                        .min_size(vec2(72.0, 36.0));
                    save = ui.add(button).on_hover_text("Write the changes to the library file (Ctrl+S)").clicked();
                    ui.label(RichText::new("Unsaved changes").font(theme::regular(12.0)).color(theme::TEXT_DIM));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                pill(ui, |ui| {
                    dot(ui, if self.connection.is_some() { theme::GREEN } else { theme::ARM_OFF }, 8.0);
                    ui.label(RichText::new(&self.status).font(theme::regular(12.0)).color(theme::TEXT_DIM));
                });
            });
        });
        save
    }

    fn rail(&mut self, ui: &mut Ui) {
        Frame::new().fill(theme::PANEL).corner_radius(CornerRadius::same(CARD_RADIUS)).inner_margin(Margin::symmetric(6, 8)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for (screen, hint) in [(Screen::Editor, "Signal chain"), (Screen::CustomTone, "CustomTone")] {
                let (rect, response) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::click());
                let active = self.screen == screen;
                if active {
                    ui.painter().rect_filled(rect, CornerRadius::same(12), theme::RAISED_HI);
                }
                let color = if active { theme::ACCENT } else { theme::TEXT_DIM };
                match screen {
                    Screen::Editor => chain_icon(ui.painter(), rect.center(), color),
                    Screen::CustomTone => cloud_icon(ui.painter(), rect.center(), color),
                }
                if response.on_hover_text(hint).clicked() {
                    self.screen = screen;
                }
            }
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
            match clicked {
                Some(ChainClick::Select(key)) => self.selected_block = Some(key),
                Some(ChainClick::Toggle(key)) => self.toggle_block(&key),
                None => {}
            }
        });
    }

    fn paint_chain(&self, ui: &Ui, grid: &Grid, lanes: &[Lane], branch: Option<Branch>, rejoins: bool) -> Option<ChainClick> {
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
                    clicked = Some(ChainClick::Select(tile.block_key.clone()));
                }
                if self.editable() {
                    let power = Rect::from_center_size(power_center(rect), vec2(28.0, 28.0));
                    let response = ui.interact(power, ui.scope_id().with(("power", &tile.block_key)), Sense::click());
                    if response.on_hover_text(if tile.enabled { "Bypass" } else { "Turn on" }).clicked() {
                        clicked = Some(ChainClick::Toggle(tile.block_key.clone()));
                    }
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
        let power = power_center(rect);
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
        let address = self.live().and_then(|c| c.addresses.get(&key)).cloned();
        let file = self.showing_file.is_some();
        let can_toggle = address.is_some() || file;
        let mut toggle = false;
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
                    let response = Frame::new()
                        .fill(theme::RAISED)
                        .corner_radius(CornerRadius::same(255))
                        .inner_margin(Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                dot(ui, if block.enabled { theme::ACCENT } else { theme::SWITCH_OFF }, 8.0);
                                let state = if block.enabled { "On" } else { "Bypassed" };
                                ui.label(RichText::new(state).font(theme::semibold(13.0)).color(if block.enabled { theme::TEXT } else { theme::TEXT_DIM }));
                            });
                        })
                        .response;
                    if can_toggle && response.interact(Sense::click()).on_hover_text("Turn on or bypass").clicked() {
                        toggle = true;
                    }
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
                            (Some(d), Some(v)) if file || address.as_ref().and_then(|a| a.param(name)).is_some() => dragged(v, d, 0.0).map(|_| (d, v)),
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
                        if response.drag_stopped()
                            && let Some(drag) = self.drag.take()
                        {
                            let after = turned.as_ref().filter(|(n, _)| n == name).map_or(v, |(_, next)| *next);
                            self.history.push(Turn { block: drag.block, param: drag.param, before: drag.start, after });
                        }
                    }
                });
                ui.add_space(GAP);
            }
        });
        if toggle {
            self.toggle_block(&key);
        }
        let Some((name, value)) = turned else { return };
        self.set_param(&key, &name, value);
    }

    /// Sets a parameter of the preset shown: on the pedal too for the pedal's preset, until saved
    /// for a library file.
    fn set_param(&mut self, key: &str, name: &str, value: f64) {
        if let Some(preset) = self.preset.as_mut() {
            preset.set_param(key, name, serde_json::json!(value));
        }
        if self.showing_file.is_some() {
            self.unsaved = true;
            return;
        }
        if let Some(connection) = self.live()
            && let Some(address) = connection.addresses.get(key)
            && let Some(param) = address.param(name)
        {
            connection.live.set(connection.generation, address.slot, address.sub, param, value as f32);
        }
    }

    /// Ctrl+Z takes the last knob turn back, Ctrl+Shift+Z or Ctrl+Y makes it again, Ctrl+S saves a
    /// library file; only while no text field has the keyboard.
    fn edit_keys(&mut self, ctx: &egui::Context) {
        if !self.editable() || self.leaving.is_some() || ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        if self.unsaved && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
            self.save_file();
        }
        let (undo, redo) = ctx.input(|i| {
            let z = i.modifiers.command && i.key_pressed(egui::Key::Z);
            (z && !i.modifiers.shift, (z && i.modifiers.shift) || (i.modifiers.command && i.key_pressed(egui::Key::Y)))
        });
        let step = if undo {
            self.history.undo().map(|t| (t.block, t.param, t.before))
        } else if redo {
            self.history.redo().map(|t| (t.block, t.param, t.after))
        } else {
            None
        };
        if let Some((block, param, value)) = step {
            self.selected_block = Some(block.clone());
            self.set_param(&block, &param, value);
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
        match pick {
            Some((_, true)) => self.leave(Leave::Pedal),
            Some((index, false)) if self.showing_file.is_some() => self.leave(Leave::Select(index)),
            Some((index, false)) => self.select_preset(index),
            None => {}
        }
    }

    fn select_preset(&mut self, index: u64) {
        let running = self.connection.as_ref().is_some_and(|c| c.live.running());
        let Some(connection) = self.connection.as_mut().filter(|_| running) else { return };
        connection.live.select(0, index);
        // Knobs stay read-only until the pedal reports the new preset and it is shown.
        connection.addresses.clear();
        self.status = format!("Switching to {}…", nx_proto::slot_label(index));
    }

    /// Goes elsewhere, first asking what to do with unsaved edits to the library file shown.
    fn leave(&mut self, to: Leave) {
        if self.unsaved {
            self.leaving = Some(to);
        } else {
            self.go(to);
        }
    }

    fn go(&mut self, to: Leave) {
        // Opening a file clears `unsaved` only once it opened, so a file that does not open keeps
        // the edits it could not replace.
        match to {
            Leave::Open(path) => self.open_file(path),
            Leave::Pedal => {
                self.unsaved = false;
                self.show_pedal();
            }
            Leave::Select(index) => {
                self.unsaved = false;
                self.show_pedal();
                self.select_preset(index);
            }
            Leave::Quit => self.closing = true,
        }
    }

    /// Writes the library file shown; false, with the reason in the status, when that failed.
    fn save_file(&mut self) -> bool {
        let (Some(path), Some(preset)) = (&self.showing_file, &self.preset) else { return false };
        match crate::library::save(path, preset) {
            Ok(()) => {
                self.status = format!("Saved {}", preset.name());
                self.unsaved = false;
                self.rescan();
                true
            }
            Err(e) => {
                self.status = format!("Not saved: {e}");
                false
            }
        }
    }

    /// Asks whether to save the library file's edits before going on.
    fn leave_modal(&mut self, ctx: &egui::Context) {
        let Some(to) = self.leaving.clone() else { return };
        let name = self.preset.as_ref().map_or_else(String::new, |p| p.name().to_owned());
        let mut choice = None;
        let frame = Frame::new().fill(theme::PANEL).corner_radius(CornerRadius::same(18)).inner_margin(Margin::same(24));
        let modal = egui::Modal::new(egui::Id::unique("unsaved-changes")).frame(frame).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.spacing_mut().item_spacing.y = 12.0;
            ui.label(RichText::new(format!("Save the changes to {name}?")).font(theme::semibold(20.0)));
            ui.label(RichText::new("They are only in nx-edit; the library file still has the old version.").font(theme::regular(13.0)).color(theme::TEXT_DIM));
            ui.horizontal(|ui| {
                let button = |text: &str, primary: bool| {
                    let label = RichText::new(text).font(theme::semibold(15.0));
                    let button = if primary { egui::Button::new(label.color(theme::BG)).fill(theme::ACCENT) } else { egui::Button::new(label) };
                    button.corner_radius(CornerRadius::same(255)).min_size(vec2(100.0, 40.0))
                };
                if ui.add(button("Save and continue", true)).clicked() {
                    choice = Some(true);
                }
                if ui.add(button("Discard", false)).clicked() {
                    choice = Some(false);
                }
                if ui.add(button("Cancel", false)).clicked() {
                    self.leaving = None;
                }
            });
        });
        if modal.should_close() {
            self.leaving = None;
        }
        match choice {
            Some(true) if self.save_file() => {
                self.leaving = None;
                self.go(to);
            }
            Some(false) => {
                self.leaving = None;
                self.go(to);
            }
            _ => {}
        }
    }

    fn library(&mut self, ui: &mut Ui) {
        let dir = crate::library::default_dir();
        ui.horizontal(|ui| {
            if ui.button("Rescan").clicked() {
                self.rescan();
            }
            if ui.button("Open folder").clicked() {
                open_in_file_manager(&dir);
            }
        });
        ui.label(RichText::new("Drop .hlx files on the window to add them. Right-click a preset to rename or delete it.").font(theme::regular(12.0)).color(theme::TEXT_DIM));
        if self.library.entries.is_empty() {
            ui.label(RichText::new(format!("No presets in {}", dir.display())).color(theme::TEXT_DIM));
        }
        let mut action = None;
        egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
            let mut folder = None;
            for entry in &self.library.entries {
                if folder != Some(&entry.folder) {
                    folder = Some(&entry.folder);
                    ui.add_space(4.0);
                    ui.label(RichText::new(&entry.folder).font(theme::semibold(12.0)).color(theme::TEXT_DIM));
                }
                if let Some((path, name)) = self.renaming.as_mut().filter(|(p, _)| p == &entry.path) {
                    ui.horizontal(|ui| {
                        let edit = ui.add(egui::TextEdit::singleline(name).desired_width(ui.available_width() - 110.0));
                        edit.request_focus();
                        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if ui.button("Rename").clicked() || enter {
                            action = Some(LibraryAction::Rename(path.clone(), name.clone()));
                        }
                        if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            action = Some(LibraryAction::CancelRename);
                        }
                    });
                    continue;
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
                    action = Some(LibraryAction::Open(entry.path.clone()));
                }
                response.context_menu(|ui| {
                    if ui.button("Rename…").clicked() {
                        action = Some(LibraryAction::StartRename(entry.path.clone(), entry.name.clone()));
                        ui.close();
                    }
                    if ui.button("Move to library trash").clicked() {
                        action = Some(LibraryAction::Trash(entry.path.clone()));
                        ui.close();
                    }
                    if ui.button("Show in folder").clicked() {
                        if let Some(parent) = entry.path.parent() {
                            open_in_file_manager(parent);
                        }
                        ui.close();
                    }
                });
            }
        });
        if !self.library.skipped.is_empty() {
            ui.label(RichText::new(format!("{} files could not be read", self.library.skipped.len())).font(theme::regular(12.0)).color(theme::TEXT_DIM));
        }
        let shown = |path: &std::path::PathBuf| self.showing_file.as_ref() == Some(path);
        match action {
            Some(LibraryAction::Open(path)) => self.leave(Leave::Open(path)),
            Some(LibraryAction::StartRename(path, name)) => self.renaming = Some((path, name)),
            Some(LibraryAction::CancelRename) => self.renaming = None,
            Some(LibraryAction::Rename(path, name)) => {
                let is_shown = shown(&path);
                match crate::library::rename(&path, &name) {
                    Ok(()) => {
                        self.status = format!("Renamed to {}", name.trim());
                        // Unsaved edits keep the new name when they are saved later.
                        if let Some(preset) = self.preset.as_mut().filter(|_| is_shown) {
                            preset.set_name(name.trim());
                        }
                    }
                    Err(e) => self.status = e,
                }
                self.renaming = None;
                self.rescan();
            }
            Some(LibraryAction::Trash(path)) => {
                let is_shown = shown(&path);
                match crate::library::trash(&dir, &path) {
                    Ok(kept) => {
                        if is_shown {
                            self.close_file();
                        }
                        self.status = format!("Moved to {}", kept.display());
                    }
                    Err(e) => self.status = e,
                }
                self.rescan();
            }
            None => {}
        }
    }

    fn rescan(&mut self) {
        self.library = crate::library::scan(&crate::library::default_dir());
    }

    /// Copies `.hlx` files dropped on the window into the library.
    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if dropped.is_empty() {
            return;
        }
        let dir = crate::library::default_dir();
        let (mut added, mut failed) = (0, Vec::new());
        for path in dropped {
            match crate::library::import(&dir, &path) {
                Ok(_) => added += 1,
                Err(e) => failed.push(e),
            }
        }
        self.status = match failed.first() {
            None => format!("Added {added} preset(s) to the library"),
            Some(e) => format!("Added {added}, {} not added: {e}", failed.len()),
        };
        self.rescan();
        self.panel = Panel::Library;
        self.screen = Screen::Editor;
    }

    pub fn open_file(&mut self, path: std::path::PathBuf) {
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
                self.unsaved = false;
                self.history.clear();
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
        self.history.clear();
        self.status = "Connected · changes not saved".to_owned();
    }

    /// Stops showing a library file that is gone, for the pedal's preset when there is one.
    fn close_file(&mut self) {
        self.unsaved = false;
        if self.pedal_preset.is_some() {
            self.show_pedal();
            return;
        }
        self.preset = None;
        self.showing_file = None;
        self.selected_block = None;
        self.drag = None;
        self.history.clear();
    }

    /// The connection, when what is shown is the pedal's preset and its worker still runs; a
    /// library file never reaches it.
    fn live(&self) -> Option<&Connection> {
        self.connection.as_ref().filter(|c| self.showing_file.is_none() && c.live.running())
    }

    /// Whether knobs, block switches and snapshots of what is shown can be changed: a library file
    /// always, the pedal's preset while connected.
    fn editable(&self) -> bool {
        self.showing_file.is_some() || self.live().is_some()
    }

    fn toggle_block(&mut self, key: &str) {
        let Some(block) = self.preset.as_ref().and_then(|p| p.blocks().into_iter().find(|b| b.key == key)) else { return };
        if self.showing_file.is_some() {
            if let Some(preset) = self.preset.as_mut() {
                preset.set_enabled(key, !block.enabled);
            }
            self.unsaved = true;
            return;
        }
        let Some(connection) = self.live() else { return };
        let Some(slot) = connection.addresses.get(key).map(|a| a.slot) else { return };
        connection.live.set_enabled(connection.generation, slot, !block.enabled);
        if let Some(preset) = self.preset.as_mut() {
            preset.set_enabled(key, !block.enabled);
        }
    }

    /// Shows another snapshot. In a library file this alone does not count as an unsaved edit; the
    /// snapshot shown is written along with the next save.
    fn select_snapshot(&mut self, index: usize) {
        if self.showing_file.is_some() {
            if let Some(preset) = self.preset.as_mut() {
                preset.select_snapshot(index);
            }
            return;
        }
        let Some(connection) = self.live() else { return };
        connection.live.select_snapshot(index as u64);
        if let Some(preset) = self.preset.as_mut() {
            preset.set_current_snapshot(index);
        }
    }

    /// Applies what the pedal reported, in its order.
    fn take_updates(&mut self) {
        let Some(connection) = self.connection.as_ref() else { return };
        for update in connection.live.updates() {
            match update {
                Update::Changed(generation, change) => self.apply_change(generation, change),
                Update::Loaded(loaded) => self.apply_loaded(loaded),
            }
        }
    }

    /// A parameter change reported for the preset shown, skipping the knob being dragged here so its
    /// own echoes do not pull it back. A change from an older generation belongs to another preset.
    fn apply_change(&mut self, generation: u64, change: ParamChange) {
        let Some(connection) = self.connection.as_ref().filter(|c| c.generation == generation) else { return };
        let found = connection.addresses.iter().find(|(_, a)| a.slot == change.slot && a.sub == change.sub);
        let Some((key, name)) = found.and_then(|(key, a)| Some((key.clone(), a.params.get(change.param as usize)?.clone()))) else { return };
        if self.drag.as_ref().is_some_and(|d| d.block == key && d.param == name) {
            return;
        }
        let target = if self.showing_file.is_some() { self.pedal_preset.as_mut() } else { self.preset.as_mut() };
        if let Some(preset) = target {
            preset.set_param(&key, &name, serde_json::json!(change.value));
        }
    }

    /// Shows the preset the pedal holds after a switch, by a click here or a footswitch, or after a
    /// snapshot or block change, which keeps the selected block. While a library file is shown, only
    /// the kept pedal preset is updated, so a footswitch never throws away edits to the file. A
    /// preset nx-edit cannot show leaves the knobs read-only.
    fn apply_loaded(&mut self, loaded: Loaded) {
        let Some(connection) = self.connection.as_mut() else { return };
        let Loaded { generation, preset: loaded } = loaded;
        let same_preset = loaded.index.is_some() && loaded.index == connection.current;
        connection.generation = generation;
        connection.current = loaded.index;
        let shown = loaded.to_hlx(&connection.syms).and_then(|hlx| Ok((hlx, loaded.addresses(&connection.syms)?)));
        let (hlx, addresses) = match shown {
            Ok(shown) => shown,
            Err(e) => {
                connection.addresses.clear();
                self.status = format!("HX Stomp: {e}; knobs are read-only");
                return;
            }
        };
        connection.addresses = addresses;
        if let Some(entry) = connection.presets.iter_mut().find(|p| Some(p.index) == loaded.index) {
            entry.name = loaded.name.clone();
        }
        if self.showing_file.is_some() {
            self.pedal_preset = Some(hlx);
            return;
        }
        self.history.clear();
        let keep = same_preset && self.selected_block.as_ref().is_some_and(|k| hlx.blocks().iter().any(|b| &b.key == k));
        if !keep {
            self.selected_block = hlx.blocks().first().map(|b| b.key.clone());
            self.drag = None;
        }
        self.preset = Some(hlx);
        self.showing_file = None;
        self.pedal_preset = None;
        self.status = "Connected · changes not saved".to_owned();
    }

    fn snapshots(&mut self, ui: &mut Ui) {
        let live = self.editable();
        let Some(preset) = &self.preset else { return };
        let current = preset.current_snapshot();
        let mut pick = None;
        for (i, name) in preset.snapshot_names().iter().enumerate() {
            let active = i == current;
            let sense = if live { Sense::click() } else { Sense::hover() };
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), sense);
            if response.clicked() && !active {
                pick = Some(i);
            }
            let painter = ui.painter();
            let fill = if active || (live && response.hovered()) { theme::RAISED_HI } else { theme::RAISED };
            painter.rect_filled(rect, CornerRadius::same(12), fill);
            if active {
                painter.rect_stroke(rect, CornerRadius::same(12), Stroke::new(2.0, theme::ACCENT), StrokeKind::Inside);
            }
            let badge = Rect::from_center_size(pos2(rect.left() + 27.0, rect.center().y), vec2(30.0, 30.0));
            painter.rect_filled(badge, CornerRadius::same(9), if active { theme::ACCENT } else { theme::BG });
            painter.text(badge.center(), Align2::CENTER_CENTER, (i + 1).to_string(), mono(13.0), if active { theme::BG } else { theme::TEXT_DIM });
            painter.text(pos2(rect.left() + 54.0, rect.center().y), Align2::LEFT_CENTER, name, theme::semibold(14.0), theme::TEXT);
            ui.add_space(6.0);
        }
        if let Some(index) = pick {
            self.select_snapshot(index);
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

/// What a click in the library list asked for.
enum LibraryAction {
    Open(std::path::PathBuf),
    StartRename(std::path::PathBuf, String),
    Rename(std::path::PathBuf, String),
    CancelRename,
    Trash(std::path::PathBuf),
}

fn open_in_file_manager(dir: &std::path::Path) {
    _ = std::process::Command::new("xdg-open").arg(dir).spawn();
}

/// What a click in the chain asked for.
enum ChainClick {
    Select(String),
    Toggle(String),
}

/// The centre of a block tile's power icon.
fn power_center(rect: Rect) -> Pos2 {
    pos2(rect.right() - 20.0, rect.top() + 22.0)
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

pub(crate) fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
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

pub(crate) fn chip(ui: &mut Ui, text: &str) {
    Frame::new().fill(theme::RAISED).corner_radius(CornerRadius::same(255)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.label(RichText::new(text).font(theme::regular(12.0)).color(theme::TEXT_DIM));
    });
}

pub(crate) fn dot(ui: &mut Ui, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    ui.painter().circle_filled(rect.center(), size / 2.0, color);
}

pub(crate) fn mono(size: f32) -> FontId {
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

fn chain_icon(painter: &egui::Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(2.0, color);
    for dx in [-7.0, 7.0] {
        painter.rect_stroke(Rect::from_center_size(center + vec2(dx, 0.0), vec2(6.0, 6.0)), CornerRadius::same(2), stroke, StrokeKind::Middle);
    }
    painter.line_segment([center - vec2(4.0, 0.0), center + vec2(4.0, 0.0)], stroke);
}

fn cloud_icon(painter: &egui::Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(2.0, color);
    let arc = |c: Pos2, r: f32, from: f32, to: f32| -> Vec<Pos2> {
        (0..=16).map(|i| {
            let a = (from + (to - from) * i as f32 / 16.0).to_radians();
            c + vec2(a.cos(), a.sin()) * r
        }).collect()
    };
    painter.add(Shape::line(arc(center + vec2(-4.0, 1.0), 5.0, 90.0, 270.0), stroke));
    painter.add(Shape::line(arc(center + vec2(1.0, -1.0), 6.0, 190.0, 340.0), stroke));
    painter.add(Shape::line(arc(center + vec2(5.5, 2.0), 4.0, 270.0, 450.0), stroke));
    painter.line_segment([center + vec2(-4.0, 6.0), center + vec2(5.5, 6.0)], stroke);
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
        self.take_updates();
        self.take_dropped_files(ui.ctx());
        self.edit_keys(ui.ctx());
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) && self.unsaved && !self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.leaving = Some(Leave::Quit);
        }
        self.leave_modal(&ctx);
        if self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if let Some(connection) = &self.connection {
            if let Some(error) = connection.live.errors().pop() {
                self.status = format!("HX Stomp: {error}");
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        egui::Panel::top("top").frame(Frame::new().fill(theme::BG).inner_margin(Margin::same(10))).show(ui, |ui| {
            if self.top_bar(ui) {
                self.save_file();
            }
        });
        egui::Panel::left("rail")
            .exact_size(76.0)
            .show_separator_line(false)
            .resizable(false)
            .frame(Frame::new().fill(theme::BG).inner_margin(Margin { left: 10, right: 0, top: 10, bottom: 10 }))
            .show(ui, |ui| self.rail(ui));
        egui::CentralPanel::default().frame(Frame::new().fill(theme::BG).inner_margin(Margin::same(10))).show(ui, |ui| {
            if self.screen == Screen::CustomTone {
                self.customtone.show(ui);
                let saved = self.customtone.take_saved();
                if let Some(last) = saved.last() {
                    self.status = format!("Downloaded to {}", last.display());
                    self.rescan();
                }
                return;
            }
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    fn library_file(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nx-view-{}-{test}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["Revv_DM.hlx", "Mark_Lead.hlx"] {
            std::fs::copy(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nx-hlx/tests/fixtures").join(name), dir.join(name)).unwrap();
        }
        dir.join("Revv_DM.hlx")
    }

    fn editor_on(file: &Path) -> Harness<'static, EditorApp> {
        let file = file.to_path_buf();
        let mut harness = Harness::builder().with_size(vec2(1400.0, 900.0)).build_eframe(move |cc| {
            theme::install(&cc.egui_ctx);
            let mut app = EditorApp::new(None, None, String::new(), None);
            app.open_file(file);
            app
        });
        harness.run();
        harness
    }

    fn drive_on_disk(file: &Path) -> serde_json::Value {
        let preset = Preset::from_slice(&std::fs::read(file).unwrap()).unwrap();
        preset.blocks().into_iter().find(|b| b.key == "block2").unwrap().params.into_iter().find(|(k, _)| k == "Drive").unwrap().1
    }

    #[test]
    fn a_library_file_is_edited_and_saved_with_the_save_button() {
        let file = library_file("save-button");
        let mut harness = editor_on(&file);
        assert!(harness.query_by_label("Save").is_none());
        harness.state_mut().set_param("block2", "Drive", 0.7);
        harness.run();
        harness.get_by_label("Save").click();
        harness.run();
        assert_eq!(drive_on_disk(&file), serde_json::json!(0.7));
        assert!(!harness.state().unsaved);
        assert!(harness.query_by_label("Save").is_none());
    }

    #[test]
    fn ctrl_s_saves_a_library_file() {
        let file = library_file("ctrl-s");
        let mut harness = editor_on(&file);
        harness.state_mut().toggle_block("block2");
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
        harness.run();
        let saved = Preset::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert!(!saved.blocks().into_iter().find(|b| b.key == "block2").unwrap().enabled);
    }

    #[test]
    fn opening_another_file_asks_first_and_discard_keeps_the_old_file() {
        let file = library_file("discard");
        let before = std::fs::read(&file).unwrap();
        let mut harness = editor_on(&file);
        harness.state_mut().set_param("block2", "Drive", 0.7);
        harness.state_mut().leave(Leave::Open(file.with_file_name("Mark_Lead.hlx")));
        harness.run();
        assert_eq!(harness.state().preset.as_ref().unwrap().name(), "Revv DM");
        harness.get_by_label("Discard").click();
        harness.run();
        assert_eq!(harness.state().preset.as_ref().unwrap().name(), "Mark Lead");
        assert_eq!(std::fs::read(&file).unwrap(), before);
    }

    #[test]
    fn a_file_that_does_not_open_keeps_the_edits_unsaved() {
        let file = library_file("open-fails");
        let mut harness = editor_on(&file);
        harness.state_mut().set_param("block2", "Drive", 0.7);
        harness.state_mut().leave(Leave::Open(file.with_file_name("gone.hlx")));
        harness.run();
        harness.get_by_label("Discard").click();
        harness.run();
        assert_eq!(harness.state().showing_file.as_deref(), Some(file.as_path()));
        assert!(harness.state().unsaved);
    }

    #[test]
    fn cancel_stays_on_the_file_with_its_edits() {
        let file = library_file("cancel");
        let mut harness = editor_on(&file);
        harness.state_mut().set_param("block2", "Drive", 0.7);
        harness.state_mut().leave(Leave::Open(file.with_file_name("Mark_Lead.hlx")));
        harness.run();
        harness.get_by_label("Cancel").click();
        harness.run();
        assert!(harness.state().leaving.is_none() && harness.state().unsaved);
        assert_eq!(harness.state().showing_file.as_deref(), Some(file.as_path()));
    }

    #[test]
    fn saving_from_the_question_writes_before_moving_on() {
        let file = library_file("save-then-open");
        let mut harness = editor_on(&file);
        harness.state_mut().set_param("block2", "Drive", 0.7);
        harness.state_mut().leave(Leave::Open(file.with_file_name("Mark_Lead.hlx")));
        harness.run();
        harness.get_by_label("Save and continue").click();
        harness.run();
        assert_eq!(drive_on_disk(&file), serde_json::json!(0.7));
        assert_eq!(harness.state().preset.as_ref().unwrap().name(), "Mark Lead");
    }

    #[test]
    fn undo_takes_back_a_knob_turn_on_a_library_file() {
        let file = library_file("undo");
        let mut harness = editor_on(&file);
        let start = drive_on_disk(&file).as_f64().unwrap();
        harness.state_mut().history.push(Turn { block: "block2".into(), param: "Drive".into(), before: start, after: 0.7 });
        harness.state_mut().set_param("block2", "Drive", 0.7);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        harness.run();
        let preset = harness.state().preset.clone().unwrap();
        let drive = preset.blocks().into_iter().find(|b| b.key == "block2").unwrap().params.into_iter().find(|(k, _)| k == "Drive").unwrap().1;
        assert_eq!(drive.as_f64(), Some(start));
    }
}

#[cfg(test)]
mod pedal_tests {
    use std::path::PathBuf;

    use nx_session::Session;
    use nx_session::fake::FakeDevice;

    use super::*;

    fn connection() -> Option<(Connection, FakeDevice)> {
        let dir = std::env::var_os("NX_HXEDIT_RES").map(PathBuf::from).unwrap_or_else(nx_catalog::default_res_dir);
        let Ok(syms) = nx_catalog::ModelSymbols::load(&dir.join("Helix.sym")) else {
            eprintln!("skipped: no Helix.sym in {} (set NX_HXEDIT_RES)", dir.display());
            return None;
        };
        let device = FakeDevice::new();
        let live = Live::start(Session::connect(device.transport()).unwrap());
        Some((Connection { live, addresses: HashMap::new(), presets: Vec::new(), current: Some(0), generation: 0, syms }, device))
    }

    #[test]
    fn a_preset_switch_on_the_pedal_keeps_the_library_file_and_its_edits() {
        let Some((connection, _device)) = connection() else { return };
        let dir = std::env::temp_dir().join(format!("nx-view-{}-footswitch", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Mark_Lead.hlx");
        std::fs::copy(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nx-hlx/tests/fixtures/Mark_Lead.hlx"), &file).unwrap();
        let mut app = EditorApp::new(None, None, String::new(), Some(connection));
        app.open_file(file.clone());
        app.toggle_block("block0");
        let capture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../captures/02-select-preset-01C.pcapng");
        let switched = crate::device::preset_from_capture(&capture).unwrap();
        app.apply_loaded(Loaded { generation: 1, preset: switched });
        assert_eq!(app.showing_file.as_deref(), Some(file.as_path()));
        assert!(app.unsaved);
        assert_eq!(app.preset.as_ref().unwrap().name(), "Mark Lead");
        assert_eq!(app.pedal_preset.as_ref().unwrap().name(), "Revv DM");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
