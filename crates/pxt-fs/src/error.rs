#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("percorso non trovato: {0}")]
    NotFound(String),
    #[error("permesso negato: {0}")]
    PermissionDenied(String),
    #[error("errore SFTP: {0}")]
    Sftp(String),
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
