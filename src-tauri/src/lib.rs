use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use pxt_core::{
    subtree_ids, transfer, AuditEvent, AuditLog, AuthMethod, CredentialRef, FollowCwd,
    KeyringSecretProvider, LocalSessionProvider, LocalSettings, Outcome, Protocol, Secret,
    SecretProvider, Session, SessionNode, SessionProvider, Settings,
};
use pxt_fs::ops as fs_ops;
use pxt_fs::transfer as fs_transfer;
use pxt_fs::transfer::{ConflictPolicy, Progress};
use pxt_fs::{Entry, RemoteFs, SftpFs};
use pxt_ssh::{
    ConnectParams, Credentials, Hop, HostKeyDecision, HostKeyInfo, HostKeyVerifier, KnownHosts,
    Shell, ShellEvent, SshError,
};
use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Manager, State};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

/// Codice d'errore (letto dalla UI) quando manca la password e va chiesta all'utente.
const PASSWORD_REQUIRED: &str = "password-required";
/// Come sopra, per la passphrase di una chiave privata cifrata.
const PASSPHRASE_REQUIRED: &str = "passphrase-required";

struct AppState {
    sessions: LocalSessionProvider,
    settings: LocalSettings,
    audit: AuditLog,
    secrets: KeyringSecretProvider,
    data_dir: PathBuf,
    known_hosts: PathBuf,
    shells: Mutex<HashMap<u64, Shell>>,
    /// Sessioni SFTP, una per connessione, aperte al primo uso sulla stessa connessione SSH.
    sftp: tokio::sync::Mutex<HashMap<u64, Arc<SftpFs>>>,
    /// Piani di trasferimento in attesa di conferma (una scelta sui conflitti) o di avvio.
    plans: Mutex<HashMap<u64, StoredPlan>>,
    /// Trasferimenti in corso: segnale di annullamento di ciascuno.
    jobs: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    /// Destinazione di ogni connessione, per l'audit dei trasferimenti.
    conn_ctx: Mutex<HashMap<u64, AuditCtx>>,
    pending_hostkeys: Mutex<HashMap<u64, oneshot::Sender<HostKeyDecision>>>,
    next_id: AtomicU64,
}

impl AppState {
    fn new(data_dir: PathBuf) -> Self {
        Self {
            sessions: LocalSessionProvider::new(data_dir.join("sessions.json")),
            settings: LocalSettings::new(data_dir.join("settings.json")),
            audit: AuditLog::new(data_dir.join("logs")),
            secrets: KeyringSecretProvider::new(pxt_core::APP_NAME),
            known_hosts: data_dir.join("known_hosts"),
            data_dir,
            shells: Mutex::default(),
            sftp: tokio::sync::Mutex::default(),
            plans: Mutex::default(),
            jobs: Mutex::default(),
            conn_ctx: Mutex::default(),
            pending_hostkeys: Mutex::default(),
            next_id: AtomicU64::new(0),
        }
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Registra un evento di audit. Un errore di scrittura del log non deve bloccare l'uso dell'app.
    fn audit(&self, ctx: &AuditCtx, event: &str, outcome: Outcome, detail: Option<String>) {
        let _ = self.audit.record(&AuditEvent {
            event: event.into(),
            protocol: "ssh".into(),
            host: ctx.host.clone(),
            port: ctx.port,
            user: ctx.user.clone(),
            outcome,
            via: ctx.via.clone(),
            detail,
        });
    }
}

/// Destinazione di una connessione, per il log di audit.
#[derive(Clone)]
struct AuditCtx {
    host: String,
    port: u16,
    user: String,
    /// Jump host attraversati (`host:porta`).
    via: Vec<String>,
}

/// Dettagli di una host key cambiata, anche se avvenuta su un jump host.
fn hostkey_changed(e: &SshError) -> Option<(String, u16, String, String)> {
    match e {
        SshError::HostKeyChanged {
            host,
            port,
            expected,
            got,
        } => Some((host.clone(), *port, expected.clone(), got.clone())),
        SshError::Jump { source, .. } | SshError::Target { source, .. } => hostkey_changed(source),
        _ => None,
    }
}

/// Codice d'errore letto dalla UI: la connessione è stata bloccata per host key cambiata.
const HOSTKEY_CHANGED: &str = "hostkey-changed";

type Shared<'a> = State<'a, Arc<AppState>>;

fn err<E: ToString>(e: E) -> String {
    e.to_string()
}

/// Cartella dati: accanto all'exe se c'è `portable.flag`, altrimenti quella di configurazione utente.
fn resolve_data_dir(app: &tauri::App) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("portable.flag").exists() {
                return Ok(dir.join("data"));
            }
        }
    }
    let name = if cfg!(target_os = "linux") {
        "picoxterm"
    } else {
        pxt_core::APP_NAME
    };
    Ok(app.path().config_dir()?.join(name))
}

