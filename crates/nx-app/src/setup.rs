//! The first-run screen: importing HX Edit's model data from its installer or an installed HX Edit,
//! picked from what nx-edit finds, typed, or dropped on the window.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use egui::{CornerRadius, Frame, Margin, RichText, Ui, vec2};

use crate::theme;
use crate::view::card;

pub struct Setup {
    /// Where the data is imported to and loaded from.
    dest: PathBuf,
    pub path: String,
    importing: Option<(PathBuf, Receiver<Result<(), String>>)>,
    pub error: Option<String>,
    found: Option<Vec<PathBuf>>,
}

impl Default for Setup {
    fn default() -> Self {
        Self::new(nx_catalog::default_res_dir())
    }
}

impl Setup {
    pub fn new(dest: PathBuf) -> Self {
        Self { dest, path: String::new(), importing: None, error: None, found: None }
    }

    /// Starts importing from `source` on a thread of its own.
    pub fn import(&mut self, source: PathBuf) {
        if self.importing.is_some() {
            return;
        }
        let (done, result) = mpsc::channel();
        let (from, dest) = (source.clone(), self.dest.clone());
        let spawned = std::thread::Builder::new()
            .name("nx-import".into())
            .spawn(move || _ = done.send(nx_catalog::import_hxedit(&from, &dest).map_err(|e| e.to_string())));
        match spawned {
            Ok(_) => {
                self.error = None;
                self.importing = Some((source, result));
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn busy(&self) -> bool {
        self.importing.is_some()
    }

    /// The catalog, once an import finished and it loads.
    pub fn take_imported(&mut self) -> Option<nx_catalog::Catalog> {
        let (_, result) = self.importing.as_ref()?;
        let outcome = match result.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("the import stopped unexpectedly".to_owned()),
        };
        self.importing = None;
        match outcome.and_then(|()| nx_catalog::Catalog::load(&self.dest).map_err(|e| e.to_string())) {
            Ok(catalog) => Some(catalog),
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    /// The card asking for HX Edit's data; `problem` is why the data that is there did not load.
    pub fn show(&mut self, ui: &mut Ui, problem: Option<&str>) {
        let found = self.found.get_or_insert_with(candidates).clone();
        let mut pick = None;
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            ui.label(RichText::new("nx-edit needs HX Edit's model data").font(theme::semibold(20.0)));
            ui.label(
                RichText::new(
                    "Block and parameter names come from Line 6's HX Edit, which nx-edit does not ship. Import them once from the \
                     HX Edit installer (.exe, unpacked with 7-Zip, never run) or from an installed HX Edit folder.",
                )
                .font(theme::regular(14.0))
                .color(theme::TEXT_MID),
            );
            if let Some(problem) = problem {
                ui.label(RichText::new(problem).font(theme::regular(13.0)).color(theme::ORANGE));
            }
            if let Some((source, _)) = &self.importing {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(16.0).color(theme::ACCENT));
                    ui.label(RichText::new(format!("Importing from {}…", source.display())).color(theme::TEXT_MID));
                });
                return;
            }
            for path in &found {
                let label = format!("Import from {}", path.display());
                if ui.add(egui::Button::new(RichText::new(label).font(theme::semibold(14.0)).color(theme::BG)).fill(theme::ACCENT).corner_radius(CornerRadius::same(255)).min_size(vec2(0.0, 36.0))).clicked() {
                    pick = Some(path.clone());
                }
            }
            ui.label(RichText::new("Drop the installer or the HX Edit folder on this window, or type where it is:").font(theme::regular(13.0)).color(theme::TEXT_DIM));
            ui.horizontal(|ui| {
                let field = Frame::new().fill(theme::BG).corner_radius(CornerRadius::same(255)).inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.path).hint_text("HX Edit v3.82 Installer.exe or C:\\Program Files (x86)\\Line6\\HX Edit").frame(Frame::NONE).desired_width(ui.available_width() - 110.0))
                });
                let enter = field.inner.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let typed = !self.path.trim().is_empty();
                if (ui.add_enabled(typed, egui::Button::new(RichText::new("Import").font(theme::semibold(14.0))).corner_radius(CornerRadius::same(255)).min_size(vec2(90.0, 36.0))).clicked() || enter) && typed {
                    pick = Some(PathBuf::from(self.path.trim()));
                }
            });
            if let Some(error) = &self.error {
                ui.label(RichText::new(format!("Not imported: {error}")).font(theme::regular(13.0)).color(theme::ORANGE));
            }
        });
        if let Some(source) = pick {
            self.import(source);
        }
    }
}

/// An HX Edit install or installer where one usually is.
fn candidates() -> Vec<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let mut found: Vec<PathBuf> = [r"C:\Program Files (x86)\Line6\HX Edit", r"C:\Program Files\Line6\HX Edit"]
        .into_iter()
        .map(PathBuf::from)
        .filter(|p| cfg!(windows) && p.join("res").is_dir())
        .collect();
    if let Some(home) = home {
        found.extend(installers_in(&home.join("Downloads")));
    }
    found
}

/// HX Edit installers in `dir`, newest name last.
fn installers_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut installers: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(is_installer_name))
        .collect();
    installers.sort();
    installers
}

fn is_installer_name(name: &str) -> bool {
    let name = name.to_lowercase();
    name.starts_with("hx edit") && name.ends_with(".exe")
}

/// Whether a file dropped on the window is something to import HX Edit's data from rather than a
/// preset for the library.
pub fn is_import_source(path: &Path) -> bool {
    path.is_dir() || path.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installers_are_recognised_by_their_name() {
        assert!(is_installer_name("HX Edit v3.82 Installer.exe"));
        assert!(is_installer_name("hx edit 3.70.exe"));
        assert!(!is_installer_name("HX Edit Manual.pdf"));
        assert!(!is_installer_name("setup.exe"));
    }

    #[test]
    fn a_dropped_folder_or_exe_is_imported_and_a_preset_is_not() {
        assert!(is_import_source(Path::new("HX Edit v3.82 Installer.EXE")));
        assert!(is_import_source(&std::env::temp_dir()));
        assert!(!is_import_source(Path::new("Revv DM.hlx")));
    }

    #[test]
    fn a_failed_import_says_why() {
        let mut setup = Setup::new(std::env::temp_dir().join(format!("nx-setup-{}", std::process::id())));
        setup.import(PathBuf::from("/nonexistent/HX Edit"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while setup.busy() {
            assert!(std::time::Instant::now() < deadline);
            assert!(setup.take_imported().is_none());
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(setup.error.is_some());
    }
}
