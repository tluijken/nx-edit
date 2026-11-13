//! The CustomTone view: Line 6's shared HX Stomp presets, one page at a time, and signing in to
//! line6.com. Network work runs on threads so the window never waits; one client carries the
//! site's cookies for both.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use egui::{CornerRadius, Frame, Margin, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use nx_customtone::{Account, BrowsePage, Client, Query, Sort};

use crate::theme;
use crate::view::{card, chip, dot, mono};

enum Reply {
    Page(Result<(BrowsePage, Account), String>),
    SignedIn { user: String, outcome: Result<Account, String> },
    SignedOut,
}

/// The sign-in form while it is open.
#[derive(Default)]
struct SignInForm {
    user: String,
    password: String,
    remember: bool,
}

pub struct CustomToneView {
    client: Arc<Client>,
    replies: (Sender<Reply>, Receiver<Reply>),
    busy: usize,
    query: Query,
    search: String,
    page: Option<BrowsePage>,
    account: Account,
    user: Option<String>,
    form: Option<SignInForm>,
    error: Option<String>,
    open: Option<u64>,
    started: bool,
    signing_in: bool,
}

impl Default for CustomToneView {
    fn default() -> Self {
        Self {
            client: Arc::new(Client::default()),
            replies: mpsc::channel(),
            busy: 0,
            query: Query::default(),
            search: String::new(),
            page: None,
            account: Account::SignedOut,
            user: None,
            form: None,
            error: None,
            open: None,
            started: false,
            signing_in: false,
        }
    }
}

impl CustomToneView {
    fn spawn(&mut self, ctx: &egui::Context, work: impl FnOnce(&Client) -> Reply + Send + 'static) {
        let (client, tx, ctx) = (self.client.clone(), self.replies.0.clone(), ctx.clone());
        std::thread::spawn(move || {
            _ = tx.send(work(&client));
            ctx.request_repaint();
        });
        self.busy += 1;
        self.error = None;
    }

    fn fetch(&mut self, ctx: &egui::Context) {
        let query = self.query.clone();
        self.spawn(ctx, move |client| Reply::Page(client.browse(&query).map_err(|e| e.to_string())));
    }

    fn sign_in(&mut self, ctx: &egui::Context, user: String, password: String, remember: bool) {
        self.signing_in = true;
        self.spawn(ctx, move |client| {
            let outcome = client.sign_in(&user, &password).map_err(|e| e.to_string());
            let outcome = match outcome {
                Ok(account) if remember => crate::account::remember(&user, &password).map(|()| account),
                other => other,
            };
            Reply::SignedIn { user, outcome }
        });
    }

