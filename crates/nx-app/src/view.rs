//! The editor window after the "Preset editor — Scythe style" design: a top bar, the signal chain
//! card, and below it the block editor beside a snapshots and models panel. Only what works is
//! drawn; knobs are live while connected to the pedal.

use std::collections::HashMap;

use egui::{Align2, Color32, CornerRadius, FontId, Frame, Margin, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use nx_catalog::Catalog;
use nx_hlx::Preset;

use crate::connect::{Connect, Connector, Linked};
use crate::device::BlockAddress;
use crate::history::{History, Turn};
use crate::layout::{
    Branch, Control, Lane, PATH_POSITIONS, Tile, branch, choice_value, chosen, control_for, display_value, dragged, empty_positions, lanes, slot_columns,
    slot_of, typed_value, wire_value,
};
use crate::live::{Live, Loaded, ParamChange, Update};
use crate::notice::Notices;
use crate::theme;

const GAP: f32 = 10.0;
const CARD_RADIUS: u8 = 18;
const ROW_H: f32 = 128.0;
const BLOCK_H: f32 = 104.0;
/// A block column narrower than this makes the chain scroll sideways instead.
const MIN_COLUMN: f32 = 104.0;
const SIDE_W: f32 = 340.0;
const KNOB_CARD: egui::Vec2 = vec2(116.0, 140.0);
/// The `.hlx` key of a delay's or reverb's trails switch.
const TRAILS: &str = "@trails";

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
    /// The document of the preset shown, as the pedal last sent it; moving a block edits a copy.
    pub document: Vec<u8>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    notices: Notices,
    connection: Option<Connection>,
    /// Looks for the pedal while there is no connection; none when nx-edit shows a capture.
    connector: Option<Connector>,
    /// The connection that was lost, while there is no new one.
    lost: Option<Lost>,
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
    /// Edits on the pedal not yet saved to the preset's place, as the pedal reported them or as made
    /// here since.
    pedal_unsaved: bool,
    /// An empty position of path A picked in the chain, for a model to be put there.
    selected_empty: Option<u8>,
    /// The block to select once the pedal reports the preset again, by path and position: one just
    /// put into an empty slot.
    select_after_load: Option<(u8, u8)>,
    /// Where to go once the unsaved edits are saved or discarded, while that is asked.
    leaving: Option<Leave>,
    /// The window may close: the unsaved edits were saved or discarded.
    closing: bool,
    /// A library preset whose place on the pedal is being chosen.
    assigning: Option<crate::assign::Assign>,
    /// Importing HX Edit's model data while there is none.
    setup: crate::setup::Setup,
    /// Why the HX Edit data that is there did not load.
    pub catalog_problem: Option<String>,
}