/// Eventi di controllo verso la scheda (JSON). I dati del terminale viaggiano a parte, in binario.
#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
enum TermEvent {
    #[serde(rename_all = "camelCase")]
    HostKey {
        req_id: u64,
        host: String,
        port: u16,
        key_type: String,
        fingerprint: String,
    },
    /// La chiave del server è diversa da quella salvata: connessione bloccata (SEC-04).
    HostKeyChanged {
        host: String,
        port: u16,
        expected: String,
        got: String,
    },
    /// Esito dell'attivazione del tracciamento della directory del terminale.
    Tracking {
        /// `active`, `unsupported` (shell non gestita), `off` (disattivato nella sessione), `error`.
        state: &'static str,
        shell: Option<String>,
        message: Option<String>,
    },
    Closed {
        exit: Option<u32>,
    },
    Cwd {
        path: String,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectRequest {
    /// Sessione salvata da aprire; se assente si usano host/port/username (quick connect).
    session_id: Option<Uuid>,
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    /// Password fornita ora dall'utente; ha la precedenza su quella nel keyring.
    password: Option<String>,
}

/// Informazioni per la finestra "Informazioni su PiCoXterm".
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AboutInfo {
    name: &'static str,
    version: &'static str,
    commit: &'static str,
    build_date: &'static str,
    tauri_version: &'static str,
    webview_version: Option<String>,
    os: &'static str,
    arch: &'static str,
    /// Con `portable.flag` accanto all'eseguibile i dati stanno nella cartella `data/`.
    portable: bool,
    data_dir: String,
    audit_log: String,
}

#[tauri::command]
fn about_info(state: Shared<'_>) -> AboutInfo {
    let portable = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("portable.flag").exists()))
        .unwrap_or(false);
    AboutInfo {
        name: pxt_core::APP_NAME,
        version: env!("CARGO_PKG_VERSION"),
        commit: env!("PXT_GIT_COMMIT"),
        build_date: env!("PXT_BUILD_DATE"),
        tauri_version: tauri::VERSION,
        webview_version: tauri::webview_version().ok(),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        portable,
        data_dir: state.data_dir.display().to_string(),
        audit_log: state.audit.path().display().to_string(),
    }
}

