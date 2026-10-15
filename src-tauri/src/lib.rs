use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use pxt_core::{
    subtree_ids, AuthMethod, CredentialRef, KeyringSecretProvider, LocalSessionProvider, Protocol,
    Secret, SecretProvider, SessionNode, SessionProvider,
};
use pxt_ssh::{
    ConnectParams, Credentials, HostKeyInfo, HostKeyVerifier, KnownHosts, Shell, ShellEvent,
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
    secrets: KeyringSecretProvider,
    known_hosts: PathBuf,
    shells: Mutex<HashMap<u64, Shell>>,
    pending_hostkeys: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
    next_id: AtomicU64,
}

impl AppState {
    fn new(data_dir: PathBuf) -> Self {
        Self {
            sessions: LocalSessionProvider::new(data_dir.join("sessions.json")),
            secrets: KeyringSecretProvider::new(pxt_core::APP_NAME),
            known_hosts: data_dir.join("known_hosts"),
            shells: Mutex::default(),
            pending_hostkeys: Mutex::default(),
            next_id: AtomicU64::new(0),
        }
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed) + 1
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
    let (host, port, username, stored_ref, method, key_path) = match request.session_id {
        Some(id) => {
            let nodes = state.sessions.list().await.map_err(err)?;
            let Some(SessionNode::Session(s)) = nodes.into_iter().find(|n| n.id() == id) else {
                return Err("sessione non trovata".into());
            };
            if s.protocol != Protocol::Ssh {
                return Err("protocollo non ancora supportato".into());
            }
            (
                s.host,
                s.port,
                s.username,
                s.credential_ref,
                s.auth.method,
                s.auth.key_path,
            )
        }
        None => (
            request.host.ok_or("host mancante")?,
            request.port.unwrap_or(22),
            request.username.ok_or("utente mancante")?,
            None,
            AuthMethod::Password,
            None,
        ),
    };

    // Segreto fornito ora dall'utente, altrimenti quello nel keyring (se c'è).
    let secret = match request.password {
        Some(p) => Some(p),
        None => match &stored_ref {
            Some(r) => state
                .secrets
                .get(r)
                .await
                .map_err(err)?
                .map(|s| s.expose().to_string()),
            None => None,
        },
    };

    let auth = match method {
        AuthMethod::Password => Credentials::Password(secret.ok_or(PASSWORD_REQUIRED)?),
        // La passphrase è opzionale: se la chiave è cifrata la richiede la connessione.
        AuthMethod::Key => Credentials::Key {
            path: key_path
                .filter(|p| !p.trim().is_empty())
                .ok_or("percorso della chiave privata mancante")?
                .into(),
            passphrase: secret,
        },
        AuthMethod::Agent => Credentials::Agent,
        AuthMethod::KeyboardInteractive => {
            return Err("keyboard-interactive non ancora supportato".into())
        }
    };

    let verifier: HostKeyVerifier = {
        let state = Arc::clone(&state);
        let on_event = on_event.clone();
        Arc::new(move |info: HostKeyInfo| {
            let state = Arc::clone(&state);
            let on_event = on_event.clone();
            Box::pin(async move {
                let req_id = state.next_id();
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
                rx.await.unwrap_or(false)
            })
        })
    };

    let (ev_tx, mut ev_rx) = mpsc::channel::<ShellEvent>(64);
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
                    let _ = on_event.send(TermEvent::Closed { exit });
                    break;
                }
            }
        }
    });

    let shell = pxt_ssh::connect_shell(
        ConnectParams {
            host,
            port,
            username,
            auth,
            cols,
            rows,
            term: "xterm-256color".into(),
        },
        KnownHosts::new(state.known_hosts.clone()),
        verifier,
        ev_tx,
    )
    .await
    .map_err(|e| match e {
        SshError::PassphraseRequired => PASSPHRASE_REQUIRED.to_string(),
        other => other.to_string(),
    })?;

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
            sessions_list,
            session_save,
            session_delete,
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
