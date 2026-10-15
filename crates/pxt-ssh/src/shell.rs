use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, Handler};
use russh::keys::{HashAlg, PublicKeyOrCertificate};
use russh::ChannelMsg;
use tokio::sync::mpsc;

use crate::osc7::Osc7Scanner;
use crate::{HostKeyStatus, KnownHosts, SshError};

pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
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

/// Chiede all'utente se accettare una host key sconosciuta.
pub type HostKeyVerifier =
    Arc<dyn Fn(HostKeyInfo) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

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
}

impl Shell {
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

struct Client {
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
                if (self.verifier)(info).await {
                    self.known
                        .add(&self.host, self.port, &key_type, &fingerprint)?;
                    Ok(true)
                } else {
                    Err(SshError::HostKeyRejected)
                }
            }
        }
    }
}

/// Apre la connessione, autentica con password, apre una shell con PTY.
/// L'output arriva su `events`; l'input si invia con i metodi di [`Shell`].
pub async fn connect_shell(
    p: ConnectParams,
    known: KnownHosts,
    verifier: HostKeyVerifier,
    events: mpsc::Sender<ShellEvent>,
) -> Result<Shell, SshError> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    });
    let handler = Client {
        host: p.host.clone(),
        port: p.port,
        known,
        verifier,
    };
    // Il timeout copre solo il collegamento TCP e lo scambio chiavi, non il dialogo host key.
    let mut handle = client::connect(config, (p.host.as_str(), p.port), handler).await?;

    if !handle
        .authenticate_password(p.username, p.password)
        .await?
        .success()
    {
        return Err(SshError::AuthFailed);
    }

    let mut channel = handle.channel_open_session().await?;
    channel
        .request_pty(true, &p.term, p.cols, p.rows, 0, 0, &[])
        .await?;
    channel.request_shell(true).await?;

    let (tx, mut rx) = mpsc::channel::<Cmd>(256);
    tokio::spawn(async move {
        let _keep_alive = handle; // chiudere il task chiude la connessione
        let mut exit = None;
        let mut osc7 = Osc7Scanner::new();
        loop {
            tokio::select! {
                msg = channel.wait() => match msg {
                    Some(ChannelMsg::Data { data }) => {
                        for path in osc7.feed(&data) {
                            let _ = events.send(ShellEvent::Cwd(path)).await;
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

    Ok(Shell { tx })
}
