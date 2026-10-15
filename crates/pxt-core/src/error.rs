#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("errore di I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("file delle sessioni non valido: {0}")]
    Json(#[from] serde_json::Error),
    #[error("versione dello schema non supportata: {found} (massima nota: {supported})")]
    UnsupportedSchema { found: u32, supported: u32 },
    #[error("nodo non trovato: {0}")]
    NotFound(uuid::Uuid),
    #[error("provider in sola lettura")]
    ReadOnly,
    #[error("keyring: {0}")]
    Keyring(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;