#[tauri::command]
fn app_info() -> String {
    format!("{} {}", pxt_core::APP_NAME, env!("CARGO_PKG_VERSION"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppPaths {
    data_dir: String,
    audit_log: String,
}

/// Dove l'app tiene i suoi dati (utile per trasparenza verso l'utente e l'IT).
#[tauri::command]
fn app_paths(state: Shared<'_>) -> AppPaths {
    AppPaths {
        data_dir: state.data_dir.display().to_string(),
        audit_log: state.audit.path().display().to_string(),
    }
}

#[tauri::command]
fn settings_get(state: Shared<'_>) -> Result<Settings, String> {
    state.settings.load().map_err(err)
}

/// Salva le impostazioni e restituisce i valori effettivi (dopo i limiti di validità).
#[tauri::command]
fn settings_set(state: Shared<'_>, settings: Settings) -> Result<Settings, String> {
    state.settings.save(settings).map_err(err)
}

#[tauri::command]
async fn sessions_list(state: Shared<'_>) -> Result<Vec<SessionNode>, String> {
    state.sessions.list().await.map_err(err)
}

/// Salva un nodo. La password, se presente, va nel keyring e nel file resta solo il riferimento.
#[tauri::command]
async fn session_save(
    state: Shared<'_>,
    node: SessionNode,
    password: Option<String>,
    clear_password: bool,
) -> Result<SessionNode, String> {
    let mut node = node;
    if let SessionNode::Session(s) = &mut node {
        s.ui = s.ui.clone().sanitized()?;
        if clear_password {
            if let Some(r) = s.credential_ref.take() {
                state.secrets.delete(&r).await.map_err(err)?;
            }
        }
        if let Some(pw) = password.filter(|p| !p.is_empty()) {
            let r = s
                .credential_ref
                .clone()
                .unwrap_or_else(CredentialRef::new_local);
            state.secrets.set(&r, Secret::new(pw)).await.map_err(err)?;
            s.credential_ref = Some(r);
        }
    }
    state.sessions.save(node.clone()).await.map_err(err)?;
    Ok(node)
}

/// Duplica una sessione. Se ha una password nel keyring, la copia ne riceve una propria (stesso valore).
#[tauri::command]
async fn session_duplicate(
    state: Shared<'_>,
    id: Uuid,
    copy_suffix: String,
) -> Result<SessionNode, String> {
    let nodes = state.sessions.list().await.map_err(err)?;
    let Some(SessionNode::Session(original)) = nodes.iter().find(|n| n.id() == id) else {
        return Err("sessione non trovata".into());
    };
    let mut copy = original.duplicate(format!("{} {}", original.name, copy_suffix.trim()));
    if let Some(r) = &original.credential_ref {
        if let Some(secret) = state.secrets.get(r).await.map_err(err)? {
            let new_ref = CredentialRef::new_local();
            state.secrets.set(&new_ref, secret).await.map_err(err)?;
            copy.credential_ref = Some(new_ref);
        }
    }
    let node = SessionNode::Session(Box::new(copy));
    state.sessions.save(node.clone()).await.map_err(err)?;
    Ok(node)
}

/// Elimina il nodo (e il suo contenuto) insieme alle password salvate.
/// Esporta le sessioni (senza segreti) nel file scelto dall'utente.
#[tauri::command]
async fn sessions_export(state: Shared<'_>, path: String) -> Result<usize, String> {
    let nodes = state.sessions.list().await.map_err(err)?;
    let json = transfer::export_json(&nodes).map_err(err)?;
    std::fs::write(&path, json).map_err(err)?;
    Ok(nodes.len())
}

/// Dimensione massima di un file di sessioni importabile.
const MAX_IMPORT_BYTES: u64 = 20 * 1024 * 1024;

/// Importa un file di sessioni dentro una nuova cartella; non sovrascrive nulla di esistente.
#[tauri::command]
async fn sessions_import(
    state: Shared<'_>,
    path: String,
    folder_name: String,
) -> Result<usize, String> {
    if std::fs::metadata(&path).map_err(err)?.len() > MAX_IMPORT_BYTES {
        return Err("file troppo grande per essere un elenco di sessioni".into());
    }
    let text = std::fs::read_to_string(&path).map_err(err)?;
    let nodes = transfer::import_json(&text, &folder_name).map_err(err)?;
    let count = nodes.len() - 1; // la cartella di import non conta
    state.sessions.save_many(nodes).await.map_err(err)?;
    Ok(count)
}

#[tauri::command]
async fn session_delete(state: Shared<'_>, id: Uuid) -> Result<(), String> {
    let nodes = state.sessions.list().await.map_err(err)?;
    let doomed = subtree_ids(&nodes, id);
    let refs: Vec<CredentialRef> = nodes
        .iter()
        .filter(|n| doomed.contains(&n.id()))
        .filter_map(|n| match n {
            SessionNode::Session(s) => s.credential_ref.clone(),
            SessionNode::Folder(_) => None,
        })
        .collect();
    state.sessions.delete(id).await.map_err(err)?;
    for r in refs {
        state.secrets.delete(&r).await.map_err(err)?;
    }
    Ok(())
}

/// Numero massimo di jump host in catena.
const MAX_JUMPS: usize = 5;

/// Dati di connessione di una sessione SSH salvata.
struct Endpoint {
    host: String,
    port: u16,
    username: String,
    credential_ref: Option<CredentialRef>,
    method: AuthMethod,
    key_path: Option<String>,
    follow_cwd: FollowCwd,
}

impl Endpoint {
    fn of(s: &Session) -> Self {
        Self {
            host: s.host.clone(),
            port: s.port,
            username: s.username.clone(),
            credential_ref: s.credential_ref.clone(),
            method: s.auth.method,
            key_path: s.auth.key_path.clone(),
            follow_cwd: s.ssh.follow_cwd,
        }
    }
}

enum CredError {
    NeedSecret,
    Other(String),
}

async fn stored_secret(
    state: &AppState,
    r: &Option<CredentialRef>,
) -> Result<Option<String>, String> {
    match r {
        Some(r) => Ok(state
            .secrets
            .get(r)
            .await
            .map_err(err)?
            .map(|s| s.expose().to_string())),
        None => Ok(None),
    }
}

fn build_credentials(
    method: AuthMethod,
    key_path: Option<String>,
    secret: Option<String>,
) -> Result<Credentials, CredError> {
    match method {
        AuthMethod::Password => secret
            .map(Credentials::Password)
            .ok_or(CredError::NeedSecret),
        // La passphrase è opzionale: se la chiave è cifrata la richiede la connessione.
        AuthMethod::Key => {
            let path = key_path
                .filter(|p| !p.trim().is_empty())
                .ok_or_else(|| CredError::Other("percorso della chiave privata mancante".into()))?;
            Ok(Credentials::Key {
                path: path.into(),
                passphrase: secret,
            })
        }
        AuthMethod::Agent => Ok(Credentials::Agent),
        AuthMethod::KeyboardInteractive => Err(CredError::Other(
            "keyboard-interactive non ancora supportato".into(),
        )),
    }
}

/// Jump host: le credenziali devono essere già salvate (o agent / chiave senza passphrase).
async fn jump_hops(
    state: &AppState,
    nodes: &[SessionNode],
    jump_ids: &[Uuid],
) -> Result<Vec<Hop>, String> {
    let mut hops = Vec::new();
    for jid in jump_ids {
        let Some(SessionNode::Session(j)) = nodes.iter().find(|n| n.id() == *jid) else {
            return Err("jump host non trovato (sessione eliminata?)".into());
        };
        let ep = Endpoint::of(j);
        let secret = stored_secret(state, &ep.credential_ref).await?;
        let auth =
            build_credentials(ep.method, ep.key_path.clone(), secret).map_err(|e| match e {
                CredError::NeedSecret => format!(
                    "jump host \"{}\": password non salvata (salvala nella sessione del jump host)",
                    j.name
                ),
                CredError::Other(m) => format!("jump host \"{}\": {m}", j.name),
            })?;
        hops.push(Hop {
            host: ep.host,
            port: ep.port,
            username: ep.username,
            auth,
        });
    }
    Ok(hops)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TestResult {
    /// `ok`, `needSecret`, `unknownHostKey`, `hostKeyChanged` o `error`.
    status: &'static str,
    message: String,
    elapsed_ms: Option<u64>,
    key_type: Option<String>,
    fingerprint: Option<String>,
}

impl TestResult {
    fn new(status: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            elapsed_ms: None,
            key_type: None,
            fingerprint: None,
        }
    }
}

/// Prova una sessione (anche non ancora salvata) senza aprire la shell.
/// Non si fida mai di una host key sconosciuta e non salva nulla: la riporta soltanto.
#[tauri::command]
async fn session_test(
    state: Shared<'_>,
    session: Session,
    password: Option<String>,
) -> Result<TestResult, String> {
    if session.protocol != Protocol::Ssh {
        return Ok(TestResult::new("error", "protocollo non ancora supportato"));
    }
    let nodes = state.sessions.list().await.map_err(err)?;
    let mut hops = jump_hops(&state, &nodes, &session.jump_hosts).await?;

    let dest = Endpoint::of(&session);
    let secret = match password {
        Some(p) if !p.is_empty() => Some(p),
        _ => stored_secret(&state, &dest.credential_ref).await?,
    };
    let auth = match build_credentials(dest.method, dest.key_path.clone(), secret) {
        Ok(a) => a,
        Err(CredError::NeedSecret) => {
            return Ok(TestResult::new(
                "needSecret",
                "Inserisci la password nel campo per provare la connessione.",
            ))
        }
        Err(CredError::Other(m)) => return Ok(TestResult::new("error", m)),
    };
    let ctx = AuditCtx {
        host: dest.host.clone(),
        port: dest.port,
        user: dest.username.clone(),
        via: hops
            .iter()
            .map(|h| format!("{}:{}", h.host, h.port))
            .collect(),
    };
    hops.push(Hop {
        host: dest.host,
        port: dest.port,
        username: dest.username,
        auth,
    });

    // Chiave sconosciuta: il test la ricorda ma risponde "no" (non si fida e non salva).
    let unknown: Arc<Mutex<Option<HostKeyInfo>>> = Arc::new(Mutex::new(None));
    let verifier: HostKeyVerifier = {
        let unknown = Arc::clone(&unknown);
        Arc::new(move |info: HostKeyInfo| {
            let unknown = Arc::clone(&unknown);
            Box::pin(async move {
                unknown.lock().unwrap().get_or_insert(info);
                HostKeyDecision::Reject
            })
        })
    };

    let outcome = pxt_ssh::test_connection(
        hops,
        KnownHosts::new(state.known_hosts.clone()),
        verifier,
        std::time::Duration::from_secs(15),
    )
    .await;

    Ok(match outcome {
        Ok(report) => {
            state.audit(&ctx, "connection_test", Outcome::Ok, None);
            let mut r = TestResult::new("ok", "Connessione e autenticazione riuscite.");
            r.elapsed_ms = Some(report.elapsed_ms);
            r
        }
        Err(SshError::HostKeyRejected) if unknown.lock().unwrap().is_some() => {
            let info = unknown.lock().unwrap().take().unwrap();
            state.audit(
                &ctx,
                "connection_test",
                Outcome::Ok,
                Some("host raggiungibile, host key non ancora salvata".into()),
            );
            let mut r = TestResult::new(
                "unknownHostKey",
                "Il server risponde, ma la sua host key non è ancora salvata. Apri la sessione una volta per verificarla e accettarla.",
            );
            r.key_type = Some(info.key_type);
            r.fingerprint = Some(info.fingerprint);
            r
        }
        Err(SshError::PassphraseRequired) => TestResult::new(
            "needSecret",
            "La chiave privata è cifrata: inserisci la passphrase nel campo per provare.",
        ),
        Err(e) => {
            let status = if hostkey_changed(&e).is_some() {
                state.audit(
                    &ctx,
                    "hostkey_changed",
                    Outcome::Failed,
                    Some(e.to_string()),
                );
                "hostKeyChanged"
            } else {
                state.audit(
                    &ctx,
                    "connection_test",
                    Outcome::Failed,
                    Some(e.to_string()),
                );
                "error"
            };
            TestResult::new(status, e.to_string())
        }
    })
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn ssh_connect(
    state: Shared<'_>,
    request: ConnectRequest,
    cols: u32,
    rows: u32,
    on_data: Channel<InvokeResponseBody>,
    on_event: Channel<TermEvent>,
) -> Result<u64, String> {
    let nodes = state.sessions.list().await.map_err(err)?;
    let (dest, jump_ids) = match request.session_id {
        Some(id) => {
            let Some(SessionNode::Session(s)) = nodes.iter().find(|n| n.id() == id) else {
                return Err("sessione non trovata".into());
            };
            if s.protocol != Protocol::Ssh {
                return Err("protocollo non ancora supportato".into());
            }
            (Endpoint::of(s), s.jump_hosts.clone())
        }
        None => (
            Endpoint {
                host: request.host.ok_or("host mancante")?,
                port: request.port.unwrap_or(22),
                username: request.username.ok_or("utente mancante")?,
                credential_ref: None,
                method: AuthMethod::Password,
                key_path: None,
                follow_cwd: FollowCwd::Auto,
            },
            Vec::new(),
        ),
    };
    if jump_ids.len() > MAX_JUMPS {
        return Err(format!("troppi jump host (massimo {MAX_JUMPS})"));
    }

    let mut hops = jump_hops(&state, &nodes, &jump_ids).await?;

    // Destinazione: segreto fornito ora dall'utente, altrimenti quello nel keyring (se c'è).
    let secret = match request.password {
        Some(p) => Some(p),
        None => stored_secret(&state, &dest.credential_ref).await?,
    };
    let auth = build_credentials(dest.method, dest.key_path, secret).map_err(|e| match e {
        CredError::NeedSecret => PASSWORD_REQUIRED.to_string(),
        CredError::Other(m) => m,
    })?;
    let ctx = AuditCtx {
        host: dest.host.clone(),
        port: dest.port,
        user: dest.username.clone(),
        via: hops
            .iter()
            .map(|h| format!("{}:{}", h.host, h.port))
            .collect(),
    };
    hops.push(Hop {
        host: dest.host,
        port: dest.port,
        username: dest.username,
        auth,
    });

    let verifier: HostKeyVerifier = {
        let state = Arc::clone(&state);
        let on_event = on_event.clone();
        let ctx = ctx.clone();
        Arc::new(move |info: HostKeyInfo| {
            let state = Arc::clone(&state);
            let on_event = on_event.clone();
            let ctx = ctx.clone();
            Box::pin(async move {
                let req_id = state.next_id();
                // Dati pubblici della chiave del server: si possono registrare.
                let key_detail = format!(
                    "{}:{} {} {}",
                    info.host, info.port, info.key_type, info.fingerprint
                );
                let (tx, rx) = oneshot::channel();
                state.pending_hostkeys.lock().unwrap().insert(req_id, tx);
                let sent = on_event.send(TermEvent::HostKey {
                    req_id,
                    host: info.host,
                    port: info.port,
                    key_type: info.key_type,
                    fingerprint: info.fingerprint,
                });
                if sent.is_err() {
                    state.pending_hostkeys.lock().unwrap().remove(&req_id);
                    return HostKeyDecision::Reject;
                }
                let decision = rx.await.unwrap_or(HostKeyDecision::Reject);
                let (event, outcome, how) = match decision {
                    HostKeyDecision::Reject => ("hostkey_rejected", Outcome::Failed, ""),
                    HostKeyDecision::AcceptOnce => {
                        ("hostkey_accepted", Outcome::Ok, " (una volta)")
                    }
                    HostKeyDecision::AcceptAndSave => {
                        ("hostkey_accepted", Outcome::Ok, " (salvata)")
                    }
                };
                state.audit(&ctx, event, outcome, Some(format!("{key_detail}{how}")));
                decision
            })
        })
    };

    let (ev_tx, mut ev_rx) = mpsc::channel::<ShellEvent>(64);
    let on_event_for_errors = on_event.clone();
    let pump_state = Arc::clone(&state);
    let pump_ctx = ctx.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = ev_rx.recv().await {
            match ev {
                ShellEvent::Data(d) => {
                    if on_data.send(InvokeResponseBody::Raw(d)).is_err() {
                        break;
                    }
                }
                ShellEvent::Cwd(path) => {
                    let _ = on_event.send(TermEvent::Cwd { path });
                }
                ShellEvent::Closed(exit) => {
                    pump_state.audit(&pump_ctx, "disconnect", Outcome::Ok, None);
                    let _ = on_event.send(TermEvent::Closed { exit });
                    break;
                }
            }
        }
    });

    let shell = pxt_ssh::connect_shell(
        ConnectParams {
            hops,
            cols,
            rows,
            term: "xterm-256color".into(),
        },
        KnownHosts::new(state.known_hosts.clone()),
        verifier,
        ev_tx,
    )
    .await;
    let shell = match shell {
        Ok(s) => {
            state.audit(&ctx, "connect", Outcome::Ok, None);
            s
        }
        Err(e) => {
            // La richiesta di passphrase non è un fallimento: l'utente la fornirà.
            if let Some((host, port, expected, got)) = hostkey_changed(&e) {
                state.audit(
                    &ctx,
                    "hostkey_changed",
                    Outcome::Failed,
                    Some(e.to_string()),
                );
                let _ = on_event_for_errors.send(TermEvent::HostKeyChanged {
                    host,
                    port,
                    expected,
                    got,
                });
                return Err(HOSTKEY_CHANGED.to_string());
            } else if !matches!(e, SshError::PassphraseRequired) {
                state.audit(&ctx, "connect", Outcome::Failed, Some(e.to_string()));
            }
            return Err(match e {
                SshError::PassphraseRequired => PASSPHRASE_REQUIRED.to_string(),
                other => other.to_string(),
            });
        }
    };

    let id = state.next_id();
    state.shells.lock().unwrap().insert(id, shell.clone());
    state.conn_ctx.lock().unwrap().insert(id, ctx.clone());

    // Tracciamento della directory (6.4): in automatico, salvo `followCwd` = `off` / `osc7-only`.
    let auto = dest.follow_cwd == FollowCwd::Auto;
    let tracking_events = on_event_for_errors;
    tauri::async_runtime::spawn(async move {
        let event = if auto {
            // Il tempo che la shell mostri il primo prompt, così la riga iniettata viene letta subito.
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            tracking_event(shell.enable_cwd_tracking().await)
        } else {
            TermEvent::Tracking {
                state: "off",
                shell: None,
                message: None,
            }
        };
        let _ = tracking_events.send(event);
    });
    Ok(id)
}

/// Traduce l'esito dell'attivazione del tracciamento in un evento per la UI.
fn tracking_event(result: Result<String, SshError>) -> TermEvent {
    match result {
        Ok(shell) => TermEvent::Tracking {
            state: "active",
            shell: Some(shell),
            message: None,
        },
        Err(SshError::UnsupportedShell(shell)) => TermEvent::Tracking {
            state: "unsupported",
            shell: Some(shell),
            message: None,
        },
        Err(e) => TermEvent::Tracking {
            state: "error",
            shell: None,
            message: Some(e.to_string()),
        },
    }
}

/// Attiva a mano il tracciamento della directory (rileva la shell e inietta la riga).
#[tauri::command]
async fn ssh_enable_cwd_tracking(state: Shared<'_>, id: u64) -> Result<TermEvent, String> {
    let shell = get_shell(&state, id)?;
    Ok(tracking_event(shell.enable_cwd_tracking().await))
}

fn get_shell(state: &AppState, id: u64) -> Result<Shell, String> {
    state
        .shells
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| "sessione inesistente".to_string())
}

/// Sessione SFTP della connessione: la apre al primo uso (nessuna seconda autenticazione).
async fn sftp_for(state: &AppState, id: u64) -> Result<Arc<SftpFs>, String> {
    let mut open = state.sftp.lock().await;
    if let Some(fs) = open.get(&id) {
        return Ok(Arc::clone(fs));
    }
    let shell = get_shell(state, id)?;
    let stream = shell.open_sftp_stream().await.map_err(err)?;
    let fs = Arc::new(SftpFs::connect(stream).await.map_err(err)?);
    open.insert(id, Arc::clone(&fs));
    Ok(fs)
}

/// Cartella di partenza dell'SFTP (di norma la home dell'utente sul server).
#[tauri::command]
async fn sftp_home(state: Shared<'_>, id: u64) -> Result<String, String> {
    sftp_for(&state, id)
        .await?
        .canonicalize(".")
        .await
        .map_err(err)
}

/// Contenuto di una cartella remota.
#[tauri::command]
async fn sftp_list(state: Shared<'_>, id: u64, path: String) -> Result<Vec<Entry>, String> {
    sftp_for(&state, id).await?.list(&path).await.map_err(err)
}

/// Registra nell'audit un'operazione di modifica sui file di una connessione (solo conteggi, mai nomi).
fn audit_conn(state: &AppState, conn: u64, event: &str, ok: bool, detail: String) {
    if let Some(ctx) = state.conn_ctx.lock().unwrap().get(&conn).cloned() {
        let outcome = if ok { Outcome::Ok } else { Outcome::Failed };
        state.audit(&ctx, event, outcome, Some(detail));
    }
}

/// Nome valido per una voce: non vuoto, senza `/` né caratteri di controllo, e non `.` o `..`.
fn valid_name(name: &str) -> Result<&str, String> {
    let n = name.trim();
    if n.is_empty() || n == "." || n == ".." || n.contains('/') || n.chars().any(|c| c.is_control())
    {
        return Err("nome non valido".into());
    }
    Ok(n)
}

fn parent_of(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => trimmed[..i].to_string(),
    }
}

