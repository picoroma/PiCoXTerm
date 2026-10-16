//! Motore di trasferimento, generico sul filesystem remoto (sezione 7.3 delle specifiche).
//!
//! Funzionamento in due tempi: `plan_upload` / `plan_download` costruiscono il piano (cosa si copia,
//! quali destinazioni esistono già) senza toccare nulla; `run` lo esegue con una politica unica per
//! i conflitti ("applica a tutti"). I dati passano a blocchi da `CHUNK` byte: mai un file intero in
//! memoria. Ogni file viene scritto in un file parziale (`part_path`) e rinominato solo a copia
//! completata. Si copiano più file insieme; un lavoro si può mettere in pausa e riprendere, e un
//! parziale valido riparte dal punto raggiunto.

use std::collections::HashSet;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt};

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
    #[error("trasferimento in pausa")]
    Paused,
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

// ---- Controllo, stato e avanzamento ----

const RUNNING: u8 = 0;
const PAUSED: u8 = 1;
const CANCELLED: u8 = 2;

/// Segnale di controllo di un trasferimento in corso.
#[derive(Debug, Default)]
pub struct Control(AtomicU8);

impl Control {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ferma il trasferimento **tenendo** i file parziali: si potrà riprendere da dove era arrivato.
    pub fn pause(&self) {
        let _ = self
            .0
            .compare_exchange(RUNNING, PAUSED, Ordering::Relaxed, Ordering::Relaxed);
    }

    /// Ferma il trasferimento ed elimina il file parziale in corso.
    pub fn cancel(&self) {
        self.0.store(CANCELLED, Ordering::Relaxed);
    }

    /// Il motivo per cui ci si deve fermare, se c'è.
    fn stop(&self) -> Option<TransferError> {
        match self.0.load(Ordering::Relaxed) {
            PAUSED => Some(TransferError::Paused),
            CANCELLED => Some(TransferError::Cancelled),
            _ => None,
        }
    }
}

