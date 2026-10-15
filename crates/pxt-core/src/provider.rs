//! Astrazioni centrali (sezione 3.3): sessioni e segreti passano solo da questi trait.

use std::future::Future;

use uuid::Uuid;
use zeroize::Zeroize;

use crate::error::Result;
use crate::model::{CredentialRef, SessionNode};

/// Fonte dell'albero delle sessioni (locale in v1, RDM in M6).
pub trait SessionProvider: Send + Sync {
    fn id(&self) -> &str;
    fn is_read_only(&self) -> bool;
    fn list(&self) -> impl Future<Output = Result<Vec<SessionNode>>> + Send;
    /// Inserisce o aggiorna (per id) un nodo.
    fn save(&self, node: SessionNode) -> impl Future<Output = Result<()>> + Send;
    /// Inserisce o aggiorna più nodi. Le implementazioni possono ottimizzarlo in una sola scrittura.
    fn save_many(&self, nodes: Vec<SessionNode>) -> impl Future<Output = Result<()>> + Send {
        async move {
            for node in nodes {
                self.save(node).await?;
            }
            Ok(())
        }
    }
    /// Elimina il nodo; per una cartella elimina anche tutto il contenuto.
    fn delete(&self, id: Uuid) -> impl Future<Output = Result<()>> + Send;
}

/// Segreto in memoria: azzerato al drop, mai stampato (SEC-11).
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// Fonte delle credenziali (keyring in v1, RDM in M6).
pub trait SecretProvider: Send + Sync {
    fn get(&self, r: &CredentialRef) -> impl Future<Output = Result<Option<Secret>>> + Send;
    fn set(&self, r: &CredentialRef, s: Secret) -> impl Future<Output = Result<()>> + Send;
    fn delete(&self, r: &CredentialRef) -> impl Future<Output = Result<()>> + Send;
}