/// What the pedal held when the connection to it was lost.
struct Lost {
    place: Option<u64>,
    name: String,
    unsaved: bool,
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
        let mut notices = Notices::default();
        if !status.is_empty() {
            notices.info(status);
        }
        Self {
            preset,
            catalog,
            selected_block,
            selected_category: 1,
            notices,
            connection,
            connector: None,
            lost: None,
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
            pedal_unsaved: false,
            selected_empty: None,
            select_after_load: None,
            leaving: None,
            assigning: None,
            closing: false,
            setup: Default::default(),
            catalog_problem: None,
        }
    }

    /// Looks for the pedal with `connect`, now and again whenever there is no connection.
    pub fn connect_with(&mut self, connect: Connect) {
        self.connector = Some(Connector::new(connect));
    }

    /// Draws the top bar; true when its Save button was clicked.
    fn top_bar(&mut self, ui: &mut Ui) -> bool {
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
                if self.can_save() {
                    let button = egui::Button::new(RichText::new("Save").font(theme::semibold(13.0)).color(theme::BG))
                        .fill(theme::ACCENT)
                        .corner_radius(CornerRadius::same(255))
                        .min_size(vec2(72.0, 36.0));
                    let (hint, note) = match self.connection.as_ref().and_then(|c| c.current).filter(|_| self.showing_file.is_none()) {
                        Some(index) => (format!("Save on the pedal at {} (Ctrl+S)", nx_proto::slot_label(index)), "Not saved on the pedal"),
                        None => ("Write the changes to the library file (Ctrl+S)".to_owned(), "Unsaved changes"),
                    };
                    save = ui.add(button).on_hover_text(hint).clicked();
                    ui.label(RichText::new(note).font(theme::regular(12.0)).color(theme::TEXT_DIM));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.connection_pill(ui)
                    && let Some(connector) = self.connector.as_mut()
                {
                    connector.retry_now();
                }
                self.notice_pill(ui);
            });
        });
        save
    }

    /// Whether nx-edit is connected to the pedal, looking for it, or why it is not; true when
    /// clicked to look now.
    fn connection_pill(&self, ui: &mut Ui) -> bool {
        let running = self.connection.as_ref().is_some_and(|c| c.live.running());
        let connector = self.connector.as_ref();
        let (color, text, hint) = if running {
            (theme::GREEN, "HX Stomp connected".to_owned(), None)
        } else if connector.is_some_and(Connector::trying) {
            (theme::ORANGE, "Looking for the HX Stomp…".to_owned(), None)
        } else if let Some(connector) = connector {
            (theme::ARM_OFF, "Not connected · Connect".to_owned(), connector.failure.clone())
        } else {
            (theme::ARM_OFF, "Not connected".to_owned(), None)
        };
        let clickable = !running && connector.is_some();
        let response = pill(ui, |ui| {
            dot(ui, color, 8.0);
            ui.label(RichText::new(&text).font(theme::regular(12.0)).color(theme::TEXT_MID));
        });
        let response = if clickable { response.interact(Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand) } else { response };
        response.widget_info(|| egui::WidgetInfo::labeled(if clickable { egui::Role::Button } else { egui::Role::Label }, true, &text));
        let hint = hint.unwrap_or_else(|| if clickable { "Look for the HX Stomp now".to_owned() } else { String::new() });
        let response = if hint.is_empty() { response } else { response.on_hover_text(hint) };
        clickable && response.clicked()
    }

    /// The latest message, in the error colour for an error, which stays until its × is clicked;
    /// a click on the message lists the recent ones.
    fn notice_pill(&mut self, ui: &mut Ui) {
        let Some(notice) = self.notices.current(std::time::Instant::now()).cloned() else { return };
        let mut dismiss = false;
        let response = pill(ui, |ui| {
            if notice.error {
                let close = ui.add(egui::Button::new(RichText::new("×").font(theme::semibold(14.0)).color(theme::TEXT_DIM)).frame(false));
                if close.on_hover_text("Dismiss").clicked() {
                    dismiss = true;
                }
            }
            let color = if notice.error { theme::ORANGE } else { theme::TEXT_DIM };
            ui.add(egui::Label::new(RichText::new(&notice.text).font(theme::regular(12.0)).color(color)).truncate());
        });
        let response = response.interact(Sense::click()).on_hover_text("Recent messages");
        if dismiss {
            self.notices.dismiss();
        }
        let frame = Frame::new().fill(theme::PANEL).corner_radius(CornerRadius::same(12)).stroke(Stroke::new(1.0, theme::RAISED_HI)).inner_margin(Margin::same(10));
        egui::Popup::menu(&response).align(egui::RectAlign::BOTTOM_END).width(420.0).gap(4.0).frame(frame).show(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let now = std::time::Instant::now();
            for notice in self.notices.recent() {
                let ago = now.duration_since(notice.at).as_secs();
                let when = if ago < 60 { format!("{ago} s ago") } else { format!("{} min ago", ago / 60) };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(when).font(mono(11.0)).color(theme::TEXT_DIM));
                    let color = if notice.error { theme::ORANGE } else { theme::TEXT_MID };
                    ui.label(RichText::new(&notice.text).font(theme::regular(13.0)).color(color));
                });
            }
        });
    }

    /// Follows the connection: errors the pedal reported, a connection that stopped, and a new one
    /// the connector found.
    fn watch_connection(&mut self) {
        if let Some(connection) = &self.connection {
            let errors = connection.live.errors();
            let stopped = !connection.live.running();
            for error in errors {
                self.notices.error(format!("HX Stomp: {error}"));
            }
            if stopped {
                self.lose_connection();
            }
        }
        if self.connection.is_none()
            && let Some(linked) = self.connector.as_mut().and_then(|c| c.poll(std::time::Instant::now()))
        {
            self.apply_link(linked);
        }
    }

    /// The pedal went away: what it showed stays, read-only, until it is back.
    fn lose_connection(&mut self) {
        let Some(connection) = self.connection.take() else { return };
        let name = connection.presets.iter().find(|p| Some(p.index) == connection.current).map(|p| p.name.clone()).unwrap_or_default();
        self.lost = Some(Lost { place: connection.current, name, unsaved: self.pedal_unsaved });
        self.pedal_unsaved = false;
        self.pedal_preset = None;
        self.selected_empty = None;
        self.select_after_load = None;
        if self.panel == Panel::Presets {
            self.panel = Panel::Library;
        }
        self.notices.error("The HX Stomp was disconnected");
        if let Some(connector) = self.connector.as_mut() {
            connector.retry_now();
        }
    }

    /// A connection the connector made: the pedal's preset is shown, unless a library file is.
    fn apply_link(&mut self, linked: Linked) {
        let Linked { connection, preset, modified, notes } = linked;
        let place = connection.current.map(|i| format!(" · {} {}", nx_proto::slot_label(i), preset.name())).unwrap_or_default();
        self.connection = Some(connection);
        self.pedal_unsaved = modified;
        self.lost = None;
        self.notices.info(format!("Connected to the HX Stomp{place}"));
        for note in notes {
            self.notices.error(note);
        }
        if self.showing_file.is_some() {
            self.pedal_preset = Some(preset);
            return;
        }
        self.selected_block = preset.blocks().first().map(|b| b.key.clone());
        self.selected_empty = None;
        self.history.clear();
        self.preset = Some(preset);
        if self.panel == Panel::Library {
            self.panel = Panel::Presets;
        }
    }

    /// What to do when there is nothing to edit yet.
    fn nothing_open(&mut self, ui: &mut Ui) {
        let mut connect = false;
        let mut library = false;
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.label(RichText::new("Nothing open yet").font(theme::semibold(20.0)));
            let how = if self.connector.as_ref().is_some_and(Connector::trying) {
                "nx-edit is looking for the HX Stomp. Plug it in over USB and switch it on, or open a preset from the library."
            } else {
                "Plug in the HX Stomp over USB and switch it on, or open a preset from the library."
            };
            ui.label(RichText::new(how).font(theme::regular(14.0)).color(theme::TEXT_MID));
            ui.horizontal(|ui| {
                let button = |text: &str| egui::Button::new(RichText::new(text).font(theme::semibold(14.0))).corner_radius(CornerRadius::same(255)).min_size(vec2(130.0, 36.0));
                if self.connector.is_some() {
                    connect = ui.add(button("Look for the HX Stomp")).clicked();
                }
                library = ui.add(button("Open the library")).clicked();
            });
        });
        if connect && let Some(connector) = self.connector.as_mut() {
            connector.retry_now();
        }
        if library {
            self.panel = Panel::Library;
        }
    }

    /// Says the pedal is gone and what that means for the edits it had.
    fn lost_banner(&mut self, ui: &mut Ui) {
        let Some(lost) = &self.lost else { return };
        let place = lost.place.map(|i| format!("{} {}", nx_proto::slot_label(i), lost.name)).unwrap_or_else(|| "The preset".to_owned());
        let mut detail = if self.showing_file.is_none() && self.preset.is_some() {
            "What is shown is how it was; it can be edited again once the pedal is back.".to_owned()
        } else {
            String::new()
        };
        if lost.unsaved {
            detail = format!("{place} had edits it had not saved; they stay in its edit buffer while it stays switched on. {detail}");
        }
        let mut reconnect = false;
        Frame::new().fill(theme::with_alpha(theme::ORANGE, 0x22)).corner_radius(CornerRadius::same(CARD_RADIUS)).inner_margin(Margin::same(14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("The HX Stomp is disconnected").font(theme::semibold(15.0)).color(theme::ORANGE));
                    if !detail.is_empty() {
                        ui.label(RichText::new(detail.trim()).font(theme::regular(13.0)).color(theme::TEXT_MID));
                    }
                });
                if self.connector.is_some() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        reconnect = ui.add(egui::Button::new(RichText::new("Reconnect").font(theme::semibold(13.0))).corner_radius(CornerRadius::same(255)).min_size(vec2(100.0, 34.0))).clicked();
                    });
                }
            });
        });
        if reconnect && let Some(connector) = self.connector.as_mut() {
            connector.retry_now();
        }
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
        // The pedal's preset shows every position of path A, so a model can be put in any free one.
        let empties = if self.live().is_some() { empty_positions(preset) } else { Vec::new() };
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Signal chain").font(theme::semibold(16.0)));
                if lanes.len() > 1 {
                    chip(ui, "Path A and B");
                }
            });
            ui.add_space(6.0);
            let used = if empties.is_empty() { slot_columns(lanes, branch) } else { slot_columns(lanes, branch).max(PATH_POSITIONS) };
            let columns = usize::from(used) + 2;
            let column = (ui.available_width() / columns as f32).max(MIN_COLUMN);
            let size = vec2(column * columns as f32, ROW_H * lanes.len() as f32);
            let mut clicked = None;
            egui::ScrollArea::horizontal().show(ui, |ui| {
                let (area, _) = ui.allocate_exact_size(size, Sense::hover());
                let grid = Grid { origin: area.min, column, columns };
                clicked = self.paint_chain(ui, &grid, lanes, branch, rejoins, &empties);
            });
            match clicked {
                Some(ChainClick::Select(key)) => {
                    self.selected_block = Some(key);
                    self.selected_empty = None;
                }
                Some(ChainClick::Toggle(key)) => self.toggle_block(&key),
                Some(ChainClick::Empty(position)) => self.select_empty(position),
                Some(ChainClick::Remove(key)) => self.remove_block(&key),
                Some(ChainClick::Move(key, position)) => self.move_block(&key, position),
                None => {}
            }
        });
    }

    fn paint_chain(&self, ui: &Ui, grid: &Grid, lanes: &[Lane], branch: Option<Branch>, rejoins: bool, empties: &[u8]) -> Option<ChainClick> {
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
        let inset = ((grid.column - 96.0) / 2.0).clamp(4.0, 16.0);
        for &position in empties {
            let rect = grid.cell(usize::from(position) + 1, 0).shrink2(vec2(inset, (ROW_H - BLOCK_H) / 2.0));
            let response = ui.interact(rect, ui.scope_id().with(("empty", position)), Sense::click());
            let response = response.on_hover_text("Put a block here");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Empty slot {}", position + 1)));
            if response.clicked() {
                clicked = Some(ChainClick::Empty(position));
            }
            let selected = self.selected_empty == Some(position);
            let fill = if response.hovered() { theme::RAISED } else { theme::PANEL };
            painter.rect_filled(rect, CornerRadius::same(14), fill);
            let stroke = if selected { Stroke::new(2.0, theme::ACCENT) } else { Stroke::new(1.0, theme::RAISED_HI) };
            painter.rect_stroke(rect, CornerRadius::same(14), stroke, StrokeKind::Inside);
            let plus = if selected { theme::ACCENT } else { theme::TEXT_DIM };
            let c = rect.center();
            painter.line_segment([c - vec2(9.0, 0.0), c + vec2(9.0, 0.0)], Stroke::new(2.0, plus));
            painter.line_segment([c - vec2(0.0, 9.0), c + vec2(0.0, 9.0)], Stroke::new(2.0, plus));
        }
        let mut dragging = None;
        for (row, lane) in lanes.iter().enumerate() {
            for tile in &lane.tiles {
                let rect = grid.cell(usize::from(tile.position) + 1, row).shrink2(vec2(inset, (ROW_H - BLOCK_H) / 2.0));
                // Blocks of path A on the pedal move by dragging them to another position of it.
                let movable = row == 0 && self.live().is_some();
                let sense = if movable { Sense::click_and_drag() } else { Sense::click() };
                let response = ui.interact(rect, ui.scope_id().with(("block", &tile.block_key)), sense);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &tile.label));
                if response.clicked() {
                    clicked = Some(ChainClick::Select(tile.block_key.clone()));
                }
                let pointer = response.interact_pointer_pos();
                if response.dragged()
                    && let Some(at) = pointer
                {
                    dragging = Some((tile, rect, at, grid.position_at(at)));
                }
                if response.drag_stopped()
                    && let Some(position) = pointer.and_then(|at| grid.position_at(at))
                    && position != tile.position
                {
                    clicked = Some(ChainClick::Move(tile.block_key.clone(), position));
                }
                if self.live().is_some() {
                    response.context_menu(|ui| {
                        if ui.button(format!("Remove {}", tile.label)).clicked() {
                            clicked = Some(ChainClick::Remove(tile.block_key.clone()));
                            ui.close();
                        }
                    });
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
        if let Some((tile, rect, at, target)) = dragging {
            if let Some(position) = target.filter(|p| *p != tile.position) {
                let drop = grid.cell(usize::from(position) + 1, 0).shrink2(vec2(inset, (ROW_H - BLOCK_H) / 2.0));
                painter.rect_stroke(drop, CornerRadius::same(14), Stroke::new(2.0, theme::ACCENT), StrokeKind::Inside);
            }
            let ghost = rect.translate(at - rect.center());
            painter.rect_filled(ghost, CornerRadius::same(14), theme::with_alpha(theme::RAISED_HI, 0xd0));
            painter.text(ghost.center(), Align2::CENTER_CENTER, &tile.label, theme::semibold(13.0), theme::TEXT);
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
        if self.selected_block.is_none()
            && let Some(position) = self.selected_empty
        {
            card(ui, |ui| {
                ui.label(RichText::new("Empty slot").font(theme::semibold(22.0)));
                ui.label(RichText::new(format!("Path A · slot {}", slot_of(0, position))).font(theme::regular(13.0)).color(theme::TEXT_DIM));
                ui.add_space(8.0);
                ui.label(RichText::new("Pick a model in Models to put it here.").font(theme::regular(14.0)).color(theme::TEXT_MID));
            });
            return;
        }
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
            let mut cards = block.params.clone();
            cards.extend(block.trails.map(|on| (TRAILS.to_owned(), serde_json::json!(on))));
            for row in cards.chunks(per_row) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for (name, value) in row {
                        let def = model.as_ref().and_then(|m| m.params.iter().find(|p| &p.id == name));
                        let reachable = file || address.as_ref().and_then(|a| a.target(name)).is_some();
                        let label = def.map_or(name.as_str(), |d| d.name.as_str());
                        let choice = def.map(|d| (d, control_for(d, self.catalog.as_ref())));
                        if let Some((def, Control::Buttons(names) | Control::List(names))) = &choice {
                            let list = matches!(choice, Some((_, Control::List(_))));
                            let card = ChoiceCard { width, label, names, current: chosen(value, def), color, enabled: reachable, list };
                            if let Some(index) = card.show(ui) {
                                let before = value.as_bool().map_or_else(|| value.as_f64().unwrap_or_default(), |on| f64::from(u8::from(on)));
                                let after = choice_value(index, def);
                                if (after - before).abs() > 1e-9 {
                                    turned = Some((name.clone(), after));
                                    self.history.push(Turn { block: key.clone(), param: name.clone(), before, after });
                                }
                            }
                            continue;
                        }
                        let options = def.and_then(|d| self.catalog.as_ref()?.options(d.display_type.as_deref()?));
                        let live = match (def, value.as_f64()) {
                            (Some(d), Some(v)) if reachable => dragged(v, d, 0.0).map(|_| (d, v)),
                            _ => None,
                        };
                        let shown = display_value(value, def, options);
                        let response = knob_card(ui, width, label, &shown, knob_fraction(value, def), color, live.is_some());
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

    /// Picks an empty position of path A; the models list then says what to put there.
    fn select_empty(&mut self, position: u8) {
        self.selected_empty = Some(position);
        self.selected_block = None;
        self.panel = Panel::Models;
    }

    /// A model picked in the models list: put into the empty slot picked in the chain, or given to
    /// the selected block, on the pedal's preset. `stereo` picks the stereo variant where there is one.
    fn choose_model(&mut self, id: &str, stereo: bool) {
        let Some(connection) = self.live() else { return };
        let name = self.catalog.as_ref().and_then(|c| c.model(id)).and_then(|m| m.name.clone()).unwrap_or_else(|| id.to_owned());
        let Some(model) = connection.syms.number_for(id, stereo) else {
            self.notices.error(format!("{name} is not in HX Edit's model numbering"));
            return;
        };
        let generation = connection.generation;
        if let Some(position) = self.selected_empty {
            let category = self.catalog.as_ref().and_then(|c| c.model(id)).and_then(|m| m.category);
            let symbol = connection.syms.get(model).map_or(id, |s| s.symbol.as_str());
            let Some(engine) = nx_catalog::engine(category, symbol) else {
                self.notices.error(format!("What the pedal needs to add {name} is not known yet"));
                return;
            };
            connection.live.insert(generation, slot_of(0, position), model, engine);
            self.select_after_load = Some((0, position));
            self.notices.info(format!("Adding {name}…"));
        } else if let Some(address) = self.selected_block.as_ref().and_then(|key| connection.addresses.get(key)) {
            connection.live.change_model(generation, address.slot, model);
            self.notices.info(format!("Changing to {name}…"));
        } else {
            return;
        }
        self.pedal_unsaved = true;
    }

    /// Takes a block out of the pedal's preset.
    fn remove_block(&mut self, key: &str) {
        let Some(connection) = self.live() else { return };
        let Some(slot) = connection.addresses.get(key).map(|a| a.slot) else { return };
        connection.live.remove(connection.generation, slot);
        if self.selected_block.as_deref() == Some(key) {
            self.selected_block = None;
        }
        self.pedal_unsaved = true;
        self.notices.info("Removing the block…");
    }

    /// Moves a block of path A to `position`, trading places with what is there, by writing the
    /// preset back with the two slots swapped (op 21), as HX Edit moves a block.
    fn move_block(&mut self, key: &str, position: u8) {
        let Some(connection) = self.live() else { return };
        let Some(from) = connection.addresses.get(key).map(|a| a.slot) else { return };
        let to = slot_of(0, position);
        if from == to {
            return;
        }
        let moved = nx_proto::Document::decode(&connection.document).and_then(|mut doc| {
            doc.swap_slots(from as usize, to as usize)?;
            doc.encode()
        });
        match moved {
            Ok(document) => {
                connection.live.write_document(connection.generation, document);
                self.select_after_load = Some((0, position));
                self.pedal_unsaved = true;
                self.notices.info("Moving the block…");
            }
            Err(e) => self.notices.error(format!("Cannot move the block: {e}")),
        }
    }

    /// The model's definition of a parameter of a block in the preset shown.
    fn param_def(&self, key: &str, name: &str) -> Option<&nx_catalog::ParamDef> {
        let model = self.preset.as_ref()?.blocks().into_iter().find(|b| b.key == key)?.model;
        self.catalog.as_ref()?.model(&model)?.params.iter().find(|p| p.id == name)
    }

    /// Sets a parameter of the preset shown: on the pedal too for the pedal's preset, until saved
    /// for a library file. `value` is a float, a whole number or 0/1, and is written with the
    /// parameter's own type.
    fn set_param(&mut self, key: &str, name: &str, value: f64) {
        let typed = typed_value(self.param_def(key, name), value);
        if let Some(preset) = self.preset.as_mut() {
            write_param(preset, key, name, typed.clone());
        }
        if self.showing_file.is_some() {
            self.unsaved = true;
            return;
        }
        if let Some(connection) = self.live()
            && let Some(address) = connection.addresses.get(key)
            && let Some(param) = address.target(name)
            && let Some(wire) = wire_value(&typed)
        {
            connection.live.set_value(connection.generation, address.slot, address.sub, param, wire);
            self.pedal_unsaved = true;
        }
    }

    /// Ctrl+Z takes the last knob turn back, Ctrl+Shift+Z or Ctrl+Y makes it again, Ctrl+S saves
    /// what is shown, to its library file or on the pedal; only while no text field has the
    /// keyboard.
    fn edit_keys(&mut self, ctx: &egui::Context) {
        if !self.editable() || self.leaving.is_some() || ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        if self.can_save() && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
            self.save();
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
            Some((index, false)) => self.leave(Leave::Select(index)),
            None => {}
        }
    }

    fn select_preset(&mut self, index: u64) {
        let running = self.connection.as_ref().is_some_and(|c| c.live.running());
        let Some(connection) = self.connection.as_mut().filter(|_| running) else { return };
        connection.live.select(0, index);
        // Knobs stay read-only until the pedal reports the new preset and it is shown.
        connection.addresses.clear();
        self.notices.info(format!("Switching to {}…", nx_proto::slot_label(index)));
    }

    /// Goes elsewhere, first asking what to do with unsaved edits to the library file shown, or to
    /// the pedal's preset when another preset would replace them on the pedal.
    fn leave(&mut self, to: Leave) {
        if self.unsaved || self.pedal_edits_lost_by(&to) {
            self.leaving = Some(to);
        } else {
            self.go(to);
        }
    }

    /// Only a preset switch drops the pedal's edit buffer; snapshots and library files leave it be.
    fn pedal_edits_lost_by(&self, to: &Leave) -> bool {
        matches!(to, Leave::Select(_)) && self.showing_file.is_none() && self.can_save()
    }

    /// The answer to the question `leave` asked: save first or not, then go on.
    fn decide(&mut self, save: bool) {
        let Some(to) = self.leaving.take() else { return };
        if save && !self.save() {
            self.leaving = Some(to);
            return;
        }
        self.go(to);
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

    /// Whether what is shown has edits Save would keep: a library file's, or the pedal's while
    /// connected.
    fn can_save(&self) -> bool {
        if self.showing_file.is_some() { self.unsaved } else { self.pedal_unsaved && self.live().is_some() }
    }

    /// Saves what is shown; false when that could not even be started.
    fn save(&mut self) -> bool {
        if self.showing_file.is_some() { self.save_file() } else { self.save_to_pedal() }
    }

    /// Has the pedal store its edits at the preset's own place under its own name, as HX Edit's
    /// Save does; a message says when the pedal has done so. Commands to the pedal run in order,
    /// so a switch asked for after this happens after the save.
    fn save_to_pedal(&mut self) -> bool {
        let Some(connection) = self.live() else { return false };
        let (Some(index), Some(preset)) = (connection.current, &self.preset) else { return false };
        connection.live.save(0, index, preset.name());
        self.notices.info(format!("Saving to {}…", nx_proto::slot_label(index)));
        true
    }

    /// Writes the library file shown; false, with the reason in a message, when that failed.
    fn save_file(&mut self) -> bool {
        let (Some(path), Some(preset)) = (&self.showing_file, &self.preset) else { return false };
        match crate::library::save(path, preset) {
            Ok(()) => {
                self.notices.info(format!("Saved {}", preset.name()));
                self.unsaved = false;
                self.rescan();
                true
            }
            Err(e) => {
                self.notices.error(format!("Not saved: {e}"));
                false
            }
        }
    }

    /// Asks whether to save the edits to what is shown, a library file or the pedal's preset,
    /// before going on.
    fn leave_modal(&mut self, ctx: &egui::Context) {
        if self.leaving.is_none() {
            return;
        }
        let name = self.preset.as_ref().map_or_else(String::new, |p| p.name().to_owned());
        let (question, detail) = match self.connection.as_ref().and_then(|c| c.current).filter(|_| self.showing_file.is_none()) {
            Some(index) => (
                format!("Save the changes to {name} at {}?", nx_proto::slot_label(index)),
                "They are only in the pedal's edit buffer; switching presets drops them, as a footswitch does.",
            ),
            None => (format!("Save the changes to {name}?"), "They are only in nx-edit; the library file still has the old version."),
        };
        let mut choice = None;
        let frame = Frame::new().fill(theme::PANEL).corner_radius(CornerRadius::same(18)).inner_margin(Margin::same(24));
        let modal = egui::Modal::new(egui::Id::unique("unsaved-changes")).frame(frame).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.spacing_mut().item_spacing.y = 12.0;
            ui.label(RichText::new(question).font(theme::semibold(20.0)));
            ui.label(RichText::new(detail).font(theme::regular(13.0)).color(theme::TEXT_DIM));
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
        if let Some(save) = choice {
            self.decide(save);
        }
    }

    fn start_assign(&mut self, file: std::path::PathBuf, name: String) {
        self.assigning = Some(crate::assign::Assign { file, name, place: None, filter: String::new(), error: None });
    }

    /// Sends the file being assigned to its chosen place, written as HX Edit writes an import; the
    /// dialog stays open with the reason when it cannot be.
    fn send_assign(&mut self) {
        let Some(assign) = self.assigning.as_mut() else { return };
        let Some(place) = assign.place else { return };
        let Some(connection) = self.connection.as_ref() else { return };
        let built = (|| {
            let catalog = self.catalog.as_ref().ok_or("HX Edit's model data is not imported yet")?;
            let bytes = std::fs::read(&assign.file).map_err(|e| format!("{}: {e}", assign.file.display()))?;
            let hlx = Preset::from_slice(&bytes).map_err(|e| e.to_string())?;
            let firmware = nx_proto::Document::decode(&connection.document).and_then(|d| d.preset()).map_err(|e| e.to_string())?.firmware;
            nx_device::import::hlx_document(&hlx, &connection.syms, catalog, &firmware)
        })();
        match built {
            Ok(document) => {
                connection.live.assign(0, place, &assign.name, document);
                self.notices.info(format!("Sending \"{}\" to {}", assign.name, nx_proto::slot_label(place)));
                self.assigning = None;
            }
            Err(e) => assign.error = Some(format!("Cannot send this preset: {e}")),
        }
    }

    /// A preset was written to `index`; the list shows it, and when the pedal holds that place its
    /// preset is read again.
    fn apply_assigned(&mut self, index: u64, presets: Vec<nx_proto::PresetEntry>) {
        let Some(connection) = self.connection.as_mut() else { return };
        if !presets.is_empty() {
            connection.presets = presets;
        }
        let name = connection.presets.iter().find(|p| p.index == index).map(|p| p.name.clone()).unwrap_or_default();
        self.notices.info(format!("\"{name}\" is on {}", nx_proto::slot_label(index)));
        if connection.current == Some(index) {
            connection.live.select(0, index);
        }
    }

    /// Choosing the place on the pedal for a library preset, and saying what it replaces.
    fn assign_modal(&mut self, ctx: &egui::Context) {
        let Some(assign) = self.assigning.as_mut() else { return };
        let presets = self.connection.as_ref().map(|c| c.presets.as_slice()).unwrap_or_default();
        let mut close = false;
        let mut send = false;
        let live = self.connection.as_ref().is_some_and(|c| c.live.running());
        let frame = Frame::new().fill(theme::PANEL).corner_radius(CornerRadius::same(18)).inner_margin(Margin::same(24));
        let modal = egui::Modal::new(egui::Id::unique("assign-to-place")).frame(frame).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.spacing_mut().item_spacing.y = 12.0;
            ui.label(RichText::new(format!("Assign \"{}\" to a place", assign.name)).font(theme::semibold(20.0)));
            ui.label(RichText::new(assign.file.display().to_string()).font(theme::regular(12.0)).color(theme::TEXT_DIM));
            if presets.is_empty() {
                ui.label(RichText::new("Connect the HX Stomp to choose one of its places.").font(theme::regular(14.0)).color(theme::TEXT_DIM));
            } else {
                Frame::new().fill(theme::BG).corner_radius(CornerRadius::same(255)).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut assign.filter).hint_text("Find a place: 03B or a name").frame(Frame::NONE).desired_width(f32::INFINITY));
                });
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for entry in crate::assign::places(presets, &assign.filter) {
                        let chosen = assign.place == Some(entry.index);
                        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                        if chosen {
                            ui.painter().rect_filled(rect, CornerRadius::same(10), theme::RAISED_HI);
                            ui.painter().rect_stroke(rect, CornerRadius::same(10), Stroke::new(2.0, theme::ACCENT), StrokeKind::Inside);
                        } else if response.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(10), theme::RAISED);
                        }
                        let label = nx_proto::slot_label(entry.index);
                        ui.painter().text(rect.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, &label, mono(13.0), if chosen { theme::ACCENT } else { theme::TEXT_DIM });
                        ui.painter().text(rect.left_center() + vec2(60.0, 0.0), Align2::LEFT_CENTER, &entry.name, theme::regular(14.0), theme::TEXT_MID);
                        if response.clicked() {
                            assign.place = Some(entry.index);
                        }
                    }
                });
                if let Some(place) = presets.iter().find(|p| Some(p.index) == assign.place) {
                    ui.label(RichText::new(crate::assign::replacing(place, &assign.name)).font(theme::regular(14.0)).color(theme::TEXT));
                }
            }
            if let Some(error) = &assign.error {
                ui.label(RichText::new(error).font(theme::regular(14.0)).color(theme::ORANGE));
            }
            ui.horizontal(|ui| {
                let label = assign.place.map_or_else(|| "Replace".to_owned(), |p| format!("Replace {}", nx_proto::slot_label(p)));
                let replace = egui::Button::new(RichText::new(label).font(theme::semibold(15.0)).color(theme::BG))
                    .fill(theme::ACCENT)
                    .corner_radius(CornerRadius::same(255))
                    .min_size(vec2(130.0, 40.0));
                if ui.add_enabled(live && assign.place.is_some(), replace).clicked() {
                    send = true;
                }
                let cancel = egui::Button::new(RichText::new("Cancel").font(theme::semibold(15.0))).corner_radius(CornerRadius::same(255)).min_size(vec2(100.0, 40.0));
                if ui.add(cancel).clicked() {
                    close = true;
                }
            });
        });
        if send {
            self.send_assign();
        } else if close || modal.should_close() {
            self.assigning = None;
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
                    if ui.button("Assign to a place on the pedal…").clicked() {
                        action = Some(LibraryAction::Assign(entry.path.clone(), entry.name.clone()));
                        ui.close();
                    }
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
            Some(LibraryAction::Assign(path, name)) => self.start_assign(path, name),
            Some(LibraryAction::StartRename(path, name)) => self.renaming = Some((path, name)),
            Some(LibraryAction::CancelRename) => self.renaming = None,
            Some(LibraryAction::Rename(path, name)) => {
                let is_shown = shown(&path);
                match crate::library::rename(&path, &name) {
                    Ok(()) => {
                        self.notices.info(format!("Renamed to {}", name.trim()));
                        // Unsaved edits keep the new name when they are saved later.
                        if let Some(preset) = self.preset.as_mut().filter(|_| is_shown) {
                            preset.set_name(name.trim());
                        }
                    }
                    Err(e) => self.notices.error(e),
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
                        self.notices.info(format!("Moved to {}", kept.display()));
                    }
                    Err(e) => self.notices.error(e),
                }
                self.rescan();
            }
            None => {}
        }
    }

    fn rescan(&mut self) {
        self.library = crate::library::scan(&crate::library::default_dir());
    }

    /// Copies `.hlx` files dropped on the window into the library; while HX Edit's data is missing,
    /// an installer or HX Edit folder dropped is imported instead.
    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let mut dropped: Vec<std::path::PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if self.catalog.is_none()
            && let Some(at) = dropped.iter().position(|p| crate::setup::is_import_source(p))
        {
            self.setup.import(dropped.remove(at));
        }
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
        match failed.first() {
            None => self.notices.info(format!("Added {} to the library", presets(added))),
            Some(e) => self.notices.error(format!("Added {}, {} not added: {e}", presets(added), failed.len())),
        }
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
                self.preset = Some(preset);
                self.showing_file = Some(path);
                self.drag = None;
                self.unsaved = false;
                self.history.clear();
            }
            Err(e) => self.notices.error(format!("{}: {e}", path.display())),
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
        self.pedal_unsaved = true;
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

    /// Gives a snapshot another footswitch LED colour, in the file or on the pedal.
    fn set_snapshot_color(&mut self, index: usize, color: u64) {
        if self.showing_file.is_some() {
            if let Some(preset) = self.preset.as_mut() {
                preset.set_snapshot_color(index, color);
            }
            self.unsaved = true;
            return;
        }
        let Some(connection) = self.live() else { return };
        connection.live.set_snapshot_color(index as u64, color);
        self.pedal_unsaved = true;
        if let Some(preset) = self.preset.as_mut() {
            preset.set_snapshot_color(index, color);
        }
    }

    /// Applies what the pedal reported, in its order.
    fn take_updates(&mut self) {
        let Some(connection) = self.connection.as_ref() else { return };
        for update in connection.live.updates() {
            match update {
                Update::Changed(generation, change) => self.apply_change(generation, change),
                Update::Loaded(loaded) => self.apply_loaded(loaded),
                Update::Saved(index) => self.apply_saved(index),
                Update::Assigned { index, presets } => self.apply_assigned(index, presets),
            }
        }
    }

    /// A parameter change reported for the preset shown, skipping the knob being dragged here so its
    /// own echoes do not pull it back. A change from an older generation belongs to another preset.
    fn apply_change(&mut self, generation: u64, change: ParamChange) {
        let Some(connection) = self.connection.as_ref().filter(|c| c.generation == generation) else { return };
        let found = connection.addresses.iter().find(|(_, a)| a.slot == change.slot && a.sub == change.sub);
        let Some((key, name)) = found.and_then(|(key, a)| Some((key.clone(), a.name_of(change.param)?.to_owned()))) else { return };
        let Some(value) = crate::device::json_value(&change.value) else { return };
        if self.drag.as_ref().is_some_and(|d| d.block == key && d.param == name) {
            return;
        }
        let target = if self.showing_file.is_some() { self.pedal_preset.as_mut() } else { self.preset.as_mut() };
        if let Some(preset) = target {
            write_param(preset, &key, &name, value);
        }
        self.pedal_unsaved = true;
    }

    /// The pedal stored its edits at `index`; when that is the preset it holds, nothing is unsaved.
    fn apply_saved(&mut self, index: u64) {
        if self.connection.as_ref().is_some_and(|c| c.current == Some(index)) {
            self.pedal_unsaved = false;
            self.notices.info(format!("Saved to {}", nx_proto::slot_label(index)));
        }
    }

    /// Shows the preset the pedal holds after a switch, by a click here or a footswitch, or after a
    /// snapshot or block change, which keeps the selected block. While a library file is shown, only
    /// the kept pedal preset is updated, so a footswitch never throws away edits to the file. A
    /// preset nx-edit cannot show leaves the knobs read-only.
    fn apply_loaded(&mut self, loaded: Loaded) {
        let Some(connection) = self.connection.as_mut() else { return };
        let Loaded { generation, preset: loaded } = loaded;
        self.pedal_unsaved = loaded.modified;
        let same_preset = loaded.index.is_some() && loaded.index == connection.current;
        connection.generation = generation;
        connection.current = loaded.index;
        let shown = loaded.to_hlx(&connection.syms).and_then(|hlx| Ok((hlx, loaded.addresses(&connection.syms)?)));
        let (hlx, addresses) = match shown {
            Ok(shown) => shown,
            Err(e) => {
                connection.addresses.clear();
                self.notices.error(format!("nx-edit cannot show this preset ({e}); its knobs are read-only"));
                return;
            }
        };
        connection.addresses = addresses;
        connection.document = loaded.document.clone();
        if let Some(entry) = connection.presets.iter_mut().find(|p| Some(p.index) == loaded.index) {
            entry.name = loaded.name.clone();
        }
        if self.showing_file.is_some() {
            self.pedal_preset = Some(hlx);
            return;
        }
        self.history.clear();
        let keep = same_preset && self.selected_block.as_ref().is_some_and(|k| hlx.blocks().iter().any(|b| &b.key == k));
        let placed = self.select_after_load.take().and_then(|(path, position)| hlx.blocks().into_iter().find(|b| b.path == path && b.position == position));
        let still_empty = same_preset && self.selected_empty.is_some_and(|p| empty_positions(&hlx).contains(&p));
        if let Some(block) = placed {
            self.selected_block = Some(block.key);
            self.selected_empty = None;
        } else if still_empty {
            self.selected_block = None;
        } else if !keep {
            self.selected_block = hlx.blocks().first().map(|b| b.key.clone());
            self.selected_empty = None;
            self.drag = None;
        }
        self.preset = Some(hlx);
        self.showing_file = None;
        self.pedal_preset = None;
    }

    fn snapshots(&mut self, ui: &mut Ui) {
        let live = self.editable();
        let Some(preset) = &self.preset else { return };
        let current = preset.current_snapshot();
        let colors = preset.snapshot_colors();
        let mut pick = None;
        let mut recolor = None;
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
            let color = colors.get(i).copied().unwrap_or(0);
            if let Some(chosen) = led_picker(ui, rect, i, color, live) {
                recolor = Some((i, chosen));
            }
            ui.add_space(6.0);
        }
        if let Some((index, color)) = recolor {
            self.set_snapshot_color(index, color);
        } else if let Some(index) = pick {
            self.select_snapshot(index);
        }
    }

    fn models(&mut self, ui: &mut Ui) {
        let Some(catalog) = &self.catalog else {
            ui.label(RichText::new("The models are listed once HX Edit's model data is imported.").color(theme::TEXT_DIM));
            return;
        };
        Frame::new().fill(theme::BG).corner_radius(CornerRadius::same(255)).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search models").frame(Frame::NONE).desired_width(f32::INFINITY));
        });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            // Each chip is one widget of its own size, so the row wraps inside the panel; a framed
            // group of a dot and a label did not wrap and stretched the panel past the window.
            for (i, cat) in catalog.categories().iter().enumerate().skip(1) {
                let active = i == self.selected_category;
                let text = if active { theme::TEXT } else { theme::TEXT_DIM };
                let galley = ui.painter().layout_no_wrap(cat.name.clone(), theme::semibold(12.0), text);
                let size = vec2(galley.size().x + 12.0 + 7.0 + 6.0 + 12.0, galley.size().y + 14.0);
                let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, active, &cat.name));
                ui.painter().rect_filled(rect, CornerRadius::same(255), if active { theme::RAISED_HI } else { theme::RAISED });
                ui.painter().circle_filled(rect.left_center() + vec2(12.0 + 3.5, 0.0), 3.5, theme::from_rgb_u32(cat.color));
                ui.painter().galley(rect.left_center() + vec2(12.0 + 7.0 + 6.0, -galley.size().y / 2.0), galley, text);
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
        // A model can be picked for an empty slot or the selected block of the pedal's preset.
        let target = self.selected_empty.is_some() || self.selected_block.as_ref().is_some_and(|k| self.live().is_some_and(|c| c.addresses.contains_key(k)));
        let pickable = target && self.live().is_some();
        if pickable {
            let hint = if self.selected_empty.is_some() { "Pick a model to put in the empty slot" } else { "Pick a model to replace the selected block's" };
            ui.label(RichText::new(hint).font(theme::regular(12.0)).color(theme::TEXT_DIM));
            ui.add_space(4.0);
        }
        let mut picked = None;
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            let name_of = |id: &String| catalog.model(id).and_then(|m| m.name.clone()).unwrap_or_else(|| id.clone());
            if !needle.is_empty() {
                for cat in catalog.categories().iter().skip(1) {
                    for id in cat.model_ids.iter().filter(|id| name_of(id).to_lowercase().contains(&needle)) {
                        if model_row(ui, &name_of(id), current.as_ref() == Some(id), pickable) {
                            picked = Some((id.clone(), false));
                        }
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
                    if model_row(ui, &name_of(id), current.as_ref() == Some(id), pickable) {
                        picked = Some((id.clone(), group.name == "Stereo"));
                    }
                }
            }
        });
        if let Some((id, stereo)) = picked {
            self.choose_model(&id, stereo);
        }
    }
}

