//! Modello dati, provider di sessioni e segreti, policy, errori.
//! Questo crate non dipende da Tauri.

pub mod error;
pub mod local;
pub mod model;
pub mod provider;
pub mod secrets;

pub use error::{CoreError, Result};
pub use local::LocalSessionProvider;
pub use model::*;
pub use provider::{Secret, SecretProvider, SessionProvider};
pub use secrets::KeyringSecretProvider;

/// Nome del prodotto, usato per cartelle dati e keyring.
pub const APP_NAME: &str = "PiCoXterm";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_name_is_set() {
        assert_eq!(APP_NAME, "PiCoXterm");
    }
}
