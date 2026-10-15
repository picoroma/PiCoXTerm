//! Motore di trasferimento, generico sul filesystem remoto (sezione 7.3 delle specifiche).
//!
//! Funzionamento in due tempi: `plan_upload` / `plan_download` costruiscono il piano (cosa si copia,
//! quali destinazioni esistono già) senza toccare nulla; `run` lo esegue con una politica unica per
//! i conflitti ("applica a tutti"). I dati passano a blocchi da `CHUNK` byte: mai un file intero in
//! memoria. Ogni file viene scritto in un file temporaneo e rinominato solo a copia completata.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{EntryKind, FsError, RemoteFs};

/// Dimensione dei blocchi copiati.
const CHUNK: usize = 256 * 1024;
/// Suffisso dei file temporanei; sparisce con il rename finale.
pub const PART_SUFFIX: &str = ".pxt-part";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Upload,
    Download,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    File,
    Dir,
}

/// Una voce del piano. `src` e `dst` sono percorsi locali o remoti a seconda della direzione.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    pub kind: ItemKind,
    pub src: String,
    pub dst: String,
    pub size: u64,
    /// La destinazione esiste già (solo per i file).
    pub conflict: bool,
    pub mode: Option<u32>,
    pub modified: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub direction: Direction,
    pub items: Vec<PlanItem>,
    pub total_bytes: u64,
    pub file_count: usize,
    pub conflicts: usize,
}

impl Plan {
    fn new(direction: Direction, items: Vec<PlanItem>) -> Self {
        let files = items.iter().filter(|i| i.kind == ItemKind::File);
        Plan {
            direction,
            total_bytes: files.clone().map(|i| i.size).sum(),
            file_count: files.clone().count(),
            conflicts: files.filter(|i| i.conflict).count(),
            items,
        }
    }
}

/// Cosa fare quando la destinazione esiste già.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictPolicy {
    Overwrite,
    Skip,
    /// Copia con un nome nuovo: `nome (1).ext`.
    Rename,
    OverwriteIfNewer,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub files_done: usize,
    pub files_total: usize,
    /// File in corso.
    pub current: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Failure {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub files_done: usize,
    pub files_skipped: usize,
    pub bytes: u64,
    pub failed: Vec<Failure>,
}

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("trasferimento annullato")]
    Cancelled,
    #[error(transparent)]
    Fs(#[from] FsError),
    #[error("errore locale: {0}")]
    Io(String),
}

impl From<std::io::Error> for TransferError {
    fn from(e: std::io::Error) -> Self {
        TransferError::Io(e.to_string())
    }
}

type Result<T> = std::result::Result<T, TransferError>;

// ---- Nomi e percorsi ----

/// Unisce percorsi remoti (sempre con `/`).
pub fn remote_join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Nome adatto al filesystem locale. Su Windows sostituisce i caratteri vietati e toglie punti e
/// spazi finali; altrove lascia il nome com'è.
pub fn local_name(remote_name: &str) -> String {
    if !cfg!(windows) {
        return remote_name.to_string();
    }
    let cleaned: String = remote_name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim_end_matches(['.', ' ']);
    if trimmed.is_empty() {
        "_".to_string()
    } else {
        trimmed.to_string()
    }
}

/// `dir/file.txt` -> `dir/file (2).txt`; senza estensione `dir/file (2)`.
pub fn numbered_name(path: &str, n: u32) -> String {
    let (dir, name) = match path.rfind(['/', '\\']) {
        Some(i) => (&path[..=i], &path[i + 1..]),
        None => ("", path),
    };
    // Il punto iniziale di un file nascosto (`.bashrc`) non è un'estensione.
    match name.rfind('.').filter(|&i| i > 0) {
        Some(i) => format!("{dir}{} ({n}){}", &name[..i], &name[i..]),
        None => format!("{dir}{name} ({n})"),
    }
}

fn secs(t: SystemTime) -> Option<i64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
}

