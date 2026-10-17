//! Filesystem remoto unico per tutti i file browser (sezione 3.3 delle specifiche).
//! Questo crate non dipende da Tauri né da `pxt-ssh`: il backend SFTP lavora su un qualsiasi
//! flusso `AsyncRead + AsyncWrite`, che chi lo usa ottiene dalla connessione SSH.

pub mod edit;
mod entry;
mod error;
pub mod external;
pub mod local;
pub mod ops;
mod sftp;
pub mod transfer;

pub use entry::{Entry, EntryKind, FsCapabilities};
pub use error::FsError;
pub use local::LocalFs;
pub use sftp::SftpFs;

use std::future::Future;

use tokio::io::{AsyncRead, AsyncWrite};

pub type Result<T> = std::result::Result<T, FsError>;

/// Filesystem remoto. Il contenuto dei file passa solo da flussi (`Reader`, `Writer`): mai un file
/// intero in memoria (regola 7).
pub trait RemoteFs: Send + Sync {
    type Reader: AsyncRead + Unpin + Send + 'static;
    type Writer: AsyncWrite + Unpin + Send + 'static;

    /// Contenuto di una cartella (senza `.` e `..`), non ordinato.
    fn list(&self, path: &str) -> impl Future<Output = Result<Vec<Entry>>> + Send;
    /// Informazioni su un percorso (segue i link simbolici).
    fn stat(&self, path: &str) -> impl Future<Output = Result<Entry>> + Send;
    /// Come `stat`, ma **senza seguire** il link simbolico finale: per un link `is_symlink` è vero e
    /// `kind` è `Other`.
    fn lstat(&self, path: &str) -> impl Future<Output = Result<Entry>> + Send;
    /// Percorso assoluto e normalizzato; `"."` è la cartella di partenza (di norma la home).
    fn canonicalize(&self, path: &str) -> impl Future<Output = Result<String>> + Send;

    fn mkdir(&self, path: &str) -> impl Future<Output = Result<()>> + Send;
    fn remove_file(&self, path: &str) -> impl Future<Output = Result<()>> + Send;
    fn remove_dir(&self, path: &str) -> impl Future<Output = Result<()>> + Send;
    /// Rinomina o sposta. Non sovrascrive: se `to` esiste va tolto prima.
    fn rename(&self, from: &str, to: &str) -> impl Future<Output = Result<()>> + Send;
    /// Imposta i permessi (solo i bit `0o7777`).
    fn set_permissions(&self, path: &str, mode: u32) -> impl Future<Output = Result<()>> + Send;
    /// Imposta la data di ultima modifica (secondi dal 1970).
    fn set_modified(&self, path: &str, secs: i64) -> impl Future<Output = Result<()>> + Send;

    /// Apre un file in lettura.
    fn open_read(&self, path: &str) -> impl Future<Output = Result<Self::Reader>> + Send;
    /// Crea (o svuota) un file e lo apre in scrittura.
    fn open_write(&self, path: &str) -> impl Future<Output = Result<Self::Writer>> + Send;
    /// Apre un file in lettura a partire da `offset` (per riprendere un trasferimento).
    fn open_read_at(
        &self,
        path: &str,
        offset: u64,
    ) -> impl Future<Output = Result<Self::Reader>> + Send;
    /// Apre un file **esistente** in scrittura, senza svuotarlo, posizionandosi su `offset`.
    fn open_write_at(
        &self,
        path: &str,
        offset: u64,
    ) -> impl Future<Output = Result<Self::Writer>> + Send;

    fn capabilities(&self) -> FsCapabilities;
}