fn join_remote(dir: &str, name: &str) -> String {
    fs_transfer::remote_join(dir, name)
}

/// Elimina file e cartelle (con tutto il contenuto). I link simbolici non vengono mai seguiti.
#[tauri::command]
async fn sftp_delete(
    state: Shared<'_>,
    id: u64,
    paths: Vec<String>,
) -> Result<fs_ops::OpReport, String> {
    let fs = sftp_for(&state, id).await?;
    let cancel = AtomicBool::new(false);
    let mut total = fs_ops::OpReport::default();
    for p in &paths {
        match fs_ops::remove_recursive(&*fs, p, &cancel).await {
            Ok(r) => {
                total.done += r.done;
                total.failed.extend(r.failed);
            }
            Err(e) => total.failed.push(fs_ops::OpFailure {
                path: p.clone(),
                message: e.to_string(),
            }),
        }
    }
    audit_conn(
        &state,
        id,
        "sftp_delete",
        total.failed.is_empty(),
        format!(
            "{} elementi eliminati, {} errori",
            total.done,
            total.failed.len()
        ),
    );
    Ok(total)
}

/// Conta file, cartelle e byte di un percorso (per le proprietà di una cartella).
#[tauri::command]
async fn sftp_tree_info(
    state: Shared<'_>,
    id: u64,
    path: String,
) -> Result<fs_ops::TreeInfo, String> {
    let fs = sftp_for(&state, id).await?;
    fs_ops::tree_info(&*fs, &path, &AtomicBool::new(false))
        .await
        .map_err(err)
}

