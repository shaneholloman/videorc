//! OBS-style local secret store (owner decision 2026-06-11).
//!
//! Secrets (stream keys, OAuth tokens) live in a 0600-permission JSON file next
//! to the app database instead of the macOS keychain. The keychain demanded the
//! login password on every dev rebuild — each unsigned binary is a brand-new ACL
//! identity, so "Always Allow" never stuck. OBS avoids the prompt by not using
//! the keychain at all (service configs are plain files); we adopt the same
//! model: a single-user desktop machine where owner-only file permissions are
//! the protection boundary.
//!
//! Cross-platform by construction: the store is plain JSON under the per-user
//! app-data dir, so it works unchanged on Windows. The 0600 hardening is
//! `cfg(unix)`; on Windows the file inherits the per-user `%APPDATA%` ACL,
//! which is the equivalent boundary.

use anyhow::{Context, Result, anyhow};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// Serializes read-modify-write cycles within this process (the app runs exactly
/// one backend; cross-process locking is not a real concern here).
static STORE_CACHE: Mutex<Option<StoreCache>> = Mutex::new(None);

#[derive(Debug, Clone)]
struct StoreCache {
    path: PathBuf,
    secrets: BTreeMap<String, String>,
}

pub fn init_native_secret_store() {
    // Nothing to initialize: the store is a file created on first write. The
    // hook stays so main's startup sequence reads unchanged.
}

fn secrets_path() -> PathBuf {
    if let Some(custom) = std::env::var_os("VIDEORC_SECRETS_PATH") {
        return PathBuf::from(custom);
    }
    crate::storage::default_database_path()
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("videorc-secrets.json")
}

fn read_all(path: &PathBuf) -> Result<BTreeMap<String, String>> {
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .with_context(|| format!("Secret store {} is not valid JSON", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => {
            Err(error).with_context(|| format!("Could not read secret store {}", path.display()))
        }
    }
}

fn load_cache<'a>(cache: &'a mut Option<StoreCache>, path: &PathBuf) -> Result<&'a mut StoreCache> {
    let should_load = cache
        .as_ref()
        .map(|store| store.path != *path)
        .unwrap_or(true);
    if should_load {
        *cache = Some(StoreCache {
            path: path.clone(),
            secrets: read_all(path)?,
        });
    }
    Ok(cache.as_mut().expect("secret cache was just loaded"))
}

fn write_all(path: &PathBuf, secrets: &BTreeMap<String, String>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Could not create secret store dir {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let payload = serde_json::to_string_pretty(secrets).context("Could not encode secrets")?;
    std::fs::write(&tmp, payload)
        .with_context(|| format!("Could not write secret store {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Could not set permissions on {}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("Could not commit secret store {}", path.display()))
}

pub fn put_secret(secret_ref: &str, value: &str) -> Result<()> {
    let mut guard = STORE_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = secrets_path();
    let store = load_cache(&mut guard, &path)?;
    let mut secrets = store.secrets.clone();
    secrets.insert(secret_ref.to_string(), value.to_string());
    write_all(&path, &secrets)?;
    store.secrets = secrets;
    Ok(())
}

pub fn get_secret(secret_ref: &str) -> Result<String> {
    let mut guard = STORE_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = secrets_path();
    let store = load_cache(&mut guard, &path)?;
    store
        .secrets
        .get(secret_ref)
        .cloned()
        .ok_or_else(|| anyhow!("Secret ref {secret_ref} is not stored."))
}

/// Like [`get_secret`], but absence is a normal outcome (`Ok(None)`) instead of
/// an error — for callers that need to know whether a secret exists at all.
pub fn try_get_secret(secret_ref: &str) -> Result<Option<String>> {
    let mut guard = STORE_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = secrets_path();
    let store = load_cache(&mut guard, &path)?;
    Ok(store.secrets.get(secret_ref).cloned())
}

pub fn delete_secret(secret_ref: &str) -> Result<()> {
    let mut guard = STORE_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = secrets_path();
    let store = load_cache(&mut guard, &path)?;
    let mut secrets = store.secrets.clone();
    if secrets.remove(secret_ref).is_some() {
        write_all(&path, &secrets)?;
        store.secrets = secrets;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_store_roundtrip_and_permissions() {
        let dir = std::env::temp_dir().join(format!("videorc-secrets-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("videorc-secrets.json");
        // SAFETY: tests in this module run in one test fn; no parallel env races.
        unsafe { std::env::set_var("VIDEORC_SECRETS_PATH", &path) };

        assert!(get_secret("stream-target:twitch:manual-stream-key").is_err());
        assert_eq!(
            try_get_secret("stream-target:twitch:manual-stream-key").unwrap(),
            None
        );

        put_secret("stream-target:twitch:manual-stream-key", "live_abc").unwrap();
        assert_eq!(
            try_get_secret("stream-target:twitch:manual-stream-key").unwrap(),
            Some("live_abc".to_string())
        );
        put_secret("platform:x:oauth:refresh", "tok").unwrap();
        assert_eq!(
            get_secret("stream-target:twitch:manual-stream-key").unwrap(),
            "live_abc"
        );

        // Once warmed, reads come from the in-process cache instead of
        // re-reading and re-parsing the whole JSON file per secret lookup.
        std::fs::write(&path, b"not valid json").unwrap();
        assert_eq!(
            get_secret("stream-target:twitch:manual-stream-key").unwrap(),
            "live_abc"
        );

        // Overwrite keeps the latest value.
        put_secret("stream-target:twitch:manual-stream-key", "live_def").unwrap();
        assert_eq!(
            get_secret("stream-target:twitch:manual-stream-key").unwrap(),
            "live_def"
        );

        // Owner-only permissions: the file is the protection boundary.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "secret store must be 0600");
        }

        // Deleting is idempotent and removes only the named entry.
        delete_secret("stream-target:twitch:manual-stream-key").unwrap();
        delete_secret("stream-target:twitch:manual-stream-key").unwrap();
        assert!(get_secret("stream-target:twitch:manual-stream-key").is_err());
        assert_eq!(get_secret("platform:x:oauth:refresh").unwrap(), "tok");

        unsafe { std::env::remove_var("VIDEORC_SECRETS_PATH") };
        let _ = std::fs::remove_dir_all(&dir);
    }
}
