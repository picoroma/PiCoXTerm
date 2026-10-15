//! Connessione SSH, canale shell, known_hosts. Non dipende da Tauri.

mod error;
mod known_hosts;
mod shell;

pub use error::SshError;
pub use known_hosts::{HostKeyStatus, KnownHosts};
pub use shell::{connect_shell, ConnectParams, HostKeyInfo, HostKeyVerifier, Shell, ShellEvent};