/// What a click in the library list asked for.
enum LibraryAction {
    Open(std::path::PathBuf),
    StartRename(std::path::PathBuf, String),
    Rename(std::path::PathBuf, String),
    CancelRename,
    Trash(std::path::PathBuf),
    Assign(std::path::PathBuf, String),
}

fn open_in_file_manager(dir: &std::path::Path) {
    let opener = if cfg!(windows) { "explorer" } else { "xdg-open" };
    _ = std::process::Command::new(opener).arg(dir).spawn();
}

/// What a click in the chain asked for.
enum ChainClick {
    Select(String),
    Toggle(String),
    /// An empty position of path A.
    Empty(u8),
    Remove(String),
    /// A block dragged to a position of path A.
    Move(String, u8),
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

    /// The position of path A under `at`, if it is over one: column 0 is the input.
    fn position_at(&self, at: Pos2) -> Option<u8> {
        if !(self.origin.y..self.origin.y + ROW_H).contains(&at.y) {
            return None;
        }
        let column = ((at.x - self.origin.x) / self.column).floor();
        (1.0..=f32::from(PATH_POSITIONS)).contains(&column).then(|| column as u8 - 1)
    }
}

pub(crate) fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    Frame::new().fill(theme::PANEL).corner_radius(CornerRadius::same(CARD_RADIUS)).inner_margin(Margin::same(16)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
}

