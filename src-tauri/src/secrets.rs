//! OS keychain storage for connection passwords.
//!
//! profiles.json never holds a real password: on save the password moves to
//! the platform keychain (macOS Keychain, Windows Credential Manager,
//! Secret Service on Linux) and the file keeps the `__keychain__` sentinel.
//! Existing plaintext profiles are migrated on first load. If the keychain
//! is unavailable (locked, headless), saving falls back to plaintext rather
//! than losing the credential — connecting always works.

use keyring::Entry;

pub const SENTINEL: &str = "__keychain__";

const SERVICE: &str = "QueryDeck";

fn entry(profile_id: &str) -> keyring::Result<Entry> {
    Entry::new(SERVICE, profile_id)
}

/// Store a password; true on success.
pub fn store(profile_id: &str, password: &str) -> bool {
    entry(profile_id)
        .and_then(|e| e.set_password(password))
        .map_err(|e| eprintln!("keychain store failed for {profile_id}: {e}"))
        .is_ok()
}

/// Fetch the password for a profile, if the keychain has one.
pub fn fetch(profile_id: &str) -> Option<String> {
    entry(profile_id).ok()?.get_password().ok()
}

pub fn delete(profile_id: &str) {
    if let Ok(e) = entry(profile_id) {
        let _ = e.delete_credential();
    }
}
