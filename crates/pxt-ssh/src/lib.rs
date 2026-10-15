//! Connessione SSH, canale shell, known_hosts. Non dipende da Tauri.

mod auth;
mod error;
mod inject;
mod known_hosts;
mod osc7;
mod shell;

pub use auth::Credentials;
pub use error::SshError;
pub use known_hosts::{HostKeyStatus, KnownHosts};
pub use shell::{connect_shell, ConnectParams, HostKeyInfo, HostKeyVerifier, Shell, ShellEvent};
