//! Backend FTP e FTPS (TLS esplicito o implicito) su `suppaftp`, per le sessioni FTP (M3).
//!
//! FTP non permette più di un trasferimento per connessione di controllo, e il motore copia più file
//! insieme: `FtpFs` tiene quindi un piccolo **gruppo di connessioni** (al massimo `max_connections`),
//! ne presta una a ogni operazione e la restituisce al termine. Una connessione si riusa solo se
//! l'ultimo trasferimento è finito bene; altrimenti si chiude.
//!
//! Sicurezza: la password resta in memoria solo per aprire le connessioni (e non finisce mai nei
//! messaggi d'errore); FTPS verifica il certificato con i certificati di sistema o con il pin
//! dell'utente (vedi `ftp_tls`); l'FTP in chiaro è una scelta esplicita (`FtpSecurity::Plain`) che la
//! policy può vietare.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, UNIX_EPOCH};

use suppaftp::list::{File as ListedFile, ListParser, PosixPexQuery};
use suppaftp::tokio::{
    AsyncFtpStream, AsyncNoTlsStream, AsyncRustlsConnector, AsyncRustlsFtpStream,
    AsyncRustlsStream, TransferStream,
};
use suppaftp::{FtpError, Status};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::ftp_tls::{client_config, SeenCert};
use crate::{Entry, EntryKind, FsCapabilities, FsError, RemoteFs, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FtpSecurity {
    /// FTP in chiaro: password e dati viaggiano senza cifratura.
    Plain,
    /// FTPS esplicito: si parte in chiaro e si passa a TLS con `AUTH TLS` (porta 21).
    Explicit,
    /// FTPS implicito: TLS subito (di norma porta 990).
    Implicit,
}

pub struct FtpParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub security: FtpSecurity,
    /// Impronta SHA-256 del certificato che l'utente ha accettato per questo server.
    pub pinned_cert: Option<String>,
    /// Connessioni di controllo contemporanee (e quindi trasferimenti insieme).
    pub max_connections: usize,
}

// La password non deve finire in un `{:?}`.
impl std::fmt::Debug for FtpParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FtpParams")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"<nascosta>")
            .field("security", &self.security)
            .finish_non_exhaustive()
    }
}

/// Connessione di controllo: in chiaro o dentro TLS.
enum Conn {
    Plain(AsyncFtpStream),
    Tls(AsyncRustlsFtpStream),
}

/// Esegue lo stesso codice sul client in chiaro e su quello TLS (i tipi sono diversi, i metodi uguali).
macro_rules! on {
    ($conn:expr, $f:ident => $body:expr) => {
        match $conn {
            Conn::Plain($f) => $body,
            Conn::Tls($f) => $body,
        }
    };
}

/// Cosa il server sa fare, dedotto da `FEAT` alla prima connessione.
#[derive(Debug, Clone, Copy, Default)]
struct Features {
    mlsd: bool,
    mlst: bool,
    mfmt: bool,
}

struct Inner {
    params: FtpParams,
    idle: Mutex<Vec<(Conn, Instant)>>,
    permits: Arc<Semaphore>,
    features: Mutex<Features>,
    seen: SeenCert,
}

#[derive(Clone)]
pub struct FtpFs {
    inner: Arc<Inner>,
}

/// Una connessione data in prestito: alla fine torna nel gruppo, se è ancora buona.
struct Lease {
    conn: Option<Conn>,
    inner: Arc<Inner>,
    _permit: OwnedSemaphorePermit,
    reuse: bool,
}

impl Lease {
    fn conn(&mut self) -> &mut Conn {
        self.conn.as_mut().expect("connessione presente")
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let (Some(c), true) = (self.conn.take(), self.reuse) {
            self.inner.idle.lock().unwrap().push((c, Instant::now()));
        }
    }
}

/// Dopo questo tempo di inattività si verifica che la connessione sia ancora viva prima di usarla.
const STALE_AFTER: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

fn map_err(e: FtpError, path: &str) -> FsError {
    match e {
        FtpError::UnexpectedResponse(r) => {
            let body = String::from_utf8_lossy(&r.body).to_lowercase();
            if body.contains("no such")
                || body.contains("not found")
                || body.contains("does not exist")
            {
                FsError::NotFound(path.to_string())
            } else if body.contains("denied")
                || body.contains("permission")
                || body.contains("not allowed")
            {
                FsError::PermissionDenied(path.to_string())
            } else {
                FsError::Ftp(format!(
                    "{path}: {}",
                    String::from_utf8_lossy(&r.body).trim()
                ))
            }
        }
        // Per i dettagli del certificato ci pensa `FtpFs::connect`; il testo non contiene segreti.
        other => FsError::Ftp(format!("{path}: {other}")),
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Percorso assoluto senza `.` e `..`.
fn normalize(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            p => out.push(p),
        }
    }
    format!("/{}", out.join("/"))
}

