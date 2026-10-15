//! Filesystem remoto unico per tutti i file browser (sezione 3.3 delle specifiche).
//! Questo crate non dipende da Tauri né da `pxt-ssh`: il backend SFTP lavora su un qualsiasi
//! flusso `AsyncRead + AsyncWrite`, che chi lo usa ottiene dalla connessione SSH.

mod entry;
mod error;
mod sftp;

pub use entry::{Entry, EntryKind, FsCapabilities};
pub use error::FsError;
pub use sftp::SftpFs;

use std::future::Future;

pub type Result<T> = std::result::Result<T, FsError>;

/// Operazioni di sola lettura sul filesystem remoto. Le operazioni di modifica e il trasferimento
/// in streaming arrivano nei passi successivi di M2.
pub trait RemoteFs: Send + Sync {
    /// Contenuto di una cartella (senza `.` e `..`), non ordinato.
    fn list(&self, path: &str) -> impl Future<Output = Result<Vec<Entry>>> + Send;
    /// Informazioni su un percorso (segue i link simbolici).
    fn stat(&self, path: &str) -> impl Future<Output = Result<Entry>> + Send;
    /// Percorso assoluto e normalizzato; `"."` è la cartella di partenza (di norma la home).
    fn canonicalize(&self, path: &str) -> impl Future<Output = Result<String>> + Send;
    fn capabilities(&self) -> FsCapabilities;
}
