use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::CatalogError;

const FILES: &[&str] = &["HX_ModelCatalog.json", "HelixControls.json", "Helix.sym"];

/// Where nx-edit keeps its imported copy of HX Edit's data.
pub fn default_res_dir() -> PathBuf {
    data_root(cfg!(windows), |name| std::env::var_os(name)).join("nx-edit").join("hxedit-res")
}

fn data_root(windows: bool, var: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    let dir = if windows {
        var("APPDATA").map(PathBuf::from)
    } else {
        var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".local/share")))
    };
    dir.unwrap_or_else(|| PathBuf::from("."))
}

/// Imports the model data from an installed HX Edit (its folder or its `res` folder), or else from
/// its installer.
pub fn import_hxedit(source: &Path, dest: &Path) -> Result<(), CatalogError> {
    if source.is_dir() { import_res_dir(source, dest) } else { import_installer(source, dest) }
}

fn import_res_dir(source: &Path, dest: &Path) -> Result<(), CatalogError> {
    let res = if source.join("res").is_dir() { source.join("res") } else { source.to_owned() };
    if let Some(missing) = FILES.iter().copied().find(|f| !res.join(f).is_file()) {
        return Err(CatalogError::NotHxEditRes { path: res, missing });
    }
    let scratch = dest.with_extension("importing");
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).map_err(io(&scratch))?;
    for entry in fs::read_dir(&res).map_err(io(&res))? {
        let path = entry.map_err(io(&res))?.path();
        if is_model_data(&path) {
            fs::copy(&path, scratch.join(path.file_name().unwrap())).map_err(io(&path))?;
        }
    }
    replace_dir_contents(&scratch, dest)?;
    fs::remove_dir_all(&scratch).map_err(io(&scratch))
}

fn is_model_data(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.is_file() && (name.ends_with(".models") || FILES.contains(&name.as_ref()))
}

/// Copies the model data out of a Windows HX Edit installer (an NSIS archive) with `7z`.
pub fn import_installer(installer: &Path, dest: &Path) -> Result<(), CatalogError> {
    let scratch = dest.with_extension("extracting");
    let _ = fs::remove_dir_all(&scratch);
    let output = Command::new("7z")
        .arg("x")
        .arg("-y")
        .arg(format!("-o{}", scratch.display()))
        .arg(installer)
        .arg("HX Edit/res/*.models")
        .args(FILES.iter().map(|f| format!("HX Edit/res/{f}")))
        .output()
        .map_err(|e| CatalogError::Extract(format!("running 7z: {e} (an HX Edit folder needs no 7z)")))?;
    if !output.status.success() {
        return Err(CatalogError::Extract(String::from_utf8_lossy(&output.stderr).into_owned()));
    }
    replace_dir_contents(&scratch.join("HX Edit/res"), dest)?;
    fs::remove_dir_all(&scratch).map_err(io(&scratch))?;
    Ok(())
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> CatalogError {
    let path = path.to_owned();
    move |source| CatalogError::Io { path, source }
}

/// A newer HX Edit may rename or drop `.models` files; leftovers would still load and could shadow
/// current definitions, so the destination is emptied first.
fn replace_dir_contents(src: &Path, dest: &Path) -> Result<(), CatalogError> {
    if dest.exists() {
        fs::remove_dir_all(dest).map_err(io(dest))?;
    }
    fs::create_dir_all(dest).map_err(io(dest))?;
    for entry in fs::read_dir(src).map_err(io(src))? {
        let path = entry.map_err(io(src))?.path();
        fs::copy(&path, dest.join(path.file_name().unwrap())).map_err(io(&path))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        |name| vars.iter().find(|(n, _)| *n == name).map(|(_, v)| OsString::from(v))
    }

    #[test]
    fn linux_data_follows_xdg_data_home() {
        let vars = [("XDG_DATA_HOME", "/xdg"), ("HOME", "/home/t"), ("APPDATA", "/appdata")];
        assert_eq!(data_root(false, env(&vars)), PathBuf::from("/xdg"));
    }

    #[test]
    fn linux_data_falls_back_to_local_share() {
        let vars = [("HOME", "/home/t"), ("APPDATA", "/appdata")];
        assert_eq!(data_root(false, env(&vars)), PathBuf::from("/home/t").join(".local/share"));
    }

    #[test]
    fn windows_data_lives_in_appdata() {
        let vars = [("XDG_DATA_HOME", "/xdg"), ("HOME", "/home/t"), ("APPDATA", "/appdata")];
        assert_eq!(data_root(true, env(&vars)), PathBuf::from("/appdata"));
    }

    #[test]
    fn without_a_known_home_data_stays_in_the_current_directory() {
        assert_eq!(data_root(false, env(&[])), PathBuf::from("."));
        assert_eq!(data_root(true, env(&[("HOME", "/home/t")])), PathBuf::from("."));
    }

    #[test]
    fn the_res_dir_is_under_nx_edit() {
        assert!(default_res_dir().ends_with(Path::new("nx-edit").join("hxedit-res")));
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("nx-catalog-{name}-{}", std::process::id()));
        _ = fs::remove_dir_all(&root);
        root
    }

    fn fake_res(root: &Path) -> PathBuf {
        let res = root.join("HX Edit").join("res");
        fs::create_dir_all(&res).unwrap();
        for name in FILES.iter().chain(&["amp.models", "fx.models", "HX Edit.png"]) {
            fs::write(res.join(name), name).unwrap();
        }
        res
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> =
            fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        names
    }

    #[test]
    fn importing_a_res_folder_copies_only_the_model_data() {
        let root = scratch("res-folder");
        let dest = root.join("dest");
        import_hxedit(&fake_res(&root), &dest).unwrap();
        let expected = ["HX_ModelCatalog.json", "Helix.sym", "HelixControls.json", "amp.models", "fx.models"];
        assert_eq!(names(&dest), expected);
        assert_eq!(fs::read_to_string(dest.join("amp.models")).unwrap(), "amp.models");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn importing_the_hx_edit_folder_finds_its_res() {
        let root = scratch("install-folder");
        fake_res(&root);
        let dest = root.join("dest");
        import_hxedit(&root.join("HX Edit"), &dest).unwrap();
        assert!(dest.join("Helix.sym").is_file());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_folder_without_model_data_is_refused_and_keeps_the_old_data() {
        let root = scratch("not-res");
        let (empty, dest) = (root.join("empty"), root.join("dest"));
        fs::create_dir_all(&empty).unwrap();
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("amp.models"), "old").unwrap();
        let err = import_hxedit(&empty, &dest).unwrap_err();
        assert!(err.to_string().contains("HX_ModelCatalog.json is missing"), "{err}");
        assert_eq!(names(&dest), ["amp.models"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn replacing_the_data_removes_stale_files() {
        let root = std::env::temp_dir().join(format!("nx-catalog-replace-{}", std::process::id()));
        let (src, dest) = (root.join("src"), root.join("dest"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dest).unwrap();
        fs::write(src.join("amp.models"), "[]").unwrap();
        fs::write(dest.join("renamed-away.models"), "[]").unwrap();
        replace_dir_contents(&src, &dest).unwrap();
        let names: Vec<_> = fs::read_dir(&dest).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("amp.models")]);
        fs::remove_dir_all(&root).unwrap();
    }
}