fn pill(ui: &mut Ui, add: impl FnOnce(&mut Ui)) -> egui::Response {
    Frame::new()
        .fill(theme::PANEL)
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                add(ui);
            });
        })
        .response
}

/// "1 preset", "3 presets".
fn presets(n: usize) -> String {
    if n == 1 { "1 preset".to_owned() } else { format!("{n} presets") }
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

/// A model in the models list; clicked when `pickable` and the model was picked.
fn model_row(ui: &mut Ui, name: &str, current: bool, pickable: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), if pickable { Sense::click() } else { Sense::hover() });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, pickable, name));
    if current {
        ui.painter().rect_filled(rect, CornerRadius::same(10), theme::RAISED_HI);
    } else if pickable && response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(10), theme::RAISED);
    }
    let font = if current { theme::semibold(14.0) } else { theme::regular(14.0) };
    ui.painter().text(rect.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, name, font, if current { theme::TEXT } else { theme::TEXT_MID });
    if current {
        let c = rect.right_center() - vec2(20.0, 0.0);
        let check = vec![c + vec2(-6.0, 0.0), c + vec2(-1.5, 4.5), c + vec2(6.0, -5.0)];
        ui.painter().add(Shape::line(check, Stroke::new(2.4, theme::ACCENT)));
    }
    pickable && !current && response.clicked()
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

