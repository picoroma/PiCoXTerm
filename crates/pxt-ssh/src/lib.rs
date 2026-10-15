//! Connessione SSH, canale shell, known_hosts. Non dipende da Tauri.

mod auth;
mod error;
mod inject;
mod known_hosts;
mod osc7;
mod shell;

pub use auth::Credentials;
pub use error::SshError;
pub use known_hosts::{HostKeyStatus, KnownHostEntry, KnownHosts};
pub use shell::{
    connect_shell, ConnectParams, Hop, HostKeyDecision, HostKeyInfo, HostKeyVerifier, Shell,
    ShellEvent,
};
