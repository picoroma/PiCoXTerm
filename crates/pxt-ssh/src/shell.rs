use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Handler};
use russh::keys::{HashAlg, PublicKeyOrCertificate};
use russh::ChannelMsg;
use tokio::sync::{mpsc, oneshot};

use crate::auth::{authenticate, Credentials};
use crate::osc7::{Osc7Scanner, OscEvent};
use crate::{HostKeyStatus, KnownHosts, SshError};

/// Un salto della catena: l'ultimo è la destinazione, i precedenti sono jump host.
pub struct Hop {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: Credentials,
}

pub struct ConnectParams {
    /// Dal primo jump host alla destinazione (almeno un elemento).
    pub hops: Vec<Hop>,
    pub cols: u32,
    pub rows: u32,
    pub term: String,
}

#[derive(Debug, Clone)]
pub struct HostKeyInfo {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
}

/// Scelta dell'utente davanti a una host key sconosciuta (SEC-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyDecision {
    Reject,
    /// Si fida per questa connessione, senza salvare la chiave.
    AcceptOnce,
    AcceptAndSave,
}

/// Chiede all'utente se accettare una host key sconosciuta.
pub type HostKeyVerifier =
    Arc<dyn Fn(HostKeyInfo) -> Pin<Box<dyn Future<Output = HostKeyDecision> + Send>> + Send + Sync>;

#[derive(Debug)]
pub enum ShellEvent {
    Data(Vec<u8>),
    /// Directory corrente comunicata dalla shell remota via OSC 7.
    Cwd(String),
    Closed(Option<u32>),
}

pub(crate) enum Cmd {
    Write(Vec<u8>),
    Resize(u32, u32),
    Close,
}

#[derive(Clone)]
pub struct Shell {
    pub(crate) tx: mpsc::Sender<Cmd>,
    /// Connessione condivisa: serve ad aprire altri canali (SFTP) senza una seconda autenticazione.
    pub(crate) handle: Arc<client::Handle<Client>>,
    /// Chi aspetta la risposta della sonda "quale shell è?" (vedi `detect_current_shell`).
    pub(crate) probe: Arc<Mutex<Option<oneshot::Sender<String>>>>,
}

impl Shell {
    /// Apre un canale verso il sottosistema `sftp` sulla stessa connessione della shell.
    /// Il flusso va dato a un client SFTP (per esempio `pxt_fs::SftpFs::connect`).
    pub async fn open_sftp_stream(&self) -> Result<russh::ChannelStream<client::Msg>, SshError> {
        let channel = self.handle.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        Ok(channel.into_stream())
    }

    pub async fn write(&self, data: Vec<u8>) -> Result<(), SshError> {
        self.tx
            .send(Cmd::Write(data))
            .await
            .map_err(|_| SshError::ChannelClosed)
    }

    pub async fn resize(&self, cols: u32, rows: u32) -> Result<(), SshError> {
        self.tx
            .send(Cmd::Resize(cols, rows))
            .await
            .map_err(|_| SshError::ChannelClosed)
    }

    pub async fn close(&self) {
        let _ = self.tx.send(Cmd::Close).await;
    }
}

pub(crate) struct Client {
    host: String,
    port: u16,
    known: KnownHosts,
    verifier: HostKeyVerifier,
}

impl Handler for Client {
    type Error = SshError;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, SshError> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return Err(SshError::UnsupportedHostKey);
        };
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let key_type = key.algorithm().to_string();
        match self
            .known
            .check(&self.host, self.port, &key_type, &fingerprint)?
        {
            HostKeyStatus::Known => Ok(true),
            HostKeyStatus::Changed(expected) => Err(SshError::HostKeyChanged {
                host: self.host.clone(),
                port: self.port,
                expected,
                got: fingerprint,
            }),
            HostKeyStatus::Unknown => {
                let info = HostKeyInfo {
                    host: self.host.clone(),
                    port: self.port,
                    key_type: key_type.clone(),
                    fingerprint: fingerprint.clone(),
                };
                match (self.verifier)(info).await {
                    HostKeyDecision::Reject => Err(SshError::HostKeyRejected),
                    HostKeyDecision::AcceptOnce => Ok(true),
                    HostKeyDecision::AcceptAndSave => {
                        self.known
                            .add(&self.host, self.port, &key_type, &fingerprint)?;
                        Ok(true)
                    }
                }
            }
        }
    }
}