#[cfg(unix)]
fn local_mode(meta: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn local_mode(_: &std::fs::Metadata) -> Option<u32> {
    None
}

// ---- Piano ----

/// Piano per caricare `local` (file e cartelle) dentro la cartella remota `remote_dir`.
/// I link simbolici locali non vengono seguiti.
pub async fn plan_upload<F: RemoteFs>(fs: &F, local: &[PathBuf], remote_dir: &str) -> Result<Plan> {
    let mut items = Vec::new();
    for top in local {
        let name = top
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| TransferError::Io(format!("percorso non valido: {}", top.display())))?;
        let mut stack = vec![(top.clone(), remote_join(remote_dir, &name))];
        while let Some((path, dst)) = stack.pop() {
            let meta = std::fs::symlink_metadata(&path)?;
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                items.push(PlanItem {
                    kind: ItemKind::Dir,
                    src: path.to_string_lossy().into_owned(),
                    dst: dst.clone(),
                    size: 0,
                    conflict: false,
                    mode: local_mode(&meta),
                    modified: meta.modified().ok().and_then(secs),
                });
                let mut children: Vec<_> = std::fs::read_dir(&path)?
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .collect();
                children.sort();
                // In ordine inverso, così si estraggono in ordine alfabetico.
                for child in children.into_iter().rev() {
                    if let Some(n) = child.file_name() {
                        stack.push((child.clone(), remote_join(&dst, &n.to_string_lossy())));
                    }
                }
            } else if meta.is_file() {
                let conflict = fs.stat(&dst).await.is_ok();
                items.push(PlanItem {
                    kind: ItemKind::File,
                    src: path.to_string_lossy().into_owned(),
                    dst,
                    size: meta.len(),
                    conflict,
                    mode: local_mode(&meta),
                    modified: meta.modified().ok().and_then(secs),
                });
            }
        }
    }
    Ok(Plan::new(Direction::Upload, items))
}

/// Piano per scaricare i percorsi remoti `remote` dentro la cartella locale `local_dir`.
/// Le cartelle raggiunte tramite link simbolico (a parte quelle scelte dall'utente) non vengono
/// attraversate, per evitare cicli.
pub async fn plan_download<F: RemoteFs>(
    fs: &F,
    remote: &[String],
    local_dir: &Path,
) -> Result<Plan> {
    let mut items = Vec::new();
    for top in remote {
        let entry = fs.stat(top).await?;
        let mut stack = vec![(entry, local_dir.join(local_name(&entry_name(top))))];
        while let Some((entry, dst)) = stack.pop() {
            match entry.kind {
                EntryKind::Dir => {
                    items.push(PlanItem {
                        kind: ItemKind::Dir,
                        src: entry.path.clone(),
                        dst: dst.to_string_lossy().into_owned(),
                        size: 0,
                        conflict: false,
                        mode: entry.mode,
                        modified: entry.modified,
                    });
                    let mut children = fs.list(&entry.path).await?;
                    children.sort_by(|a, b| a.name.cmp(&b.name));
                    for child in children.into_iter().rev() {
                        if child.is_symlink && child.kind == EntryKind::Dir {
                            continue;
                        }
                        let child_dst = dst.join(local_name(&child.name));
                        stack.push((child, child_dst));
                    }
                }
                EntryKind::File => items.push(PlanItem {
                    kind: ItemKind::File,
                    src: entry.path.clone(),
                    conflict: dst.exists(),
                    dst: dst.to_string_lossy().into_owned(),
                    size: entry.size,
                    mode: entry.mode,
                    modified: entry.modified,
                }),
                EntryKind::Other => {}
            }
        }
    }
    Ok(Plan::new(Direction::Download, items))
}

fn entry_name(path: &str) -> String {
    path.rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("file")
        .to_string()
}

// ---- Esecuzione ----

