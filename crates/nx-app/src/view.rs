//! Layout B: HX Edit's arrangement in Scythe's colours. Read-only until the device crates exist.

use egui::{Align2, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use nx_catalog::Catalog;
use nx_hlx::Preset;

use crate::layout::{Tile, display_value, lanes};
use crate::theme;

const TILE: egui::Vec2 = vec2(96.0, 56.0);
const GAP: f32 = 22.0;

pub struct EditorApp {
    preset: Option<Preset>,
    catalog: Option<Catalog>,
    selected_block: Option<String>,
    selected_category: usize,
    status: String,
}

impl EditorApp {
    pub fn new(preset: Option<Preset>, catalog: Option<Catalog>, status: String) -> Self {
        let selected_block = preset.as_ref().and_then(|p| p.blocks().first().map(|b| b.key.clone()));
        Self { preset, catalog, selected_block, selected_category: 1, status }
    }

    fn top_bar(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let name = self.preset.as_ref().map_or("Geen preset", |p| p.name());
            ui.label(egui::RichText::new(name).font(theme::semibold(16.0)));
            if let Some(preset) = &self.preset {
                for (i, snap) in preset.snapshot_names().iter().enumerate() {
                    let (fill, text) = if i == 0 { (theme::ACCENT, theme::BG) } else { (theme::RAISED_HI, theme::TEXT) };
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
        let has_send = preset.blocks().iter().any(|b| b.model.starts_with("HD2_Send"));
        let origin = ui.max_rect().left_top() + vec2(16.0, 24.0);
        let painter = ui.painter().clone();
        let mut clicked = None;
        for (row, lane) in lanes.iter().enumerate() {
            let y = origin.y + row as f32 * (TILE.y + 40.0);
            let first = lane.tiles.first().map_or(0.0, |t| f32::from(t.position));
            let last = lanes.iter().flat_map(|l| &l.tiles).map(|t| t.position).max().map_or(0.0, |p| f32::from(p) + 1.0);
            let lane_rect = Rect::from_min_size(
                pos2(origin.x - 8.0 + first * (TILE.x + GAP), y - 8.0),
                vec2((last - first) * (TILE.x + GAP) + 110.0, TILE.y + 16.0),
            );
            painter.rect_filled(lane_rect, CornerRadius::same(8), theme::LANE);
            let mut previous_end: Option<f32> = None;
            for tile in &lane.tiles {
                let x = origin.x + f32::from(tile.position) * (TILE.x + GAP);
                let rect = Rect::from_min_size(pos2(x, y), TILE);
                if let Some(end) = previous_end {
                    let a = pos2(end + 4.0, y + TILE.y / 2.0);
                    painter.line_segment([a, pos2(x - 4.0, a.y)], Stroke::new(1.5, theme::TEXT_DIM));
                }
                previous_end = Some(rect.right());
                let response = ui.interact(rect, ui.scope_id().with(&tile.block_key), Sense::click());
                if response.clicked() {
                    clicked = Some(tile.block_key.clone());
                }
                self.paint_tile(&painter, rect, tile);
            }
            let end = origin.x + last * (TILE.x + GAP);
            let output = if lane.path == 1 && has_send { "Send" } else { "Main L/R" };
            painter.text(pos2(end, y + TILE.y / 2.0), Align2::LEFT_CENTER, format!("→ {output}"), theme::semibold(12.0), theme::TEXT_DIM);
        }
        if let Some(key) = clicked {
            self.selected_block = Some(key);
        }
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

    fn knobs(&self, ui: &mut Ui) {
        let (Some(preset), Some(key)) = (&self.preset, &self.selected_block) else { return };
        let Some(block) = preset.blocks().into_iter().find(|b| &b.key == key) else { return };
        let model = self.catalog.as_ref().and_then(|c| c.model(&block.model));
        let title = model.and_then(|m| m.name.clone()).unwrap_or_else(|| block.model.clone());
        ui.label(egui::RichText::new(title).font(theme::semibold(14.0)));
        ui.add_space(4.0);
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                for (name, value) in &block.params {
                    let def = model.and_then(|m| m.params.iter().find(|p| &p.id == name));
                    knob(ui, def.map_or(name.as_str(), |d| d.name.as_str()), &display_value(value, def), knob_fraction(value, def));
                }
            });
        });
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

fn knob(ui: &mut Ui, label: &str, value: &str, fraction: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(64.0, 78.0), Sense::hover());
    let painter = ui.painter();
    let center = rect.center_top() + vec2(0.0, 22.0);
    painter.circle_filled(center, 17.0, theme::RAISED_HI);
    painter.circle_stroke(center, 17.0, Stroke::new(2.0, theme::SWITCH_OFF));
    let angle = std::f32::consts::PI * (0.75 + 1.5 * fraction);
    let tip = center + vec2(angle.cos(), angle.sin()) * 13.0;
    painter.line_segment([center, tip], Stroke::new(2.0, theme::ACCENT));
    painter.text(rect.center_top() + vec2(0.0, 48.0), Align2::CENTER_CENTER, label, theme::regular(11.0), theme::TEXT_MID);
    painter.text(rect.center_top() + vec2(0.0, 63.0), Align2::CENTER_CENTER, value, theme::regular(11.0), theme::TEXT_DIM);
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
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
