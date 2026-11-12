mod cli;
mod layout;
mod theme;
mod view;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let preset_path = match cli::parse(&args) {
        cli::Command::Open(path) => path,
        cli::Command::Import(installer) => {
            let dest = nx_catalog::default_res_dir();
            match nx_catalog::import_installer(&installer, &dest) {
                Ok(()) => println!("HX Edit-data geïmporteerd in {}", dest.display()),
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
    let mut status = Vec::new();
    let preset = preset_path.as_ref().and_then(|path| {
        match std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| nx_hlx::Preset::from_slice(&b).map_err(|e| e.to_string())) {
            Ok(p) => Some(p),
            Err(e) => {
                status.push(format!("{}: {e}", path.display()));
                None
            }
        }
    });
    let res_dir = nx_catalog::default_res_dir();
    let catalog = match nx_catalog::Catalog::load(&res_dir) {
        Ok(c) => Some(c),
        Err(e) => {
            status.push(cli::catalog_status(&e, &res_dir));
            None
        }
    };
    if status.is_empty() {
        status.push("niet verbonden · alleen lezen".to_owned());
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
            Ok(Box::new(view::EditorApp::new(preset, catalog, status.join(" · "))))
        }),
    )
}