/// Cambia i permessi, anche su tutto il contenuto se `recursive`.
#[tauri::command]
async fn sftp_chmod(
    state: Shared<'_>,
    id: u64,
    path: String,
    mode: u32,
    recursive: bool,
) -> Result<fs_ops::OpReport, String> {
    if mode > 0o7777 {
        return Err("permessi non validi".into());
    }
    let fs = sftp_for(&state, id).await?;
    let report = fs_ops::chmod(&*fs, &path, mode, recursive, &AtomicBool::new(false))
        .await
        .map_err(err)?;
    audit_conn(
        &state,
        id,
        "sftp_chmod",
        report.failed.is_empty(),
        format!(
            "{:o}{}: {} elementi, {} errori",
            mode,
            if recursive { " ricorsivo" } else { "" },
            report.done,
            report.failed.len()
        ),
    );
    Ok(report)
}

/// Rinomina una voce restando nella stessa cartella. Non sovrascrive: se il nome è già usato dà errore.
#[tauri::command]
async fn sftp_rename(
    state: Shared<'_>,
    id: u64,
    path: String,
    new_name: String,
) -> Result<String, String> {
    let name = valid_name(&new_name)?;
    let fs = sftp_for(&state, id).await?;
    let target = join_remote(&parent_of(&path), name);
    if target == path {
        return Ok(target);
    }
    if fs.lstat(&target).await.is_ok() {
        return Err(format!("esiste già una voce chiamata \"{name}\""));
    }
    fs.rename(&path, &target).await.map_err(err)?;
    audit_conn(&state, id, "sftp_rename", true, "1 elemento".into());
    Ok(target)
}