/// `modify=YYYYMMDDHHMMSS` -> secondi dal 1970, già interpretato da suppaftp.
fn entry_from(dir: &str, f: &ListedFile) -> Entry {
    let mut mode = 0u32;
    for (who, shift) in [
        (PosixPexQuery::Owner, 6),
        (PosixPexQuery::Group, 3),
        (PosixPexQuery::Others, 0),
    ] {
        mode |= (u32::from(f.can_read(who)) * 4
            + u32::from(f.can_write(who)) * 2
            + u32::from(f.can_execute(who)))
            << shift;
    }
    let is_symlink = f.is_symlink();
    let kind = if f.is_directory() {
        EntryKind::Dir
    } else if f.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    Entry {
        name: f.name().to_string(),
        path: join(dir, f.name()),
        kind,
        is_symlink,
        size: if kind == EntryKind::Dir {
            0
        } else {
            f.size() as u64
        },
        modified: f
            .modified()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_secs()).ok()),
        // Senza informazioni Unix i permessi risultano tutti negati: meglio dire "non noti".
        mode: (mode != 0).then_some(mode),
        owner: None,
        group: None,
        uid: f.uid(),
        gid: f.gid(),
    }
}

impl FtpFs {
    /// Si collega, autentica e verifica il server. Con FTPS e un certificato che non passa la verifica
    /// né corrisponde al pin, restituisce `FsError::UntrustedCertificate` con l'impronta da mostrare.
    pub async fn connect(params: FtpParams) -> Result<FtpFs> {
        let max = params.max_connections.clamp(1, 8);
        let inner = Arc::new(Inner {
            params,
            idle: Mutex::default(),
            permits: Arc::new(Semaphore::new(max)),
            features: Mutex::default(),
            seen: Arc::default(),
        });
        let fs = FtpFs { inner };
        let mut first = open_conn(&fs.inner).await?;
        // Funzioni opzionali del server: si chiede una volta sola.
        let feats = on!(&mut first, f => f.feat().await).ok();
        if let Some(feats) = feats {
            let has = |k: &str| feats.keys().any(|name| name.eq_ignore_ascii_case(k));
            *fs.inner.features.lock().unwrap() = Features {
                mlsd: has("MLST"),
                mlst: has("MLST"),
                mfmt: has("MFMT"),
            };
        }
        // Nomi con accenti: UTF-8, se il server lo permette (se no si resta con quello che c'è).
        let _ = on!(&mut first, f => f.opts("UTF8", Some("ON")).await);
        fs.inner.idle.lock().unwrap().push((first, Instant::now()));
        Ok(fs)
    }

    async fn lease(&self) -> Result<Lease> {
        let permit = Arc::clone(&self.inner.permits)
            .acquire_owned()
            .await
            .map_err(|_| FsError::Ftp("connessione chiusa".into()))?;
        let mut reuse = None;
        loop {
            let candidate = self.inner.idle.lock().unwrap().pop();
            let Some((mut conn, last)) = candidate else {
                break;
            };
            // Una connessione ferma da un po' potrebbe essere stata chiusa dal server.
            if last.elapsed() > STALE_AFTER && on!(&mut conn, f => f.noop().await).is_err() {
                continue;
            }
            reuse = Some(conn);
            break;
        }
        let conn = match reuse {
            Some(c) => c,
            None => open_conn(&self.inner).await?,
        };
        Ok(Lease {
            conn: Some(conn),
            inner: Arc::clone(&self.inner),
            _permit: permit,
            reuse: true,
        })
    }

    fn features(&self) -> Features {
        *self.inner.features.lock().unwrap()
    }

    /// Cartella corrente del server (di norma la home dell'utente).
    async fn pwd(&self) -> Result<String> {
        let mut l = self.lease().await?;
        on!(l.conn(), f => f.pwd().await).map_err(|e| map_err(e, "."))
    }
}

