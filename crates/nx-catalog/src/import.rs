use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::CatalogError;

const FILES: &[&str] = &["HX_ModelCatalog.json", "HelixControls.json", "Helix.sym"];

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
    let res = scratch.join("HX Edit/res");
    let io = |path: &Path| {
        let path = path.to_owned();
        move |source| CatalogError::Io { path, source }
    };
    fs::create_dir_all(dest).map_err(io(dest))?;
    for entry in fs::read_dir(&res).map_err(io(&res))? {
        let path = entry.map_err(io(&res))?.path();
        fs::copy(&path, dest.join(path.file_name().unwrap())).map_err(io(&path))?;
    }
    fs::remove_dir_all(&scratch).map_err(io(&scratch))?;
    Ok(())
}
