//! The CustomTone view: Line 6's shared HX Stomp presets, one page at a time, fetched on a thread
//! so the window never waits on the network.

use std::sync::mpsc::{self, Receiver};

use egui::{CornerRadius, Frame, Margin, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use nx_customtone::{BrowsePage, Query, Sort};

use crate::theme;
use crate::view::{card, chip, dot, mono};

#[derive(Default)]
pub struct CustomToneView {
    query: Query,
    search: String,
    page: Option<BrowsePage>,
    pending: Option<Receiver<Result<BrowsePage, String>>>,
    error: Option<String>,
    open: Option<u64>,
}

impl CustomToneView {
    fn fetch(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let query = self.query.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            _ = tx.send(nx_customtone::fetch(&query).map_err(|e| e.to_string()));
            ctx.request_repaint();
        });
        self.pending = Some(rx);
        self.error = None;
    }

    fn take_result(&mut self) {
        let Some(result) = self.pending.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
        self.pending = None;
        match result {
            Ok(page) => {
                self.query.page = page.page;
                self.page = Some(page);
                self.open = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        if self.page.is_none() && self.pending.is_none() && self.error.is_none() {
            self.fetch(ui.ctx());
        }
        self.take_result();
        card(ui, |ui| {
            self.toolbar(ui);
            ui.add_space(6.0);
            ui.label(
                RichText::new("Downloading needs an HX Stomp registered on your Line 6 account; until then nx-edit can only list tones.")
                    .font(theme::regular(12.0))
                    .color(theme::TEXT_DIM),
            );
            ui.add_space(10.0);
            if let Some(error) = &self.error {
                ui.label(RichText::new(error).color(theme::ACCENT));
            }
            if self.pending.is_some() {
                ui.label(RichText::new("Loading…").color(theme::TEXT_DIM));
            }
            self.list(ui);
        });
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(10.0, 8.0);
            ui.label(RichText::new("CustomTone").font(theme::semibold(16.0)));
            chip(ui, "HX Stomp");
            let mut go = false;
            Frame::new().fill(theme::BG).corner_radius(CornerRadius::same(255)).inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| {
                let response = ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search tones, bands, amps").frame(Frame::NONE).desired_width(240.0));
                go = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            });
            let before = (self.query.sort, self.query.descending);
            egui::ComboBox::from_id_salt("ct-sort").selected_text(format!("Sort: {}", self.query.sort.label())).show_ui(ui, |ui| {
                for sort in Sort::ALL {
                    ui.selectable_value(&mut self.query.sort, sort, sort.label());
                }
            });
            let direction = if self.query.descending { "Highest first" } else { "Lowest first" };
            if ui.button(direction).clicked() {
                self.query.descending = !self.query.descending;
            }
            if go || before != (self.query.sort, self.query.descending) {
                self.query.search = self.search.trim().to_owned();
                self.query.page = 1;
                self.fetch(ui.ctx());
            }
            if let Some(page) = &self.page {
                let (current, pages) = (page.page, page.pages);
                ui.label(RichText::new(format!("{} tones · page {current} of {pages}", page.total)).font(theme::regular(12.0)).color(theme::TEXT_DIM));
                let busy = self.pending.is_some();
                if ui.add_enabled(!busy && current > 1, egui::Button::new("Previous")).clicked() {
                    self.query.page = current - 1;
                    self.fetch(ui.ctx());
                }
                if ui.add_enabled(!busy && current < pages, egui::Button::new("Next")).clicked() {
                    self.query.page = current + 1;
                    self.fetch(ui.ctx());
                }
            }
        });
    }

    fn list(&mut self, ui: &mut Ui) {
        let Some(page) = &self.page else { return };
        let mut toggle = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for tone in &page.tones {
                let open = self.open == Some(tone.id);
                let response = Frame::new()
                    .fill(if open { theme::RAISED_HI } else { theme::RAISED })
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&tone.name).font(theme::semibold(16.0)));
                            ui.label(RichText::new(format!("by {}", tone.author)).font(theme::regular(13.0)).color(theme::TEXT_DIM));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(format!("{} downloads", tone.downloads)).font(mono(12.0)).color(theme::TEXT_DIM));
                                ui.label(RichText::new(&tone.date).font(mono(12.0)).color(theme::TEXT_DIM));
                            });
                        });
                        let tags: Vec<String> = [("Band", &tone.band), ("Guitarist", &tone.guitarist), ("Song", &tone.song)]
                            .into_iter()
                            .filter(|(_, v)| !v.is_empty())
                            .map(|(k, v)| format!("{k}: {v}"))
                            .collect();
                        if !tags.is_empty() {
                            ui.horizontal_wrapped(|ui| {
                                for tag in &tags {
                                    chip(ui, tag);
                                }
                            });
                        }
                        if !tone.style.is_empty() {
                            ui.label(RichText::new(&tone.style).font(theme::regular(13.0)).color(theme::TEXT_MID));
                        }
                        if open {
                            ui.add_space(6.0);
                            if !tone.comment.is_empty() {
                                ui.label(RichText::new(&tone.comment).font(theme::regular(13.0)).color(theme::TEXT_MID));
                            }
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                dot(ui, theme::ACCENT, 8.0);
                                ui.hyperlink_to("Open on line6.com", format!("https://line6.com/customtone/tone/{}/", tone.id));
                            });
                        }
                    })
                    .response
                    .interact(Sense::click());
                if response.hovered() && !open {
                    ui.painter().rect_stroke(response.rect, CornerRadius::same(14), Stroke::new(1.0, theme::SWITCH_OFF), StrokeKind::Inside);
                }
                if response.clicked() {
                    toggle = Some(tone.id);
                }
                ui.add_space(8.0);
            }
        });
        if let Some(id) = toggle {
            self.open = if self.open == Some(id) { None } else { Some(id) };
        }
    }
}