/// Catena di connessioni già autenticate: l'ultima è la destinazione, le altre sono i jump host.
struct Chain {
    handle: client::Handle<Client>,
    /// I salti già attraversati restano aperti finché dura la sessione.
    upstream: Vec<client::Handle<Client>>,
}

/// Collega e autentica ogni salto, dal primo jump host alla destinazione.
/// `step_timeout`: limite per ogni salto (da non usare se serve attendere una risposta dell'utente).
async fn connect_chain(
    hops: Vec<Hop>,
    known: &KnownHosts,
    verifier: &HostKeyVerifier,
    step_timeout: Option<Duration>,
) -> Result<Chain, SshError> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    });
    let hop_count = hops.len();
    // I salti già attraversati restano aperti finché dura la sessione.
    let mut upstream: Vec<client::Handle<Client>> = Vec::new();
    let mut current: Option<client::Handle<Client>> = None;
    for (i, hop) in hops.into_iter().enumerate() {
        let label = format!("{}:{}", hop.host, hop.port);
        let handler = Client {
            host: hop.host.clone(),
            port: hop.port,
            known: known.clone(),
            verifier: verifier.clone(),
        };
        let step = async {
            let mut handle = match current.take() {
                None => {
                    client::connect(config.clone(), (hop.host.as_str(), hop.port), handler).await?
                }
                Some(prev) => {
                    // Il salto successivo viaggia dentro un canale direct-tcpip del precedente.
                    let channel = prev
                        .channel_open_direct_tcpip(
                            hop.host.clone(),
                            u32::from(hop.port),
                            "127.0.0.1",
                            0,
                        )
                        .await
                        .map_err(|e| match e {
                            russh::Error::ChannelOpenFailure(
                                russh::ChannelOpenFailure::AdministrativelyProhibited,
                            ) => SshError::ForwardingDenied,
                            other => other.into(),
                        })?;
                    upstream.push(prev);
                    client::connect_stream(config.clone(), channel.into_stream(), handler).await?
                }
            };
            authenticate(&mut handle, &hop.username, hop.auth).await?;
            Ok::<_, SshError>(handle)
        };
        let result = match step_timeout {
            Some(limit) => tokio::time::timeout(limit, step)
                .await
                .unwrap_or(Err(SshError::Timeout(limit.as_secs()))),
            None => step.await,
        };
        let handle = result.map_err(|e| {
            if i + 1 < hop_count {
                SshError::Jump {
                    host: label,
                    source: Box::new(e),
                }
            } else if matches!(e, SshError::Io(_) | SshError::Ssh(_)) {
                // Errori di rete: dire a quale host si riferiscono.
                SshError::Target {
                    host: label,
                    source: Box::new(e),
                }
            } else {
                e
            }
        })?;
        current = Some(handle);
    }
    let handle = current.ok_or_else(|| SshError::Connect("nessun host indicato".into()))?;
    Ok(Chain { handle, upstream })
}

/// Esito di una prova di connessione.
#[derive(Debug, Clone, Copy)]
pub struct TestReport {
    /// Tempo totale per collegare e autenticare tutti i salti.
    pub elapsed_ms: u64,
}

/// Verifica raggiungibilità e autenticazione fino alla destinazione, senza aprire una shell.
/// Con un `verifier` che rifiuta le chiavi sconosciute non si fida di nulla e non salva nulla.
pub async fn test_connection(
    hops: Vec<Hop>,
    known: KnownHosts,
    verifier: HostKeyVerifier,
    timeout: Duration,
) -> Result<TestReport, SshError> {
    let started = std::time::Instant::now();
    let chain = connect_chain(hops, &known, &verifier, Some(timeout)).await?;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let _ = chain
        .handle
        .disconnect(russh::Disconnect::ByApplication, "test", "en")
        .await;
    Ok(TestReport { elapsed_ms })
}

