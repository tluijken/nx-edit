//! The Line 6 account nx-edit signs in with: the user name in a file next to the library, the
//! password in the desktop's Secret Service, never on disk in the clear.

use std::path::{Path, PathBuf};

const SERVICE: &str = "nx-edit";
const USER_FILE: &str = "line6-user";

/// Where the user name file lives: beside the library, in nx-edit's data directory.
pub fn data_dir() -> PathBuf {
    let library = crate::library::default_dir();
    library.parent().map_or(library.clone(), Path::to_path_buf)
}

fn load_user(dir: &Path) -> Option<String> {
    let user = std::fs::read_to_string(dir.join(USER_FILE)).ok()?;
    Some(user.trim().to_owned()).filter(|u| !u.is_empty())
}

fn save_user(dir: &Path, user: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(USER_FILE), user.trim())
}

fn forget_user(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(dir.join(USER_FILE)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn entry(user: &str) -> Result<keyring::v1::Entry, String> {
    keyring::v1::Entry::new(SERVICE, &format!("line6:{user}")).map_err(|e| e.to_string())
}

/// The remembered user name and its password, when both are there.
pub fn remembered() -> Option<(String, String)> {
    let user = load_user(&data_dir())?;
    let password = entry(&user).ok()?.get_password().ok()?;
    Some((user, password))
}

/// Keeps this account for the next start; an account remembered before is forgotten first, so
/// only one password is ever kept.
pub fn remember(user: &str, password: &str) -> Result<(), String> {
    let user = user.trim();
    if load_user(&data_dir()).is_some_and(|old| old != user) {
        forget()?;
    }
    entry(user)?.set_password(password).map_err(|e| format!("Secret Service: {e}"))?;
    save_user(&data_dir(), user).map_err(|e| e.to_string())
}

/// Deletes the kept password, then the user name; if the keyring refuses, the name stays so the
/// password is not left behind unnamed.
pub fn forget() -> Result<(), String> {
    let dir = data_dir();
    if let Some(user) = load_user(&dir) {
        match entry(&user)?.delete_credential() {
            Ok(()) | Err(keyring::v1::Error::NoEntry) => {}
            Err(e) => return Err(format!("Secret Service: {e}")),
        }
    }
    forget_user(&dir).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_user_name_is_remembered_beside_the_library_and_forgotten() {
        let dir = std::env::temp_dir().join(format!("nx-account-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load_user(&dir), None);
        save_user(&dir, "  thomas@example.com \n").unwrap();
        assert_eq!(load_user(&dir).as_deref(), Some("thomas@example.com"));
        assert!(!std::fs::read_to_string(dir.join(USER_FILE)).unwrap().contains("pass"));
        forget_user(&dir).unwrap();
        assert_eq!(load_user(&dir), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
