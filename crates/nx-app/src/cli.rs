use std::path::{Path, PathBuf};

pub const USAGE: &str = "usage: nx-edit [preset.hlx]\n       nx-edit --from-capture <capture.pcapng>\n       nx-edit --from-device\n       nx-edit --import-hxedit <HX Edit installer.exe>";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Open(Option<PathBuf>),
    OpenCapture(PathBuf),
    OpenDevice,
    Import(PathBuf),
    Usage,
}

pub fn parse(args: &[String]) -> Command {
    match args {
        [] => Command::Open(None),
        [flag, installer] if flag == "--import-hxedit" => Command::Import(PathBuf::from(installer)),
        [flag, capture] if flag == "--from-capture" => Command::OpenCapture(PathBuf::from(capture)),
        [flag] if flag == "--from-device" => Command::OpenDevice,
        [path] if !path.starts_with("--") => Command::Open(Some(PathBuf::from(path))),
        _ => Command::Usage,
    }
}

/// What the status line says when the catalog does not load: a missing directory means nothing was
/// imported yet; anything else is a real error the user should see.
pub fn catalog_status(err: &nx_catalog::CatalogError, res_dir: &Path) -> String {
    match err {
        nx_catalog::CatalogError::Io { path, source }
            if path == res_dir && source.kind() == std::io::ErrorKind::NotFound =>
        {
            "geen HX Edit-data; importeer met nx-edit --import-hxedit <installer>".to_owned()
        }
        other => format!("HX Edit-data in {} niet geladen: {other}", res_dir.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_arguments_opens_an_empty_editor() {
        assert_eq!(parse(&args(&[])), Command::Open(None));
    }

    #[test]
    fn a_path_opens_that_preset() {
        assert_eq!(parse(&args(&["a.hlx"])), Command::Open(Some(PathBuf::from("a.hlx"))));
    }

    #[test]
    fn import_needs_an_installer() {
        assert_eq!(parse(&args(&["--import-hxedit"])), Command::Usage);
        assert_eq!(parse(&args(&["--import-hxedit", "x.exe"])), Command::Import(PathBuf::from("x.exe")));
    }

    #[test]
    fn from_capture_opens_the_pedal_preset_in_a_capture() {
        assert_eq!(parse(&args(&["--from-capture", "c.pcapng"])), Command::OpenCapture(PathBuf::from("c.pcapng")));
        assert_eq!(parse(&args(&["--from-capture"])), Command::Usage);
    }

    #[test]
    fn from_device_reads_the_pedal() {
        assert_eq!(parse(&args(&["--from-device"])), Command::OpenDevice);
        assert_eq!(parse(&args(&["--from-device", "x"])), Command::Usage);
    }

    #[test]
    fn unknown_flags_print_usage() {
        assert_eq!(parse(&args(&["--frobnicate"])), Command::Usage);
        assert_eq!(parse(&args(&["a.hlx", "b.hlx"])), Command::Usage);
    }

    #[test]
    fn missing_catalog_suggests_importing() {
        let dir = Path::new("/nonexistent/nx-edit/res");
        let err = nx_catalog::Catalog::load(dir).unwrap_err();
        assert!(catalog_status(&err, dir).contains("--import-hxedit"));
    }

    #[test]
    fn broken_catalog_shows_the_error() {
        let dir = std::env::temp_dir().join(format!("nx-app-broken-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("amp.models"), "not json").unwrap();
        let err = nx_catalog::Catalog::load(&dir).unwrap_err();
        let status = catalog_status(&err, &dir);
        assert!(status.contains("amp.models"), "{status}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