    fn take_replies(&mut self, ctx: &egui::Context) {
        let replies: Vec<Reply> = self.replies.1.try_iter().collect();
        for reply in replies {
            self.busy = self.busy.saturating_sub(1);
            if matches!(reply, Reply::SignedIn { .. }) {
                self.signing_in = false;
            }
            match reply {
                Reply::Page(Ok((page, account))) => {
                    self.query.page = page.page;
                    self.page = Some(page);
                    self.account = account;
                    self.open = None;
                }
                Reply::SignedIn { user, outcome: Ok(account) } => {
                    self.account = account;
                    self.user = Some(user);
                    self.form = None;
                    self.fetch(ctx);
                }
                Reply::SignedOut => {
                    self.account = Account::SignedOut;
                    self.user = None;
                }
                Reply::Page(Err(e)) | Reply::SignedIn { outcome: Err(e), .. } => self.error = Some(e),
            }
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        if !self.started {
            self.started = true;
            match crate::account::remembered() {
                Some((user, password)) => self.sign_in(ui.ctx(), user, password, false),
                None => self.fetch(ui.ctx()),
            }
        }
        self.take_replies(ui.ctx());
        card(ui, |ui| {
            self.toolbar(ui);
            ui.add_space(6.0);
            self.account_line(ui);
            ui.add_space(10.0);
            if let Some(error) = &self.error {
                ui.label(RichText::new(error).color(theme::ACCENT));
            }
            if self.busy > 0 {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0).color(theme::ACCENT));
                    ui.label(RichText::new("Loading…").color(theme::TEXT_DIM));
                });
            }
            self.list(ui);
        });
        if self.form.is_some() {
            self.sign_in_modal(ui.ctx());
        }
    }

    fn account_line(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            let (color, text) = match (&self.account, &self.user) {
                (Account::SignedIn { can_download: true }, user) => (theme::GREEN, format!("Signed in{}", as_user(user))),
                (Account::SignedIn { can_download: false }, user) => (
                    theme::ACCENT,
                    format!("Signed in{} · downloading needs an HX Stomp registered on this account", as_user(user)),
                ),
                (Account::SignedOut, _) => (theme::ARM_OFF, "Not signed in · listing only".to_owned()),
            };
            dot(ui, color, 8.0);
            ui.label(RichText::new(text).font(theme::regular(12.0)).color(theme::TEXT_DIM));
            let signed_in = matches!(self.account, Account::SignedIn { .. });
            if !signed_in && self.form.is_none() && ui.small_button("Sign in").clicked() {
                self.form = Some(SignInForm { remember: true, ..Default::default() });
            }
            if signed_in && ui.small_button("Sign out").clicked() {
                _ = crate::account::forget();
                self.spawn(ui.ctx(), |client| {
                    _ = client.sign_out();
                    Reply::SignedOut
                });
            }
        });
    }

    /// The sign-in form as a modal over the view; while the site answers it shows a spinner and
    /// takes no input.
    fn sign_in_modal(&mut self, ctx: &egui::Context) {
        let signing_in = self.signing_in;
        let error = self.error.clone();
        let Some(form) = self.form.as_mut() else { return };
        let mut submit = None;
        let mut cancel = false;
        let frame = Frame::new().fill(theme::PANEL).corner_radius(CornerRadius::same(18)).inner_margin(Margin::same(24));
        let modal = egui::Modal::new(egui::Id::unique("line6-sign-in")).frame(frame).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.spacing_mut().item_spacing.y = 12.0;
            ui.label(RichText::new("Sign in to line6.com").font(theme::semibold(20.0)));
            ui.label(RichText::new("For CustomTone downloads. The password is sent to line6.com only.").font(theme::regular(13.0)).color(theme::TEXT_DIM));
            let field = || Frame::new().fill(theme::BG).corner_radius(CornerRadius::same(12)).inner_margin(Margin::symmetric(14, 12));
            ui.add_enabled_ui(!signing_in, |ui| {
                field().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut form.user).hint_text("E-mail or user name").font(theme::regular(16.0)).frame(Frame::NONE).desired_width(f32::INFINITY));
                });
                let password = field()
                    .show(ui, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut form.password).password(true).hint_text("Password").font(theme::regular(16.0)).frame(Frame::NONE).desired_width(f32::INFINITY))
                    })
                    .inner;
                ui.checkbox(&mut form.remember, RichText::new("Remember me (password in the desktop keyring)").font(theme::regular(14.0)));
                let enter = password.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let ready = !form.user.trim().is_empty() && !form.password.is_empty();
                if enter && ready {
                    submit = Some((form.user.trim().to_owned(), form.password.clone(), form.remember));
                }
                ui.horizontal(|ui| {
                    let sign_in = egui::Button::new(RichText::new("Sign in").font(theme::semibold(15.0)).color(theme::BG))
                        .fill(theme::ACCENT)
                        .corner_radius(CornerRadius::same(255))
                        .min_size(vec2(120.0, 40.0));
                    if ui.add_enabled(ready, sign_in).clicked() {
                        submit = Some((form.user.trim().to_owned(), form.password.clone(), form.remember));
                    }
                    let cancel_button = egui::Button::new(RichText::new("Cancel").font(theme::semibold(15.0))).corner_radius(CornerRadius::same(255)).min_size(vec2(100.0, 40.0));
                    if ui.add(cancel_button).clicked() {
                        cancel = true;
                    }
                });
            });
            if signing_in {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(18.0).color(theme::ACCENT));
                    ui.label(RichText::new("Signing in…").font(theme::regular(14.0)).color(theme::TEXT_MID));
                });
            } else if let Some(error) = error {
                ui.label(RichText::new(error).font(theme::regular(14.0)).color(theme::ACCENT));
            }
        });
        if modal.should_close() && !signing_in {
            cancel = true;
        }
        if cancel {
            self.form = None;
            self.error = None;
        } else if let Some((user, password, remember)) = submit {
            if let Some(form) = self.form.as_mut() {
                form.password.clear();
            }
            self.sign_in(ctx, user, password, remember);
        }
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
                let busy = self.busy > 0;
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

fn as_user(user: &Option<String>) -> String {
    user.as_ref().map_or_else(String::new, |u| format!(" as {u}"))
}
