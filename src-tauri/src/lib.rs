use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use pxt_core::{
    subtree_ids, transfer, AuditEvent, AuditLog, AuthMethod, CredentialRef, KeyringSecretProvider,
    LocalSessionProvider, LocalSettings, Outcome, Protocol, Secret, SecretProvider, Session,
    SessionNode, SessionProvider, Settings,
};
use pxt_ssh::{
    ConnectParams, Credentials, Hop, HostKeyInfo, HostKeyVerifier, KnownHosts, Shell, ShellEvent,
    SshError,
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
    pending_hostkeys: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
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

fn is_hostkey_changed(e: &SshError) -> bool {
    match e {
        SshError::HostKeyChanged { .. } => true,
        SshError::Jump { source, .. } | SshError::Target { source, .. } => {
            is_hostkey_changed(source)
        }
        _ => false,
    }
}

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
            },
            Vec::new(),
        ),
    };
    if jump_ids.len() > MAX_JUMPS {
        return Err(format!("troppi jump host (massimo {MAX_JUMPS})"));
    }

    // Jump host: le credenziali devono essere già salvate (o agent / chiave senza passphrase).
    let mut hops = Vec::new();
    for jid in &jump_ids {
        let Some(SessionNode::Session(j)) = nodes.iter().find(|n| n.id() == *jid) else {
            return Err("jump host non trovato (sessione eliminata?)".into());
        };
        let ep = Endpoint::of(j);
        let secret = stored_secret(&state, &ep.credential_ref).await?;
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
                    return false;
                }
                let accepted = rx.await.unwrap_or(false);
                let event = if accepted {
                    "hostkey_accepted"
                } else {
                    "hostkey_rejected"
                };
                let outcome = if accepted {
                    Outcome::Ok
                } else {
                    Outcome::Failed
                };
                state.audit(&ctx, event, outcome, Some(key_detail));
                accepted
            })
        })
    };

    let (ev_tx, mut ev_rx) = mpsc::channel::<ShellEvent>(64);
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
            if is_hostkey_changed(&e) {
                state.audit(
                    &ctx,
                    "hostkey_changed",
                    Outcome::Failed,
                    Some(e.to_string()),
                );
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
    state.shells.lock().unwrap().insert(id, shell);
    Ok(id)
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
    let shell = state.shells.lock().unwrap().remove(&id);
    if let Some(s) = shell {
        s.close().await;
    }
    Ok(())
}

#[tauri::command]
fn hostkey_reply(state: Shared<'_>, req_id: u64, accept: bool) {
    if let Some(tx) = state.pending_hostkeys.lock().unwrap().remove(&req_id) {
        let _ = tx.send(accept);
    }
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
            app_paths,
            settings_get,
            settings_set,
            sessions_list,
            session_save,
            session_delete,
            sessions_export,
            sessions_import,
            ssh_connect,
            ssh_write,
            ssh_resize,
            ssh_close,
            ssh_inject_cwd,
            hostkey_reply
        ])
        .run(tauri::generate_context!())
        .expect("errore nell'avvio di PiCoXterm");
}