async fn open_conn(inner: &Inner) -> Result<Conn> {
    let p = &inner.params;
    let addr = format!("{}:{}", p.host, p.port);
    let tls = || -> Result<AsyncRustlsConnector> {
        let config = client_config(p.pinned_cert.clone(), Arc::clone(&inner.seen))
            .map_err(|e| FsError::Ftp(format!("TLS: {e}")))?;
        Ok(AsyncRustlsConnector::from(
            tokio_rustls::TlsConnector::from(Arc::new(config)),
        ))
    };
    inner.seen.lock().unwrap().take();
    let connect = async {
        match p.security {
            FtpSecurity::Plain => {
                let mut f = AsyncFtpStream::connect(&addr).await?;
                f.login(&p.username, &p.password).await?;
                f.transfer_type(suppaftp::types::FileType::Binary).await?;
                Ok::<Conn, FtpError>(Conn::Plain(f))
            }
            FtpSecurity::Explicit => {
                let f = AsyncRustlsFtpStream::connect(&addr).await?;
                let mut f = f
                    .into_secure(
                        tls().map_err(|e| FtpError::SecureError(e.to_string()))?,
                        &p.host,
                    )
                    .await?;
                f.login(&p.username, &p.password).await?;
                f.transfer_type(suppaftp::types::FileType::Binary).await?;
                Ok(Conn::Tls(f))
            }
            FtpSecurity::Implicit => {
                let mut f = AsyncRustlsFtpStream::connect_secure_implicit(
                    &addr,
                    tls().map_err(|e| FtpError::SecureError(e.to_string()))?,
                    &p.host,
                )
                .await?;
                f.login(&p.username, &p.password).await?;
                f.transfer_type(suppaftp::types::FileType::Binary).await?;
                Ok(Conn::Tls(f))
            }
        }
    };
    match tokio::time::timeout(CONNECT_TIMEOUT, connect).await {
        Ok(Ok(c)) => Ok(c),
        Ok(Err(e)) => {
            // Un certificato rifiutato ha la precedenza su qualunque altro messaggio.
            if let Some(fingerprint) = inner.seen.lock().unwrap().take() {
                return Err(FsError::UntrustedCertificate {
                    fingerprint,
                    changed: p.pinned_cert.is_some(),
                });
            }
            Err(match e {
                FtpError::UnexpectedResponse(r) if r.status == Status::NotLoggedIn => {
                    FsError::Ftp("autenticazione rifiutata dal server".into())
                }
                other => FsError::Ftp(format!("{}:{}: {other}", p.host, p.port)),
            })
        }
        Err(_) => Err(FsError::Ftp(format!(
            "{}:{}: il server non risponde (tempo scaduto)",
            p.host, p.port
        ))),
    }
}

// ---- Trasferimenti: lettore e scrittore che tengono in prestito una connessione ----

enum Stream {
    Plain(TransferStream<AsyncNoTlsStream>),
    Tls(TransferStream<AsyncRustlsStream>),
}

type Finishing = Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>;

/// Flusso dei dati di un trasferimento FTP: lettore (`RETR`) o scrittore (`STOR`).
pub struct FtpTransfer {
    stream: Option<Stream>,
    finishing: Option<Finishing>,
    /// Il trasferimento è finito bene e la connessione si può riusare.
    clean: bool,
    lease: Lease,
}

pub type FtpReader = FtpTransfer;
pub type FtpWriter = FtpTransfer;

impl Drop for FtpTransfer {
    fn drop(&mut self) {
        // Un trasferimento interrotto lascia la connessione in uno stato incerto: si chiude.
        self.lease.reuse = self.clean;
    }
}

fn closed() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        "trasferimento FTP già concluso",
    )
}

impl AsyncRead for FtpTransfer {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let polled = match self.stream.as_mut() {
            Some(Stream::Plain(s)) => Pin::new(s).poll_read(cx, buf),
            Some(Stream::Tls(s)) => Pin::new(s).poll_read(cx, buf),
            None => return Poll::Ready(Err(closed())),
        };
        // Zero byte letti = fine del file: il trasferimento è completo.
        if matches!(polled, Poll::Ready(Ok(()))) && buf.filled().len() == before {
            self.clean = true;
        }
        polled
    }
}

impl AsyncWrite for FtpTransfer {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.stream.as_mut() {
            Some(Stream::Plain(s)) => Pin::new(s).poll_write(cx, data),
            Some(Stream::Tls(s)) => Pin::new(s).poll_write(cx, data),
            None => Poll::Ready(Err(closed())),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.stream.as_mut() {
            Some(Stream::Plain(s)) => Pin::new(s).poll_flush(cx),
            Some(Stream::Tls(s)) => Pin::new(s).poll_flush(cx),
            None => Poll::Ready(Ok(())),
        }
    }

