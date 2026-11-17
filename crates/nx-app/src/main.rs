#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod account;
mod assign;
mod cli;
mod connect;
mod customtone;
mod history;
mod layout;
mod library;
mod notice;
mod setup;
mod theme;
mod view;

use nx_device::{device, live};

/// A GUI-subsystem program gets no console of its own; borrowing the one it was started from keeps
/// `--help` and `--import-hxedit` output visible in a terminal.
#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

fn main() -> eframe::Result {
    #[cfg(windows)]
    attach_parent_console();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut status = Vec::new();
    let res_dir = nx_catalog::default_res_dir();
    let (preset_path, capture) = match cli::parse(&args) {
        cli::Command::Open(path) => (path, None),
        cli::Command::OpenCapture(path) => (None, Some(path)),
        cli::Command::Import(source) => {
            let dest = nx_catalog::default_res_dir();
            match nx_catalog::import_hxedit(&source, &dest) {
                Ok(()) => println!("HX Edit data imported into {}", dest.display()),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        cli::Command::Usage => {
            eprintln!("{}", cli::USAGE);
            std::process::exit(2);
        }
    };
    // A capture is a pedal that is not there, so nx-edit does not look for the real one meanwhile.
    let preset = capture.as_ref().and_then(|path| {
        let loaded = nx_catalog::ModelSymbols::load(&res_dir.join("Helix.sym"))
            .map_err(|e| e.to_string())
            .and_then(|syms| device::preset_from_capture(path)?.to_hlx(&syms));
        match loaded {
            Ok(p) => {
                status.push(format!("From capture {}", path.display()));
                Some(p)
            }
            Err(e) => {
                status.push(e);
                None
            }
        }
    });
    let (catalog, catalog_problem) = match nx_catalog::Catalog::load(&res_dir) {
        Ok(c) => (Some(c), None),
        Err(e) => (None, cli::catalog_problem(&e, &res_dir)),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("nx-edit")
            .with_app_id("nx-edit")
            .with_inner_size([1200.0, 720.0])
            .with_min_inner_size([800.0, 500.0]),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "nx-edit",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            let mut app = view::EditorApp::new(preset, catalog, status.join(" · "), None);
            app.catalog_problem = catalog_problem;
            if capture.is_none() {
                app.connect_with(connect::usb());
            }
            if let Some(path) = preset_path {
                app.open_file(path);
            }
            Ok(Box::new(app))
        }),
    )
}
