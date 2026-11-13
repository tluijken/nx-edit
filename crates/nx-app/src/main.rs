mod account;
mod assign;
mod cli;
mod customtone;
mod device;
mod history;
mod layout;
mod library;
mod live;
mod theme;
mod view;

enum Source {
    Capture(std::path::PathBuf),
    Device,
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut status = Vec::new();
    let res_dir = nx_catalog::default_res_dir();
    let (preset_path, source) = match cli::parse(&args) {
        cli::Command::Open(path) => (path, None),
        cli::Command::OpenCapture(path) => (None, Some(Source::Capture(path))),
        cli::Command::OpenDevice => (None, Some(Source::Device)),
        cli::Command::Import(installer) => {
            let dest = nx_catalog::default_res_dir();
            match nx_catalog::import_installer(&installer, &dest) {
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
    let mut connection = None;
    let from_pedal = source.as_ref().and_then(|source| {
        let loaded = nx_catalog::ModelSymbols::load(&res_dir.join("Helix.sym")).map_err(|e| e.to_string()).and_then(|syms| {
            let (preset, link) = match source {
                Source::Capture(path) => (device::preset_from_capture(path)?, None),
                Source::Device => {
                    let device::Connected { session, presets, preset } = device::connect_device()?;
                    (preset, Some((session, presets)))
                }
            };
            let hlx = preset.to_hlx(&syms)?;
            let mut list_error = None;
            let link = match link {
                Some((session, presets)) => Some(view::Connection {
                    presets: presets.unwrap_or_else(|e| {
                        list_error = Some(format!("no preset list: {e}"));
                        Vec::new()
                    }),
                    addresses: preset.addresses(&syms)?,
                    live: live::Live::start(session),
                    current: preset.index,
                    generation: 0,
                    syms,
                }),
                None => None,
            };
            Ok((hlx, link, list_error))
        });
        match loaded {
            Ok((p, link, list_error)) => {
                status.push(match source {
                    Source::Capture(path) => format!("From capture {}", path.display()),
                    Source::Device => "Connected · changes not saved".to_owned(),
                });
                status.extend(list_error);
                connection = link;
                Some(p)
            }
            Err(e) => {
                status.push(e);
                None
            }
        }
    });
    let preset = from_pedal;
    let catalog = match nx_catalog::Catalog::load(&res_dir) {
        Ok(c) => Some(c),
        Err(e) => {
            status.push(cli::catalog_status(&e, &res_dir));
            None
        }
    };
    if status.is_empty() {
        status.push("Not connected · read-only".to_owned());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("nx-edit")
            .with_app_id("nx-edit")
            .with_inner_size([1200.0, 720.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "nx-edit",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            let mut app = view::EditorApp::new(preset, catalog, status.join(" · "), connection);
            if let Some(path) = preset_path {
                app.open_file(path);
            }
            Ok(Box::new(app))
        }),
    )
}