fn chevron(painter: &egui::Painter, center: Pos2, color: Color32) {
    let stroke = Stroke::new(2.0, color);
    painter.line_segment([center + vec2(-5.0, -2.5), center + vec2(0.0, 2.5)], stroke);
    painter.line_segment([center + vec2(0.0, 2.5), center + vec2(5.0, -2.5)], stroke);
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
/// Writes a parameter into a preset. A delay's or reverb's trails sit under `@trails`, which
/// `Preset::set_param` leaves alone as metadata.
fn write_param(preset: &mut Preset, key: &str, name: &str, value: serde_json::Value) {
    if name == TRAILS {
        if let Some(on) = value.as_bool() {
            preset.set_trails(key, on);
        }
    } else {
        preset.set_param(key, name, value);
    }
}

/// A parameter set by picking one of its named options: a row of buttons, or a drop-down for a
/// `list`.
struct ChoiceCard<'a> {
    width: f32,
    label: &'a str,
    names: &'a [String],
    current: Option<usize>,
    color: Color32,
    enabled: bool,
    list: bool,
}

impl ChoiceCard<'_> {
    /// Draws the card; the option picked, if one was.
    fn show(&self, ui: &mut Ui) -> Option<usize> {
        if self.list {
            return self.show_list(ui);
        }
        if self.names == ["Off", "On"] {
            return self.show_switch(ui);
        }
        let (rect, _) = ui.allocate_exact_size(vec2(self.width, KNOB_CARD.y), Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::same(14), theme::RAISED);
        ui.painter().text(rect.center_top() + vec2(0.0, 104.0), Align2::CENTER_CENTER, self.label, theme::regular(13.0), theme::TEXT_MID);
        let count = self.names.len().max(1);
        // Two options share one pill side by side; more are stacked so their names fit the card.
        let (cols, rows) = if count <= 2 { (count, 1) } else { (1, count) };
        let height = if rows == 1 { 30.0 } else { (78.0 / rows as f32 - 3.0).min(26.0) };
        let top = rect.top() + 12.0 + (78.0 - (height + 3.0) * rows as f32 + 3.0) / 2.0;
        let width = (self.width - 20.0) / cols as f32;
        let mut picked = None;
        for (i, name) in self.names.iter().enumerate() {
            let (col, row) = (i % cols, i / cols);
            let cell = Rect::from_min_size(pos2(rect.left() + 10.0 + width * col as f32, top + (height + 3.0) * row as f32), vec2(width, height));
            let sense = if self.enabled { Sense::click() } else { Sense::hover() };
            let response = ui.interact(cell, ui.next_auto_id().with(i), sense);
            let on = self.current == Some(i);
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, self.enabled, on, name));
            let radius = if cols == 1 {
                CornerRadius::same(8)
            } else if col == 0 {
                CornerRadius { nw: 15, sw: 15, ne: 0, se: 0 }
            } else {
                CornerRadius { nw: 0, sw: 0, ne: 15, se: 15 }
            };
            let fill = if on {
                self.color
            } else if self.enabled && response.hovered() {
                theme::with_alpha(self.color, 0x40)
            } else {
                theme::RAISED_HI
            };
            ui.painter().rect_filled(cell, radius, fill);
            let text = if on { theme::BG } else { theme::TEXT_MID };
            let galley = ui.painter().layout_no_wrap(name.clone(), theme::semibold(12.0), text);
            let font = if galley.size().x > cell.width() - 6.0 { theme::semibold(10.0) } else { theme::semibold(12.0) };
            ui.painter().text(cell.center(), Align2::CENTER_CENTER, name, font, text);
            if response.clicked() && !on {
                picked = Some(i);
            }
        }
        picked
    }

    /// An off/on parameter as a switch, its state written under it the way a knob shows its value;
    /// a click anywhere on the card flips it.
    fn show_switch(&self, ui: &mut Ui) -> Option<usize> {
        let sense = if self.enabled { Sense::click() } else { Sense::hover() };
        let (rect, response) = ui.allocate_exact_size(vec2(self.width, KNOB_CARD.y), sense);
        let on = self.current == Some(1);
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, self.enabled, on, self.label));
        let painter = ui.painter();
        let hot = self.enabled && response.hovered();
        painter.rect_filled(rect, CornerRadius::same(14), if hot { theme::RAISED_HI } else { theme::RAISED });
        let track = Rect::from_center_size(rect.center_top() + vec2(0.0, 52.0), vec2(60.0, 32.0));
        painter.rect_filled(track, CornerRadius::same(16), if on { self.color } else { theme::BG });
        let knob = if on { track.right_center() - vec2(16.0, 0.0) } else { track.left_center() + vec2(16.0, 0.0) };
        painter.circle_filled(knob, 12.0, if on { theme::TEXT } else { theme::TEXT_DIM });
        painter.text(rect.center_top() + vec2(0.0, 104.0), Align2::CENTER_CENTER, self.label, theme::regular(13.0), theme::TEXT_MID);
        painter.text(rect.center_top() + vec2(0.0, 123.0), Align2::CENTER_CENTER, if on { "On" } else { "Off" }, mono(15.0), theme::TEXT);
        (response.clicked()).then_some(usize::from(!on))
    }

    /// The card shows the current option the way a knob shows its value; a click opens the options
    /// in a list styled like the preset list.
    fn show_list(&self, ui: &mut Ui) -> Option<usize> {
        let sense = if self.enabled { Sense::click() } else { Sense::hover() };
        let (rect, response) = ui.allocate_exact_size(vec2(self.width, KNOB_CARD.y), sense);
        let response = if self.enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, self.enabled, self.label));
        let painter = ui.painter();
        let hot = self.enabled && response.hovered();
        painter.rect_filled(rect, CornerRadius::same(14), if hot { theme::RAISED_HI } else { theme::RAISED });
        let shown = self.current.and_then(|i| self.names.get(i)).map_or("?", String::as_str);
        let color = if self.enabled { theme::TEXT } else { theme::TEXT_DIM };
        let galley = painter.layout(shown.to_owned(), theme::semibold(15.0), color, rect.width() - 20.0);
        painter.galley(rect.center_top() + vec2(-galley.size().x / 2.0, 50.0 - galley.size().y / 2.0), galley, color);
        chevron(painter, rect.center_top() + vec2(0.0, 80.0), if hot { self.color } else { theme::TEXT_DIM });
        painter.text(rect.center_top() + vec2(0.0, 104.0), Align2::CENTER_CENTER, self.label, theme::regular(13.0), theme::TEXT_MID);
        let frame = Frame::new()
            .fill(theme::PANEL)
            .corner_radius(CornerRadius::same(12))
            .stroke(Stroke::new(1.0, theme::RAISED_HI))
            .inner_margin(Margin::same(6));
        let mut picked = None;
        let popup = egui::Popup::menu(&response).align(egui::RectAlign::BOTTOM_START).align_alternatives(&[egui::RectAlign::TOP_START]);
        popup.width(rect.width().max(200.0)).gap(4.0).frame(frame).show(|ui| {
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (i, name) in self.names.iter().enumerate() {
                    let current = self.current == Some(i);
                    let (row, clicked) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                    clicked.widget_info(|| egui::WidgetInfo::selected(egui::Role::ListBoxOption, true, current, name));
                    if current {
                        ui.painter().rect_filled(row, CornerRadius::same(8), theme::RAISED_HI);
                    } else if clicked.hovered() {
                        ui.painter().rect_filled(row, CornerRadius::same(8), theme::RAISED);
                    }
                    let font = if current { theme::semibold(14.0) } else { theme::regular(14.0) };
                    let color = if current { theme::ACCENT } else { theme::TEXT_MID };
                    ui.painter().text(row.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, name, font, color);
                    if clicked.clicked() && !current {
                        picked = Some(i);
                    }
                }
            });
        });
        picked
    }
}