/// Connessione SSH senza shell né terminale: serve solo ad aprire il sottosistema SFTP (sessioni
/// SFTP, anche per server che non offrono una shell).
#[derive(Clone)]
pub struct SftpConnection {
    handle: Arc<client::Handle<Client>>,
    /// I jump host restano aperti finché esiste la connessione.
    _upstream: Arc<Vec<client::Handle<Client>>>,
}

impl SftpConnection {
    /// Apre un canale verso il sottosistema `sftp` (da dare a `pxt_fs::SftpFs::connect`).
    pub async fn open_sftp_stream(&self) -> Result<russh::ChannelStream<client::Msg>, SshError> {
        let channel = self.handle.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        Ok(channel.into_stream())
    }

    pub async fn close(&self) {
        let _ = self
            .handle
            .disconnect(russh::Disconnect::ByApplication, "chiusura", "it")
            .await;
    }
}

/// Collega e autentica (jump host compresi) senza aprire alcuna shell.
pub async fn connect_sftp(
    hops: Vec<Hop>,
    known: KnownHosts,
    verifier: HostKeyVerifier,
) -> Result<SftpConnection, SshError> {
    let Chain { handle, upstream } = connect_chain(hops, &known, &verifier, None).await?;
    Ok(SftpConnection {
        handle: Arc::new(handle),
        _upstream: Arc::new(upstream),
    })
}

/// Apre la connessione, autentica, apre una shell con PTY.
/// L'output arriva su `events`; l'input si invia con i metodi di [`Shell`].
pub async fn connect_shell(
    p: ConnectParams,
    known: KnownHosts,
    verifier: HostKeyVerifier,
    events: mpsc::Sender<ShellEvent>,
) -> Result<Shell, SshError> {
    let Chain { handle, upstream } = connect_chain(p.hops, &known, &verifier, None).await?;
    let handle = Arc::new(handle);

    let mut channel = handle.channel_open_session().await?;
    channel
        .request_pty(true, &p.term, p.cols, p.rows, 0, 0, &[])
        .await?;
    channel.request_shell(true).await?;

    let (tx, mut rx) = mpsc::channel::<Cmd>(256);
    let probe: Arc<Mutex<Option<oneshot::Sender<String>>>> = Arc::default();
    let task_probe = Arc::clone(&probe);
    let task_handle = Arc::clone(&handle);
    tokio::spawn(async move {
        // La connessione (jump host compresi) resta aperta finché esiste almeno una copia di `Shell`
        // o questo task.
        let _keep_alive = (task_handle, upstream);
        let mut exit = None;
        let mut osc7 = Osc7Scanner::new();
        loop {
            tokio::select! {
                msg = channel.wait() => match msg {
                    Some(ChannelMsg::Data { data }) => {
                        for ev in osc7.feed(&data) {
                            match ev {
                                OscEvent::Cwd(path) => {
                                    let _ = events.send(ShellEvent::Cwd(path)).await;
                                }
                                OscEvent::Shell(name) => {
                                    let waiting = task_probe.lock().unwrap().take();
                                    if let Some(reply) = waiting {
                                        let _ = reply.send(name);
                                    }
                                }
                            }
                        }
                        if events.send(ShellEvent::Data(data.to_vec())).await.is_err() { break; }
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        if events.send(ShellEvent::Data(data.to_vec())).await.is_err() { break; }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => exit = Some(exit_status),
                    Some(ChannelMsg::Eof | ChannelMsg::Close) | None => break,
                    _ => {}
                },
                cmd = rx.recv() => match cmd {
                    Some(Cmd::Write(d)) => { if channel.data_bytes(d).await.is_err() { break; } }
                    Some(Cmd::Resize(c, r)) => { let _ = channel.window_change(c, r, 0, 0).await; }
                    Some(Cmd::Close) | None => { let _ = channel.close().await; break; }
                },
            }
        }
        let _ = events.send(ShellEvent::Closed(exit)).await;
    });

    Ok(Shell { tx, handle, probe })
}
