//! Saved database passwords in the OS password store: Windows Credential
//! Manager, macOS Keychain, Linux Secret Service (GNOME Keyring / KWallet).
//! Passwords are never written to `telepad.db` or any other file.

use super::Secret;

const SERVICE: &str = "Telepad database";

/// `Ok` when the platform store can be used; else why not (shown in the
/// connect dialog, "Save password" is then disabled).
pub fn available() -> Result<(), String> {
    keyring::Entry::store_status()
        .as_ref()
        .map(|_| ())
        .map_err(|e| format!("No password store available: {e}"))
}

fn entry(key: &str) -> Result<keyring::Entry, String> {
    available()?;
    keyring::Entry::new(SERVICE, key).map_err(|e| e.to_string())
}

/// The saved password for a connection key, `None` when there is none.
pub fn load(key: &str) -> Result<Option<Secret>, String> {
    match entry(key)?.get_password() {
        Ok(p) => Ok(Some(Secret::new(p))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn save(key: &str, password: &Secret) -> Result<(), String> {
    entry(key)?
        .set_password(password.expose())
        .map_err(|e| e.to_string())
}

/// Remove the saved password; no error when there was none.
pub fn forget(key: &str) -> Result<(), String> {
    match entry(key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
