use std::path::{Path, PathBuf};

pub const USAGE: &str = "usage: nx-edit [preset.hlx]\n       nx-edit --from-capture <capture.pcapng>\n       nx-edit --import-hxedit <HX Edit installer.exe | HX Edit folder>";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Open(Option<PathBuf>),
    OpenCapture(PathBuf),
    Import(PathBuf),
    Usage,
}

pub fn parse(args: &[String]) -> Command {
    match args {
        [] => Command::Open(None),
        [flag, source] if flag == "--import-hxedit" => Command::Import(PathBuf::from(source)),
        [flag, capture] if flag == "--from-capture" => Command::OpenCapture(PathBuf::from(capture)),
        // nx-edit looks for the pedal by itself now; the flag that asked for that still starts it.
        [flag] if flag == "--from-device" => Command::Open(None),
        [path] if !path.starts_with("--") => Command::Open(Some(PathBuf::from(path))),
        _ => Command::Usage,
    }
}

/// Why the catalog did not load, for the import screen: none when nothing was imported yet, which
/// that screen says itself.
pub fn catalog_problem(err: &nx_catalog::CatalogError, res_dir: &Path) -> Option<String> {
    match err {
        nx_catalog::CatalogError::Io { path, source } if path == res_dir && source.kind() == std::io::ErrorKind::NotFound => None,
        other => Some(format!("The HX Edit data in {} did not load: {other}", res_dir.display())),
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
    fn import_needs_an_installer_or_a_folder() {
        assert_eq!(parse(&args(&["--import-hxedit"])), Command::Usage);
        assert_eq!(parse(&args(&["--import-hxedit", "x.exe"])), Command::Import(PathBuf::from("x.exe")));
        assert_eq!(parse(&args(&["--import-hxedit", "HX Edit"])), Command::Import(PathBuf::from("HX Edit")));
    }

    #[test]
    fn from_capture_opens_the_pedal_preset_in_a_capture() {
        assert_eq!(parse(&args(&["--from-capture", "c.pcapng"])), Command::OpenCapture(PathBuf::from("c.pcapng")));
        assert_eq!(parse(&args(&["--from-capture"])), Command::Usage);
    }

    #[test]
    fn from_device_still_starts_the_editor() {
        assert_eq!(parse(&args(&["--from-device"])), Command::Open(None));
        assert_eq!(parse(&args(&["--from-device", "x"])), Command::Usage);
    }

    #[test]
    fn unknown_flags_print_usage() {
        assert_eq!(parse(&args(&["--frobnicate"])), Command::Usage);
        assert_eq!(parse(&args(&["a.hlx", "b.hlx"])), Command::Usage);
    }

    #[test]
    fn a_catalog_never_imported_is_no_problem_to_report() {
        let dir = Path::new("/nonexistent/nx-edit/res");
        let err = nx_catalog::Catalog::load(dir).unwrap_err();
        assert_eq!(catalog_problem(&err, dir), None);
    }

    #[test]
    fn broken_catalog_shows_the_error() {
        let dir = std::env::temp_dir().join(format!("nx-app-broken-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("amp.models"), "not json").unwrap();
        let err = nx_catalog::Catalog::load(&dir).unwrap_err();
        let problem = catalog_problem(&err, &dir).unwrap();
        assert!(problem.contains("amp.models"), "{problem}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
