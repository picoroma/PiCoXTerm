//! Modifica con l'editor di sistema (sezione 7.6): il file remoto si scarica in una cartella
//! temporanea, lo apre il programma associato dal sistema e, quando viene salvato, si ricarica sul
//! server. A differenza dell'editor integrato il contenuto passa dal disco, quindi qui il file
//! temporaneo va sempre eliminato (lo fa chi chiama). Il trasferimento è in streaming.

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;

use crate::edit::{SaveOutcome, Stamp};
use crate::transfer::local_name;
use crate::{EntryKind, FsError, RemoteFs, Result};

/// Dimensione massima di un file da aprire con l'editor di sistema.
pub const MAX_EXTERNAL_BYTES: u64 = 100 * 1024 * 1024;

/// Un file remoto scaricato per essere modificato.
#[derive(Debug, Clone)]
pub struct Fetched {
    pub local_path: PathBuf,
    /// Stato del file remoto al momento del download, per accorgersi di modifiche concorrenti.
    pub stamp: Stamp,
}

fn io_err(e: std::io::Error) -> FsError {
    FsError::Sftp(e.to_string())
}

/// Scarica `remote_path` dentro `local_dir` (che deve esistere) e restituisce il percorso locale.
pub async fn fetch<F: RemoteFs>(
    fs: &F,
    remote_path: &str,
    local_dir: &Path,
    max: u64,
) -> Result<Fetched> {
    let meta = fs.stat(remote_path).await?;
    if meta.kind != EntryKind::File {
        return Err(FsError::NotRegularFile(remote_path.to_string()));
    }
    if meta.size > max {
        return Err(FsError::TooLarge {
            size: meta.size,
            max,
        });
    }
    let local_path = local_dir.join(local_name(&meta.name));
    let mut reader = fs.open_read(remote_path).await?;
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    // Il file può contenere dati riservati: sui sistemi Unix lo legge solo il proprietario.
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&local_path).await.map_err(io_err)?;
    let copied = tokio::io::copy(&mut reader, &mut file).await;
    let finished = match copied {
        Ok(_) => file.flush().await,
        Err(e) => Err(e),
    };
    drop(file);
    if let Err(e) = finished {
        let _ = tokio::fs::remove_file(&local_path).await;
        return Err(io_err(e));
    }
    Ok(Fetched {
        local_path,
        stamp: Stamp {
            size: meta.size,
            modified: meta.modified,
        },
    })
}

/// Ricarica `local_path` su `remote_path`. Se `base` è presente e il file remoto è cambiato dopo il
/// download (o è sparito) non scrive nulla e restituisce `Conflict`.
///
/// Come per l'editor integrato (D-20) la scrittura è **sul posto**: permessi, proprietario e link
/// restano invariati, e funziona anche quando si può scrivere il file ma non la sua cartella.
pub async fn push<F: RemoteFs>(
    fs: &F,
    local_path: &Path,
    remote_path: &str,
    base: Option<Stamp>,
    max: u64,
) -> Result<SaveOutcome> {
    let len = tokio::fs::metadata(local_path).await.map_err(io_err)?.len();
    if len > max {
        return Err(FsError::TooLarge { size: len, max });
    }
    if let Some(base) = base {
        match fs.stat(remote_path).await {
            Ok(now) => {
                if now.size != base.size || now.modified != base.modified {
                    return Ok(SaveOutcome::Conflict {
                        size: Some(now.size),
                        modified: now.modified,
                        missing: false,
                    });
                }
            }
            Err(FsError::NotFound(_)) => {
                return Ok(SaveOutcome::Conflict {
                    size: None,
                    modified: None,
                    missing: true,
                })
            }
            Err(e) => return Err(e),
        }
    }
    let mut src = tokio::fs::File::open(local_path).await.map_err(io_err)?;
    let mut writer = fs.open_write(remote_path).await?;
    tokio::io::copy(&mut src, &mut writer)
        .await
        .map_err(io_err)?;
    writer.shutdown().await.map_err(io_err)?;
    drop(writer);
    let after = fs.stat(remote_path).await?;
    Ok(SaveOutcome::Saved {
        size: after.size,
        modified: after.modified,
    })
}