/// Esegue il piano. Gli errori su singoli file non fermano il lotto: finiscono in `Summary::failed`.
/// L'annullamento (`cancel`) ferma tutto, elimina il file temporaneo in corso e restituisce `Cancelled`.
pub async fn run<F: RemoteFs>(
    fs: &F,
    plan: &Plan,
    policy: ConflictPolicy,
    cancel: &AtomicBool,
    on_progress: &mut (dyn FnMut(&Progress) + Send),
) -> Result<Summary> {
    let mut progress = Progress {
        bytes_total: plan.total_bytes,
        files_total: plan.file_count,
        ..Progress::default()
    };
    let mut summary = Summary::default();
    let mut last_report = Instant::now();

    for item in &plan.items {
        if cancel.load(Ordering::Relaxed) {
            return Err(TransferError::Cancelled);
        }
        match item.kind {
            ItemKind::Dir => {
                if let Err(e) = make_dir(fs, plan.direction, &item.dst).await {
                    summary.failed.push(Failure {
                        path: item.dst.clone(),
                        message: e.to_string(),
                    });
                }
            }
            ItemKind::File => {
                progress.current = item.src.clone();
                let target = match resolve(fs, plan.direction, item, policy).await {
                    Ok(Some(t)) => t,
                    Ok(None) => {
                        summary.files_skipped += 1;
                        progress.bytes_done += item.size;
                        progress.files_done += 1;
                        continue;
                    }
                    Err(e) => {
                        summary.failed.push(Failure {
                            path: item.src.clone(),
                            message: e.to_string(),
                        });
                        continue;
                    }
                };
                let outcome = match plan.direction {
                    Direction::Upload => {
                        upload_file(
                            fs,
                            item,
                            &target,
                            cancel,
                            &mut progress,
                            &mut last_report,
                            on_progress,
                        )
                        .await
                    }
                    Direction::Download => {
                        download_file(
                            fs,
                            item,
                            &target,
                            cancel,
                            &mut progress,
                            &mut last_report,
                            on_progress,
                        )
                        .await
                    }
                };
                match outcome {
                    Ok(()) => {
                        summary.files_done += 1;
                        summary.bytes += item.size;
                        progress.files_done += 1;
                    }
                    Err(TransferError::Cancelled) => return Err(TransferError::Cancelled),
                    Err(e) => summary.failed.push(Failure {
                        path: item.src.clone(),
                        message: e.to_string(),
                    }),
                }
            }
        }
    }
    on_progress(&progress);
    Ok(summary)
}

async fn make_dir<F: RemoteFs>(fs: &F, direction: Direction, dst: &str) -> Result<()> {
    match direction {
        Direction::Download => Ok(std::fs::create_dir_all(dst)?),
        Direction::Upload => match fs.stat(dst).await {
            Ok(e) if e.kind == EntryKind::Dir => Ok(()),
            Ok(_) => Err(TransferError::Io(format!(
                "esiste già un file con lo stesso nome: {dst}"
            ))),
            Err(_) => Ok(fs.mkdir(dst).await?),
        },
    }
}

async fn exists<F: RemoteFs>(fs: &F, direction: Direction, path: &str) -> bool {
    match direction {
        Direction::Upload => fs.stat(path).await.is_ok(),
        Direction::Download => Path::new(path).exists(),
    }
}

/// Destinazione effettiva di un file secondo la politica; `None` = salta.
async fn resolve<F: RemoteFs>(
    fs: &F,
    direction: Direction,
    item: &PlanItem,
    policy: ConflictPolicy,
) -> Result<Option<String>> {
    // Si ricontrolla l'esistenza: dal piano all'esecuzione la situazione può essere cambiata.
    if !exists(fs, direction, &item.dst).await {
        return Ok(Some(item.dst.clone()));
    }
    match policy {
        ConflictPolicy::Overwrite => Ok(Some(item.dst.clone())),
        ConflictPolicy::Skip => Ok(None),
        ConflictPolicy::Rename => {
            for n in 1..1000 {
                let candidate = numbered_name(&item.dst, n);
                if !exists(fs, direction, &candidate).await {
                    return Ok(Some(candidate));
                }
            }
            Err(TransferError::Io(format!(
                "nessun nome libero per {}",
                item.dst
            )))
        }
        ConflictPolicy::OverwriteIfNewer => {
            let dst_modified = match direction {
                Direction::Upload => fs.stat(&item.dst).await.ok().and_then(|e| e.modified),
                Direction::Download => std::fs::metadata(&item.dst)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(secs),
            };
            Ok(match (item.modified, dst_modified) {
                (Some(src), Some(dst)) if src <= dst => None,
                _ => Some(item.dst.clone()),
            })
        }
    }
}

async fn upload_file<F: RemoteFs>(
    fs: &F,
    item: &PlanItem,
    target: &str,
    cancel: &AtomicBool,
    progress: &mut Progress,
    last_report: &mut Instant,
    on_progress: &mut (dyn FnMut(&Progress) + Send),
) -> Result<()> {
    let tmp = format!("{target}{PART_SUFFIX}");
    let mut src = tokio::fs::File::open(&item.src).await?;
    let mut writer = fs.open_write(&tmp).await?;
    let copied = pump(
        &mut src,
        &mut writer,
        cancel,
        progress,
        last_report,
        on_progress,
    )
    .await;
    let finished = match copied {
        Ok(()) => writer.shutdown().await.map_err(TransferError::from),
        Err(e) => Err(e),
    };
    drop(writer);
    if let Err(e) = finished {
        let _ = fs.remove_file(&tmp).await;
        return Err(e);
    }
    // Il rename non sovrascrive: se la destinazione esiste (politica "sovrascrivi") va tolta.
    let _ = fs.remove_file(target).await;
    if let Err(e) = fs.rename(&tmp, target).await {
        let _ = fs.remove_file(&tmp).await;
        return Err(e.into());
    }
    if let Some(mode) = item.mode {
        let _ = fs.set_permissions(target, mode).await;
    }
    if let Some(t) = item.modified {
        let _ = fs.set_modified(target, t).await;
    }
    Ok(())
}

