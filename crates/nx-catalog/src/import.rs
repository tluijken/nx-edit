use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::CatalogError;

const FILES: &[&str] = &["HX_ModelCatalog.json", "HelixControls.json"];

/// Where nx-edit keeps its imported copy of HX Edit's data.
pub fn default_res_dir() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    data.join("nx-edit/hxedit-res")
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
        .map_err(|e| CatalogError::Extract(format!("running 7z: {e}")))?;
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
