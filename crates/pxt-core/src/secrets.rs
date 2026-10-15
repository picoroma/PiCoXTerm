//! `KeyringSecretProvider`: segreti nel keyring di sistema (Credential Manager su Windows).

use crate::error::{CoreError, Result};
use crate::model::CredentialRef;
use crate::provider::{Secret, SecretProvider};

pub struct KeyringSecretProvider {
    service: String,
}

impl KeyringSecretProvider {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, r: &CredentialRef) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, &r.id).map_err(|e| CoreError::Keyring(e.to_string()))
    }
}

impl SecretProvider for KeyringSecretProvider {
    async fn get(&self, r: &CredentialRef) -> Result<Option<Secret>> {
        match self.entry(r)?.get_password() {
            Ok(p) => Ok(Some(Secret::new(p))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(CoreError::Keyring(e.to_string())),
        }
    }

    async fn set(&self, r: &CredentialRef, s: Secret) -> Result<()> {
        self.entry(r)?
            .set_password(s.expose())
            .map_err(|e| CoreError::Keyring(e.to_string()))
    }

    async fn delete(&self, r: &CredentialRef) -> Result<()> {
        match self.entry(r)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(CoreError::Keyring(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn set_get_delete_roundtrip() {
        let p = KeyringSecretProvider::new("PiCoXterm-test");
        let r = CredentialRef {
            provider: "local".into(),
            id: uuid::Uuid::new_v4().to_string(),
        };
        assert!(p.get(&r).await.unwrap().is_none());
        p.set(&r, Secret::new("s3greto")).await.unwrap();
        assert_eq!(p.get(&r).await.unwrap().unwrap().expose(), "s3greto");
        p.delete(&r).await.unwrap();
        assert!(p.get(&r).await.unwrap().is_none());
        // eliminare di nuovo non è un errore
        p.delete(&r).await.unwrap();
    }

    #[test]
    fn secret_debug_is_redacted() {
        assert_eq!(format!("{:?}", Secret::new("x")), "Secret(***)");
    }
}
