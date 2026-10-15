//! Modello dati delle sessioni (sezione 4 delle specifiche). Qui non ci sono mai segreti.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::settings::{self, Theme};

pub const SCHEMA_VERSION: u32 = 1;

/// Riferimento a una credenziale custodita da un `SecretProvider`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRef {
    pub provider: String,
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Ssh,
    Sftp,
    Ftp,
    S3,
    Rdp,
    Vnc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMethod {
    Password,
    Key,
    Agent,
    KeyboardInteractive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Auth {
    pub method: AuthMethod,
    /// Percorso del file della chiave privata (solo per `method: key`). Mai il contenuto.
    #[serde(default)]
    pub key_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FollowCwd {
    Auto,
    Osc7Only,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshOptions {
    pub follow_cwd: FollowCwd,
    pub keep_alive_secs: u32,
    pub x11_forwarding: bool,
    pub initial_command: Option<String>,
    pub terminal_type: String,
    pub encoding: String,
}

impl Default for SshOptions {
    fn default() -> Self {
        Self {
            follow_cwd: FollowCwd::Auto,
            keep_alive_secs: 30,
            x11_forwarding: false,
            initial_command: None,
            terminal_type: "xterm-256color".into(),
            encoding: "utf-8".into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpOptions {
    pub start_path: Option<String>,
    pub show_hidden: bool,
}

/// Aspetto della sessione. I campi `None` ereditano dalle impostazioni globali.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiOptions {
    /// Colore della scheda e dell'albero, `#rrggbb`.
    pub color: Option<String>,
    pub icon: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Tema del terminale per questa sessione.
    #[serde(default)]
    pub theme: Option<Theme>,
    #[serde(default)]
    pub font_family: Option<String>,
    #[serde(default)]
    pub font_size: Option<u8>,
    /// Sfondo del terminale (sovrascrive quello del tema), `#rrggbb`.
    #[serde(default)]
    pub background: Option<String>,
}

fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

impl UiOptions {
    /// I colori finiscono in stili CSS: accetta solo `#rrggbb`. Riporta anche la dimensione del font nei limiti.
    pub fn sanitized(mut self) -> std::result::Result<Self, String> {
        for (name, value) in [("colore", &self.color), ("sfondo", &self.background)] {
            if let Some(v) = value {
                if !is_hex_color(v) {
                    return Err(format!("{name} non valido: atteso #rrggbb"));
                }
            }
        }
        self.font_size = self
            .font_size
            .map(|s| s.clamp(settings::MIN_FONT_SIZE, settings::MAX_FONT_SIZE));
        self.font_family = self.font_family.filter(|f| !f.trim().is_empty());
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: Uuid,
    pub name: String,
    pub parent_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub credential_ref: Option<CredentialRef>,
    pub auth: Auth,
    #[serde(default)]
    pub jump_hosts: Vec<Uuid>,
    #[serde(default)]
    pub ssh: SshOptions,
    #[serde(default)]
    pub sftp: SftpOptions,
    #[serde(default)]
    pub ui: UiOptions,
    #[serde(default)]
    pub notes: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Session {
    /// Nuova sessione SSH con password, valori di default per il resto.
    pub fn new_ssh(name: &str, host: &str, port: u16, username: &str) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            parent_id: None,
            name: name.into(),
            protocol: Protocol::Ssh,
            host: host.into(),
            port,
            username: username.into(),
            credential_ref: None,
            auth: Auth {
                method: AuthMethod::Password,
                key_path: None,
            },
            jump_hosts: Vec::new(),
            ssh: SshOptions::default(),
            sftp: SftpOptions::default(),
            ui: UiOptions::default(),
            notes: String::new(),
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SessionNode {
    Folder(Folder),
    Session(Box<Session>),
}

impl SessionNode {
    pub fn id(&self) -> Uuid {
        match self {
            Self::Folder(f) => f.id,
            Self::Session(s) => s.id,
        }
    }

    pub fn parent_id(&self) -> Option<Uuid> {
        match self {
            Self::Folder(f) => f.parent_id,
            Self::Session(s) => s.parent_id,
        }
    }
}

/// Id del nodo e di tutti i suoi discendenti.
pub fn subtree_ids(nodes: &[SessionNode], root: Uuid) -> Vec<Uuid> {
    let mut ids = vec![root];
    let mut i = 0;
    while i < ids.len() {
        let parent = ids[i];
        ids.extend(
            nodes
                .iter()
                .filter(|n| n.parent_id() == Some(parent))
                .map(|n| n.id()),
        );
        i += 1;
    }
    ids
}

impl CredentialRef {
    /// Nuovo riferimento per una credenziale custodita dal provider locale.
    pub fn new_local() -> Self {
        Self {
            provider: "local".into(),
            id: Uuid::new_v4().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsFile {
    pub schema_version: u32,
    pub nodes: Vec<SessionNode>,
}

impl Default for SessionsFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            nodes: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_matches_spec_shape() {
        let folder = SessionNode::Folder(Folder {
            id: Uuid::new_v4(),
            name: "Clienti".into(),
            parent_id: None,
        });
        let v = serde_json::to_value(&folder).unwrap();
        assert_eq!(v["type"], "folder");
        assert!(v["parentId"].is_null());

        let mut s = Session::new_ssh("BGP PRD", "10.0.1.15", 22, "bgpadm");
        s.credential_ref = Some(CredentialRef {
            provider: "local".into(),
            id: "c9".into(),
        });
        let v = serde_json::to_value(SessionNode::Session(Box::new(s))).unwrap();
        assert_eq!(v["type"], "session");
        assert_eq!(v["protocol"], "ssh");
        assert_eq!(v["auth"]["method"], "password");
        assert_eq!(v["ssh"]["followCwd"], "auto");
        assert_eq!(v["credentialRef"]["provider"], "local");
        // Nessun campo che possa contenere una password
        assert!(v.get("password").is_none());
    }

    #[test]
    fn ui_options_validate_colors_and_clamp_font() {
        let ok = UiOptions {
            color: Some("#c0392b".into()),
            background: Some("#2a0000".into()),
            font_size: Some(200),
            font_family: Some("  ".into()),
            theme: Some(Theme::Dracula),
            ..UiOptions::default()
        }
        .sanitized()
        .unwrap();
        assert_eq!(ok.font_size, Some(settings::MAX_FONT_SIZE));
        assert_eq!(ok.font_family, None);

        for bad in ["red", "#12345", "#gggggg", "url(x)", "#ff0000;x"] {
            let r = UiOptions {
                background: Some(bad.into()),
                ..UiOptions::default()
            }
            .sanitized();
            assert!(r.is_err(), "{bad} doveva essere rifiutato");
        }
    }

    #[test]
    fn old_ui_block_without_new_fields_still_loads() {
        let ui: UiOptions =
            serde_json::from_str(r#"{"color":null,"icon":null,"tags":["PRD"]}"#).unwrap();
        assert_eq!(ui.theme, None);
        assert_eq!(ui.tags, vec!["PRD"]);
    }

    #[test]
    fn optional_blocks_default_when_missing() {
        let json = r#"{"type":"session","id":"7b0f6c2e-0000-4000-8000-000000000001","parentId":null,
            "name":"x","protocol":"ssh","host":"h","port":22,"username":"u","credentialRef":null,
            "auth":{"method":"agent"},"createdAt":"2026-10-15T08:00:00Z","updatedAt":"2026-10-15T08:00:00Z"}"#;
        let n: SessionNode = serde_json::from_str(json).unwrap();
        let SessionNode::Session(s) = n else {
            panic!("atteso session")
        };
        assert_eq!(s.ssh.keep_alive_secs, 30);
        assert!(s.jump_hosts.is_empty());
    }
}