/// The footswitch LED colours in HX Edit's menu order, numbered as the pedal numbers them: Red 2
/// and Violet 9 were seen (capture 27), the rest follow the menu. Auto and Off have no colour.
const LED_COLORS: [(&str, Option<Color32>); 12] = [
    ("Auto", None),
    ("White", Some(Color32::from_rgb(0xf4, 0xf4, 0xf0))),
    ("Red", Some(Color32::from_rgb(0xf9, 0x26, 0x72))),
    ("Dark Orange", Some(Color32::from_rgb(0xe8, 0x6a, 0x10))),
    ("Light Orange", Some(Color32::from_rgb(0xfd, 0xa8, 0x4a))),
    ("Yellow", Some(Color32::from_rgb(0xe6, 0xdb, 0x74))),
    ("Green", Some(Color32::from_rgb(0xa6, 0xe2, 0x2e))),
    ("Turquoise", Some(Color32::from_rgb(0x4e, 0xd8, 0xc8))),
    ("Blue", Some(Color32::from_rgb(0x4a, 0x8c, 0xf0))),
    ("Violet", Some(Color32::from_rgb(0xae, 0x81, 0xff))),
    ("Pink", Some(Color32::from_rgb(0xf5, 0x9a, 0xc8))),
    ("Off", None),
];

fn led_name(color: u64) -> &'static str {
    LED_COLORS.get(color as usize).map_or("Unknown", |(name, _)| name)
}

/// A snapshot's LED as a dot: filled in its colour, a ring with an A for Auto, a struck-through
/// ring for Off.
fn led_dot(painter: &egui::Painter, center: Pos2, radius: f32, color: u64) {
    match LED_COLORS.get(color as usize) {
        Some((_, Some(fill))) => {
            painter.circle_filled(center, radius, *fill);
        }
        Some(("Auto", None)) => {
            painter.circle_stroke(center, radius - 1.0, Stroke::new(2.0, theme::TEXT_DIM));
            painter.text(center, Align2::CENTER_CENTER, "A", theme::semibold(radius), theme::TEXT_DIM);
        }
        _ => {
            painter.circle_stroke(center, radius - 1.0, Stroke::new(2.0, theme::SWITCH_OFF));
            let d = (radius - 3.0) * std::f32::consts::FRAC_1_SQRT_2;
            painter.line_segment([center + vec2(-d, d), center + vec2(d, -d)], Stroke::new(2.0, theme::SWITCH_OFF));
        }
    }
}

/// The LED dot at the right of snapshot `index`'s row, with a menu of the colours when `live`;
/// the colour picked, if another.
fn led_picker(ui: &mut Ui, row: Rect, index: usize, color: u64, live: bool) -> Option<u64> {
    let spot = Rect::from_center_size(pos2(row.right() - 28.0, row.center().y), vec2(32.0, 32.0));
    let sense = if live { Sense::click() } else { Sense::hover() };
    let response = ui.interact(spot, ui.scope_id().with(("snapshot-led", index)), sense);
    let response = if live { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response };
    let label = format!("LED colour of snapshot {}: {}", index + 1, led_name(color));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, live, &label));
    if live && response.hovered() {
        ui.painter().circle_filled(spot.center(), 16.0, theme::BG);
    }
    led_dot(ui.painter(), spot.center(), 9.0, color);
    let frame = Frame::new()
        .fill(theme::PANEL)
        .corner_radius(CornerRadius::same(12))
        .stroke(Stroke::new(1.0, theme::RAISED_HI))
        .inner_margin(Margin::same(6));
    let mut picked = None;
    let popup = egui::Popup::menu(&response).align(egui::RectAlign::BOTTOM_END).align_alternatives(&[egui::RectAlign::TOP_END]);
    popup.width(180.0).gap(4.0).frame(frame).show(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (i, (name, _)) in LED_COLORS.iter().enumerate() {
            let current = i as u64 == color;
            let (row, clicked) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
            clicked.widget_info(|| egui::WidgetInfo::selected(egui::Role::ListBoxOption, true, current, *name));
            if current {
                ui.painter().rect_filled(row, CornerRadius::same(8), theme::RAISED_HI);
            } else if clicked.hovered() {
                ui.painter().rect_filled(row, CornerRadius::same(8), theme::RAISED);
            }
            led_dot(ui.painter(), row.left_center() + vec2(18.0, 0.0), 7.0, i as u64);
            let font = if current { theme::semibold(14.0) } else { theme::regular(14.0) };
            let text = if current { theme::ACCENT } else { theme::TEXT_MID };
            ui.painter().text(row.left_center() + vec2(36.0, 0.0), Align2::LEFT_CENTER, *name, font, text);
            if clicked.clicked() && !current {
                picked = Some(i as u64);
            }
        }
    });
    picked
}

