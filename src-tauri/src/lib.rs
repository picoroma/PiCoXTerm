use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use pxt_core::{
    subtree_ids, transfer, AuditEvent, AuditLog, AuthMethod, CredentialRef, FollowCwd,
    KeyringSecretProvider, LocalSessionProvider, LocalSettings, Outcome, Protocol, Secret,
    SecretProvider, Session, SessionNode, SessionProvider, Settings,
};
use pxt_fs::edit as fs_edit;
use pxt_fs::external as fs_external;
use pxt_fs::ops as fs_ops;
use pxt_fs::transfer as fs_transfer;
use pxt_fs::transfer::{ConflictPolicy, Progress};
use pxt_fs::{AnyFs, Entry, FtpFs, FtpParams, RemoteFs, S3Fs, S3Params, SftpFs};
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
    /// Connessioni delle sessioni SFTP: senza shell, solo il sottosistema SFTP.
    sftp_conns: Mutex<HashMap<u64, pxt_ssh::SftpConnection>>,
    /// Sessioni VNC: il lato comandi (invio dei byte di noVNC, chiusura) di ciascuna.
    vnc: Mutex<HashMap<u64, pxt_vnc::VncHandle>>,
    /// Sessioni RDP integrate: il lato comandi (input, chiusura) di ciascuna.
    rdp: Mutex<HashMap<u64, Arc<RdpEntry>>>,
    /// Sessioni SFTP, una per connessione, aperte al primo uso sulla stessa connessione SSH.
    sftp: tokio::sync::Mutex<HashMap<u64, Arc<AnyFs>>>,
    /// Policy dell'organizzazione (`policy.json`, sola lettura) e l'eventuale avviso se non è valida.
    policy: pxt_core::Policy,
    policy_warning: Option<String>,
    /// Piani di trasferimento in attesa di conferma (una scelta sui conflitti) o di avvio.
    plans: Mutex<HashMap<u64, StoredPlan>>,
    /// Trasferimenti in corso: segnale di controllo (pausa, annullamento) di ciascuno.
    jobs: Mutex<HashMap<u64, Arc<fs_transfer::Control>>>,
    /// Lavori che si possono riprendere (in pausa, o conclusi con errori su alcuni file).
    resumable: Mutex<HashMap<u64, JobRecord>>,
    /// File remoti aperti con l'editor di sistema (copie temporanee seguite fino al salvataggio).
    externals: Mutex<HashMap<u64, Arc<ExternalEdit>>>,
    /// Destinazione di ogni connessione, per l'audit dei trasferimenti.
    conn_ctx: Mutex<HashMap<u64, AuditCtx>>,
    pending_hostkeys: Mutex<HashMap<u64, oneshot::Sender<HostKeyDecision>>>,
    next_id: AtomicU64,
}

