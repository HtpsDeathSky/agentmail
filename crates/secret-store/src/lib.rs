use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("secret not found: {0}")]
    NotFound(String),
    #[error("secret backend error: {0}")]
    Backend(String),
}

pub type SecretResult<T> = Result<T, SecretError>;

pub trait SecretStore: Send + Sync {
    fn set_secret(&self, target: &str, secret: &str) -> SecretResult<()>;
    fn get_secret(&self, target: &str) -> SecretResult<String>;
    fn delete_secret(&self, target: &str) -> SecretResult<()>;
}

pub fn mail_password_target(account_id: &str) -> String {
    format!("AgentMail/mail-password/{account_id}")
}

#[derive(Clone, Default)]
pub struct MemorySecretStore {
    values: Arc<Mutex<HashMap<String, String>>>,
}

impl SecretStore for MemorySecretStore {
    fn set_secret(&self, target: &str, secret: &str) -> SecretResult<()> {
        self.values
            .lock()
            .insert(target.to_string(), secret.to_string());
        Ok(())
    }

    fn get_secret(&self, target: &str) -> SecretResult<String> {
        self.values
            .lock()
            .get(target)
            .cloned()
            .ok_or_else(|| SecretError::NotFound(target.to_string()))
    }

    fn delete_secret(&self, target: &str) -> SecretResult<()> {
        self.values.lock().remove(target);
        Ok(())
    }
}

#[cfg(not(windows))]
pub type PlatformSecretStore = MemorySecretStore;

#[cfg(windows)]
mod windows_store {
    use super::{SecretError, SecretResult, SecretStore};
    use std::ffi::c_void;
    use std::ptr::null_mut;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_FLAGS,
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };

    #[derive(Clone, Default)]
    pub struct WindowsCredentialStore;

    impl WindowsCredentialStore {
        fn to_wide(value: &str) -> Vec<u16> {
            value.encode_utf16().chain(std::iter::once(0)).collect()
        }
    }

    impl SecretStore for WindowsCredentialStore {
        fn set_secret(&self, target: &str, secret: &str) -> SecretResult<()> {
            let target_w = Self::to_wide(target);
            let mut target_w = target_w;
            let mut user_w = Self::to_wide("AgentMail");
            let secret_bytes = secret.as_bytes();
            let mut credential = CREDENTIALW {
                Flags: CRED_FLAGS(0),
                Type: CRED_TYPE_GENERIC,
                TargetName: PWSTR(target_w.as_mut_ptr()),
                Comment: PWSTR::null(),
                LastWritten: FILETIME::default(),
                CredentialBlobSize: secret_bytes.len() as u32,
                CredentialBlob: secret_bytes.as_ptr() as *mut u8,
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                AttributeCount: 0,
                Attributes: null_mut(),
                TargetAlias: PWSTR::null(),
                UserName: PWSTR(user_w.as_mut_ptr()),
            };

            unsafe {
                CredWriteW(&mut credential, 0)
                    .map_err(|err| SecretError::Backend(err.message().to_string()))?;
            }
            Ok(())
        }

        fn get_secret(&self, target: &str) -> SecretResult<String> {
            let target_w = Self::to_wide(target);
            let mut credential_ptr: *mut CREDENTIALW = null_mut();
            unsafe {
                CredReadW(
                    PCWSTR(target_w.as_ptr()),
                    CRED_TYPE_GENERIC,
                    0,
                    &mut credential_ptr,
                )
                .map_err(|_| SecretError::NotFound(target.to_string()))?;

                let credential = &*credential_ptr;
                let blob = std::slice::from_raw_parts(
                    credential.CredentialBlob as *const u8,
                    credential.CredentialBlobSize as usize,
                );
                let value = String::from_utf8(blob.to_vec())
                    .map_err(|err| SecretError::Backend(err.to_string()));
                CredFree(credential_ptr as *const c_void);
                value
            }
        }

        fn delete_secret(&self, target: &str) -> SecretResult<()> {
            let target_w = Self::to_wide(target);
            unsafe {
                CredDeleteW(PCWSTR(target_w.as_ptr()), CRED_TYPE_GENERIC, 0)
                    .map_err(|err| SecretError::Backend(err.message().to_string()))?;
            }
            Ok(())
        }
    }

    pub type PlatformSecretStore = WindowsCredentialStore;
}

#[cfg(windows)]
pub use windows_store::PlatformSecretStore;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trips_secret() {
        let store = MemorySecretStore::default();
        store.set_secret("target", "secret").unwrap();
        assert_eq!(store.get_secret("target").unwrap(), "secret");
        store.delete_secret("target").unwrap();
        assert!(store.get_secret("target").is_err());
    }
}
