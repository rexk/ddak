use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("keychain unavailable: {0}")]
    KeychainUnavailable(String),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("decode error: {0}")]
    Decode(String),
}

pub trait KeychainProvider {
    fn set(&self, key: &str, value: &str) -> Result<(), SecretError>;
    fn get(&self, key: &str) -> Result<Option<String>, SecretError>;
    fn delete(&self, key: &str) -> Result<(), SecretError>;
}

pub struct UnavailableKeychain;

impl KeychainProvider for UnavailableKeychain {
    fn set(&self, _key: &str, _value: &str) -> Result<(), SecretError> {
        Err(SecretError::KeychainUnavailable(
            "no keychain backend configured".to_string(),
        ))
    }

    fn get(&self, _key: &str) -> Result<Option<String>, SecretError> {
        Err(SecretError::KeychainUnavailable(
            "no keychain backend configured".to_string(),
        ))
    }

    fn delete(&self, _key: &str) -> Result<(), SecretError> {
        Err(SecretError::KeychainUnavailable(
            "no keychain backend configured".to_string(),
        ))
    }
}

pub struct SecretManager<K: KeychainProvider> {
    keychain: K,
    fallback_path: PathBuf,
}

impl<K: KeychainProvider> SecretManager<K> {
    pub fn new(keychain: K, fallback_path: impl Into<PathBuf>) -> Self {
        Self {
            keychain,
            fallback_path: fallback_path.into(),
        }
    }

    pub fn store_token(&self, key: &str, token: &str) -> Result<(), SecretError> {
        if self.keychain.set(key, token).is_ok() {
            return Ok(());
        }

        let encoded = encode_secret(token);
        write_secure_file(&self.fallback_path_for(key), encoded.as_bytes())
    }

    pub fn load_token(&self, key: &str) -> Result<Option<String>, SecretError> {
        if let Ok(value) = self.keychain.get(key)
            && value.is_some()
        {
            return Ok(value);
        }

        let path = self.fallback_path_for(key);
        if !path.exists() {
            return Ok(None);
        }

        let data = fs::read_to_string(path).map_err(|err| SecretError::Io(err.to_string()))?;
        decode_secret(&data).map(Some)
    }

    pub fn rotate_token(&self, key: &str, new_token: &str) -> Result<(), SecretError> {
        self.store_token(key, new_token)
    }

    pub fn revoke_token(&self, key: &str) -> Result<(), SecretError> {
        let _ = self.keychain.delete(key);
        let path = self.fallback_path_for(key);
        if path.exists() {
            fs::remove_file(path).map_err(|err| SecretError::Io(err.to_string()))?;
        }
        Ok(())
    }

    fn fallback_path_for(&self, key: &str) -> PathBuf {
        let safe_key = key.replace(['/', '\\', ':'], "_");
        self.fallback_path.join(format!("{safe_key}.secret"))
    }
}

fn encode_secret(secret: &str) -> String {
    let key = derive_key();
    let bytes: Vec<u8> = secret
        .as_bytes()
        .iter()
        .enumerate()
        .map(|(idx, b)| b ^ key[idx % key.len()])
        .collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode_secret(encoded: &str) -> Result<String, SecretError> {
    let key = derive_key();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|err| SecretError::Decode(err.to_string()))?;
    let bytes: Vec<u8> = decoded
        .iter()
        .enumerate()
        .map(|(idx, b)| b ^ key[idx % key.len()])
        .collect();
    String::from_utf8(bytes).map_err(|err| SecretError::Decode(err.to_string()))
}

fn derive_key() -> Vec<u8> {
    let user = std::env::var("USER").unwrap_or_else(|_| "unknown-user".to_string());
    format!("ddak-secret-key::{user}").into_bytes()
}

fn write_secure_file(path: &Path, data: &[u8]) -> Result<(), SecretError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| SecretError::Io(err.to_string()))?;
    }

    fs::write(path, data).map_err(|err| SecretError::Io(err.to_string()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)
            .map_err(|err| SecretError::Io(err.to_string()))?
            .permissions();
        perms.set_mode(0o600);
        fs::set_permissions(path, perms).map_err(|err| SecretError::Io(err.to_string()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::{KeychainProvider, SecretError, SecretManager};

    #[derive(Clone, Default)]
    struct MockKeychain {
        values: Arc<Mutex<HashMap<String, String>>>,
        fail_all: bool,
    }

    impl MockKeychain {
        fn failing() -> Self {
            Self {
                values: Arc::new(Mutex::new(HashMap::new())),
                fail_all: true,
            }
        }
    }

    impl KeychainProvider for MockKeychain {
        fn set(&self, key: &str, value: &str) -> Result<(), SecretError> {
            if self.fail_all {
                return Err(SecretError::KeychainUnavailable("mock failure".to_string()));
            }
            self.values
                .lock()
                .expect("mutex should lock")
                .insert(key.to_string(), value.to_string());
            Ok(())
        }

        fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
            if self.fail_all {
                return Err(SecretError::KeychainUnavailable("mock failure".to_string()));
            }
            Ok(self
                .values
                .lock()
                .expect("mutex should lock")
                .get(key)
                .cloned())
        }

        fn delete(&self, key: &str) -> Result<(), SecretError> {
            if self.fail_all {
                return Err(SecretError::KeychainUnavailable("mock failure".to_string()));
            }
            self.values.lock().expect("mutex should lock").remove(key);
            Ok(())
        }
    }

    #[test]
    fn fallback_file_is_not_plaintext_and_has_secure_permissions() {
        let temp = std::env::temp_dir().join("ddak-secrets-test-a");
        let _ = std::fs::remove_dir_all(&temp);

        let manager = SecretManager::new(MockKeychain::failing(), &temp);
        manager
            .store_token("linear_api", "super-secret-token")
            .expect("store should succeed with fallback");

        let file = temp.join("linear_api.secret");
        let bytes = std::fs::read(&file).expect("fallback file should exist");
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("super-secret-token"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file)
                .expect("metadata should exist")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn rotate_and_revoke_token_work() {
        let temp = std::env::temp_dir().join("ddak-secrets-test-b");
        let _ = std::fs::remove_dir_all(&temp);

        let manager = SecretManager::new(MockKeychain::failing(), &temp);
        manager
            .store_token("linear_api", "first")
            .expect("initial store should succeed");
        manager
            .rotate_token("linear_api", "second")
            .expect("rotate should succeed");

        let loaded = manager
            .load_token("linear_api")
            .expect("load should succeed")
            .expect("token should exist");
        assert_eq!(loaded, "second");

        manager
            .revoke_token("linear_api")
            .expect("revoke should succeed");
        let loaded = manager
            .load_token("linear_api")
            .expect("load should succeed");
        assert!(loaded.is_none());
    }
}
