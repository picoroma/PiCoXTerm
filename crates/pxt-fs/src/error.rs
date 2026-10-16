#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("percorso non trovato: {0}")]
    NotFound(String),
    #[error("permesso negato: {0}")]
    PermissionDenied(String),
    #[error("errore SFTP: {0}")]
    Sftp(String),
    #[error("file troppo grande per l'editor ({size} byte, limite {max})")]
    TooLarge { size: u64, max: u64 },
    #[error("il file non sembra di testo (contiene dati binari)")]
    NotText,
    #[error("non è un file regolare: {0}")]
    NotRegularFile(String),
    #[error("il carattere \"{0}\" non è rappresentabile nella codifica del file (ISO-8859-1)")]
    Unencodable(char),
}

impl FsError {
    /// Traduce un errore del protocollo, tenendo il percorso a cui si riferiva.
    pub(crate) fn from_sftp(e: russh_sftp::client::error::Error, path: &str) -> Self {
        use russh_sftp::client::error::Error;
        use russh_sftp::protocol::StatusCode;
        match &e {
            Error::Status(s) if s.status_code == StatusCode::NoSuchFile => {
                FsError::NotFound(path.to_string())
            }
            Error::Status(s) if s.status_code == StatusCode::PermissionDenied => {
                FsError::PermissionDenied(path.to_string())
            }
            _ => FsError::Sftp(e.to_string()),
        }
    }
}