/// A parameter's card, dragged up and down when `live`.
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
        self.assign_modal(&ctx);
        if self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.watch_connection();
        if let Some(catalog) = self.setup.take_imported() {
            self.catalog = Some(catalog);
            self.catalog_problem = None;
            self.notices.info("HX Edit's model data is imported");
            if let Some(connector) = self.connector.as_mut() {
                connector.retry_now();
            }
        }
        if self.setup.busy() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.connection.is_some() || self.connector.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        } else if self.notices.current(std::time::Instant::now()).is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
        }
        egui::Panel::top("top").frame(Frame::new().fill(theme::BG).inner_margin(Margin::same(10))).show(ui, |ui| {
            if self.top_bar(ui) {
                self.save();
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
                    self.notices.info(format!("Downloaded to {}", last.path.display()));
                    self.rescan();
                }
                if let Some(tone) = saved.into_iter().rfind(|d| d.assign) {
                    let name = std::fs::read(&tone.path).ok().and_then(|b| Preset::from_slice(&b).ok()).map(|p| p.name().to_owned());
                    self.start_assign(tone.path, name.unwrap_or_default());
                }
                return;
            }
            let lanes = self.preset.as_ref().map(|p| lanes(p, self.catalog.as_ref())).unwrap_or_default();
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = GAP;
                if self.connection.is_none() {
                    self.lost_banner(ui);
                }
                if self.catalog.is_none() {
                    self.setup.show(ui, self.catalog_problem.as_deref());
                }
                if self.preset.is_none() {
                    self.nothing_open(ui);
                }
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

    /// A library file in the editor with HX Edit's real model data, the block `key` selected.
    fn editor_with_catalog(file: &Path, key: &str) -> Option<Harness<'static, EditorApp>> {
        let dir = std::env::var_os("NX_HXEDIT_RES").map(PathBuf::from).unwrap_or_else(nx_catalog::default_res_dir);
        let catalog = Catalog::load(&dir).ok()?;
        let (file, key) = (file.to_path_buf(), key.to_owned());
        let mut harness = Harness::builder().with_size(vec2(1400.0, 900.0)).build_eframe(move |cc| {
            theme::install(&cc.egui_ctx);
            let mut app = EditorApp::new(None, Some(catalog), String::new(), None);
            app.open_file(file);
            app.selected_block = Some(key);
            app
        });
        harness.run();
        Some(harness)
    }

    fn param_of(harness: &Harness<'static, EditorApp>, key: &str, name: &str) -> serde_json::Value {
        let preset = harness.state().preset.clone().unwrap();
        preset.blocks().into_iter().find(|b| b.key == key).unwrap().params.into_iter().find(|(k, _)| k == name).unwrap().1
    }

    #[test]
    fn without_hx_edit_data_a_dropped_installer_is_imported() {
        let mut harness = Harness::builder().with_size(vec2(1200.0, 720.0)).build_eframe(|cc| {
            theme::install(&cc.egui_ctx);
            let mut app = EditorApp::new(None, None, String::new(), None);
            app.setup = crate::setup::Setup::new(std::env::temp_dir().join(format!("nx-view-{}-import", std::process::id())));
            app
        });
        harness.run();
        harness.get_by_label("nx-edit needs HX Edit's model data");
        harness.get_by_label("Nothing open yet");
        #[derive(Debug)]
        struct Dropped(PathBuf);
        impl egui::DroppedFile for Dropped {
            fn path(&self) -> &Path {
                &self.0
            }
            fn bytes(&self) -> Result<Vec<u8>, String> {
                Err("not read".into())
            }
        }
        harness.input_mut().dropped_files.push(std::sync::Arc::new(Dropped("/nonexistent/HX Edit v3.82 Installer.exe".into())));
        harness.step();
        assert!(harness.state().setup.busy() || harness.state().setup.error.is_some(), "the drop started an import");
        assert!(harness.state().library.entries.iter().all(|e| !e.name.contains("Installer")));
    }

    #[test]
    fn a_snapshot_led_colour_is_picked_from_its_dot() {
        let file = library_file("led-colour");
        let mut harness = editor_on(&file);
        harness.state_mut().panel = Panel::Snapshots;
        harness.run();
        harness.get_by_label("LED colour of snapshot 2: Auto").click();
        harness.run();
        harness.get_by_label("Violet").click();
        harness.run();
        assert_eq!(harness.state().preset.as_ref().unwrap().snapshot_colors(), [0, 9, 0]);
        assert_eq!(harness.state().preset.as_ref().unwrap().current_snapshot(), 0, "picking a colour does not switch snapshots");
        assert!(harness.state().unsaved);
        harness.get_by_label("LED colour of snapshot 2: Violet");
    }

    #[test]
    fn a_mic_is_picked_from_a_drop_down() {
        let file = library_file("mic-list");
        let Some(mut harness) = editor_with_catalog(&file, "block6") else { return };
        harness.get_by_label("Mic").click();
        harness.run();
        harness.get_by_label("121 Ribbon").click();
        harness.run();
        assert_eq!(param_of(&harness, "block6", "Mic"), serde_json::json!(5));
        assert!(harness.state().unsaved);
    }

    #[test]
    fn a_switch_is_set_with_its_button() {
        let file = library_file("switch-buttons");
        let Some(mut harness) = editor_with_catalog(&file, "block4") else { return };
        harness.get_by_label("Gating").click();
        harness.run();
        assert_eq!(param_of(&harness, "block4", "DynType"), serde_json::json!(true));
    }

    #[test]
    fn an_off_on_switch_flips_with_a_click() {
        let file = library_file("switch-toggle");
        let Some(mut harness) = editor_with_catalog(&file, "block4") else { return };
        harness.get_by_label("Tempo Sync").click();
        harness.run();
        assert_eq!(param_of(&harness, "block4", "TempoSync1"), serde_json::json!(true));
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

    use egui_kittest::kittest::Queryable;
    use nx_proto::Request;
    use nx_session::Session;
    use nx_session::fake::FakeDevice;

    use super::*;

    pub(super) fn connection() -> Option<(Connection, FakeDevice)> {
        let dir = std::env::var_os("NX_HXEDIT_RES").map(PathBuf::from).unwrap_or_else(nx_catalog::default_res_dir);
        let Ok(syms) = nx_catalog::ModelSymbols::load(&dir.join("Helix.sym")) else {
            eprintln!("skipped: no Helix.sym in {} (set NX_HXEDIT_RES)", dir.display());
            return None;
        };
        let device = FakeDevice::new();
        let live = Live::start(Session::connect(device.transport()).unwrap());
        Some((Connection { live, addresses: HashMap::new(), presets: Vec::new(), current: Some(0), generation: 0, syms, document: Vec::new() }, device))
    }

    #[test]
    fn a_snapshot_colour_on_the_pedal_is_sent_with_op_98() {
        let Some((connection, device)) = connection() else { return };
        let mut app = EditorApp::new(None, None, String::new(), Some(connection));
        let document = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nx-proto/tests/fixtures/02-select-preset-01C.doc")).unwrap();
        app.apply_loaded(Loaded { generation: 0, preset: crate::device::DevicePreset { name: "Revv DM".into(), index: Some(2), document, modified: false } });
        app.set_snapshot_color(2, 2);
        assert_eq!(app.preset.as_ref().unwrap().snapshot_colors(), [0, 0, 2]);
        assert!(app.pedal_unsaved);
        drop(app);
        assert!(device.requests().into_iter().any(|(_, _, r)| r == nx_proto::Request::SetSnapshotColor { index: 2, color: 2 }));
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
        let document = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nx-proto/tests/fixtures/02-select-preset-01C.doc")).unwrap();
        let switched = crate::device::DevicePreset { name: "Revv DM".into(), index: Some(2), document, modified: false };
        app.apply_loaded(Loaded { generation: 1, preset: switched });
        assert_eq!(app.showing_file.as_deref(), Some(file.as_path()));
        assert!(app.unsaved);
        assert_eq!(app.preset.as_ref().unwrap().name(), "Mark Lead");
        assert_eq!(app.pedal_preset.as_ref().unwrap().name(), "Revv DM");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The app showing 03A "NX Test" as the pedal reported it, with `modified` as key 117 said.
    fn on_nx_test(modified: bool) -> Option<(EditorApp, FakeDevice)> {
        let (connection, device) = connection()?;
        let mut app = EditorApp::new(None, None, String::new(), Some(connection));
        let document = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nx-proto/tests/fixtures/18-export-preset.doc")).unwrap();
        let preset = crate::device::DevicePreset { name: "NX Test".into(), index: Some(6), document, modified };
        app.apply_loaded(Loaded { generation: 0, preset });
        Some((app, device))
    }

    fn saves(device: &FakeDevice) -> Vec<Request> {
        device.requests().into_iter().map(|(_, _, r)| r).filter(|r| matches!(r, Request::SavePreset { .. })).collect()
    }

    #[test]
    fn a_knob_turn_on_the_pedal_can_be_saved_to_its_place() {
        let Some((mut app, device)) = on_nx_test(false) else { return };
        assert!(!app.can_save());
        app.set_param("block0", "Bass", 0.3);
        assert!(app.can_save());
        app.save();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while app.can_save() {
            assert!(std::time::Instant::now() < deadline, "the save was never reported");
            std::thread::sleep(std::time::Duration::from_millis(5));
            app.take_updates();
        }
        assert_eq!(saves(&device), [Request::SavePreset { setlist: 0, index: 6, name: "NX Test".into() }]);
        assert_eq!(app.notices.last(), "Saved to 03A");
    }

    #[test]
    fn edits_the_pedal_reports_unsaved_can_be_saved() {
        let Some((app, _device)) = on_nx_test(true) else { return };
        assert!(app.can_save());
    }

    /// The RPC requests the pedal got, once it has had `n` of the kinds in `want`.
    fn sent_once(device: &FakeDevice, want: impl Fn(&Request) -> bool, n: usize) -> Vec<Request> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let sent: Vec<Request> = device.requests().into_iter().map(|(_, _, r)| r).filter(|r| want(r)).collect();
            if sent.len() >= n {
                return sent;
            }
            assert!(std::time::Instant::now() < deadline, "only {sent:?} sent");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn save_or_select(r: &Request) -> bool {
        matches!(r, Request::SavePreset { .. } | Request::SelectPreset { .. })
    }

    #[test]
    fn switching_away_from_unsaved_pedal_edits_asks_first() {
        let Some((mut app, device)) = on_nx_test(true) else { return };
        app.leave(Leave::Select(5));
        assert!(matches!(app.leaving, Some(Leave::Select(5))));
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(device.requests().iter().all(|(_, _, r)| !save_or_select(r)), "nothing is sent before the answer");
    }

    #[test]
    fn save_and_continue_saves_on_the_pedal_before_switching() {
        let Some((mut app, device)) = on_nx_test(true) else { return };
        app.leave(Leave::Select(5));
        app.decide(true);
        let sent = sent_once(&device, save_or_select, 2);
        assert_eq!(
            sent,
            [Request::SavePreset { setlist: 0, index: 6, name: "NX Test".into() }, Request::SelectPreset { setlist: 0, index: 5 }]
        );
    }

    #[test]
    fn discard_switches_without_saving() {
        let Some((mut app, device)) = on_nx_test(true) else { return };
        app.leave(Leave::Select(5));
        app.decide(false);
        assert_eq!(sent_once(&device, save_or_select, 1), [Request::SelectPreset { setlist: 0, index: 5 }]);
    }

    fn real_catalog() -> Option<Catalog> {
        let dir = std::env::var_os("NX_HXEDIT_RES").map(PathBuf::from).unwrap_or_else(nx_catalog::default_res_dir);
        Catalog::load(&dir).ok()
    }

    #[test]
    fn a_list_parameter_goes_to_the_pedal_as_an_integer() {
        let (Some((mut app, device)), Some(catalog)) = (on_nx_test(false), real_catalog()) else { return };
        app.catalog = Some(catalog);
        app.set_param("block1", "Mic", 9.0);
        let sent = sent_once(&device, |r| matches!(r, Request::SetParam { .. }), 1);
        assert_eq!(sent, [Request::SetParam { block: 3, path: 0, param: 0, value: rmpv::Value::from(9) }]);
        let mic = app.preset.as_ref().unwrap().blocks().into_iter().find(|b| b.key == "block1").unwrap().params.into_iter().find(|(k, _)| k == "Mic").unwrap().1;
        assert_eq!(mic, serde_json::json!(9));
    }

    fn delay_trails(app: &EditorApp) -> Option<bool> {
        app.preset.as_ref().unwrap().blocks().into_iter().find(|b| b.key == "block4").unwrap().trails
    }

    #[test]
    fn trails_switch_on_the_pedal_as_the_first_extra_value() {
        let (Some((connection, device)), Some(catalog)) = (connection(), real_catalog()) else { return };
        let mut app = EditorApp::new(None, Some(catalog), String::new(), Some(connection));
        let document = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nx-proto/tests/fixtures/02-select-preset-01C.doc")).unwrap();
        app.apply_loaded(Loaded { generation: 0, preset: crate::device::DevicePreset { name: "Revv DM".into(), index: Some(2), document, modified: false } });
        assert_eq!(delay_trails(&app), Some(true));
        app.set_param("block4", "@trails", 0.0);
        assert_eq!(delay_trails(&app), Some(false));
        let sent = sent_once(&device, |r| matches!(r, Request::SetExtra { .. }), 1);
        assert_eq!(sent, [Request::SetExtra { block: 5, path: 0, index: 0, value: rmpv::Value::Boolean(false) }]);
        app.apply_change(0, ParamChange { slot: 5, sub: 0, param: crate::device::Param::Extra(0), value: rmpv::Value::Boolean(true) });
        assert_eq!(delay_trails(&app), Some(true), "the pedal's own switch is followed");
    }

    fn chain_edit(r: &Request) -> bool {
        matches!(r, Request::InsertBlock { .. } | Request::ChangeModel { .. } | Request::RemoveBlock { .. })
    }

    /// NX Test (amp in slot 2, cab in slot 3) with HX Edit's real model data.
    fn chain_app() -> Option<(EditorApp, FakeDevice)> {
        let catalog = real_catalog()?;
        let (mut app, device) = on_nx_test(false)?;
        app.catalog = Some(catalog);
        Some((app, device))
    }

    #[test]
    fn a_model_picked_for_an_empty_slot_is_put_there() {
        let Some((mut app, device)) = chain_app() else { return };
        app.select_empty(0);
        assert_eq!(app.panel, Panel::Models, "picking a model is what an empty slot is for");
        app.choose_model("HD2_DelaySimpleDelay", false);
        assert_eq!(sent_once(&device, chain_edit, 1), [Request::InsertBlock { block: 1, model: 80, engine: 8, cab: None }]);
    }

    #[test]
    fn a_click_on_an_empty_slot_and_then_a_model_adds_the_block() {
        let Some((app, device)) = chain_app() else { return };
        let mut app = Some(app);
        let mut harness = egui_kittest::Harness::builder().with_size(vec2(1400.0, 1400.0)).build_eframe(move |cc| {
            theme::install(&cc.egui_ctx);
            app.take().unwrap()
        });
        harness.run();
        harness.get_by_label("Empty slot 1").click();
        harness.run();
        assert_eq!((harness.state().selected_empty, harness.state().panel), (Some(0), Panel::Models));
        harness.state_mut().search = "Simple Delay".into();
        harness.run();
        harness.get_all_by_label("Simple Delay").next().unwrap().click();
        harness.run();
        assert!(harness.state().notices.last().starts_with("Adding"), "{}", harness.state().notices.last());
        assert_eq!(sent_once(&device, chain_edit, 1), [Request::InsertBlock { block: 1, model: 80, engine: 8, cab: None }]);
    }

    #[test]
    fn a_model_picked_for_a_block_replaces_its_model() {
        let Some((mut app, device)) = chain_app() else { return };
        app.selected_block = Some("block0".into());
        app.choose_model("HD2_AmpRevvGenRed", false);
        assert_eq!(sent_once(&device, chain_edit, 1), [Request::ChangeModel { block: 2, model: 545, cab: None }]);
    }

    #[test]
    fn removing_a_block_empties_its_slot() {
        let Some((mut app, device)) = chain_app() else { return };
        app.remove_block("block1");
        assert_eq!(sent_once(&device, chain_edit, 1), [Request::RemoveBlock { block: 3 }]);
    }

    #[test]
    fn moving_a_block_writes_the_document_with_its_slots_swapped() {
        let Some((mut app, device)) = chain_app() else { return };
        app.move_block("block0", 0);
        let sent = sent_once(&device, |r| matches!(r, Request::WriteDocument { .. }), 1);
        let [Request::WriteDocument { document }] = &sent[..] else { panic!("{sent:?}") };
        let moved = nx_proto::Document::decode(document).unwrap().preset().unwrap();
        let model_at = |slot| moved.blocks().find(|b| b.slot == slot).map(|b| b.block.model);
        assert_eq!((model_at(1), model_at(2), model_at(3)), (Some(18), None, Some(693)), "the amp moved to slot 1, the cab stayed");
    }

    #[test]
    fn dragging_a_block_onto_an_empty_slot_moves_it_there() {
        let Some((app, device)) = chain_app() else { return };
        let mut app = Some(app);
        let mut harness = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).build_eframe(move |cc| {
            theme::install(&cc.egui_ctx);
            app.take().unwrap()
        });
        harness.run();
        let from = harness.get_by_role_and_label(egui::accesskit::Role::Button, "Cali Rectifire").rect().center();
        let to = harness.get_by_label("Empty slot 1").rect().center();
        harness.hover_at(from);
        harness.run();
        harness.drag_at(from);
        harness.run();
        harness.hover_at(from.lerp(to, 0.5));
        harness.run();
        harness.hover_at(to);
        harness.run();
        harness.drop_at(to);
        harness.run();
        let sent = sent_once(&device, |r| matches!(r, Request::WriteDocument { .. }), 1);
        let [Request::WriteDocument { document }] = &sent[..] else { panic!("{sent:?}") };
        let moved = nx_proto::Document::decode(document).unwrap().preset().unwrap();
        assert_eq!(moved.blocks().find(|b| b.slot == 1).map(|b| b.block.model), Some(18));
    }

    #[test]
    fn a_block_dropped_on_its_own_place_writes_nothing() {
        let Some((mut app, device)) = chain_app() else { return };
        app.move_block("block0", 1);
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(device.requests().iter().all(|(_, _, r)| !matches!(r, Request::WriteDocument { .. })));
    }

    #[test]
    fn a_model_whose_engine_is_unknown_is_not_put_on_the_pedal() {
        let Some((mut app, device)) = chain_app() else { return };
        app.select_empty(0);
        app.choose_model("HD2_ReturnMono1", false);
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(device.requests().iter().all(|(_, _, r)| !chain_edit(r)));
        assert!(app.notices.last().contains("not known yet"), "{}", app.notices.last());
    }

    #[test]
    fn a_switch_in_a_library_file_stays_a_switch() {
        let Some(catalog) = real_catalog() else { return };
        let dir = std::env::temp_dir().join(format!("nx-view-{}-switch", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Revv_DM.hlx");
        std::fs::copy(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nx-hlx/tests/fixtures/Revv_DM.hlx"), &file).unwrap();
        let mut app = EditorApp::new(None, Some(catalog), String::new(), None);
        app.open_file(file);
        app.set_param("block4", "TempoSync1", 1.0);
        let sync = app.preset.as_ref().unwrap().blocks().into_iter().find(|b| b.key == "block4").unwrap().params.into_iter().find(|(k, _)| k == "TempoSync1").unwrap().1;
        assert_eq!(sync, serde_json::json!(true));
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "{what}");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn a_lost_pedal_leaves_its_preset_read_only_and_says_what_its_edits_are() {
        let Some((mut app, device)) = on_nx_test(true) else { return };
        device.on_request(|_, _, r| match r {
            Request::SetParam { .. } => nx_session::fake::Reply::Status(255, rmpv::Value::Nil),
            _ => nx_session::fake::Reply::Ok(rmpv::Value::Nil),
        });
        app.set_param("block0", "Bass", 0.3);
        wait_for("the worker never stopped", || !app.connection.as_ref().unwrap().live.running());
        app.watch_connection();
        assert!(app.connection.is_none());
        assert_eq!(app.preset.as_ref().unwrap().name(), "NX Test", "what was shown stays");
        assert!(!app.editable() && !app.can_save());
        let lost = app.lost.as_ref().expect("the loss is remembered");
        assert_eq!((lost.place, lost.unsaved), (Some(6), true));
        assert_eq!(app.notices.last(), "The HX Stomp was disconnected");
    }

    #[test]
    fn the_connector_shows_the_pedal_it_found() {
        let Some((connection, _device)) = connection() else { return };
        let document = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nx-proto/tests/fixtures/18-export-preset.doc")).unwrap();
        let found = crate::device::DevicePreset { name: "NX Test".into(), index: Some(6), document, modified: true };
        let preset = found.to_hlx(&connection.syms).unwrap();
        let linked = std::sync::Mutex::new(Some(crate::connect::Linked { connection, preset, modified: true, notes: Vec::new() }));
        let mut app = EditorApp::new(None, None, String::new(), None);
        app.connect_with(std::sync::Arc::new(move || linked.lock().unwrap().take().ok_or_else(|| "found once".to_owned())));
        wait_for("never connected", || {
            app.watch_connection();
            app.connection.is_some()
        });
        assert_eq!(app.preset.as_ref().unwrap().name(), "NX Test");
        assert_eq!(app.panel, Panel::Presets);
        assert!(app.can_save(), "the pedal's own unsaved edits count");
    }

    #[test]
    fn switching_from_a_saved_pedal_preset_does_not_ask() {
        let Some((mut app, device)) = on_nx_test(false) else { return };
        app.leave(Leave::Select(5));
        assert!(app.leaving.is_none());
        assert_eq!(sent_once(&device, save_or_select, 1), [Request::SelectPreset { setlist: 0, index: 5 }]);
    }
}

#[cfg(test)]
mod assign_tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    use super::*;

    fn editor(connection: Option<Connection>) -> Harness<'static, EditorApp> {
        let mut connection = connection;
        let mut harness = Harness::builder().with_size(vec2(1400.0, 900.0)).build_eframe(move |cc| {
            theme::install(&cc.egui_ctx);
            let mut app = EditorApp::new(None, None, String::new(), connection.take());
            app.start_assign("customtone/BE100 CH1+2.hlx".into(), "BE100 CH1+2".into());
            app
        });
        harness.run();
        harness
    }

    #[test]
    fn without_the_pedal_the_dialog_asks_to_connect_it() {
        let harness = editor(None);
        harness.get_by_label("Connect the HX Stomp to choose one of its places.");
        assert!(harness.get_by_label("Replace").accesskit_node().is_disabled());
    }

    #[test]
    fn a_chosen_place_says_what_it_replaces_and_cancel_closes() {
        let Some((mut connection, _device)) = super::pedal_tests::connection() else { return };
        connection.presets = (0..9).map(|i| nx_proto::PresetEntry { index: i, name: format!("Preset {i}") }).collect();
        let mut harness = editor(Some(connection));
        harness.state_mut().assigning.as_mut().unwrap().place = Some(7);
        harness.run();
        harness.get_by_label("03B \"Preset 7\" on the HX Stomp is replaced by \"BE100 CH1+2\". The preset now on 03B is gone unless it is in the library.");
        assert!(!harness.get_by_label("Replace 03B").accesskit_node().is_disabled());
        harness.get_by_label("Cancel").click();
        harness.run();
        assert!(harness.state().assigning.is_none());
    }

    fn res_catalog() -> Option<Catalog> {
        let dir = std::env::var_os("NX_HXEDIT_RES").map(std::path::PathBuf::from).unwrap_or_else(nx_catalog::default_res_dir);
        Catalog::load(&dir).ok()
    }

    #[test]
    fn replace_writes_the_file_to_the_place_and_closes() {
        let Some((mut connection, device)) = super::pedal_tests::connection() else { return };
        let Some(catalog) = res_catalog() else { return };
        let fixtures = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        connection.document = std::fs::read(fixtures.join("../nx-proto/tests/fixtures/02-select-preset-01C.doc")).unwrap();
        connection.presets = (0..9).map(|i| nx_proto::PresetEntry { index: i, name: format!("Preset {i}") }).collect();
        let mut harness = editor(Some(connection));
        harness.state_mut().catalog = Some(catalog);
        let assign = harness.state_mut().assigning.as_mut().unwrap();
        assign.file = fixtures.join("../nx-hlx/tests/fixtures/Revv_Home.hlx");
        assign.name = "Revv Home".into();
        assign.place = Some(7);
        harness.run();
        harness.get_by_label("Replace 03B").click();
        harness.run();
        assert!(harness.state().assigning.is_none());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let written = || {
            device.requests().into_iter().find_map(|(_, _, r)| match r {
                nx_proto::Request::WritePreset { index, name, .. } => Some((index, name)),
                _ => None,
            })
        };
        while written().is_none() {
            assert!(std::time::Instant::now() < deadline, "no op 8 sent");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(written(), Some((7, "Revv Home".to_owned())));
    }

    #[test]
    fn a_file_that_cannot_be_sent_keeps_the_dialog_open_with_the_reason() {
        let Some((mut connection, _device)) = super::pedal_tests::connection() else { return };
        connection.presets = (0..9).map(|i| nx_proto::PresetEntry { index: i, name: format!("Preset {i}") }).collect();
        let mut harness = editor(Some(connection));
        harness.state_mut().assigning.as_mut().unwrap().place = Some(7);
        harness.run();
        harness.get_by_label("Replace 03B").click();
        harness.run();
        let error = harness.state().assigning.as_ref().and_then(|a| a.error.clone()).expect("dialog stays open with a reason");
        assert!(error.starts_with("Cannot send this preset"), "{error}");
    }
}