async fn download_file<F: RemoteFs>(
    fs: &F,
    item: &PlanItem,
    target: &str,
    cancel: &AtomicBool,
    progress: &mut Progress,
    last_report: &mut Instant,
    on_progress: &mut (dyn FnMut(&Progress) + Send),
) -> Result<()> {
    let tmp = format!("{target}{PART_SUFFIX}");
    if let Some(parent) = Path::new(target).parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut reader = fs.open_read(&item.src).await?;
    let mut file = tokio::fs::File::create(&tmp).await?;
    let copied = pump(
        &mut reader,
        &mut file,
        cancel,
        progress,
        last_report,
        on_progress,
    )
    .await;
    let finished = match copied {
        Ok(()) => file.flush().await.map_err(TransferError::from),
        Err(e) => Err(e),
    };
    drop(file);
    if let Err(e) = finished {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    if let Some(t) = item.modified.filter(|t| *t >= 0) {
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(target) {
            let _ = f.set_modified(UNIX_EPOCH + Duration::from_secs(t as u64));
        }
    }
    #[cfg(unix)]
    if let Some(mode) = item.mode {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(target, std::fs::Permissions::from_mode(mode));
    }
    Ok(())
}

/// Attende finché `cancel` diventa vero.
async fn cancelled(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Copia a blocchi, con annullamento (anche mentre si attende la rete) e avanzamento ogni ~100 ms.
async fn pump<R, W>(
    reader: &mut R,
    writer: &mut W,
    cancel: &AtomicBool,
    progress: &mut Progress,
    last_report: &mut Instant,
    on_progress: &mut (dyn FnMut(&Progress) + Send),
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = tokio::select! {
            read = reader.read(&mut buf) => read?,
            () = cancelled(cancel) => return Err(TransferError::Cancelled),
        };
        if n == 0 {
            return Ok(());
        }
        writer.write_all(&buf[..n]).await?;
        progress.bytes_done += n as u64;
        if last_report.elapsed() >= Duration::from_millis(100) {
            on_progress(progress);
            *last_report = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_join_handles_slashes() {
        assert_eq!(remote_join("/a", "b"), "/a/b");
        assert_eq!(remote_join("/", "b"), "/b");
    }

    #[test]
    fn numbered_names_keep_extension_and_hidden_files() {
        assert_eq!(numbered_name("/d/file.txt", 2), "/d/file (2).txt");
        assert_eq!(
            numbered_name("/d/archive.tar.gz", 1),
            "/d/archive.tar (1).gz"
        );
        assert_eq!(numbered_name("/d/Makefile", 1), "/d/Makefile (1)");
        assert_eq!(numbered_name("/d/.bashrc", 1), "/d/.bashrc (1)");
        assert_eq!(numbered_name("C:\\x\\a.txt", 3), "C:\\x\\a (3).txt");
        assert_eq!(numbered_name("solo.txt", 1), "solo (1).txt");
    }

    #[cfg(windows)]
    #[test]
    fn windows_names_are_sanitized() {
        assert_eq!(local_name("a:b?.txt"), "a_b_.txt");
        assert_eq!(local_name("nome. "), "nome");
        assert_eq!(local_name("???"), "___");
        assert_eq!(local_name("..."), "_");
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_names_are_left_alone() {
        assert_eq!(local_name("a:b?.txt"), "a:b?.txt");
    }

    #[test]
    fn plan_counts_files_bytes_and_conflicts() {
        let item = |kind, size, conflict| PlanItem {
            kind,
            src: "s".into(),
            dst: "d".into(),
            size,
            conflict,
            mode: None,
            modified: None,
        };
        let plan = Plan::new(
            Direction::Upload,
            vec![
                item(ItemKind::Dir, 0, false),
                item(ItemKind::File, 10, true),
                item(ItemKind::File, 5, false),
            ],
        );
        assert_eq!(
            (plan.file_count, plan.total_bytes, plan.conflicts),
            (2, 15, 1)
        );
    }
}