/// Attende finché viene chiesto di fermarsi e restituisce il motivo.
async fn stopped(control: &Control) -> TransferError {
    loop {
        if let Some(e) = control.stop() {
            return e;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Stato di un lavoro che sopravvive a una pausa o a un fallimento parziale: quali voci del piano
/// sono già concluse e quali file parziali ha lasciato. Serve a riprendere senza rifare nulla.
#[derive(Debug)]
pub struct JobState {
    done: Vec<AtomicBool>,
    partials: Mutex<HashSet<String>>,
}

impl JobState {
    pub fn new(plan: &Plan) -> Self {
        JobState {
            done: plan.items.iter().map(|_| AtomicBool::new(false)).collect(),
            partials: Mutex::default(),
        }
    }

    fn is_done(&self, i: usize) -> bool {
        self.done[i].load(Ordering::Relaxed)
    }

    fn mark_done(&self, i: usize) {
        self.done[i].store(true, Ordering::Relaxed);
    }

    fn add_partial(&self, path: &str) {
        self.partials.lock().unwrap().insert(path.to_string());
    }

    /// Toglie e restituisce i parziali registrati (per eliminarli).
    pub fn take_partials(&self) -> Vec<String> {
        self.partials.lock().unwrap().drain().collect()
    }

    fn remove_partial(&self, path: &str) {
        self.partials.lock().unwrap().remove(path);
    }

    /// Voci del piano non ancora concluse (da rifare in una ripresa).
    pub fn remaining(&self) -> usize {
        self.done
            .iter()
            .filter(|d| !d.load(Ordering::Relaxed))
            .count()
    }
}

/// Avanzamento condiviso fra i file che si copiano insieme.
struct Shared<'a> {
    bytes_done: AtomicU64,
    files_done: AtomicUsize,
    bytes_total: u64,
    files_total: usize,
    current: Mutex<String>,
    last_report: Mutex<Instant>,
    on_progress: Mutex<&'a mut (dyn FnMut(&Progress) + Send)>,
}

impl Shared<'_> {
    fn snapshot(&self) -> Progress {
        Progress {
            bytes_done: self.bytes_done.load(Ordering::Relaxed),
            bytes_total: self.bytes_total,
            files_done: self.files_done.load(Ordering::Relaxed),
            files_total: self.files_total,
            current: self.current.lock().unwrap().clone(),
        }
    }

    /// Segnala l'avanzamento al massimo ogni ~100 ms, o subito se `force`.
    fn report(&self, force: bool) {
        {
            let mut last = self.last_report.lock().unwrap();
            if !force && last.elapsed() < Duration::from_millis(100) {
                return;
            }
            *last = Instant::now();
        }
        let p = self.snapshot();
        (self.on_progress.lock().unwrap())(&p);
    }

    fn add_bytes(&self, n: u64) {
        self.bytes_done.fetch_add(n, Ordering::Relaxed);
        self.report(false);
    }

    fn file_finished(&self) {
        self.files_done.fetch_add(1, Ordering::Relaxed);
        self.report(false);
    }
}

// ---- Esecuzione ----

/// File copiati contemporaneamente in un lavoro, se chi chiama non ne sceglie un altro numero.
pub const DEFAULT_CONCURRENCY: usize = 4;
const MAX_CONCURRENCY: usize = 16;
/// Quanti byte finali di un file parziale si confrontano con l'originale prima di riprendere.
const TAIL_CHECK: u64 = 64 * 1024;

/// Nome del file parziale: lega il parziale all'impronta dell'originale (dimensione e data), così
/// un parziale lasciato da una versione precedente del file non viene mai riutilizzato per sbaglio.
pub fn part_path(target: &str, item: &PlanItem) -> String {
    format!(
        "{target}.{}-{}{PART_SUFFIX}",
        item.size,
        item.modified.unwrap_or(0)
    )
}

/// Esegue il piano. Gli errori su singoli file non fermano il lotto: finiscono in `Summary::failed`
/// (e quei file restano "da fare" per una ripresa). Si copiano fino a `concurrency` file insieme.
///
/// `control` ferma il lavoro: con `pause` i file parziali restano e `run` restituisce `Paused`; con
/// `cancel` vengono eliminati e restituisce `Cancelled`. Richiamando `run` con lo stesso `state`
/// si riprende: le voci già concluse si saltano e i parziali validi continuano dal punto raggiunto.
pub async fn run<F: RemoteFs>(
    fs: &F,
    plan: &Plan,
    policy: ConflictPolicy,
    control: &Control,
    state: &JobState,
    concurrency: usize,
    on_progress: &mut (dyn FnMut(&Progress) + Send),
) -> Result<Summary> {
    // Quanto è già fatto da un'esecuzione precedente.
    let (mut bytes, mut files) = (0u64, 0usize);
    for (i, item) in plan.items.iter().enumerate() {
        if item.kind == ItemKind::File && state.is_done(i) {
            bytes += item.size;
            files += 1;
        }
    }
    let shared = Shared {
        bytes_done: AtomicU64::new(bytes),
        files_done: AtomicUsize::new(files),
        bytes_total: plan.total_bytes,
        files_total: plan.file_count,
        current: Mutex::default(),
        last_report: Mutex::new(Instant::now()),
        on_progress: Mutex::new(on_progress),
    };
    let ctx = Ctx {
        fs,
        plan,
        policy,
        control,
        state,
        shared: &shared,
        summary: Mutex::new(Summary::default()),
        todo: plan
            .items
            .iter()
            .enumerate()
            .filter(|(i, item)| item.kind == ItemKind::File && !state.is_done(*i))
            .map(|(i, _)| i)
            .collect(),
        next: AtomicUsize::new(0),
    };

    // Prima le cartelle (in ordine, perché una contiene l'altra), poi i file in parallelo.
    for (i, item) in plan.items.iter().enumerate() {
        if item.kind != ItemKind::Dir || state.is_done(i) {
            continue;
        }
        if let Some(e) = control.stop() {
            return Err(e);
        }
        match make_dir(fs, plan.direction, &item.dst).await {
            Ok(()) => state.mark_done(i),
            Err(e) => ctx.fail(&item.dst, &e),
        }
    }

    let workers = concurrency
        .clamp(1, MAX_CONCURRENCY)
        .min(ctx.todo.len().max(1));
    futures_util::future::join_all((0..workers).map(|_| worker_loop(&ctx))).await;

    if let Some(e) = control.stop() {
        return Err(e);
    }
    shared.report(true);
    Ok(ctx.summary.into_inner().unwrap())
}

/// Tutto ciò che serve ai lavoratori che copiano i file in parallelo.
struct Ctx<'a, F> {
    fs: &'a F,
    plan: &'a Plan,
    policy: ConflictPolicy,
    control: &'a Control,
    state: &'a JobState,
    shared: &'a Shared<'a>,
    summary: Mutex<Summary>,
    /// Indici dei file da copiare, nell'ordine del piano.
    todo: Vec<usize>,
    next: AtomicUsize,
}