/// Crea una cartella dentro `parent`.
#[tauri::command]
async fn sftp_mkdir(
    state: Shared<'_>,
    id: u64,
    parent: String,
    name: String,
) -> Result<String, String> {
    let name = valid_name(&name)?;
    let fs = sftp_for(&state, id).await?;
    let target = join_remote(&parent, name);
    if fs.lstat(&target).await.is_ok() {
        return Err(format!("esiste già una voce chiamata \"{name}\""));
    }
    fs.mkdir(&target).await.map_err(err)?;
    audit_conn(&state, id, "sftp_mkdir", true, "1 cartella".into());
    Ok(target)
}

/// Piano di trasferimento in attesa, legato alla connessione per cui è stato calcolato.
struct StoredPlan {
    conn: u64,
    plan: fs_transfer::Plan,
}

/// Quanti piani non avviati si tengono al massimo (i più vecchi vengono scartati).
const MAX_PENDING_PLANS: usize = 20;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanSummary {
    plan_id: u64,
    direction: fs_transfer::Direction,
    file_count: usize,
    dir_count: usize,
    total_bytes: u64,
    conflicts: usize,
    /// Qualche nome in conflitto, per mostrarlo nel dialogo.
    conflict_samples: Vec<String>,
}

fn store_plan(state: &AppState, conn: u64, plan: fs_transfer::Plan) -> PlanSummary {
    let plan_id = state.next_id();
    let summary = PlanSummary {
        plan_id,
        direction: plan.direction,
        file_count: plan.file_count,
        dir_count: plan
            .items
            .iter()
            .filter(|i| i.kind == fs_transfer::ItemKind::Dir)
            .count(),
        total_bytes: plan.total_bytes,
        conflicts: plan.conflicts,
        conflict_samples: plan
            .items
            .iter()
            .filter(|i| i.conflict)
            .take(5)
            .map(|i| {
                i.dst
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(&i.dst)
                    .to_string()
            })
            .collect(),
    };
    let mut plans = state.plans.lock().unwrap();
    if plans.len() >= MAX_PENDING_PLANS {
        if let Some(oldest) = plans.keys().min().copied() {
            plans.remove(&oldest);
        }
    }
    plans.insert(plan_id, StoredPlan { conn, plan });
    summary
}

