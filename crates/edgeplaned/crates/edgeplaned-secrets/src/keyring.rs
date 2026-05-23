// Linux-only keyring integration using the `keyring` crate (libsecret / D-Bus secret-service).
// Falls back silently if no keyring daemon is available.

const SERVICE: &str = "edgeplaned";

fn key_service_token(profile_name: &str) -> String {
    format!("infisical-service-token:{profile_name}")
}

/// Result type for keyring operations that may be unavailable.
pub enum KeyringResult {
    Ok,
    Unavailable(String),
}

/// Store a service token in the OS keyring under the given profile name.
///
/// Sync — call via `spawn_blocking` from async contexts.
pub fn store_service_token(profile_name: &str, token: &str) -> KeyringResult {
    let key = key_service_token(profile_name);
    let entry = match keyring::Entry::new(SERVICE, &key) {
        Ok(e) => e,
        Err(e) => return KeyringResult::Unavailable(e.to_string()),
    };
    match entry.set_password(token) {
        Ok(()) => KeyringResult::Ok,
        Err(e) => KeyringResult::Unavailable(e.to_string()),
    }
}

/// Load a service token from the OS keyring. Returns `None` if not found or keyring unavailable.
///
/// Sync — call via `spawn_blocking` from async contexts.
pub fn load_service_token(profile_name: &str) -> Option<String> {
    let key = key_service_token(profile_name);
    let entry = keyring::Entry::new(SERVICE, &key).ok()?;
    entry.get_password().ok()
}

/// Delete a service token from the OS keyring.
///
/// Sync — call via `spawn_blocking` from async contexts.
pub fn delete_service_token(profile_name: &str) -> KeyringResult {
    let key = key_service_token(profile_name);
    let entry = match keyring::Entry::new(SERVICE, &key) {
        Ok(e) => e,
        Err(e) => return KeyringResult::Unavailable(e.to_string()),
    };
    let _ = entry.delete_credential();
    KeyringResult::Ok
}

/// Migrate a legacy single-credential keyring entry (stored under the old
/// unprefixed key `"infisical-service-token:default"`) to the new multi-profile
/// slot for `target_profile`.
///
/// - Reads the legacy entry.
/// - Writes it under the target profile slot.
/// - Deletes the legacy entry.
///
/// Safe to call repeatedly — no-ops if the legacy entry is absent.
pub fn migrate_legacy_entry(target_profile: &str) -> KeyringResult {
    let legacy_key = key_service_token("default");
    let legacy_entry = match keyring::Entry::new(SERVICE, &legacy_key) {
        Ok(e) => e,
        Err(e) => return KeyringResult::Unavailable(e.to_string()),
    };
    let token = match legacy_entry.get_password() {
        Ok(t) if !t.is_empty() => t,
        _ => return KeyringResult::Ok, // nothing to migrate
    };
    let result = store_service_token(target_profile, &token);
    let _ = legacy_entry.delete_credential();
    result
}
