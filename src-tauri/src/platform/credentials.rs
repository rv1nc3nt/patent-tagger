//! OPS credential storage via the OS credential store (SPEC section 5.4):
//! Windows Credential Manager, Linux Secret Service (via `keyring`'s
//! pure-Rust `zbus` backend, not `libdbus`). Falls back to a `0600` file in
//! the data directory when no credential store is available.

use keyring::Entry;
use std::path::Path;

const SERVICE: &str = "patent-tagger";
const KEY_USER: &str = "ops_consumer_key";
const SECRET_USER: &str = "ops_consumer_secret";

pub struct OpsCredentials {
    pub consumer_key: String,
    pub consumer_secret: String,
}

pub fn save(data_dir: &Path, creds: &OpsCredentials) -> anyhow::Result<()> {
    match save_to_keyring(creds) {
        Ok(()) => Ok(()),
        Err(_) => save_to_file(data_dir, creds),
    }
}

pub fn load(data_dir: &Path) -> anyhow::Result<Option<OpsCredentials>> {
    match load_from_keyring() {
        Ok(Some(creds)) => Ok(Some(creds)),
        _ => load_from_file(data_dir),
    }
}

fn save_to_keyring(creds: &OpsCredentials) -> Result<(), keyring::Error> {
    Entry::new(SERVICE, KEY_USER)?.set_password(&creds.consumer_key)?;
    Entry::new(SERVICE, SECRET_USER)?.set_password(&creds.consumer_secret)?;
    Ok(())
}

fn load_from_keyring() -> Result<Option<OpsCredentials>, keyring::Error> {
    let consumer_key = Entry::new(SERVICE, KEY_USER)?.get_password()?;
    let consumer_secret = Entry::new(SERVICE, SECRET_USER)?.get_password()?;
    Ok(Some(OpsCredentials {
        consumer_key,
        consumer_secret,
    }))
}

fn credentials_file(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("ops_credentials.json")
}

fn save_to_file(data_dir: &Path, creds: &OpsCredentials) -> anyhow::Result<()> {
    let path = credentials_file(data_dir);
    let body = serde_json::json!({
        "consumer_key": creds.consumer_key,
        "consumer_secret": creds.consumer_secret,
    });
    std::fs::write(&path, serde_json::to_vec(&body)?)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn load_from_file(data_dir: &Path) -> anyhow::Result<Option<OpsCredentials>> {
    let path = credentials_file(data_dir);
    if !path.exists() {
        return Ok(None);
    }
    let body: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    let consumer_key = body["consumer_key"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("credentials file missing consumer_key"))?
        .to_string();
    let consumer_secret = body["consumer_secret"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("credentials file missing consumer_secret"))?
        .to_string();
    Ok(Some(OpsCredentials {
        consumer_key,
        consumer_secret,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The keyring backend needs a real Secret Service / Credential Manager,
    // which isn't available in a CI/sandbox container, so these tests cover
    // the file fallback directly rather than the full save()/load() (which
    // would first attempt the keyring and only reach the file path when
    // that fails - still true in this environment, but relying on that
    // would make the test's pass/fail depend on the test machine).

    #[test]
    fn file_fallback_round_trips_credentials() {
        let dir = tempfile::tempdir().expect("tempdir should be creatable");
        let creds = OpsCredentials {
            consumer_key: "key123".to_string(),
            consumer_secret: "secret456".to_string(),
        };
        save_to_file(dir.path(), &creds).expect("save should succeed");

        let loaded = load_from_file(dir.path())
            .expect("load should succeed")
            .expect("credentials should be present");
        assert_eq!(loaded.consumer_key, "key123");
        assert_eq!(loaded.consumer_secret, "secret456");
    }

    #[test]
    fn file_fallback_is_none_when_absent() {
        let dir = tempfile::tempdir().expect("tempdir should be creatable");
        assert!(load_from_file(dir.path()).unwrap().is_none());
    }

    #[test]
    #[cfg(unix)]
    fn file_fallback_is_not_world_or_group_readable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir should be creatable");
        let creds = OpsCredentials {
            consumer_key: "key123".to_string(),
            consumer_secret: "secret456".to_string(),
        };
        save_to_file(dir.path(), &creds).expect("save should succeed");

        let mode = std::fs::metadata(credentials_file(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "credentials file must be 0600");
    }
}
