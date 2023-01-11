use crate::lib::error::DfxResult;
use dfx_core::error::keyring::KeyringError;
use dfx_core::error::keyring::KeyringError::{
    DeletePasswordFailed, LoadMockKeyringFailed, MockUnavailable, SaveMockKeyringFailed,
};
use dfx_core::json::{load_json_file, save_json_file};

use super::TEMP_IDENTITY_PREFIX;
use anyhow::{bail, Context};
use fn_error_context::context;
use keyring;
use serde::{Deserialize, Serialize};
use slog::{trace, Logger};
use std::{collections::HashMap, path::PathBuf};

pub const KEYRING_SERVICE_NAME: &str = "internet_computer_identities";
pub const KEYRING_IDENTITY_PREFIX: &str = "internet_computer_identity_";
pub const USE_KEYRING_MOCK_ENV_VAR: &str = "DFX_CI_MOCK_KEYRING_LOCATION";
fn keyring_identity_name_from_suffix(suffix: &str) -> String {
    format!("{}{}", KEYRING_IDENTITY_PREFIX, suffix)
}

enum KeyringMockMode {
    /// Use system keyring
    NoMock,
    /// Simulate keyring where access is granted
    MockAvailable,
    /// Simulate keyring where access is rejected
    MockReject,
}

impl KeyringMockMode {
    fn current_mode() -> Self {
        match std::env::var(USE_KEYRING_MOCK_ENV_VAR) {
            Err(_) => Self::NoMock,
            Ok(location) => match location.as_str() {
                "" => Self::MockReject,
                _ => Self::MockAvailable,
            },
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct KeyringMock {
    pub kv_store: HashMap<String, String>,
}

impl KeyringMock {
    fn get_location() -> Result<PathBuf, KeyringError> {
        match std::env::var(USE_KEYRING_MOCK_ENV_VAR) {
            Ok(filename) => match filename.as_str() {
                "" => Err(MockUnavailable()),
                _ => Ok(PathBuf::from(filename)),
            },
            _ => unreachable!("Mock keyring unavailable."),
        }
    }

    pub fn load() -> Result<Self, KeyringError> {
        let location = Self::get_location()?;
        if location.exists() {
            load_json_file(&location).map_err(LoadMockKeyringFailed)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self) -> Result<(), KeyringError> {
        let location = Self::get_location()?;
        save_json_file(&location, self).map_err(SaveMockKeyringFailed)
    }
}

#[context(
    "Failed to load PEM file from keyring for identity '{}'.",
    identity_name_suffix
)]
pub fn load_pem_from_keyring(identity_name_suffix: &str) -> DfxResult<Vec<u8>> {
    let keyring_identity_name = keyring_identity_name_from_suffix(identity_name_suffix);
    match KeyringMockMode::current_mode() {
        KeyringMockMode::NoMock => {
            let entry = keyring::Entry::new(KEYRING_SERVICE_NAME, &keyring_identity_name);
            let encoded_pem = entry.get_password()?;
            let pem = hex::decode(&encoded_pem)?;
            Ok(pem)
        }
        KeyringMockMode::MockAvailable => {
            let mock = KeyringMock::load()?;
            let encoded_pem = mock.kv_store.get(&keyring_identity_name).with_context(|| {
                format!("Mock Keyring: key {} not found", &keyring_identity_name)
            })?;
            let pem = hex::decode(encoded_pem)?;
            Ok(pem)
        }
        KeyringMockMode::MockReject => bail!("Mock Keyring not available."),
    }
}

#[context(
    "Failed to write PEM file to keyring for identity '{}'.",
    identity_name_suffix
)]
pub fn write_pem_to_keyring(identity_name_suffix: &str, pem_content: &[u8]) -> DfxResult<()> {
    let keyring_identity_name = keyring_identity_name_from_suffix(identity_name_suffix);
    let encoded_pem = hex::encode(pem_content);
    match KeyringMockMode::current_mode() {
        KeyringMockMode::NoMock => {
            let entry = keyring::Entry::new(KEYRING_SERVICE_NAME, &keyring_identity_name);
            entry.set_password(&encoded_pem)?;
            Ok(())
        }
        KeyringMockMode::MockAvailable => {
            let mut mock = KeyringMock::load()?;
            mock.kv_store.insert(keyring_identity_name, encoded_pem);
            mock.save()?;
            Ok(())
        }
        KeyringMockMode::MockReject => bail!("Mock Keyring not available."),
    }
}

/// Determines if keyring is available by trying to write a dummy entry.
pub fn keyring_available(log: &Logger) -> bool {
    match KeyringMockMode::current_mode() {
        KeyringMockMode::NoMock => {
            trace!(log, "Checking for keyring availability.");
            // by using the temp identity prefix this will not clash with real identities since that would be an invalid identity name
            let dummy_entry_name = format!(
                "{}{}{}",
                KEYRING_IDENTITY_PREFIX, TEMP_IDENTITY_PREFIX, "dummy"
            );
            let entry = keyring::Entry::new(KEYRING_SERVICE_NAME, &dummy_entry_name);
            entry.set_password("dummy entry").is_ok()
        }
        KeyringMockMode::MockReject => false,
        KeyringMockMode::MockAvailable => true,
    }
}

pub fn delete_pem_from_keyring(identity_name_suffix: &str) -> Result<(), KeyringError> {
    let keyring_identity_name = keyring_identity_name_from_suffix(identity_name_suffix);
    match KeyringMockMode::current_mode() {
        KeyringMockMode::NoMock => {
            let entry = keyring::Entry::new(KEYRING_SERVICE_NAME, &keyring_identity_name);
            if entry.get_password().is_ok() {
                entry.delete_password().map_err(DeletePasswordFailed)?;
            }
        }
        KeyringMockMode::MockAvailable => {
            let mut mock = KeyringMock::load()?;
            mock.kv_store.remove(&keyring_identity_name);
            mock.save()?;
        }
        KeyringMockMode::MockReject => return Err(MockUnavailable()),
    }
    Ok(())
}
