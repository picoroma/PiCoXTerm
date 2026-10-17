//! Policy dell'organizzazione (sezione 5.4 delle specifiche): un file `policy.json` che l'amministratore
//! mette accanto ai dati dell'app. L'app lo **legge soltanto**, non lo scrive mai. Senza file valgono i
//! valori predefiniti (tutto consentito, con gli avvisi). Un file che non si legge o non si capisce non
//! allarga mai i permessi: si applica la versione più restrittiva e si segnala il problema.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::Protocol;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Policy {
    /// Consente le sessioni FTP **in chiaro** (password e dati senza cifratura).
    pub allow_plain_ftp: bool,
    /// Protocolli consentiti; assente = tutti.
    pub allowed_protocols: Option<Vec<Protocol>>,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            allow_plain_ftp: true,
            allowed_protocols: None,
        }
    }
}

impl Policy {
    /// Ciò che si applica quando il file c'è ma non è utilizzabile: niente FTP in chiaro.
    pub fn restrictive() -> Policy {
        Policy {
            allow_plain_ftp: false,
            allowed_protocols: None,
        }
    }

    /// Legge `path`. Restituisce la policy e, se il file esiste ma non è valido, un avviso.
    pub fn load(path: &Path) -> (Policy, Option<String>) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Policy::default(), None),
            Err(e) => {
                return (
                    Policy::restrictive(),
                    Some(format!(
                        "policy.json non leggibile ({e}): applico la versione restrittiva"
                    )),
                )
            }
        };
        match serde_json::from_str::<Policy>(&text) {
            Ok(p) => (p, None),
            Err(e) => (
                Policy::restrictive(),
                Some(format!(
                    "policy.json non valido ({e}): applico la versione restrittiva"
                )),
            ),
        }
    }

    pub fn allows(&self, protocol: Protocol) -> bool {
        self.allowed_protocols
            .as_ref()
            .is_none_or(|list| list.contains(&protocol))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("pxt-policy-{name}-{n}.json"))
    }

    #[test]
    fn a_missing_file_means_everything_is_allowed() {
        let (p, warning) = Policy::load(&temp("missing"));
        assert_eq!(p, Policy::default());
        assert!(warning.is_none());
        assert!(p.allow_plain_ftp && p.allows(Protocol::Rdp));
    }

    #[test]
    fn fields_are_read_and_missing_ones_keep_their_defaults() {
        let f = temp("ok");
        std::fs::write(
            &f,
            r#"{"allowPlainFtp": false, "allowedProtocols": ["ssh", "sftp"]}"#,
        )
        .unwrap();
        let (p, warning) = Policy::load(&f);
        assert!(warning.is_none());
        assert!(!p.allow_plain_ftp);
        assert!(p.allows(Protocol::Ssh) && p.allows(Protocol::Sftp));
        assert!(!p.allows(Protocol::Ftp) && !p.allows(Protocol::S3));

        std::fs::write(&f, r#"{"allowedProtocols": ["ssh"]}"#).unwrap();
        let (p, _) = Policy::load(&f);
        assert!(
            p.allow_plain_ftp,
            "i campi assenti restano ai valori predefiniti"
        );
        std::fs::remove_file(&f).unwrap();
    }

    #[test]
    fn an_invalid_file_never_loosens_the_policy() {
        let f = temp("bad");
        std::fs::write(&f, "{ non è json").unwrap();
        let (p, warning) = Policy::load(&f);
        assert!(!p.allow_plain_ftp);
        assert!(warning.unwrap().contains("restrittiva"));
        std::fs::remove_file(&f).unwrap();
    }
}