    /// Chiude il flusso dei dati e legge la risposta finale del server (`226`): solo così si sa che il
    /// file è stato scritto davvero.
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.finishing.is_none() {
            let Some(stream) = self.stream.take() else {
                return Poll::Ready(Ok(()));
            };
            self.finishing = Some(Box::pin(async move {
                let r = match stream {
                    Stream::Plain(s) => s.finish().await,
                    Stream::Tls(s) => s.finish().await,
                };
                r.map_err(|e| std::io::Error::other(e.to_string()))
            }));
        }
        let fut = self.finishing.as_mut().expect("in corso");
        match fut.as_mut().poll(cx) {
            Poll::Ready(r) => {
                self.finishing = None;
                self.clean = r.is_ok();
                Poll::Ready(r)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl FtpFs {
    async fn open_transfer(&self, path: &str, offset: u64, upload: bool) -> Result<FtpTransfer> {
        let mut lease = self.lease().await?;
        let started: std::result::Result<Stream, FtpError> = async {
            let c = lease.conn();
            if offset > 0 {
                let offset = usize::try_from(offset).unwrap_or(usize::MAX);
                match c {
                    Conn::Plain(f) => f.resume_transfer(offset).await?,
                    Conn::Tls(f) => f.resume_transfer(offset).await?,
                }
            }
            Ok(match (c, upload) {
                (Conn::Plain(f), false) => Stream::Plain(f.retr_as_stream(path).await?),
                (Conn::Plain(f), true) => Stream::Plain(f.put_with_stream(path).await?),
                (Conn::Tls(f), false) => Stream::Tls(f.retr_as_stream(path).await?),
                (Conn::Tls(f), true) => Stream::Tls(f.put_with_stream(path).await?),
            })
        }
        .await;
        match started {
            Ok(stream) => Ok(FtpTransfer {
                stream: Some(stream),
                finishing: None,
                clean: false,
                lease,
            }),
            Err(e) => {
                // Un rifiuto del server (file mancante, permessi) lascia la connessione utilizzabile.
                lease.reuse = matches!(e, FtpError::UnexpectedResponse(_));
                Err(map_err(e, path))
            }
        }
    }
}

impl RemoteFs for FtpFs {
    type Reader = FtpReader;
    type Writer = FtpWriter;

    async fn list(&self, path: &str) -> Result<Vec<Entry>> {
        let features = self.features();
        let mut l = self.lease().await?;
        let (lines, mlsx) = if features.mlsd {
            (on!(l.conn(), f => f.mlsd(Some(path)).await), true)
        } else {
            (on!(l.conn(), f => f.list(Some(path)).await), false)
        };
        let lines = lines.map_err(|e| map_err(e, path))?;
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for line in lines {
            let parsed = if mlsx {
                ListParser::parse_mlsd(&line)
            } else {
                ListParser::parse_posix(&line).or_else(|_| ListParser::parse_dos(&line))
            };
            let Ok(file) = parsed else { continue };
            let name = file.name();
            if name.is_empty() || name == "." || name == ".." || !seen.insert(name.to_string()) {
                continue;
            }
            out.push(entry_from(path, &file));
        }
        // Un collegamento simbolico si comporta come la cosa a cui punta: lo si verifica con un tentativo.
        for e in out.iter_mut().filter(|e| e.is_symlink) {
            let target = e.path.clone();
            if on!(l.conn(), f => f.cwd(&target).await).is_ok() {
                e.kind = EntryKind::Dir;
                let _ = on!(l.conn(), f => f.cwd(path).await);
            } else if let Ok(size) = on!(l.conn(), f => f.size(&target).await) {
                e.kind = EntryKind::File;
                e.size = size as u64;
            }
        }
        Ok(out)
    }

    async fn stat(&self, path: &str) -> Result<Entry> {
        let norm = normalize(path);
        if norm == "/" {
            return Ok(Entry {
                name: "/".into(),
                path: "/".into(),
                kind: EntryKind::Dir,
                is_symlink: false,
                size: 0,
                modified: None,
                mode: None,
                owner: None,
                group: None,
                uid: None,
                gid: None,
            });
        }
        let (dir, name) = norm
            .rsplit_once('/')
            .map_or(("/", norm.as_str()), |(d, n)| {
                (if d.is_empty() { "/" } else { d }, n)
            });
        if self.features().mlst {
            let mut l = self.lease().await?;
            match on!(l.conn(), f => f.mlst(Some(path)).await) {
                Ok(line) => {
                    if let Ok(file) = ListParser::parse_mlst(&line) {
                        let mut e = entry_from(dir, &file);
                        // MLST può rispondere con il percorso intero: il nome è l'ultima parte.
                        e.name = name.to_string();
                        e.path = norm.clone();
                        return Ok(e);
                    }
                }
                Err(FtpError::UnexpectedResponse(r)) if r.status == Status::FileUnavailable => {
                    return Err(FsError::NotFound(path.to_string()))
                }
                Err(_) => {}
            }
        }
        // Senza MLST: si cerca nella cartella che lo contiene.
        let found = self.list(dir).await?;
        found
            .into_iter()
            .find(|e| e.name == name)
            .ok_or_else(|| FsError::NotFound(path.to_string()))
    }

    async fn lstat(&self, path: &str) -> Result<Entry> {
        let mut e = self.stat(path).await?;
        if e.is_symlink {
            e.kind = EntryKind::Other;
        }
        Ok(e)
    }

    async fn canonicalize(&self, path: &str) -> Result<String> {
        if path == "." || path.is_empty() {
            return self.pwd().await;
        }
        if path.starts_with('/') {
            return Ok(normalize(path));
        }
        Ok(normalize(&join(&self.pwd().await?, path)))
    }

    async fn mkdir(&self, path: &str) -> Result<()> {
        let mut l = self.lease().await?;
        on!(l.conn(), f => f.mkdir(path).await).map_err(|e| map_err(e, path))
    }

    async fn remove_file(&self, path: &str) -> Result<()> {
        let mut l = self.lease().await?;
        on!(l.conn(), f => f.rm(path).await).map_err(|e| map_err(e, path))
    }

    async fn remove_dir(&self, path: &str) -> Result<()> {
        let mut l = self.lease().await?;
        on!(l.conn(), f => f.rmdir(path).await).map_err(|e| map_err(e, path))
    }

    async fn rename(&self, from: &str, to: &str) -> Result<()> {
        let mut l = self.lease().await?;
        on!(l.conn(), f => f.rename(from, to).await).map_err(|e| map_err(e, from))
    }

    async fn set_permissions(&self, path: &str, mode: u32) -> Result<()> {
        let mut l = self.lease().await?;
        let cmd = format!("CHMOD {:o} {path}", mode & 0o7777);
        on!(l.conn(), f => f.site(cmd).await)
            .map(|_| ())
            .map_err(|e| map_err(e, path))
    }

    async fn set_modified(&self, path: &str, secs: i64) -> Result<()> {
        if !self.features().mfmt {
            return Err(FsError::Ftp("il server non supporta MFMT".into()));
        }
        let when = chrono_free_timestamp(secs);
        let mut l = self.lease().await?;
        let cmd = format!("MFMT {when} {path}");
        on!(l.conn(), f => f.custom_command(cmd, &[Status::File]).await)
            .map(|_| ())
            .map_err(|e| map_err(e, path))
    }

    async fn open_read(&self, path: &str) -> Result<Self::Reader> {
        self.open_transfer(path, 0, false).await
    }

    async fn open_write(&self, path: &str) -> Result<Self::Writer> {
        self.open_transfer(path, 0, true).await
    }

    async fn open_read_at(&self, path: &str, offset: u64) -> Result<Self::Reader> {
        self.open_transfer(path, offset, false).await
    }

    async fn open_write_at(&self, path: &str, offset: u64) -> Result<Self::Writer> {
        self.open_transfer(path, offset, true).await
    }

    fn capabilities(&self) -> FsCapabilities {
        FsCapabilities {
            permissions: true,
            symlinks: true,
            owner_group: false,
        }
    }
}

/// `YYYYMMDDHHMMSS` (UTC) a partire dai secondi dal 1970, senza dipendere da una libreria di date.
fn chrono_free_timestamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Algoritmo di Howard Hinnant (civil_from_days).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}{m:02}{d:02}{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_normalized_and_joined() {
        assert_eq!(normalize("/a/b/../c/./d/"), "/a/c/d");
        assert_eq!(normalize("/.."), "/");
        assert_eq!(normalize(""), "/");
        assert_eq!(join("/a", "b"), "/a/b");
        assert_eq!(join("/", "b"), "/b");
    }

    #[test]
    fn mfmt_timestamps_are_utc() {
        assert_eq!(chrono_free_timestamp(0), "19700101000000");
        assert_eq!(chrono_free_timestamp(1_600_000_000), "20200913122640");
        assert_eq!(chrono_free_timestamp(951_782_400), "20000229000000");
    }

    #[test]
    fn the_password_never_appears_in_debug_output() {
        let p = FtpParams {
            host: "h".into(),
            port: 21,
            username: "u".into(),
            password: "segretissima".into(),
            security: FtpSecurity::Explicit,
            pinned_cert: None,
            max_connections: 4,
        };
        assert!(!format!("{p:?}").contains("segretissima"));
    }
}