impl AppState {
    fn new(data_dir: PathBuf) -> Self {
        let (policy, policy_warning) = pxt_core::Policy::load(&data_dir.join("policy.json"));
        Self {
            policy,
            policy_warning,
            sessions: LocalSessionProvider::new(data_dir.join("sessions.json")),
            settings: LocalSettings::new(data_dir.join("settings.json")),
            audit: AuditLog::new(data_dir.join("logs")),
            secrets: KeyringSecretProvider::new(pxt_core::APP_NAME),
            known_hosts: data_dir.join("known_hosts"),
            data_dir,
            shells: Mutex::default(),
            sftp_conns: Mutex::default(),
            rdp: Mutex::default(),
            vnc: Mutex::default(),
            sftp: tokio::sync::Mutex::default(),
            plans: Mutex::default(),
            jobs: Mutex::default(),
            resumable: Mutex::default(),
            externals: Mutex::default(),
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
            protocol: ctx.protocol.into(),
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
    /// Protocollo della connessione (`ssh`, `sftp`, `ftp`), per il log di audit.
    protocol: &'static str,
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
    /// FTPS: il certificato del server non è verificabile (e non è stato accettato prima, o è cambiato).
    #[serde(rename_all = "camelCase")]
    CertUntrusted {
        host: String,
        port: u16,
        fingerprint: String,
        /// C'era già un'impronta accettata e questa è diversa: potrebbe essere un attacco.
        changed: bool,
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
    /// La sessione VNC è finita.
    VncClosed {
        reason: String,
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
    if session.protocol == Protocol::Ftp {
        return Ok(ftp_test(&state, &session, password).await);
    }
    if session.protocol == Protocol::S3 {
        return Ok(s3_test(&state, &session, password).await);
    }
    if session.protocol == Protocol::Rdp {
        return Ok(rdp_test(&state, &session).await);
    }
    if session.protocol == Protocol::Vnc {
        return Ok(vnc_test(&state, &session, password).await);
    }
    if !matches!(session.protocol, Protocol::Ssh | Protocol::Sftp) {
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
        protocol: if session.protocol == Protocol::Sftp {
            "sftp"
        } else {
            "ssh"
        },
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

/// Tutto ciò che serve per collegarsi: destinazione, salti, contesto di audit e verifica della host key.
struct Prepared {
    dest: Endpoint,
    hops: Vec<Hop>,
    ctx: AuditCtx,
    verifier: HostKeyVerifier,
}

/// Risolve la sessione (o l'host indicato a mano), le credenziali e i jump host, e prepara la verifica
/// della host key, che passa dalla UI (`on_event`). `wanted`: il protocollo che la sessione deve avere.
async fn prepare_connection(
    state: &Arc<AppState>,
    request: ConnectRequest,
    wanted: Protocol,
    on_event: &Channel<TermEvent>,
) -> Result<Prepared, String> {
    let nodes = state.sessions.list().await.map_err(err)?;
    let (dest, jump_ids) = match request.session_id {
        Some(id) => {
            let Some(SessionNode::Session(s)) = nodes.iter().find(|n| n.id() == id) else {
                return Err("sessione non trovata".into());
            };
            if s.protocol != wanted {
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

    let mut hops = jump_hops(state, &nodes, &jump_ids).await?;

    // Destinazione: segreto fornito ora dall'utente, altrimenti quello nel keyring (se c'è).
    let secret = match request.password {
        Some(p) => Some(p),
        None => stored_secret(state, &dest.credential_ref).await?,
    };
    let auth =
        build_credentials(dest.method, dest.key_path.clone(), secret).map_err(|e| match e {
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
        protocol: if wanted == Protocol::Sftp {
            "sftp"
        } else {
            "ssh"
        },
    };
    hops.push(Hop {
        host: dest.host.clone(),
        port: dest.port,
        username: dest.username.clone(),
        auth,
    });

    let verifier: HostKeyVerifier = {
        let state = Arc::clone(state);
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
    Ok(Prepared {
        dest,
        hops,
        ctx,
        verifier,
    })
}

/// Registra il fallimento di una connessione e lo traduce nel codice d'errore letto dalla UI.
fn connect_failure(
    state: &AppState,
    ctx: &AuditCtx,
    on_event: &Channel<TermEvent>,
    e: SshError,
) -> String {
    // La richiesta di passphrase non è un fallimento: l'utente la fornirà.
    if let Some((host, port, expected, got)) = hostkey_changed(&e) {
        state.audit(ctx, "hostkey_changed", Outcome::Failed, Some(e.to_string()));
        let _ = on_event.send(TermEvent::HostKeyChanged {
            host,
            port,
            expected,
            got,
        });
        return HOSTKEY_CHANGED.to_string();
    }
    if !matches!(e, SshError::PassphraseRequired) {
        state.audit(ctx, "connect", Outcome::Failed, Some(e.to_string()));
    }
    match e {
        SshError::PassphraseRequired => PASSPHRASE_REQUIRED.to_string(),
        other => other.to_string(),
    }
}

/// Collega una sessione SFTP: autentica (jump host compresi) senza aprire alcuna shell.
#[tauri::command]
async fn sftp_connect(
    state: Shared<'_>,
    request: ConnectRequest,
    on_event: Channel<TermEvent>,
) -> Result<u64, String> {
    let p = prepare_connection(&state, request, Protocol::Sftp, &on_event).await?;
    let conn = pxt_ssh::connect_sftp(
        p.hops,
        KnownHosts::new(state.known_hosts.clone()),
        p.verifier,
    )
    .await
    .map_err(|e| connect_failure(&state, &p.ctx, &on_event, e))?;
    state.audit(&p.ctx, "connect", Outcome::Ok, Some("sftp".into()));
    let id = state.next_id();
    state.sftp_conns.lock().unwrap().insert(id, conn);
    state.conn_ctx.lock().unwrap().insert(id, p.ctx);
    Ok(id)
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
    let Prepared {
        dest,
        hops,
        ctx,
        verifier,
    } = prepare_connection(&state, request, Protocol::Ssh, &on_event).await?;

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
        Err(e) => return Err(connect_failure(&state, &ctx, &on_event_for_errors, e)),
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

/// Attiva a mano il tracciamento della directory ("Re-inietta"): rileva la shell **in uso adesso**
/// (dopo `su - altro_utente` non è più quella di login) e ne inietta la riga.
#[tauri::command]
async fn ssh_enable_cwd_tracking(state: Shared<'_>, id: u64) -> Result<TermEvent, String> {
    let shell = get_shell(&state, id)?;
    Ok(tracking_event(shell.reinject_cwd_tracking().await))
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

// ---- Sessioni FTP e FTPS ----

/// Codice d'errore letto dalla UI: il certificato del server va confermato dall'utente.
const CERT_UNTRUSTED: &str = "certificate-untrusted";

fn ftp_security(s: pxt_core::FtpSecurity) -> pxt_fs::FtpSecurity {
    match s {
        pxt_core::FtpSecurity::Plain => pxt_fs::FtpSecurity::Plain,
        pxt_core::FtpSecurity::Explicit => pxt_fs::FtpSecurity::Explicit,
        pxt_core::FtpSecurity::Implicit => pxt_fs::FtpSecurity::Implicit,
    }
}

/// Divieti della policy per una sessione FTP; `Err` con il motivo da mostrare.
fn ftp_policy_check(state: &AppState, session: &Session) -> Result<(), String> {
    if !state.policy.allows(Protocol::Ftp) {
        return Err("Il protocollo FTP è vietato dalla policy dell'organizzazione.".into());
    }
    if session.ftp.security == pxt_core::FtpSecurity::Plain && !state.policy.allow_plain_ftp {
        return Err(
            "L'FTP in chiaro è vietato dalla policy dell'organizzazione: usa FTPS (TLS esplicito o implicito)."
                .into(),
        );
    }
    Ok(())
}

/// Parametri di connessione di una sessione FTP. `Err(PASSWORD_REQUIRED)` se manca la password.
async fn ftp_params(
    state: &AppState,
    session: &Session,
    password: Option<String>,
    pinned_override: Option<String>,
) -> Result<FtpParams, String> {
    let secret = match password {
        Some(p) => Some(p),
        None => stored_secret(state, &session.credential_ref).await?,
    };
    let password = match secret {
        Some(p) => p,
        // L'accesso anonimo si fa con l'utente `anonymous` e un indirizzo come password.
        None if session.username.eq_ignore_ascii_case("anonymous") => "anonymous@".to_string(),
        None => return Err(PASSWORD_REQUIRED.to_string()),
    };
    Ok(FtpParams {
        host: session.host.clone(),
        port: session.port,
        username: session.username.clone(),
        password,
        security: ftp_security(session.ftp.security),
        pinned_cert: pinned_override.or_else(|| session.ftp.pinned_cert.clone()),
        max_connections: 4,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FtpConnectRequest {
    session_id: Uuid,
    /// Password digitata ora dall'utente (ha la precedenza su quella nel keyring).
    password: Option<String>,
    /// Impronta del certificato accettata ora dall'utente, anche senza salvarla nella sessione.
    pinned_cert: Option<String>,
}

fn find_ftp_session(nodes: &[SessionNode], id: Uuid) -> Result<Session, String> {
    match nodes.iter().find(|n| n.id() == id) {
        Some(SessionNode::Session(s)) if s.protocol == Protocol::Ftp => Ok((**s).clone()),
        Some(_) => Err("la sessione non è una sessione FTP".into()),
        None => Err("sessione non trovata".into()),
    }
}

/// Collega una sessione FTP o FTPS. Un certificato non verificabile non si accetta mai in silenzio:
/// la UI riceve `certUntrusted` con l'impronta e decide l'utente (poi si riprova con `pinnedCert`).
#[tauri::command]
async fn ftp_connect(
    state: Shared<'_>,
    request: FtpConnectRequest,
    on_event: Channel<TermEvent>,
) -> Result<u64, String> {
    let nodes = state.sessions.list().await.map_err(err)?;
    let session = find_ftp_session(&nodes, request.session_id)?;
    let ctx = AuditCtx {
        host: session.host.clone(),
        port: session.port,
        user: session.username.clone(),
        via: Vec::new(),
        protocol: "ftp",
    };
    if let Err(reason) = ftp_policy_check(&state, &session) {
        state.audit(
            &ctx,
            "connect",
            Outcome::Failed,
            Some("vietato dalla policy".into()),
        );
        return Err(reason);
    }
    let params = ftp_params(&state, &session, request.password, request.pinned_cert).await?;
    let mode = format!("{:?}", session.ftp.security).to_lowercase();
    match FtpFs::connect(params).await {
        Ok(fs) => {
            let detail = if session.ftp.security == pxt_core::FtpSecurity::Plain {
                format!("{mode}: senza cifratura")
            } else {
                mode
            };
            state.audit(&ctx, "connect", Outcome::Ok, Some(detail));
            let id = state.next_id();
            state.sftp.lock().await.insert(id, Arc::new(AnyFs::Ftp(fs)));
            state.conn_ctx.lock().unwrap().insert(id, ctx);
            Ok(id)
        }
        Err(pxt_fs::FsError::UntrustedCertificate {
            fingerprint,
            changed,
        }) => {
            state.audit(
                &ctx,
                if changed {
                    "cert_changed"
                } else {
                    "cert_untrusted"
                },
                Outcome::Failed,
                Some(format!("SHA256 {fingerprint}")),
            );
            let _ = on_event.send(TermEvent::CertUntrusted {
                host: session.host,
                port: session.port,
                fingerprint,
                changed,
            });
            Err(CERT_UNTRUSTED.to_string())
        }
        Err(e) => {
            state.audit(&ctx, "connect", Outcome::Failed, Some(e.to_string()));
            Err(e.to_string())
        }
    }
}

/// Prova una sessione FTP (anche non salvata) senza tenerla aperta.
async fn ftp_test(
    state: &Arc<AppState>,
    session: &Session,
    password: Option<String>,
) -> TestResult {
    if let Err(reason) = ftp_policy_check(state, session) {
        return TestResult::new("error", reason);
    }
    let params = match ftp_params(state, session, password.filter(|p| !p.is_empty()), None).await {
        Ok(p) => p,
        Err(e) if e == PASSWORD_REQUIRED => {
            return TestResult::new(
                "needSecret",
                "Inserisci la password nel campo per provare la connessione.",
            )
        }
        Err(e) => return TestResult::new("error", e),
    };
    let started = std::time::Instant::now();
    match FtpFs::connect(params).await {
        Ok(fs) => match fs.list("/").await {
            Ok(_) => {
                let mut r = TestResult::new("ok", "Connessione e autenticazione riuscite.");
                r.elapsed_ms = u64::try_from(started.elapsed().as_millis()).ok();
                r
            }
            Err(e) => TestResult::new("error", e.to_string()),
        },
        Err(pxt_fs::FsError::UntrustedCertificate {
            fingerprint,
            changed,
        }) => {
            let mut r = TestResult::new(
                if changed {
                    "hostKeyChanged"
                } else {
                    "unknownHostKey"
                },
                if changed {
                    "Il certificato del server è diverso da quello che avevi accettato."
                } else {
                    "Il certificato del server non è verificabile. Apri la sessione una volta per vederne l'impronta e accettarla."
                },
            );
            r.key_type = Some("certificato TLS".into());
            r.fingerprint = Some(fingerprint);
            r
        }
        Err(e) => TestResult::new("error", e.to_string()),
    }
}

// ---- Sessioni S3 ----

fn s3_policy_check(state: &AppState) -> Result<(), String> {
    if !state.policy.allows(Protocol::S3) {
        return Err("Il protocollo S3 è vietato dalla policy dell'organizzazione.".into());
    }
    Ok(())
}

/// Parametri di una sessione S3. Con un profilo AWS non servono chiavi; altrimenti manca la chiave
/// segreta → `Err(PASSWORD_REQUIRED)`.
async fn s3_params(
    state: &AppState,
    session: &Session,
    secret: Option<String>,
) -> Result<S3Params, String> {
    let o = &session.s3;
    let non_empty = |v: &Option<String>| v.clone().filter(|s| !s.trim().is_empty());
    let (access_key, secret_key) = if non_empty(&o.profile).is_some() {
        (None, None)
    } else {
        let secret = match secret {
            Some(p) => Some(p),
            None => stored_secret(state, &session.credential_ref).await?,
        };
        match secret {
            Some(p) => (Some(session.username.clone()), Some(p)),
            None => return Err(PASSWORD_REQUIRED.to_string()),
        }
    };
    Ok(S3Params {
        endpoint: non_empty(&o.endpoint),
        region: o.region.clone(),
        access_key,
        secret_key,
        session_token: None,
        profile: non_empty(&o.profile),
        path_style: o.path_style,
        bucket: non_empty(&o.bucket),
    })
}

fn find_s3_session(nodes: &[SessionNode], id: Uuid) -> Result<Session, String> {
    match nodes.iter().find(|n| n.id() == id) {
        Some(SessionNode::Session(s)) if s.protocol == Protocol::S3 => Ok((**s).clone()),
        Some(_) => Err("la sessione non è una sessione S3".into()),
        None => Err("sessione non trovata".into()),
    }
}

fn s3_audit_ctx(session: &Session) -> AuditCtx {
    AuditCtx {
        host: session
            .s3
            .endpoint
            .clone()
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| "aws".into()),
        port: session.port,
        user: session.username.clone(),
        via: Vec::new(),
        protocol: "s3",
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct S3ConnectRequest {
    session_id: Uuid,
    /// Chiave segreta digitata ora dall'utente (ha la precedenza su quella nel keyring).
    password: Option<String>,
}

/// Collega una sessione S3: da qui in poi la connessione si usa con gli stessi comandi `sftp_*`.
#[tauri::command]
async fn s3_connect(state: Shared<'_>, request: S3ConnectRequest) -> Result<u64, String> {
    let nodes = state.sessions.list().await.map_err(err)?;
    let session = find_s3_session(&nodes, request.session_id)?;
    let ctx = s3_audit_ctx(&session);
    if let Err(reason) = s3_policy_check(&state) {
        state.audit(
            &ctx,
            "connect",
            Outcome::Failed,
            Some("vietato dalla policy".into()),
        );
        return Err(reason);
    }
    let params = s3_params(&state, &session, request.password).await?;
    match S3Fs::connect(params).await {
        Ok(fs) => {
            state.audit(&ctx, "connect", Outcome::Ok, Some("s3".into()));
            let id = state.next_id();
            state.sftp.lock().await.insert(id, Arc::new(AnyFs::S3(fs)));
            state.conn_ctx.lock().unwrap().insert(id, ctx);
            Ok(id)
        }
        Err(e) => {
            state.audit(&ctx, "connect", Outcome::Failed, Some(e.to_string()));
            Err(e.to_string())
        }
    }
}

async fn s3_test(state: &Arc<AppState>, session: &Session, secret: Option<String>) -> TestResult {
    if let Err(reason) = s3_policy_check(state) {
        return TestResult::new("error", reason);
    }
    let params = match s3_params(state, session, secret.filter(|p| !p.is_empty())).await {
        Ok(p) => p,
        Err(e) if e == PASSWORD_REQUIRED => {
            return TestResult::new(
                "needSecret",
                "Inserisci la chiave segreta nel campo per provare la connessione.",
            )
        }
        Err(e) => return TestResult::new("error", e),
    };
    let started = std::time::Instant::now();
    match S3Fs::connect(params).await {
        Ok(_) => {
            let mut r = TestResult::new("ok", "Connessione e autenticazione riuscite.");
            r.elapsed_ms = u64::try_from(started.elapsed().as_millis()).ok();
            r
        }
        Err(e) => TestResult::new("error", e.to_string()),
    }
}

/// Il filesystem S3 di una connessione; errore se la connessione è di un altro tipo.
async fn s3_for(state: &AppState, id: u64) -> Result<Arc<AnyFs>, String> {
    let fs = sftp_for(state, id).await?;
    match &*fs {
        AnyFs::S3(_) => Ok(fs),
        _ => Err("la connessione non è S3".into()),
    }
}

macro_rules! with_s3 {
    ($fs:expr, $s3:ident => $body:expr) => {
        match &*$fs {
            AnyFs::S3($s3) => $body,
            _ => unreachable!("controllato da s3_for"),
        }
    };
}

/// Proprietà, intestazioni, metadati e tag di un oggetto.
#[tauri::command]
async fn s3_object_info(
    state: Shared<'_>,
    id: u64,
    path: String,
) -> Result<pxt_fs::ObjectInfo, String> {
    let fs = s3_for(&state, id).await?;
    with_s3!(fs, s3 => s3.object_info(&path).await.map_err(err))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct S3HeadersArg {
    content_type: Option<String>,
    cache_control: Option<String>,
    content_disposition: Option<String>,
    content_encoding: Option<String>,
    #[serde(default)]
    metadata: HashMap<String, String>,
}

#[tauri::command]
async fn s3_set_headers(
    state: Shared<'_>,
    id: u64,
    path: String,
    headers: S3HeadersArg,
) -> Result<(), String> {
    let fs = s3_for(&state, id).await?;
    let blank = |v: Option<String>| v.filter(|s| !s.trim().is_empty());
    let h = pxt_fs::ObjectHeaders {
        content_type: blank(headers.content_type),
        cache_control: blank(headers.cache_control),
        content_disposition: blank(headers.content_disposition),
        content_encoding: blank(headers.content_encoding),
        metadata: headers.metadata,
    };
    with_s3!(fs, s3 => s3.set_headers(&path, &h).await.map_err(err))?;
    audit_conn(&state, id, "s3_set_headers", true, "1 oggetto".into());
    Ok(())
}

#[tauri::command]
async fn s3_set_tags(
    state: Shared<'_>,
    id: u64,
    path: String,
    tags: Vec<(String, String)>,
) -> Result<(), String> {
    let fs = s3_for(&state, id).await?;
    with_s3!(fs, s3 => s3.set_tags(&path, &tags).await.map_err(err))?;
    audit_conn(
        &state,
        id,
        "s3_set_tags",
        true,
        format!("{} tag", tags.len()),
    );
    Ok(())
}

#[tauri::command]
async fn s3_set_storage_class(
    state: Shared<'_>,
    id: u64,
    path: String,
    class: String,
) -> Result<(), String> {
    let fs = s3_for(&state, id).await?;
    with_s3!(fs, s3 => s3.set_storage_class(&path, &class).await.map_err(err))?;
    audit_conn(&state, id, "s3_set_storage_class", true, class);
    Ok(())
}

/// Indirizzo temporaneo di download (la firma è nell'indirizzo: va trattato come un segreto di breve durata).
#[tauri::command]
async fn s3_presign(
    state: Shared<'_>,
    id: u64,
    path: String,
    expires_secs: u64,
) -> Result<String, String> {
    let fs = s3_for(&state, id).await?;
    let secs = expires_secs.clamp(60, 7 * 24 * 3600);
    let url = with_s3!(fs, s3 => s3
        .presigned_url(&path, std::time::Duration::from_secs(secs))
        .await
        .map_err(err))?;
    audit_conn(&state, id, "s3_presign", true, format!("{secs} s"));
    Ok(url)
}

// ---- Sessioni RDP (client esterno, D-08) ----

fn rdp_params(session: &Session) -> pxt_rdp::RdpParams {
    let o = &session.rdp;
    let blank = |v: &Option<String>| v.clone().filter(|s| !s.trim().is_empty());
    pxt_rdp::RdpParams {
        host: session.host.clone(),
        port: session.port,
        username: session.username.clone(),
        domain: blank(&o.domain),
        client: match o.client {
            pxt_core::RdpClient::Auto | pxt_core::RdpClient::Integrated => pxt_rdp::Client::Auto,
            pxt_core::RdpClient::Mstsc => pxt_rdp::Client::Mstsc,
            pxt_core::RdpClient::WindowsApp => pxt_rdp::Client::WindowsApp,
            pxt_core::RdpClient::Freerdp => pxt_rdp::Client::FreeRdp,
        },
        fullscreen: o.fullscreen,
        multimon: o.multimon,
        resolution: blank(&o.resolution),
        redirect_clipboard: o.redirect_clipboard,
        redirect_drives: o.redirect_drives,
        gateway: blank(&o.gateway),
    }
}

fn rdp_audit_ctx(session: &Session) -> AuditCtx {
    AuditCtx {
        host: session.host.clone(),
        port: session.port,
        user: session.username.clone(),
        via: Vec::new(),
        protocol: "rdp",
    }
}

/// Apre una sessione RDP con il client del sistema. La password (digitata ora o nel keyring) passa
/// solo dal Credential Manager di Windows e viene tolta poco dopo l'avvio.
#[tauri::command]
async fn rdp_open(
    state: Shared<'_>,
    session_id: Uuid,
    password: Option<String>,
) -> Result<(), String> {
    let nodes = state.sessions.list().await.map_err(err)?;
    let session = match nodes.iter().find(|n| n.id() == session_id) {
        Some(SessionNode::Session(s)) if s.protocol == Protocol::Rdp => (**s).clone(),
        Some(_) => return Err("la sessione non è una sessione RDP".into()),
        None => return Err("sessione non trovata".into()),
    };
    let ctx = rdp_audit_ctx(&session);
    if !state.policy.allows(Protocol::Rdp) {
        state.audit(
            &ctx,
            "connect",
            Outcome::Failed,
            Some("vietato dalla policy".into()),
        );
        return Err("Il protocollo RDP è vietato dalla policy dell'organizzazione.".into());
    }
    let secret = match password.filter(|p| !p.is_empty()) {
        Some(p) => Some(p),
        None => stored_secret(&state, &session.credential_ref).await?,
    };
    let params = rdp_params(&session);
    let launched = tokio::task::spawn_blocking(move || pxt_rdp::launch(&params, secret.as_deref()))
        .await
        .map_err(err)?;
    match launched {
        Ok(mut l) => {
            state.audit(
                &ctx,
                "connect",
                Outcome::Ok,
                Some("rdp: client di sistema".into()),
            );
            // Il client legge file e credenziali nei primi secondi: poi si toglie tutto.
            tokio::spawn(async move {
                tokio::time::sleep(pxt_rdp::CLEANUP_DELAY).await;
                l.finish();
            });
            Ok(())
        }
        Err(e) => {
            state.audit(&ctx, "connect", Outcome::Failed, Some(e.to_string()));
            Err(e.to_string())
        }
    }
}

/// Prova una sessione RDP: verifica solo che la porta risponda (l'autenticazione la fa il client).
async fn rdp_test(state: &Arc<AppState>, session: &Session) -> TestResult {
    if !state.policy.allows(Protocol::Rdp) {
        return TestResult::new(
            "error",
            "Il protocollo RDP è vietato dalla policy dell'organizzazione.",
        );
    }
    let addr = format!("{}:{}", session.host.trim(), session.port);
    let started = std::time::Instant::now();
    match tokio::time::timeout(
        std::time::Duration::from_secs(8),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    {
        Ok(Ok(_)) => {
            let mut r = TestResult::new(
                "ok",
                "Il server risponde sulla porta RDP. L'autenticazione la fa il client RDP quando apri la sessione.",
            );
            r.elapsed_ms = u64::try_from(started.elapsed().as_millis()).ok();
            r
        }
        Ok(Err(e)) => TestResult::new("error", format!("{addr}: {e}")),
        Err(_) => TestResult::new("error", format!("{addr}: nessuna risposta entro 8 secondi")),
    }
}

/// Una sessione RDP integrata: il lato comandi e la **destinazione dei fotogrammi**, che si può cambiare
/// (UNTAB: la scheda passa a un'altra finestra senza perdere la sessione).
struct RdpEntry {
    handle: pxt_rdp::RdpHandle,
    sink: Mutex<Channel<InvokeResponseBody>>,
    /// Dimensione attuale del desktop, per chi subentra.
    size: Mutex<(u16, u16)>,
}

// ---- Sessioni RDP integrate (D-32): il motore `picoxterm-rdp` in un processo a parte ----

/// Marcatori dei messaggi binari verso la scheda: aggiornamento di grafica o fine della sessione.
const RDP_MSG_UPDATE: u8 = 1;
const RDP_MSG_DISCONNECTED: u8 = 2;
const RDP_MSG_CLIPBOARD: u8 = 3;
const RDP_MSG_RESIZED: u8 = 4;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RdpConnectRequest {
    session_id: Uuid,
    /// Password digitata ora dall'utente (ha la precedenza su quella nel keyring).
    password: Option<String>,
    /// Impronta del certificato accettata ora dall'utente, anche senza salvarla nella sessione.
    pinned_cert: Option<String>,
    /// Dimensione dell'area della scheda, in pixel.
    width: u16,
    height: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RdpConnected {
    id: u64,
    width: u16,
    height: u16,
}

/// Collega una sessione RDP in una scheda. La grafica arriva su `on_frame` come byte
/// (`[1][x][y][w][h][rgba]` o `[2][motivo]`, interi a 16 bit little-endian), gli eventi di controllo
/// (certificato da confermare) su `on_event`. Un certificato non verificabile non si accetta mai in
/// silenzio: la UI riceve `certUntrusted` e riprova con `pinnedCert`.
#[tauri::command]
async fn rdp_connect(
    state: Shared<'_>,
    request: RdpConnectRequest,
    on_event: Channel<TermEvent>,
    on_frame: Channel<InvokeResponseBody>,
) -> Result<RdpConnected, String> {
    let nodes = state.sessions.list().await.map_err(err)?;
    let session = match nodes.iter().find(|n| n.id() == request.session_id) {
        Some(SessionNode::Session(s)) if s.protocol == Protocol::Rdp => (**s).clone(),
        Some(_) => return Err("la sessione non è una sessione RDP".into()),
        None => return Err("sessione non trovata".into()),
    };
    let ctx = rdp_audit_ctx(&session);
    if !state.policy.allows(Protocol::Rdp) {
        state.audit(
            &ctx,
            "connect",
            Outcome::Failed,
            Some("vietato dalla policy".into()),
        );
        return Err("Il protocollo RDP è vietato dalla policy dell'organizzazione.".into());
    }
    let password = match request.password.filter(|p| !p.is_empty()) {
        Some(p) => p,
        None => stored_secret(&state, &session.credential_ref)
            .await?
            .ok_or_else(|| PASSWORD_REQUIRED.to_string())?,
    };
    let blank = |v: &Option<String>| v.clone().filter(|s| !s.trim().is_empty());
    let params = pxt_rdp::SessionParams {
        host: session.host.clone(),
        port: session.port,
        username: session.username.clone(),
        password,
        domain: blank(&session.rdp.domain),
        width: request.width,
        height: request.height,
        pinned_cert: request
            .pinned_cert
            .or_else(|| blank(&session.rdp.pinned_cert)),
        keyboard_layout: pxt_rdp::system_keyboard_layout(),
        clipboard: session.rdp.redirect_clipboard,
    };
    match pxt_rdp::RdpSession::connect(params).await {
        Ok(s) => {
            state.audit(
                &ctx,
                "connect",
                Outcome::Ok,
                Some("rdp: client integrato".into()),
            );
            let id = state.next_id();
            let (width, height) = (s.width, s.height);
            let (mut events, handle) = s.split();
            state.conn_ctx.lock().unwrap().insert(id, ctx.clone());
            let entry = Arc::new(RdpEntry {
                handle,
                sink: Mutex::new(on_frame),
                size: Mutex::new((width, height)),
            });
            state.rdp.lock().unwrap().insert(id, Arc::clone(&entry));
            let pump_state = Arc::clone(&state);
            tokio::spawn(async move {
                while let Some(ev) = events.recv().await {
                    let (body, ended) = match ev {
                        pxt_rdp::RdpEvent::Update {
                            x,
                            y,
                            width,
                            height,
                            rgba,
                        } => {
                            let mut b = Vec::with_capacity(9 + rgba.len());
                            b.push(RDP_MSG_UPDATE);
                            for v in [x, y, width, height] {
                                b.extend_from_slice(&v.to_le_bytes());
                            }
                            b.extend_from_slice(&rgba);
                            (b, false)
                        }
                        pxt_rdp::RdpEvent::Disconnected(reason) => {
                            let mut b = vec![RDP_MSG_DISCONNECTED];
                            b.extend_from_slice(reason.as_bytes());
                            (b, true)
                        }
                        pxt_rdp::RdpEvent::Resized { width, height } => {
                            *entry.size.lock().unwrap() = (width, height);
                            let mut b = vec![RDP_MSG_RESIZED];
                            b.extend_from_slice(&width.to_le_bytes());
                            b.extend_from_slice(&height.to_le_bytes());
                            (b, false)
                        }
                        pxt_rdp::RdpEvent::ClipboardText(text) => {
                            let mut b = vec![RDP_MSG_CLIPBOARD];
                            b.extend_from_slice(text.as_bytes());
                            (b, false)
                        }
                        _ => continue,
                    };
                    // Se la finestra che disegnava è sparita (o sta cambiando) il fotogramma si perde, ma la
                    // sessione continua: la scheda può essere ripresa con `rdp_attach`.
                    let sink = entry.sink.lock().unwrap().clone();
                    let _ = sink.send(InvokeResponseBody::Raw(body));
                    if ended {
                        break;
                    }
                }
                // Fine della sessione: si libera il motore.
                pump_state.rdp.lock().unwrap().remove(&id);
                pump_state.conn_ctx.lock().unwrap().remove(&id);
                pump_state.audit(&ctx, "disconnect", Outcome::Ok, None);
            });
            Ok(RdpConnected { id, width, height })
        }
        Err(pxt_rdp::RdpError::UntrustedCertificate {
            fingerprint,
            changed,
        }) => {
            state.audit(
                &ctx,
                if changed {
                    "cert_changed"
                } else {
                    "cert_untrusted"
                },
                Outcome::Failed,
                Some(format!("SHA256 {fingerprint}")),
            );
            let _ = on_event.send(TermEvent::CertUntrusted {
                host: session.host,
                port: session.port,
                fingerprint,
                changed,
            });
            Err(CERT_UNTRUSTED.to_string())
        }
        Err(e) => {
            state.audit(&ctx, "connect", Outcome::Failed, Some(e.to_string()));
            Err(e.to_string())
        }
    }
}

/// Il motore RDP integrato è presente accanto all'app? (Se no, le sessioni RDP usano il client di sistema.)
#[tauri::command]
fn rdp_engine_available() -> bool {
    pxt_rdp::engine_path().is_ok()
}

/// Manda mouse, rotella o tasti alla sessione RDP.
#[tauri::command]
fn rdp_input(state: Shared<'_>, id: u64, input: pxt_rdp::RdpInput) -> Result<(), String> {
    let entry = state.rdp.lock().unwrap().get(&id).cloned();
    match entry {
        Some(e) if e.handle.send(input) => Ok(()),
        _ => Err("sessione RDP chiusa".into()),
    }
}

/// Rende incollabile nella sessione RDP il testo appena copiato sul computer (se gli appunti sono
/// condivisi per questa sessione).
#[tauri::command]
fn rdp_clipboard_set(state: Shared<'_>, id: u64, text: String) -> Result<(), String> {
    let entry = state.rdp.lock().unwrap().get(&id).cloned();
    match entry {
        Some(e) if e.handle.send_clipboard(text) => Ok(()),
        _ => Err("sessione RDP chiusa".into()),
    }
}

/// Chiede una nuova risoluzione del desktop remoto (si adatta all'area della scheda).
#[tauri::command]
fn rdp_resize(state: Shared<'_>, id: u64, width: u16, height: u16) -> Result<(), String> {
    let entry = state.rdp.lock().unwrap().get(&id).cloned();
    match entry {
        Some(e) if e.handle.resize(width, height) => Ok(()),
        _ => Err("sessione RDP chiusa".into()),
    }
}

/// Chiude la sessione RDP (e il motore).
#[tauri::command]
fn rdp_close(state: Shared<'_>, id: u64) {
    if let Some(e) = state.rdp.lock().unwrap().remove(&id) {
        e.handle.close();
    }
}

/// UNTAB: la sessione RDP `id` passa a un'altra finestra. I fotogrammi da ora vanno a `on_frame` e il
/// motore rimanda l'immagine intera del desktop, così la finestra nuova non parte da uno schermo vuoto.
#[tauri::command]
fn rdp_attach(
    state: Shared<'_>,
    id: u64,
    on_frame: Channel<InvokeResponseBody>,
) -> Result<RdpConnected, String> {
    let entry = state
        .rdp
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or("sessione RDP chiusa")?;
    *entry.sink.lock().unwrap() = on_frame;
    entry.handle.repaint();
    let (width, height) = *entry.size.lock().unwrap();
    Ok(RdpConnected { id, width, height })
}

// ---- Sessioni VNC (noVNC nell'interfaccia, connessione e autenticazione qui) ----

fn vnc_audit_ctx(session: &Session) -> AuditCtx {
    AuditCtx {
        host: session.host.clone(),
        port: session.port,
        user: String::new(),
        via: Vec::new(),
        protocol: "vnc",
    }
}

fn find_vnc_session(nodes: &[SessionNode], id: Uuid) -> Result<Session, String> {
    match nodes.iter().find(|n| n.id() == id) {
        Some(SessionNode::Session(s)) if s.protocol == Protocol::Vnc => Ok((**s).clone()),
        Some(_) => Err("la sessione non è una sessione VNC".into()),
        None => Err("sessione non trovata".into()),
    }
}

/// Traduce l'errore del crate nel codice letto dalla UI (`password-required`) o nel testo da mostrare.
fn vnc_error(e: pxt_vnc::VncError) -> String {
    match e {
        pxt_vnc::VncError::PasswordRequired => PASSWORD_REQUIRED.to_string(),
        other => other.to_string(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VncConnectRequest {
    session_id: Uuid,
    /// Password VNC digitata ora dall'utente (ha la precedenza su quella nel keyring).
    password: Option<String>,
}

/// Collega una sessione VNC. L'autenticazione la fa il backend (la password non arriva alla UI): a `on_data`
/// vanno i byte per noVNC (prima un finto handshake senza autenticazione, poi il flusso del server),
/// a `on_event` la fine della connessione.
#[tauri::command]
async fn vnc_connect(
    state: Shared<'_>,
    request: VncConnectRequest,
    on_data: Channel<InvokeResponseBody>,
    on_event: Channel<TermEvent>,
) -> Result<u64, String> {
    let nodes = state.sessions.list().await.map_err(err)?;
    let session = find_vnc_session(&nodes, request.session_id)?;
    let ctx = vnc_audit_ctx(&session);
    if !state.policy.allows(Protocol::Vnc) {
        state.audit(
            &ctx,
            "connect",
            Outcome::Failed,
            Some("vietato dalla policy".into()),
        );
        return Err("Il protocollo VNC è vietato dalla policy dell'organizzazione.".into());
    }
    let password = match request.password.filter(|p| !p.is_empty()) {
        Some(p) => Some(p),
        None => stored_secret(&state, &session.credential_ref).await?,
    };
    match pxt_vnc::VncSession::connect(&session.host, session.port, password.as_deref()).await {
        Ok(s) => {
            state.audit(
                &ctx,
                "connect",
                Outcome::Ok,
                Some("vnc: senza cifratura".into()),
            );
            let id = state.next_id();
            let pxt_vnc::VncSession { mut events, handle } = s;
            state.conn_ctx.lock().unwrap().insert(id, ctx.clone());
            state.vnc.lock().unwrap().insert(id, handle);
            let pump_state = Arc::clone(&state);
            tokio::spawn(async move {
                let mut reason = "chiusa".to_string();
                while let Some(ev) = events.recv().await {
                    match ev {
                        pxt_vnc::VncEvent::Data(d) => {
                            if on_data.send(InvokeResponseBody::Raw(d)).is_err() {
                                break;
                            }
                        }
                        pxt_vnc::VncEvent::Closed(r) => {
                            reason = r;
                            break;
                        }
                    }
                }
                let _ = on_event.send(TermEvent::VncClosed { reason });
                if let Some(h) = pump_state.vnc.lock().unwrap().remove(&id) {
                    h.close();
                }
                pump_state.conn_ctx.lock().unwrap().remove(&id);
                pump_state.audit(&ctx, "disconnect", Outcome::Ok, None);
            });
            Ok(id)
        }
        Err(e) => {
            state.audit(&ctx, "connect", Outcome::Failed, Some(e.to_string()));
            Err(vnc_error(e))
        }
    }
}

/// Manda al server i byte di noVNC. Il corpo della richiesta è il dato grezzo; l'id della connessione
/// sta nell'intestazione `x-conn-id`.
#[tauri::command]
async fn vnc_send(state: Shared<'_>, request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let id: u64 = request
        .headers()
        .get("x-conn-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or("manca l'id della connessione")?;
    let tauri::ipc::InvokeBody::Raw(data) = request.body() else {
        return Err("corpo non valido".into());
    };
    let handle = state.vnc.lock().unwrap().get(&id).cloned();
    match handle {
        Some(h) if h.send(data.clone()).await => Ok(()),
        _ => Err("sessione VNC chiusa".into()),
    }
}

/// Chiude la sessione VNC.
#[tauri::command]
fn vnc_close(state: Shared<'_>, id: u64) {
    if let Some(h) = state.vnc.lock().unwrap().remove(&id) {
        h.close();
    }
}

/// Prova una sessione VNC (anche non salvata): connessione e autenticazione, senza tenerla aperta.
async fn vnc_test(
    state: &Arc<AppState>,
    session: &Session,
    password: Option<String>,
) -> TestResult {
    if !state.policy.allows(Protocol::Vnc) {
        return TestResult::new(
            "error",
            "Il protocollo VNC è vietato dalla policy dell'organizzazione.",
        );
    }
    let secret = match password.filter(|p| !p.is_empty()) {
        Some(p) => Some(p),
        None => match stored_secret(state, &session.credential_ref).await {
            Ok(s) => s,
            Err(e) => return TestResult::new("error", e),
        },
    };
    let started = std::time::Instant::now();
    match pxt_vnc::VncSession::connect(&session.host, session.port, secret.as_deref()).await {
        Ok(s) => {
            s.handle.close();
            let mut r = TestResult::new(
                "ok",
                "Connessione e autenticazione riuscite. Attenzione: VNC non cifra il traffico.",
            );
            r.elapsed_ms = u64::try_from(started.elapsed().as_millis()).ok();
            r
        }
        Err(pxt_vnc::VncError::PasswordRequired) => TestResult::new(
            "needSecret",
            "Il server richiede una password VNC: inseriscila nel campo per provare la connessione.",
        ),
        Err(e) => TestResult::new("error", e.to_string()),
    }
}

// ---- UNTAB: schede in finestre proprie ----

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DetachRequest {
    /// Tipo di scheda (`sftp`, `ftp`, `s3`, `rdp`...): decide che vista mostrare nella finestra.
    kind: String,
    conn_id: u64,
    session_id: Option<Uuid>,
    title: String,
    /// Dati in più per la vista (per esempio la cartella corrente), come JSON.
    #[serde(default)]
    extra: serde_json::Value,
}

/// Apre una finestra per una scheda staccata. La connessione resta quella di prima: la finestra la
/// riprende dal numero. Se esiste già, la porta in primo piano. Restituisce l'etichetta.
#[tauri::command]
async fn window_detach(app: tauri::AppHandle, request: DetachRequest) -> Result<String, String> {
    let label = format!("detached-{}-{}", request.kind, request.conn_id);
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.set_focus();
        return Ok(label);
    }
    let init = serde_json::json!({
        "kind": request.kind,
        "connId": request.conn_id,
        "sessionId": request.session_id,
        "title": request.title,
        "extra": request.extra,
        "label": label,
    });
    // I dati arrivano alla pagina con uno script iniziale (più robusto di una query nell'indirizzo).
    tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App("index.html".into()))
        .title(&request.title)
        .inner_size(1100.0, 720.0)
        .min_inner_size(480.0, 320.0)
        .initialization_script(format!("window.__DETACHED__ = {init};"))
        .build()
        .map_err(err)?;
    Ok(label)
}

/// Porta in primo piano la finestra di una scheda staccata.
#[tauri::command]
fn window_focus(app: tauri::AppHandle, label: String) {
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Chiude la finestra di una scheda staccata e avvisa la finestra principale con l'evento
/// `detached-finished` (`action`: `redock` = riprendi la scheda, la connessione non si tocca; `ended` = la
/// sessione è stata chiusa, togli la scheda; `none` = niente, chiude solo la finestra).
#[tauri::command]
fn window_finish(app: tauri::AppHandle, label: String, action: String, info: serde_json::Value) {
    use tauri::Emitter as _;
    if action != "none" {
        let _ = app.emit_to(
            "main",
            "detached-finished",
            serde_json::json!({ "action": action, "info": info }),
        );
    }
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.destroy();
    }
}

/// Policy attiva e l'eventuale avviso (file non valido): la UI vi si adegua (per esempio nasconde l'FTP in chiaro).
#[tauri::command]
fn policy_get(state: Shared<'_>) -> serde_json::Value {
    serde_json::json!({
        "policy": state.policy,
        "warning": state.policy_warning,
    })
}

/// Sessione SFTP della connessione: la apre al primo uso (nessuna seconda autenticazione).
async fn sftp_for(state: &AppState, id: u64) -> Result<Arc<AnyFs>, String> {
    let mut open = state.sftp.lock().await;
    if let Some(fs) = open.get(&id) {
        return Ok(Arc::clone(fs));
    }
    let shell = state.shells.lock().unwrap().get(&id).cloned();
    let stream = match shell {
        Some(shell) => shell.open_sftp_stream().await.map_err(err)?,
        None => {
            let conn = state.sftp_conns.lock().unwrap().get(&id).cloned();
            conn.ok_or_else(|| "sessione inesistente".to_string())?
                .open_sftp_stream()
                .await
                .map_err(err)?
        }
    };
    let fs = Arc::new(AnyFs::Sftp(SftpFs::connect(stream).await.map_err(err)?));
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

/// Nomi di utenti e gruppi per gli id numerici, letti da `/etc/passwd` e `/etc/group` del server.
#[tauri::command]
async fn sftp_id_names(state: Shared<'_>, id: u64) -> Result<pxt_fs::names::IdNames, String> {
    let fs = sftp_for(&state, id).await?;
    // Via FTP non c'è `/etc/passwd` da leggere in modo affidabile: i nomi arrivano già dall'elenco.
    if matches!(*fs, AnyFs::Ftp(_)) {
        return Ok(pxt_fs::names::IdNames::default());
    }
    Ok(pxt_fs::names::load(&*fs).await)
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

/// Elimina file e cartelle (con tutto il contenuto) su un filesystem qualsiasi. I link simbolici non
/// vengono mai seguiti; gli errori sui singoli elementi finiscono nel resoconto.
async fn delete_paths<F: RemoteFs>(fs: &F, paths: &[String]) -> fs_ops::OpReport {
    let cancel = AtomicBool::new(false);
    let mut total = fs_ops::OpReport::default();
    for p in paths {
        match fs_ops::remove_recursive(fs, p, &cancel).await {
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
    total
}

/// Elimina file e cartelle (con tutto il contenuto). I link simbolici non vengono mai seguiti.
#[tauri::command]
async fn sftp_delete(
    state: Shared<'_>,
    id: u64,
    paths: Vec<String>,
) -> Result<fs_ops::OpReport, String> {
    let fs = sftp_for(&state, id).await?;
    let total = delete_paths(&*fs, &paths).await;
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

/// Apre un file di testo remoto per l'editor integrato (D-19): il contenuto resta in memoria.
#[tauri::command]
async fn sftp_read_text(
    state: Shared<'_>,
    id: u64,
    path: String,
) -> Result<fs_edit::TextFile, String> {
    let fs = sftp_for(&state, id).await?;
    let r = fs_edit::read_text(&*fs, &path, fs_edit::MAX_EDIT_BYTES).await;
    audit_conn(
        &state,
        id,
        "sftp_edit_open",
        r.is_ok(),
        "1 file".to_string(),
    );
    r.map_err(err)
}

/// Salva il testo dell'editor. Se `base` è presente e il file è cambiato sul server non scrive e
/// risponde `conflict`; con `base` assente (sovrascrittura voluta) scrive comunque.
#[tauri::command]
async fn sftp_write_text(
    state: Shared<'_>,
    id: u64,
    path: String,
    content: String,
    encoding: fs_edit::Encoding,
    eol: fs_edit::Eol,
    base: Option<fs_edit::Stamp>,
) -> Result<fs_edit::SaveOutcome, String> {
    let fs = sftp_for(&state, id).await?;
    let r = fs_edit::write_text(
        &*fs,
        &path,
        &content,
        encoding,
        eol,
        base,
        fs_edit::MAX_EDIT_BYTES,
    )
    .await;
    let saved = matches!(r, Ok(fs_edit::SaveOutcome::Saved { .. }));
    audit_conn(
        &state,
        id,
        "sftp_edit_save",
        saved,
        if saved {
            "1 file".to_string()
        } else {
            "0 file".to_string()
        },
    );
    r.map_err(err)
}

// ---- Modifica con l'editor di sistema (7.6) ----

/// Un file remoto scaricato in una cartella temporanea e aperto con il programma del sistema.
struct ExternalEdit {
    conn: u64,
    remote_path: String,
    local_path: PathBuf,
    /// Cartella temporanea che contiene solo questo file: si elimina alla chiusura.
    dir: PathBuf,
    /// Stato del file remoto all'ultimo download o caricamento, per le modifiche concorrenti.
    base: Mutex<fs_edit::Stamp>,
    stop: AtomicBool,
    on_event: Channel<ExternalEvent>,
}

/// Avvisi verso la UI per un file aperto con l'editor di sistema.
#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ExternalEvent {
    /// Il file locale è stato salvato dal programma: la UI decide se ricaricarlo sul server.
    #[serde(rename_all = "camelCase")]
    Changed { edit_id: u64 },
    /// La connessione è stata chiusa: il file temporaneo è già stato eliminato.
    #[serde(rename_all = "camelCase")]
    Closed { edit_id: u64 },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalOpened {
    edit_id: u64,
    name: String,
    /// Il file era già aperto: si è solo riaperto il programma.
    reused: bool,
}

/// Ogni quanto si controlla se il programma ha salvato il file.
const EXTERNAL_POLL: std::time::Duration = std::time::Duration::from_secs(1);

/// Radice delle cartelle temporanee: una per processo, così due copie dell'app non si disturbano.
fn external_root() -> PathBuf {
    std::env::temp_dir()
        .join("PiCoXterm-edit")
        .join(std::process::id().to_string())
}

/// Elimina le cartelle temporanee lasciate da esecuzioni chiuse male (più vecchie di un giorno).
fn external_cleanup_stale() {
    let parent = std::env::temp_dir().join("PiCoXterm-edit");
    let Ok(entries) = std::fs::read_dir(&parent) else {
        return;
    };
    let day = std::time::Duration::from_secs(24 * 3600);
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > day);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Estensioni che il sistema potrebbe **eseguire** invece di aprire in un editor: script e
/// programmi. Per questi si usa l'editor integrato.
const RUNNABLE_EXTENSIONS: &[&str] = &[
    "exe",
    "com",
    "bat",
    "cmd",
    "scr",
    "pif",
    "msi",
    "msp",
    "ps1",
    "psm1",
    "psd1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "hta",
    "lnk",
    "url",
    "reg",
    "jar",
    "cpl",
    "msc",
    "gadget",
    "application",
    "appref-ms",
    "scf",
    "inf",
    "sct",
    "chm",
    "sh",
    "bash",
    "zsh",
    "ksh",
    "csh",
    "command",
    "app",
    "desktop",
    "py",
    "pyw",
    "pl",
    "rb",
    "php",
    "run",
    "bin",
];

fn is_runnable(name: &str) -> bool {
    name.rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .is_some_and(|ext| RUNNABLE_EXTENSIONS.contains(&ext.as_str()))
}

/// Apre un file locale con il programma associato dal sistema.
fn open_with_system(path: &std::path::Path) -> Result<(), String> {
    opener::open(path)
        .map_err(|e| format!("impossibile aprire il file con il programma di sistema: {e}"))
}

/// Segue un file locale finché `stop` è falso e chiama `on_change` quando data o dimensione cambiano e
/// restano uguali per un intero intervallo (il programma ha finito di scrivere: i salvataggi fatti in
/// più passi, come "scrivi un temporaneo e rinomina", producono un solo avviso).
async fn watch_file(
    path: &std::path::Path,
    stop: &AtomicBool,
    poll: std::time::Duration,
    mut on_change: impl FnMut(),
) {
    let stamp = |p: &std::path::Path| {
        std::fs::metadata(p)
            .ok()
            .map(|m| (m.len(), m.modified().ok()))
    };
    let mut known = stamp(path);
    let mut candidate = None;
    while !stop.load(Ordering::Relaxed) {
        tokio::time::sleep(poll).await;
        let now = stamp(path);
        // File sparito per un attimo (rinomina in corso) o invariato: niente da segnalare.
        if now.is_none() || now == known {
            candidate = None;
            continue;
        }
        if candidate == now {
            known = now;
            candidate = None;
            on_change();
        } else {
            candidate = now;
        }
    }
}

/// Scarica un file remoto in una cartella temporanea e lo apre con il programma del sistema. Da
/// quel momento si controlla se viene salvato e la UI riceve `Changed`.
#[tauri::command]
async fn external_edit_open(
    state: Shared<'_>,
    id: u64,
    path: String,
    on_event: Channel<ExternalEvent>,
) -> Result<ExternalOpened, String> {
    let name = path
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("file")
        .to_string();
    if is_runnable(&name) {
        return Err(
            "Questo tipo di file potrebbe essere eseguito dal sistema invece di aprirsi in un editor: usa l'editor integrato (Modifica)."
                .into(),
        );
    }
    // Già aperto: si riapre solo il programma.
    let existing = state
        .externals
        .lock()
        .unwrap()
        .iter()
        .find(|(_, e)| e.conn == id && e.remote_path == path)
        .map(|(k, e)| (*k, Arc::clone(e)));
    if let Some((edit_id, e)) = existing {
        open_with_system(&e.local_path)?;
        return Ok(ExternalOpened {
            edit_id,
            name,
            reused: true,
        });
    }

    let fs = sftp_for(&state, id).await?;
    let edit_id = state.next_id();
    let dir = external_root().join(format!("{edit_id}-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).map_err(err)?;
    let fetched = match fs_external::fetch(&*fs, &path, &dir, fs_external::MAX_EXTERNAL_BYTES).await
    {
        Ok(f) => f,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            audit_conn(&state, id, "sftp_external_open", false, "0 file".into());
            return Err(err(e));
        }
    };
    if let Err(e) = open_with_system(&fetched.local_path) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    audit_conn(&state, id, "sftp_external_open", true, "1 file".into());

    let edit = Arc::new(ExternalEdit {
        conn: id,
        remote_path: path,
        local_path: fetched.local_path,
        dir,
        base: Mutex::new(fetched.stamp),
        stop: AtomicBool::new(false),
        on_event,
    });
    state
        .externals
        .lock()
        .unwrap()
        .insert(edit_id, Arc::clone(&edit));

    // Controllo periodico: quando il programma salva il file si avvisa la UI.
    tauri::async_runtime::spawn(async move {
        watch_file(&edit.local_path, &edit.stop, EXTERNAL_POLL, || {
            let _ = edit.on_event.send(ExternalEvent::Changed { edit_id });
        })
        .await;
    });
    Ok(ExternalOpened {
        edit_id,
        name,
        reused: false,
    })
}

/// Ricarica sul server il file modificato. Se sul server è cambiato dopo il download risponde
/// `conflict` senza scrivere; con `force` sovrascrive comunque.
#[tauri::command]
async fn external_edit_push(
    state: Shared<'_>,
    edit_id: u64,
    force: bool,
) -> Result<fs_edit::SaveOutcome, String> {
    let edit = state
        .externals
        .lock()
        .unwrap()
        .get(&edit_id)
        .cloned()
        .ok_or("file non più aperto con l'editor di sistema")?;
    let fs = sftp_for(&state, edit.conn).await?;
    let base = (!force).then(|| *edit.base.lock().unwrap());
    let r = fs_external::push(
        &*fs,
        &edit.local_path,
        &edit.remote_path,
        base,
        fs_external::MAX_EXTERNAL_BYTES,
    )
    .await;
    let saved = matches!(r, Ok(fs_edit::SaveOutcome::Saved { .. }));
    if let Ok(fs_edit::SaveOutcome::Saved { size, modified }) = &r {
        *edit.base.lock().unwrap() = fs_edit::Stamp {
            size: *size,
            modified: *modified,
        };
    }
    audit_conn(
        &state,
        edit.conn,
        "sftp_external_push",
        saved,
        if saved { "1 file" } else { "0 file" }.to_string(),
    );
    r.map_err(err)
}

/// Smette di seguire il file ed elimina la copia temporanea.
#[tauri::command]
fn external_edit_stop(state: Shared<'_>, edit_id: u64) {
    if let Some(edit) = state.externals.lock().unwrap().remove(&edit_id) {
        edit.stop.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_dir_all(&edit.dir);
    }
}

/// Alla chiusura di una connessione i suoi file aperti con l'editor di sistema vengono eliminati.
fn external_edit_close_conn(state: &AppState, conn: u64) {
    let doomed: Vec<(u64, Arc<ExternalEdit>)> = {
        let mut map = state.externals.lock().unwrap();
        let ids: Vec<u64> = map
            .iter()
            .filter(|(_, e)| e.conn == conn)
            .map(|(k, _)| *k)
            .collect();
        ids.into_iter()
            .filter_map(|k| map.remove(&k).map(|e| (k, e)))
            .collect()
    };
    for (edit_id, edit) in doomed {
        edit.stop.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_dir_all(&edit.dir);
        let _ = edit.on_event.send(ExternalEvent::Closed { edit_id });
    }
}

// ---- Pannello locale del doppio pannello ----
//
// Stesse operazioni dei comandi `sftp_*`, ma sul disco del computer (backend `LocalFs`). Non hanno un
// id di connessione e non finiscono nel log di audit, che riguarda i server.

/// Cartella di partenza del pannello locale (Documenti, altrimenti la home).
#[tauri::command]
async fn local_home() -> Result<String, String> {
    pxt_fs::LocalFs.canonicalize(".").await.map_err(err)
}

/// Contenuto di una cartella locale; il percorso vuoto è l'elenco dei dischi (solo Windows).
#[tauri::command]
async fn local_list(path: String) -> Result<Vec<Entry>, String> {
    pxt_fs::LocalFs.list(&path).await.map_err(err)
}

#[tauri::command]
async fn local_delete(paths: Vec<String>) -> Result<fs_ops::OpReport, String> {
    Ok(delete_paths(&pxt_fs::LocalFs, &paths).await)
}

#[tauri::command]
async fn local_tree_info(path: String) -> Result<fs_ops::TreeInfo, String> {
    fs_ops::tree_info(&pxt_fs::LocalFs, &path, &AtomicBool::new(false))
        .await
        .map_err(err)
}

#[tauri::command]
async fn local_chmod(path: String, mode: u32, recursive: bool) -> Result<fs_ops::OpReport, String> {
    if mode > 0o7777 {
        return Err("permessi non validi".into());
    }
    fs_ops::chmod(
        &pxt_fs::LocalFs,
        &path,
        mode,
        recursive,
        &AtomicBool::new(false),
    )
    .await
    .map_err(err)
}

/// Rinomina una voce locale restando nella stessa cartella. Non sovrascrive.
#[tauri::command]
async fn local_rename(path: String, new_name: String) -> Result<String, String> {
    let name = valid_name(&new_name)?;
    let from = std::path::Path::new(&path);
    let parent = from.parent().ok_or("percorso non valido")?;
    let target = parent.join(name).to_string_lossy().into_owned();
    if target == path {
        return Ok(target);
    }
    if pxt_fs::LocalFs.lstat(&target).await.is_ok() {
        return Err(format!("esiste già una voce chiamata \"{name}\""));
    }
    pxt_fs::LocalFs.rename(&path, &target).await.map_err(err)?;
    Ok(target)
}

#[tauri::command]
async fn local_mkdir(parent: String, name: String) -> Result<String, String> {
    let name = valid_name(&name)?;
    let target = std::path::Path::new(&parent)
        .join(name)
        .to_string_lossy()
        .into_owned();
    if pxt_fs::LocalFs.lstat(&target).await.is_ok() {
        return Err(format!("esiste già una voce chiamata \"{name}\""));
    }
    pxt_fs::LocalFs.mkdir(&target).await.map_err(err)?;
    Ok(target)
}

/// Apre un file locale con il programma associato dal sistema. Gli script e i programmi non si aprono
/// così: il sistema li eseguirebbe.
#[tauri::command]
async fn local_open(path: String) -> Result<(), String> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(&path);
    if is_runnable(name) {
        return Err(
            "Questo tipo di file potrebbe essere eseguito invece di aprirsi in un editor: aprilo a mano."
                .into(),
        );
    }
    let entry = pxt_fs::LocalFs.stat(&path).await.map_err(err)?;
    if entry.kind != pxt_fs::EntryKind::File {
        return Err("non è un file".into());
    }
    open_with_system(std::path::Path::new(&path))
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
    /// Concluso. `resumable`: ci sono file falliti che si possono riprovare.
    #[serde(rename_all = "camelCase")]
    Finished {
        job_id: u64,
        summary: fs_transfer::Summary,
        resumable: bool,
    },
    /// In pausa: i file parziali restano sul disco o sul server e si può riprendere.
    #[serde(rename_all = "camelCase")]
    Paused { job_id: u64 },
    #[serde(rename_all = "camelCase")]
    Cancelled { job_id: u64 },
    #[serde(rename_all = "camelCase")]
    Failed { job_id: u64, message: String },
}

/// File copiati contemporaneamente in ogni lavoro.
const TRANSFER_CONCURRENCY: usize = fs_transfer::DEFAULT_CONCURRENCY;

/// Un lavoro che si può riprendere: in pausa, oppure concluso con errori su alcuni file.
struct JobRecord {
    conn: u64,
    plan: Arc<fs_transfer::Plan>,
    policy: ConflictPolicy,
    state: Arc<fs_transfer::JobState>,
}

/// Avvia (o riavvia) l'esecuzione di un lavoro e restituisce il suo id.
fn launch(
    state: &Arc<AppState>,
    fs: Arc<AnyFs>,
    record: JobRecord,
    on_event: Channel<TransferEvent>,
) -> u64 {
    let job_id = state.next_id();
    let control = Arc::new(fs_transfer::Control::new());
    state
        .jobs
        .lock()
        .unwrap()
        .insert(job_id, Arc::clone(&control));

    let state = Arc::clone(state);
    tauri::async_runtime::spawn(async move {
        // Velocità media mobile sugli ultimi aggiornamenti. Il primo rilevamento fissa solo il
        // punto di partenza: in una ripresa i byte già fatti non contano come velocità.
        let mut last: Option<(std::time::Instant, u64)> = None;
        let mut speed = 0.0_f64;
        let mut report = |p: &Progress| {
            let now = std::time::Instant::now();
            match last {
                None => last = Some((now, p.bytes_done)),
                Some((t, b)) => {
                    let dt = now.duration_since(t).as_secs_f64();
                    if dt >= 0.2 {
                        let inst = (p.bytes_done.saturating_sub(b)) as f64 / dt;
                        speed = if speed == 0.0 {
                            inst
                        } else {
                            0.7 * speed + 0.3 * inst
                        };
                        last = Some((now, p.bytes_done));
                    }
                }
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
        let result = fs_transfer::run(
            &*fs,
            &record.plan,
            record.policy,
            &control,
            &record.state,
            TRANSFER_CONCURRENCY,
            &mut report,
        )
        .await;

        let direction = record.plan.direction;
        let event_name = match direction {
            fs_transfer::Direction::Upload => "transfer_upload",
            fs_transfer::Direction::Download => "transfer_download",
        };
        let conn = record.conn;
        let ctx = state.conn_ctx.lock().unwrap().get(&conn).cloned();
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
                if !ok {
                    state.resumable.lock().unwrap().insert(job_id, record);
                }
                (
                    TransferEvent::Finished {
                        job_id,
                        summary,
                        resumable: !ok,
                    },
                    if ok { Outcome::Ok } else { Outcome::Failed },
                    detail,
                )
            }
            Err(fs_transfer::TransferError::Paused) => {
                state.resumable.lock().unwrap().insert(job_id, record);
                (
                    TransferEvent::Paused { job_id },
                    Outcome::Ok,
                    "in pausa".to_string(),
                )
            }
            Err(fs_transfer::TransferError::Cancelled) => {
                // Un annullamento non lascia nulla: anche i parziali di una pausa precedente.
                fs_transfer::discard_partials(&*fs, direction, &record.state).await;
                (
                    TransferEvent::Cancelled { job_id },
                    Outcome::Failed,
                    "annullato".to_string(),
                )
            }
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
    job_id
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
    let job_state = Arc::new(fs_transfer::JobState::new(&stored.plan));
    let record = JobRecord {
        conn: stored.conn,
        plan: Arc::new(stored.plan),
        policy,
        state: job_state,
    };
    Ok(launch(&state, fs, record, on_event))
}

/// Riprende un lavoro in pausa o riprova i file falliti di uno concluso con errori: i file già
/// finiti si saltano e i parziali validi continuano dal punto raggiunto. Restituisce il nuovo id.
#[tauri::command]
async fn transfer_resume(
    state: Shared<'_>,
    job_id: u64,
    on_event: Channel<TransferEvent>,
) -> Result<u64, String> {
    let record = state
        .resumable
        .lock()
        .unwrap()
        .remove(&job_id)
        .ok_or("lavoro non riprendibile (già ripreso o scartato)")?;
    match sftp_for(&state, record.conn).await {
        Ok(fs) => Ok(launch(&state, fs, record, on_event)),
        Err(e) => {
            // La connessione non c'è più: il lavoro resta riprendibile finché non si scarta.
            state.resumable.lock().unwrap().insert(job_id, record);
            Err(e)
        }
    }
}

/// Mette in pausa un trasferimento: si ferma entro pochi istanti e tiene i file parziali.
#[tauri::command]
fn transfer_pause(state: Shared<'_>, job_id: u64) {
    if let Some(control) = state.jobs.lock().unwrap().get(&job_id) {
        control.pause();
    }
}

/// Annulla un trasferimento in corso: si ferma entro pochi istanti e non lascia file parziali.
#[tauri::command]
fn transfer_cancel(state: Shared<'_>, job_id: u64) {
    if let Some(control) = state.jobs.lock().unwrap().get(&job_id) {
        control.cancel();
    }
}

/// Abbandona un lavoro riprendibile ed elimina i file parziali che ha lasciato. Restituisce quanti.
#[tauri::command]
async fn transfer_discard(state: Shared<'_>, job_id: u64) -> Result<usize, String> {
    let Some(record) = state.resumable.lock().unwrap().remove(&job_id) else {
        return Ok(0);
    };
    let paths = record.state.take_partials();
    let mut removed = 0;
    match record.plan.direction {
        fs_transfer::Direction::Download => {
            for p in paths {
                removed += usize::from(std::fs::remove_file(p).is_ok());
            }
        }
        fs_transfer::Direction::Upload => {
            // Sul server si può togliere solo se la connessione è ancora aperta.
            if let Ok(fs) = sftp_for(&state, record.conn).await {
                for p in paths {
                    removed += usize::from(fs.remove_file(&p).await.is_ok());
                }
            }
        }
    }
    Ok(removed)
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
    external_edit_close_conn(&state, id);
    state.sftp.lock().await.remove(&id);
    state.conn_ctx.lock().unwrap().remove(&id);
    let shell = state.shells.lock().unwrap().remove(&id);
    if let Some(s) = shell {
        s.close().await;
    }
    let conn = state.sftp_conns.lock().unwrap().remove(&id);
    if let Some(c) = conn {
        c.close().await;
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
        protocol: "ssh",
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
        .on_window_event(|window, event| {
            use tauri::{Emitter as _, Manager as _, WindowEvent};
            if let WindowEvent::CloseRequested { api, .. } = event {
                let label = window.label();
                if label.starts_with("detached-") {
                    // La scheda staccata non si chiude da sola: la finestra chiede se riagganciare o chiudere.
                    api.prevent_close();
                    let _ = window.emit("detached-close-requested", ());
                } else if label == "main" {
                    // Chiudendo la finestra principale finisce l'app, con le schede staccate.
                    window.app_handle().exit(0);
                }
            }
        })
        .setup(|app| {
            let dir = resolve_data_dir(app)?;
            external_cleanup_stale();
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
            s3_connect,
            rdp_open,
            vnc_connect,
            vnc_send,
            vnc_close,
            rdp_connect,
            rdp_engine_available,
            rdp_input,
            rdp_clipboard_set,
            rdp_resize,
            rdp_close,
            rdp_attach,
            window_detach,
            window_focus,
            window_finish,
            s3_object_info,
            s3_set_headers,
            s3_set_tags,
            s3_set_storage_class,
            s3_presign,
            session_duplicate,
            ssh_connect,
            sftp_connect,
            ftp_connect,
            policy_get,
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
            sftp_read_text,
            sftp_id_names,
            local_home,
            local_list,
            local_delete,
            local_tree_info,
            local_chmod,
            local_rename,
            local_mkdir,
            local_open,
            external_edit_open,
            external_edit_push,
            external_edit_stop,
            sftp_write_text,
            transfer_plan_upload,
            transfer_plan_download,
            transfer_plan_discard,
            transfer_start,
            transfer_resume,
            transfer_pause,
            transfer_discard,
            transfer_cancel,
            ssh_inject_cwd,
            ssh_enable_cwd_tracking,
            hostkey_reply,
            known_hosts_list,
            known_hosts_remove
        ])
        .build(tauri::generate_context!())
        .expect("errore nell'avvio di PiCoXterm")
        .run(|_app, event| {
            // All'uscita le copie temporanee dei file aperti con l'editor di sistema spariscono.
            if let tauri::RunEvent::Exit = event {
                let _ = std::fs::remove_dir_all(external_root());
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_reports_a_saved_file_once_and_only_when_it_settles() {
        use std::sync::atomic::AtomicUsize;
        let dir = std::env::temp_dir().join(format!("pxt-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "uno").unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let count = Arc::new(AtomicUsize::new(0));
        let (stop2, count2, file2) = (Arc::clone(&stop), Arc::clone(&count), file.clone());
        let task = std::thread::spawn(move || {
            tauri::async_runtime::block_on(async {
                watch_file(&file2, &stop2, std::time::Duration::from_millis(40), || {
                    count2.fetch_add(1, Ordering::SeqCst);
                })
                .await;
            });
        });
        let wait = |ms| std::thread::sleep(std::time::Duration::from_millis(ms));
        wait(200);
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "nessuna modifica, nessun avviso"
        );
        // Salvataggio in più passi (temporaneo + rinomina): un solo avviso.
        std::fs::write(dir.join("a.tmp"), "due più lungo").unwrap();
        std::fs::remove_file(&file).unwrap();
        std::fs::rename(dir.join("a.tmp"), &file).unwrap();
        wait(400);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        // Secondo salvataggio: secondo avviso.
        std::fs::write(&file, "tre ancora più lungo").unwrap();
        wait(400);
        assert_eq!(count.load(Ordering::SeqCst), 2);
        wait(300);
        assert_eq!(count.load(Ordering::SeqCst), 2, "nessun avviso doppio");
        stop.store(true, Ordering::Relaxed);
        task.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn temp_state(policy_json: Option<&str>) -> AppState {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pxt-app-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(j) = policy_json {
            std::fs::write(dir.join("policy.json"), j).unwrap();
        }
        AppState::new(dir)
    }

    fn ftp_session(security: pxt_core::FtpSecurity) -> Session {
        let mut s = Session::new_ssh("f", "ftp.example", 21, "test");
        s.protocol = Protocol::Ftp;
        s.ftp.security = security;
        s
    }

    #[test]
    fn plain_ftp_is_refused_when_the_policy_forbids_it() {
        let open = temp_state(None);
        let strict = temp_state(Some(r#"{"allowPlainFtp": false}"#));
        let plain = ftp_session(pxt_core::FtpSecurity::Plain);
        let tls = ftp_session(pxt_core::FtpSecurity::Explicit);
        assert!(ftp_policy_check(&open, &plain).is_ok());
        assert!(ftp_policy_check(&strict, &plain)
            .unwrap_err()
            .contains("policy"));
        // FTPS resta consentito.
        assert!(ftp_policy_check(&strict, &tls).is_ok());
        assert!(ftp_policy_check(&strict, &ftp_session(pxt_core::FtpSecurity::Implicit)).is_ok());
        // Un elenco di protocolli che non include FTP lo vieta del tutto.
        let no_ftp = temp_state(Some(r#"{"allowedProtocols": ["ssh", "sftp"]}"#));
        assert!(ftp_policy_check(&no_ftp, &tls).is_err());
        // Un file danneggiato non allarga i permessi.
        let broken = temp_state(Some("{ rotto"));
        assert!(broken.policy_warning.is_some());
        assert!(ftp_policy_check(&broken, &plain).is_err());
    }

    #[test]
    fn ftp_needs_a_password_except_for_anonymous_access() {
        let st = temp_state(None);
        let mut s = ftp_session(pxt_core::FtpSecurity::Explicit);
        let params = |pw: Option<&str>, s: &Session| {
            tauri::async_runtime::block_on(ftp_params(&st, s, pw.map(String::from), None))
        };
        assert_eq!(params(None, &s).err().as_deref(), Some(PASSWORD_REQUIRED));
        assert_eq!(params(Some("pw"), &s).unwrap().password, "pw");
        s.username = "Anonymous".into();
        assert_eq!(params(None, &s).unwrap().password, "anonymous@");
        // L'impronta accettata ora ha la precedenza su quella salvata.
        s.ftp.pinned_cert = Some("AA".into());
        let p =
            tauri::async_runtime::block_on(ftp_params(&st, &s, None, Some("BB".into()))).unwrap();
        assert_eq!(p.pinned_cert.as_deref(), Some("BB"));
    }

    #[test]
    fn runnable_files_are_not_opened_with_the_system_program() {
        for name in [
            "deploy.sh",
            "x.BAT",
            "a.b.exe",
            "tool.PY",
            "run.ps1",
            "setup.cmd",
        ] {
            assert!(is_runnable(name), "{name}");
        }
        for name in [
            "notes.txt",
            "report.pdf",
            "photo.jpg",
            "Makefile",
            "archive.tar.gz",
            ".bashrc",
            "data.xlsx",
        ] {
            assert!(!is_runnable(name), "{name}");
        }
    }

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
