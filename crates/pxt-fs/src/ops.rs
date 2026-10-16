//! Operazioni di modifica sul filesystem remoto: eliminazione ricorsiva, chmod ricorsivo e dimensione
//! di un albero. Regola comune: **i link simbolici non si seguono mai** (si trattano come file),
//! così un link verso un'altra cartella non porta a toccarne il contenuto.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use crate::{EntryKind, FsError, RemoteFs, Result};

/// Oltre questo numero di voci il calcolo della dimensione si ferma e il risultato è parziale.
pub const MAX_TREE_ENTRIES: u64 = 200_000;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeInfo {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// Il conteggio si è fermato al limite o è stato annullato: i numeri sono per difetto.
    pub partial: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpFailure {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpReport {
    /// Elementi trattati con successo.
    pub done: u64,
    pub failed: Vec<OpFailure>,
}

fn fail(report: &mut OpReport, path: &str, e: impl ToString) {
    report.failed.push(OpFailure {
        path: path.to_string(),
        message: e.to_string(),
    });
}

/// Rifiuta percorsi che nessuno vuole davvero modificare in blocco.
fn guard(path: &str) -> Result<()> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.chars().all(|c| c == '/') || trimmed == "." || trimmed == ".."
    {
        return Err(FsError::Sftp(format!(
            "operazione rifiutata sul percorso \"{path}\""
        )));
    }
    Ok(())
}

/// Conta file, cartelle e byte sotto `path` (compreso). I link simbolici contano come file.
pub async fn tree_info<F: RemoteFs>(fs: &F, path: &str, cancel: &AtomicBool) -> Result<TreeInfo> {
    let mut info = TreeInfo::default();
    let top = fs.lstat(path).await?;
    if top.kind != EntryKind::Dir {
        info.files = 1;
        info.bytes = top.size;
        return Ok(info);
    }
    let mut stack = vec![path.to_string()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) || info.files + info.dirs > MAX_TREE_ENTRIES {
            info.partial = true;
            break;
        }
        info.dirs += 1;
        for child in fs.list(&dir).await? {
            if !child.is_symlink && child.kind == EntryKind::Dir {
                stack.push(child.path);
            } else {
                info.files += 1;
                info.bytes += child.size;
            }
        }
    }
    Ok(info)
}

/// Elimina `path` e tutto ciò che contiene. Gli errori su singole voci non fermano il resto: finiscono
/// nel rapporto (una cartella con voci non eliminabili resta, e viene segnalata).
pub async fn remove_recursive<F: RemoteFs>(
    fs: &F,
    path: &str,
    cancel: &AtomicBool,
) -> Result<OpReport> {
    guard(path)?;
    let mut report = OpReport::default();
    let top = fs.lstat(path).await?;
    // File, link simbolici (anche verso cartelle) e altro: si toglie la voce stessa, senza seguirla.
    if top.is_symlink || top.kind != EntryKind::Dir {
        match fs.remove_file(path).await {
            Ok(()) => report.done += 1,
            Err(e) => fail(&mut report, path, e),
        }
        return Ok(report);
    }

    // Visita in profondità: ogni cartella si elimina dopo il suo contenuto.
    let mut stack: Vec<(String, bool)> = vec![(path.to_string(), false)];
    while let Some((dir, contents_done)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return Err(FsError::Sftp("eliminazione annullata".into()));
        }
        if contents_done {
            match fs.remove_dir(&dir).await {
                Ok(()) => report.done += 1,
                Err(e) => fail(&mut report, &dir, e),
            }
            continue;
        }
        stack.push((dir.clone(), true));
        match fs.list(&dir).await {
            Ok(children) => {
                for child in children {
                    if !child.is_symlink && child.kind == EntryKind::Dir {
                        stack.push((child.path, false));
                    } else {
                        match fs.remove_file(&child.path).await {
                            Ok(()) => report.done += 1,
                            Err(e) => fail(&mut report, &child.path, e),
                        }
                    }
                }
            }
            Err(e) => fail(&mut report, &dir, e),
        }
    }
    Ok(report)
}

/// Imposta i permessi (`mode`, bit `0o7777`) su `path` e, se `recursive`, su tutto ciò che contiene.
/// Come `chmod -R`: lo stesso valore a file e cartelle; i link simbolici vengono saltati.
pub async fn chmod<F: RemoteFs>(
    fs: &F,
    path: &str,
    mode: u32,
    recursive: bool,
    cancel: &AtomicBool,
) -> Result<OpReport> {
    guard(path)?;
    let mut report = OpReport::default();
    let top = fs.lstat(path).await?;
    if top.is_symlink {
        return Err(FsError::Sftp(
            "i permessi di un link simbolico non si modificano: si modifica il suo bersaglio"
                .into(),
        ));
    }
    let mut stack = vec![(path.to_string(), top.kind == EntryKind::Dir)];
    while let Some((p, is_dir)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return Err(FsError::Sftp("operazione annullata".into()));
        }
        match fs.set_permissions(&p, mode).await {
            Ok(()) => report.done += 1,
            Err(e) => fail(&mut report, &p, e),
        }
        if recursive && is_dir {
            match fs.list(&p).await {
                Ok(children) => {
                    for c in children {
                        if !c.is_symlink && matches!(c.kind, EntryKind::Dir | EntryKind::File) {
                            stack.push((c.path, c.kind == EntryKind::Dir));
                        }
                    }
                }
                Err(e) => fail(&mut report, &p, e),
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_refuses_root_and_empty_paths() {
        for bad in ["", "   ", "/", "//", ".", ".."] {
            assert!(guard(bad).is_err(), "{bad:?}");
        }
        assert!(guard("/tmp/x").is_ok());
        assert!(guard("relativo/dir").is_ok());
    }
}