impl<F> Ctx<'_, F> {
    fn fail(&self, path: &str, e: &dyn ToString) {
        self.summary.lock().unwrap().failed.push(Failure {
            path: path.to_string(),
            message: e.to_string(),
        });
    }
}

/// Un lavoratore: prende il prossimo file da copiare finché ce ne sono o finché non si deve fermare.
async fn worker_loop<F: RemoteFs>(ctx: &Ctx<'_, F>) {
    loop {
        if ctx.control.stop().is_some() {
            return;
        }
        let Some(&idx) = ctx.todo.get(ctx.next.fetch_add(1, Ordering::Relaxed)) else {
            return;
        };
        let item = &ctx.plan.items[idx];
        *ctx.shared.current.lock().unwrap() = item.src.clone();
        let target = match resolve(ctx.fs, ctx.plan.direction, item, ctx.policy).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                ctx.summary.lock().unwrap().files_skipped += 1;
                ctx.shared.add_bytes(item.size);
                ctx.shared.file_finished();
                ctx.state.mark_done(idx);
                continue;
            }
            Err(e) => {
                ctx.fail(&item.src, &e);
                continue;
            }
        };
        let outcome = match ctx.plan.direction {
            Direction::Upload => {
                upload_file(ctx.fs, item, &target, ctx.control, ctx.shared, ctx.state).await
            }
            Direction::Download => {
                download_file(ctx.fs, item, &target, ctx.control, ctx.shared, ctx.state).await
            }
        };
        match outcome {
            Ok(()) => {
                {
                    let mut s = ctx.summary.lock().unwrap();
                    s.files_done += 1;
                    s.bytes += item.size;
                }
                ctx.shared.file_finished();
                ctx.state.mark_done(idx);
            }
            Err(TransferError::Cancelled | TransferError::Paused) => return,
            Err(e) => ctx.fail(&item.src, &e),
        }
    }
}