/// Calcola cosa serve per caricare file e cartelle locali nella cartella remota (nessuna modifica).
#[tauri::command]
async fn transfer_plan_upload(
    state: Shared<'_>,
    id: u64,
    local_paths: Vec<String>,
    remote_dir: String,
) -> Result<PlanSummary, String> {
    let fs = sftp_for(&state, id).await?;
    let paths: Vec<PathBuf> = local_paths.into_iter().map(PathBuf::from).collect();
    let plan = fs_transfer::plan_upload(&*fs, &paths, &remote_dir)
        .await
        .map_err(err)?;
    Ok(store_plan(&state, id, plan))
}

/// Calcola cosa serve per scaricare file e cartelle remote nella cartella locale (nessuna modifica).
#[tauri::command]
async fn transfer_plan_download(
    state: Shared<'_>,
    id: u64,
    remote_paths: Vec<String>,
    local_dir: String,
) -> Result<PlanSummary, String> {
    let fs = sftp_for(&state, id).await?;
    let plan = fs_transfer::plan_download(&*fs, &remote_paths, std::path::Path::new(&local_dir))
        .await
        .map_err(err)?;
    Ok(store_plan(&state, id, plan))
}

/// Scarta un piano che l'utente non ha avviato.
#[tauri::command]
fn transfer_plan_discard(state: Shared<'_>, plan_id: u64) {
    state.plans.lock().unwrap().remove(&plan_id);
}

/// Avanzamento di un trasferimento, verso la UI (canale per-trasferimento, non evento globale).
#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
enum TransferEvent {
    #[serde(rename_all = "camelCase")]
    Progress {
        job_id: u64,
        bytes_done: u64,
        bytes_total: u64,
        files_done: usize,
        files_total: usize,
        current: String,
        speed_bps: f64,
    },
    #[serde(rename_all = "camelCase")]
    Finished {
        job_id: u64,
        summary: fs_transfer::Summary,
    },
    #[serde(rename_all = "camelCase")]
    Cancelled { job_id: u64 },
    #[serde(rename_all = "camelCase")]
    Failed { job_id: u64, message: String },
}

/// Avvia un piano già calcolato con la politica scelta per i conflitti. Restituisce l'id del lavoro.
#[tauri::command]
async fn transfer_start(
    state: Shared<'_>,
    plan_id: u64,
    policy: ConflictPolicy,
    on_event: Channel<TransferEvent>,
) -> Result<u64, String> {
    let stored = state
        .plans
        .lock()
        .unwrap()
        .remove(&plan_id)
        .ok_or("piano non trovato (scaduto o già avviato)")?;
    let fs = sftp_for(&state, stored.conn).await?;
    let job_id = state.next_id();
    let cancel = Arc::new(AtomicBool::new(false));
    state
        .jobs
        .lock()
        .unwrap()
        .insert(job_id, Arc::clone(&cancel));

    let state = Arc::clone(&state);
    tauri::async_runtime::spawn(async move {
        let plan = stored.plan;
        let started = std::time::Instant::now();
        // Velocità media mobile sugli ultimi aggiornamenti.
        let mut last = (started, 0u64);
        let mut speed = 0.0_f64;
        let mut report = |p: &Progress| {
            let now = std::time::Instant::now();
            let dt = now.duration_since(last.0).as_secs_f64();
            if dt >= 0.2 {
                let inst = (p.bytes_done.saturating_sub(last.1)) as f64 / dt;
                speed = if speed == 0.0 {
                    inst
                } else {
                    0.7 * speed + 0.3 * inst
                };
                last = (now, p.bytes_done);
            }
            let _ = on_event.send(TransferEvent::Progress {
                job_id,
                bytes_done: p.bytes_done,
                bytes_total: p.bytes_total,
                files_done: p.files_done,
                files_total: p.files_total,
                current: p.current.clone(),
                speed_bps: speed,
            });
        };
        let result = fs_transfer::run(&*fs, &plan, policy, &cancel, &mut report).await;

        let event_name = match plan.direction {
            fs_transfer::Direction::Upload => "transfer_upload",
            fs_transfer::Direction::Download => "transfer_download",
        };
        let ctx = state.conn_ctx.lock().unwrap().get(&stored.conn).cloned();
        let (event, audit_outcome, detail) = match result {
            Ok(summary) => {
                let ok = summary.failed.is_empty();
                let detail = format!(
                    "{} file, {} byte, {} saltati, {} errori",
                    summary.files_done,
                    summary.bytes,
                    summary.files_skipped,
                    summary.failed.len()
                );
                (
                    TransferEvent::Finished { job_id, summary },
                    if ok { Outcome::Ok } else { Outcome::Failed },
                    detail,
                )
            }
            Err(fs_transfer::TransferError::Cancelled) => (
                TransferEvent::Cancelled { job_id },
                Outcome::Failed,
                "annullato".to_string(),
            ),
            Err(e) => (
                TransferEvent::Failed {
                    job_id,
                    message: e.to_string(),
                },
                Outcome::Failed,
                e.to_string(),
            ),
        };
        if let Some(ctx) = ctx {
            state.audit(&ctx, event_name, audit_outcome, Some(detail));
        }
        state.jobs.lock().unwrap().remove(&job_id);
        let _ = on_event.send(event);
    });
    Ok(job_id)
}

