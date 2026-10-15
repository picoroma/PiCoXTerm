//! Modello dati, provider di sessioni e segreti, policy, errori.
//! Questo crate non dipende da Tauri.

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
