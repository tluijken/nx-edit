//! Layout B: HX Edit's arrangement in Scythe's colours. Knobs are live while connected to the pedal.

use std::collections::HashMap;

use egui::{Align2, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use nx_catalog::Catalog;
use nx_hlx::Preset;

use crate::device::BlockAddress;
use crate::layout::{Tile, branch, display_value, dragged, lanes};
use crate::live::Live;
use crate::theme;

const TILE: egui::Vec2 = vec2(96.0, 56.0);
/// Tiles narrow down to this to keep the outputs in view; below it the path scrolls.
const MIN_TILE_WIDTH: f32 = 76.0;
const GAP: f32 = 22.0;
/// How far path B sits right of its slots, so the wire from the split has room to bend.
const BRANCH_INSET: f32 = 28.0;
const OUTPUT: egui::Vec2 = vec2(84.0, 28.0);

/// The open session with the pedal and where each block of the shown preset lives on it.
pub struct Connection {
    pub live: Live,
    pub addresses: HashMap<String, BlockAddress>,
}

/// A knob being dragged: its block and parameter, the value it started at, and the pixels moved up.
struct Drag {
    block: String,
    param: String,
    start: f64,
    up: f32,
}

pub struct EditorApp {
    preset: Option<Preset>,
    catalog: Option<Catalog>,
    selected_block: Option<String>,
    selected_category: usize,
    status: String,
    connection: Option<Connection>,
    drag: Option<Drag>,
}

impl EditorApp {
    pub fn new(preset: Option<Preset>, catalog: Option<Catalog>, status: String, connection: Option<Connection>) -> Self {
        let selected_block = preset.as_ref().and_then(|p| p.blocks().first().map(|b| b.key.clone()));
        Self { preset, catalog, selected_block, selected_category: 1, status, connection, drag: None }
    }

    fn top_bar(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let name = self.preset.as_ref().map_or("Geen preset", |p| p.name());
            ui.label(egui::RichText::new(name).font(theme::semibold(16.0)));
            if let Some(preset) = &self.preset {
                let current = preset.current_snapshot();
                for (i, snap) in preset.snapshot_names().iter().enumerate() {
                    let (fill, text) = if i == current { (theme::ACCENT, theme::BG) } else { (theme::RAISED_HI, theme::TEXT) };
                    egui::Frame::new().fill(fill).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::symmetric(9, 3)).show(ui, |ui| {
                        ui.label(egui::RichText::new(snap).color(text).font(theme::semibold(12.0)));
                    });
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(&self.status).color(theme::TEXT_DIM));
            });
        });
    }

    fn browser(&mut self, ui: &mut Ui) {
        let Some(catalog) = &self.catalog else {
            ui.label(egui::RichText::new("Geen HX Edit-data geïmporteerd").color(theme::TEXT_DIM));
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, cat) in catalog.categories().iter().enumerate().skip(1) {
                let color = theme::from_rgb_u32(cat.color);
                let selected = i == self.selected_category;
                let text = egui::RichText::new(&cat.name).color(if selected { theme::ACCENT } else { color });
                if ui.selectable_label(selected, text).clicked() {
                    self.selected_category = i;
                }
                if selected {
                    let ungrouped = [nx_catalog::CategoryGroup { name: String::new(), model_ids: cat.model_ids.clone() }];
                    let groups = if cat.groups.is_empty() { &ungrouped[..] } else { &cat.groups[..] };
                    for group in groups {
                        if !group.name.is_empty() {
                            ui.label(egui::RichText::new(format!("  {}", group.name)).color(theme::TEXT_DIM).font(theme::semibold(11.0)));
                        }
                        for id in &group.model_ids {
                            let name = catalog.model(id).and_then(|m| m.name.as_deref()).unwrap_or(id);
                            ui.label(egui::RichText::new(format!("    {name}")).color(theme::TEXT_MID));
                        }
                    }
                }
            }
        });
    }

    fn signal_path(&mut self, ui: &mut Ui) {
        let Some(preset) = &self.preset else {
            ui.centered_and_justified(|ui| ui.label("Open een .hlx-bestand: nx-edit <preset.hlx>"));
            return;
        };
        let lanes = lanes(preset, self.catalog.as_ref());
        let rejoins = lanes.get(1).is_some_and(|l| l.output == lanes[0].output);
        let tail = if lanes.len() > 1 { BRANCH_INSET } else { 0.0 };
        let last = lanes.iter().flat_map(|l| &l.tiles).map(|t| t.position).max().map_or(0.0, |p| f32::from(p) + 1.0);
        let fixed = 16.0 + tail + OUTPUT.x + 24.0;
        let width = ((ui.available_width() - fixed) / last.max(1.0) - GAP).clamp(MIN_TILE_WIDTH, TILE.x);
        let cell = vec2(width, TILE.y);
        let size = vec2(fixed + last * (cell.x + GAP), 24.0 + lanes.len() as f32 * (cell.y + 40.0));
        let mut clicked = None;
        egui::ScrollArea::horizontal().show(ui, |ui| {
            let (area, _) = ui.allocate_exact_size(size, Sense::hover());
            clicked = self.paint_lanes(ui, area.left_top() + vec2(16.0, 24.0), cell, preset, &lanes, rejoins);
        });
        if let Some(key) = clicked {
            self.selected_block = Some(key);
        }
    }

    fn paint_lanes(&self, ui: &Ui, origin: egui::Pos2, cell: egui::Vec2, preset: &Preset, lanes: &[crate::layout::Lane], rejoins: bool) -> Option<String> {
        let painter = ui.painter();
        let wire = Stroke::new(1.5, theme::TEXT_DIM);
        let mut clicked = None;
        let tail = if lanes.len() > 1 { BRANCH_INSET } else { 0.0 };
        let last = lanes.iter().flat_map(|l| &l.tiles).map(|t| t.position).max().map_or(0.0, |p| f32::from(p) + 1.0);
        for (row, lane) in lanes.iter().enumerate() {
            let y = origin.y + row as f32 * (cell.y + 40.0);
            let inset = if row == 1 { BRANCH_INSET } else { 0.0 };
            let first = lane.tiles.first().map_or(0.0, |t| f32::from(t.position));
            let lane_rect = Rect::from_min_max(
                pos2(origin.x - 8.0 + first * (cell.x + GAP) + inset, y - 8.0),
                pos2(origin.x + last * (cell.x + GAP) + tail + OUTPUT.x + 8.0, y + cell.y + 8.0),
            );
            painter.rect_filled(lane_rect, CornerRadius::same(8), theme::LANE);
            let mut previous_end: Option<f32> = None;
            for tile in &lane.tiles {
                let x = origin.x + f32::from(tile.position) * (cell.x + GAP) + inset;
                let rect = Rect::from_min_size(pos2(x, y), cell);
                if let Some(end) = previous_end {
                    let a = pos2(end + 4.0, y + cell.y / 2.0);
                    painter.line_segment([a, pos2(x - 4.0, a.y)], wire);
                }
                previous_end = Some(rect.right());
                let response = ui.interact(rect, ui.scope_id().with(&tile.block_key), Sense::click());
                if response.clicked() {
                    clicked = Some(tile.block_key.clone());
                }
                self.paint_tile(painter, rect, tile);
            }
            let end = origin.x + last * (cell.x + GAP) + tail;
            let below_rejoins = row == 1 && rejoins;
            if !below_rejoins && let Some(from) = previous_end {
                painter.line_segment([pos2(from + 4.0, y + cell.y / 2.0), pos2(end - 4.0, y + cell.y / 2.0)], wire);
            }
            if !below_rejoins {
                output_label(painter, pos2(end, y + (cell.y - OUTPUT.y) / 2.0), lane.output);
            }
        }
        if let (Some(branch), Some(below)) = (branch(preset), lanes.get(1)) {
            let column = |slot: u8| origin.x + f32::from(slot) * (cell.x + GAP) - GAP / 2.0;
            let (top, bottom) = (origin.y + cell.y / 2.0, origin.y + cell.y + 40.0 + cell.y / 2.0);
            let split = column(branch.split);
            if let Some(first) = below.tiles.first() {
                let x = origin.x + f32::from(first.position) * (cell.x + GAP) + BRANCH_INSET;
                curve(painter, pos2(split, top), pos2(x - 4.0, bottom), wire);
            }
            painter.circle_filled(pos2(split, top), 4.0, theme::TEXT_MID);
            if rejoins && let Some(last) = below.tiles.last() {
                let join = column(branch.join) + BRANCH_INSET;
                let x = origin.x + f32::from(last.position) * (cell.x + GAP) + cell.x + BRANCH_INSET;
                curve(painter, pos2(join, top), pos2(x + 4.0, bottom), wire);
                painter.circle_filled(pos2(join, top), 4.0, theme::TEXT_MID);
            }
        }
        clicked
    }

    fn paint_tile(&self, painter: &egui::Painter, rect: Rect, tile: &Tile) {
        let selected = self.selected_block.as_deref() == Some(tile.block_key.as_str());
        let color = tile.category_color.map_or(theme::TEXT_MID, theme::from_rgb_u32);
        painter.rect_filled(rect, CornerRadius::same(8), if selected { theme::LANE_SELECTED } else { theme::RAISED });
        if selected {
            painter.rect_stroke(rect, CornerRadius::same(8), Stroke::new(2.0, theme::ACCENT), StrokeKind::Inside);
        }
        let dot = if tile.enabled { color } else { theme::SWITCH_OFF };
        painter.circle_filled(rect.left_top() + vec2(12.0, 12.0), 4.0, dot);
        let text_color = if tile.enabled { theme::TEXT } else { theme::TEXT_DIM };
        let galley = painter.layout(tile.label.clone(), theme::regular(11.5), text_color, rect.width() - 12.0);
        painter.galley(rect.left_top() + vec2(6.0, 22.0), galley, text_color);
    }

    fn knobs(&mut self, ui: &mut Ui) {
        let (Some(preset), Some(key)) = (&self.preset, self.selected_block.clone()) else { return };
        let Some(block) = preset.blocks().into_iter().find(|b| b.key == key) else { return };
        let model = self.catalog.as_ref().and_then(|c| c.model(&block.model)).cloned();
        let title = model.as_ref().and_then(|m| m.name.clone()).unwrap_or_else(|| block.model.clone());
        ui.label(egui::RichText::new(title).font(theme::semibold(14.0)));
        ui.add_space(4.0);
        let address = self.connection.as_ref().and_then(|c| c.addresses.get(&key));
        let mut turned = None;
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                for (name, value) in &block.params {
                    let def = model.as_ref().and_then(|m| m.params.iter().find(|p| &p.id == name));
                    let current = value.as_f64().filter(|_| value.is_f64());
                    let live = match (def, current) {
                        (Some(d), Some(v)) if address.and_then(|a| a.param(name)).is_some() => dragged(v, d, 0.0).map(|_| (d, v)),
                        _ => None,
                    };
                    let label = def.map_or(name.as_str(), |d| d.name.as_str());
                    let response = knob(ui, label, &display_value(value, def), knob_fraction(value, def), live.is_some());
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

/// A knob; a live one is dragged up and down to turn it.
fn knob(ui: &mut Ui, label: &str, value: &str, fraction: f32, live: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(64.0, 78.0), if live { Sense::drag() } else { Sense::hover() });
    let response = if live { response.on_hover_cursor(egui::CursorIcon::ResizeVertical) } else { response };
    let painter = ui.painter();
    let center = rect.center_top() + vec2(0.0, 22.0);
    let ring = if live && (response.hovered() || response.dragged()) { theme::ACCENT } else { theme::SWITCH_OFF };
    painter.circle_filled(center, 17.0, theme::RAISED_HI);
    painter.circle_stroke(center, 17.0, Stroke::new(2.0, ring));
    let angle = std::f32::consts::PI * (0.75 + 1.5 * fraction);
    let tip = center + vec2(angle.cos(), angle.sin()) * 13.0;
    painter.line_segment([center, tip], Stroke::new(2.0, theme::ACCENT));
    painter.text(rect.center_top() + vec2(0.0, 48.0), Align2::CENTER_CENTER, label, theme::regular(11.0), theme::TEXT_MID);
    painter.text(rect.center_top() + vec2(0.0, 63.0), Align2::CENTER_CENTER, value, theme::regular(11.0), theme::TEXT_DIM);
    response
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        if let Some(connection) = &self.connection {
            if let Some(error) = connection.live.errors().pop() {
                self.status = format!("HX Stomp: {error}");
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        egui::Panel::top("top").frame(egui::Frame::new().fill(theme::PANEL).inner_margin(10)).show(ui, |ui| self.top_bar(ui));
        egui::Panel::left("browser")
            .resizable(true)
            .default_size(200.0)
            .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(10))
            .show(ui, |ui| self.browser(ui));
        egui::Panel::bottom("knobs")
            .default_size(130.0)
            .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(10))
            .show(ui, |ui| self.knobs(ui));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG)).show(ui, |ui| {
            self.signal_path(ui);
        });
    }
}

/// A wire from a junction on path A down into path B, leaving vertically and arriving level.
fn curve(painter: &egui::Painter, junction: egui::Pos2, end: egui::Pos2, stroke: Stroke) {
    let points = [junction, pos2(junction.x, (junction.y + end.y) / 2.0), pos2(junction.x, end.y), end];
    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(points, false, egui::Color32::TRANSPARENT, stroke));
}

/// The jack a lane ends in, drawn as a pill where its wire stops.
fn output_label(painter: &egui::Painter, at: egui::Pos2, output: &str) {
    let rect = Rect::from_min_size(at, OUTPUT);
    painter.rect_filled(rect, CornerRadius::same(14), theme::RAISED_HI);
    painter.rect_stroke(rect, CornerRadius::same(14), Stroke::new(1.0, theme::TEXT_DIM), StrokeKind::Inside);
    painter.text(rect.center(), Align2::CENTER_CENTER, output, theme::semibold(12.0), theme::TEXT);
}