/// Annulla un trasferimento in corso: si ferma entro pochi istanti e non lascia file parziali.
#[tauri::command]
fn transfer_cancel(state: Shared<'_>, job_id: u64) {
    if let Some(flag) = state.jobs.lock().unwrap().get(&job_id) {
        flag.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
async fn ssh_write(state: Shared<'_>, id: u64, data: Vec<u8>) -> Result<(), String> {
    get_shell(&state, id)?.write(data).await.map_err(err)
}

#[tauri::command]
async fn ssh_resize(state: Shared<'_>, id: u64, cols: u32, rows: u32) -> Result<(), String> {
    get_shell(&state, id)?.resize(cols, rows).await.map_err(err)
}

#[tauri::command]
async fn ssh_inject_cwd(state: Shared<'_>, id: u64, shell: String) -> Result<(), String> {
    get_shell(&state, id)?
        .inject_cwd_tracking(&shell)
        .await
        .map_err(err)
}

#[tauri::command]
async fn ssh_close(state: Shared<'_>, id: u64) -> Result<(), String> {
    state.sftp.lock().await.remove(&id);
    state.conn_ctx.lock().unwrap().remove(&id);
    let shell = state.shells.lock().unwrap().remove(&id);
    if let Some(s) = shell {
        s.close().await;
    }
    Ok(())
}

#[tauri::command]
fn hostkey_reply(state: Shared<'_>, req_id: u64, decision: String) {
    let decision = match decision.as_str() {
        "once" => HostKeyDecision::AcceptOnce,
        "save" => HostKeyDecision::AcceptAndSave,
        _ => HostKeyDecision::Reject,
    };
    if let Some(tx) = state.pending_hostkeys.lock().unwrap().remove(&req_id) {
        let _ = tx.send(decision);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KnownHostDto {
    host: String,
    port: u16,
    key_type: String,
    fingerprint: String,
}

#[tauri::command]
fn known_hosts_list(state: Shared<'_>) -> Result<Vec<KnownHostDto>, String> {
    let entries = KnownHosts::new(state.known_hosts.clone())
        .list()
        .map_err(err)?;
    Ok(entries
        .into_iter()
        .map(|e| KnownHostDto {
            host: e.host,
            port: e.port,
            key_type: e.key_type,
            fingerprint: e.fingerprint,
        })
        .collect())
}

/// Toglie le host key salvate di un host: al prossimo accesso verrà chiesta di nuovo conferma.
#[tauri::command]
fn known_hosts_remove(state: Shared<'_>, host: String, port: u16) -> Result<usize, String> {
    let removed = KnownHosts::new(state.known_hosts.clone())
        .remove(&host, port)
        .map_err(err)?;
    let ctx = AuditCtx {
        host,
        port,
        user: String::new(),
        via: Vec::new(),
    };
    state.audit(
        &ctx,
        "hostkey_removed",
        Outcome::Ok,
        Some(format!("{removed} voci")),
    );
    Ok(removed)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = resolve_data_dir(app)?;
            app.manage(Arc::new(AppState::new(dir)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            about_info,
            app_paths,
            settings_get,
            settings_set,
            sessions_list,
            session_save,
            session_delete,
            sessions_export,
            sessions_import,
            session_test,
            session_duplicate,
            ssh_connect,
            ssh_write,
            ssh_resize,
            ssh_close,
            sftp_home,
            sftp_list,
            sftp_delete,
            sftp_tree_info,
            sftp_chmod,
            sftp_rename,
            sftp_mkdir,
            transfer_plan_upload,
            transfer_plan_download,
            transfer_plan_discard,
            transfer_start,
            transfer_cancel,
            ssh_inject_cwd,
            ssh_enable_cwd_tracking,
            hostkey_reply,
            known_hosts_list,
            known_hosts_remove
        ])
        .run(tauri::generate_context!())
        .expect("errore nell'avvio di PiCoXterm");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_for_rename_and_mkdir_are_validated() {
        assert_eq!(valid_name("  nuova  ").unwrap(), "nuova");
        assert_eq!(valid_name("con spazi.txt").unwrap(), "con spazi.txt");
        for bad in ["", "   ", ".", "..", "a/b", "/", "tab\there"] {
            assert!(valid_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn parent_of_handles_root_and_trailing_slashes() {
        assert_eq!(parent_of("/home/u/file"), "/home/u");
        assert_eq!(parent_of("/home/u/dir/"), "/home/u");
        assert_eq!(parent_of("/file"), "/");
        assert_eq!(parent_of("relativo"), "/");
        assert_eq!(join_remote("/", "x"), "/x");
        assert_eq!(join_remote("/a", "x"), "/a/x");
    }
}