/// Elimina i file parziali lasciati da un lavoro in pausa (azione "scarta"). Restituisce quanti.
pub async fn discard_partials<F: RemoteFs>(
    fs: &F,
    direction: Direction,
    state: &JobState,
) -> usize {
    let paths = state.take_partials();
    let mut removed = 0;
    for p in paths {
        let ok = match direction {
            Direction::Upload => fs.remove_file(&p).await.is_ok(),
            Direction::Download => std::fs::remove_file(&p).is_ok(),
        };
        removed += usize::from(ok);
    }
    removed
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

/// I byte finali (fino a `TAIL_CHECK`) di `len` byte già copiati combaciano con l'originale?
fn tail_len(len: u64) -> usize {
    len.min(TAIL_CHECK) as usize
}

async fn upload_file<F: RemoteFs>(
    fs: &F,
    item: &PlanItem,
    target: &str,
    control: &Control,
    shared: &Shared<'_>,
    state: &JobState,
) -> Result<()> {
    let part = part_path(target, item);
    let mut src = tokio::fs::File::open(&item.src).await?;
    let src_len = src.metadata().await?.len();

    // Parziale valido? Stessa impronta (è nel nome) e gli ultimi byte combaciano con l'originale.
    let resume_at = match fs.stat(&part).await {
        Ok(e) if e.kind == EntryKind::File && e.size > 0 && e.size <= src_len => {
            let n = tail_len(e.size);
            let mut local = vec![0u8; n];
            src.seek(SeekFrom::Start(e.size - n as u64)).await?;
            src.read_exact(&mut local).await?;
            let mut remote = vec![0u8; n];
            let mut r = fs.open_read_at(&part, e.size - n as u64).await?;
            let same = r.read_exact(&mut remote).await.is_ok() && remote == local;
            same.then_some(e.size)
        }
        _ => None,
    };
    state.add_partial(&part);
    let mut writer = match resume_at {
        Some(off) => {
            src.seek(SeekFrom::Start(off)).await?;
            shared.add_bytes(off);
            fs.open_write_at(&part, off).await?
        }
        None => fs.open_write(&part).await?,
    };
    let copied = pump(&mut src, &mut writer, control, shared).await;
    let finished = match copied {
        Ok(()) => writer.shutdown().await.map_err(TransferError::from),
        Err(TransferError::Paused) => {
            // Si lascia sul server tutto ciò che è stato inviato, per poter riprendere.
            let _ = writer.flush().await;
            Err(TransferError::Paused)
        }
        Err(e) => Err(e),
    };
    drop(writer);
    match finished {
        Ok(()) => {}
        Err(TransferError::Cancelled) => {
            let _ = fs.remove_file(&part).await;
            state.remove_partial(&part);
            return Err(TransferError::Cancelled);
        }
        // Pausa o errore: il parziale resta e verrà verificato alla ripresa.
        Err(e) => return Err(e),
    }
    let sent = fs.stat(&part).await?.size;
    if sent != src_len {
        let _ = fs.remove_file(&part).await;
        state.remove_partial(&part);
        return Err(TransferError::Io(format!(
            "dimensione inattesa dopo l'invio ({sent} su {src_len} byte): il file è cambiato?"
        )));
    }
    // Il rename non sovrascrive: se la destinazione esiste (politica "sovrascrivi") va tolta.
    let _ = fs.remove_file(target).await;
    if let Err(e) = fs.rename(&part, target).await {
        return Err(e.into());
    }
    state.remove_partial(&part);
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
    control: &Control,
    shared: &Shared<'_>,
    state: &JobState,
) -> Result<()> {
    let part = part_path(target, item);
    if let Some(parent) = Path::new(target).parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Parziale valido? Stessa impronta (è nel nome) e gli ultimi byte combaciano con l'originale.
    let existing = std::fs::metadata(&part)
        .ok()
        .filter(|m| m.is_file() && m.len() > 0 && m.len() <= item.size)
        .map(|m| m.len());
    let mut resume_at = None;
    let mut reader = None;
    if let Some(len) = existing {
        let n = tail_len(len);
        let mut local = vec![0u8; n];
        {
            let mut f = tokio::fs::File::open(&part).await?;
            f.seek(SeekFrom::Start(len - n as u64)).await?;
            f.read_exact(&mut local).await?;
        }
        let mut r = fs.open_read_at(&item.src, len - n as u64).await?;
        let mut remote = vec![0u8; n];
        if r.read_exact(&mut remote).await.is_ok() && remote == local {
            resume_at = Some(len);
            reader = Some(r); // già posizionato su `len`
        }
    }
    state.add_partial(&part);
    let (mut reader, mut file) = match (reader, resume_at) {
        (Some(r), Some(off)) => {
            let mut f = tokio::fs::OpenOptions::new()
                .write(true)
                .open(&part)
                .await?;
            f.seek(SeekFrom::Start(off)).await?;
            shared.add_bytes(off);
            (r, f)
        }
        _ => (
            fs.open_read(&item.src).await?,
            tokio::fs::File::create(&part).await?,
        ),
    };
    let copied = pump(&mut reader, &mut file, control, shared).await;
    let finished = match copied {
        Ok(()) => file.flush().await.map_err(TransferError::from),
        Err(TransferError::Paused) => {
            let _ = file.flush().await;
            Err(TransferError::Paused)
        }
        Err(e) => Err(e),
    };
    drop(file);
    match finished {
        Ok(()) => {}
        Err(TransferError::Cancelled) => {
            let _ = std::fs::remove_file(&part);
            state.remove_partial(&part);
            return Err(TransferError::Cancelled);
        }
        // Pausa o errore: il parziale resta e verrà verificato alla ripresa.
        Err(e) => return Err(e),
    }
    let got = std::fs::metadata(&part)?.len();
    if got != item.size {
        let _ = std::fs::remove_file(&part);
        state.remove_partial(&part);
        return Err(TransferError::Io(format!(
            "dimensione inattesa dopo il download ({got} su {} byte): il file è cambiato?",
            item.size
        )));
    }
    // Sovrascrittura: su Windows `rename` non sostituisce un file esistente.
    let _ = std::fs::remove_file(target);
    std::fs::rename(&part, target)?;
    state.remove_partial(&part);
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

/// Copia a blocchi, con arresto (anche mentre si attende la rete) e avanzamento condiviso.
async fn pump<R, W>(
    reader: &mut R,
    writer: &mut W,
    control: &Control,
    shared: &Shared<'_>,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = tokio::select! {
            read = reader.read(&mut buf) => read?,
            e = stopped(control) => return Err(e),
        };
        if n == 0 {
            return Ok(());
        }
        writer.write_all(&buf[..n]).await?;
        shared.add_bytes(n as u64);
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
