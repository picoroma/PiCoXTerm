use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use pxt_ssh::{ConnectParams, HostKeyInfo, HostKeyVerifier, KnownHosts, Shell, ShellEvent};
use serde::Serialize;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Manager, State};
use tokio::sync::{mpsc, oneshot};

#[derive(Default)]
struct AppState {
    sessions: Mutex<HashMap<u64, Shell>>,
    pending_hostkeys: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
    next_id: AtomicU64,
}

impl AppState {
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed) + 1
    }
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
}

#[tauri::command]
fn app_info() -> String {
    format!("{} {}", pxt_core::APP_NAME, env!("CARGO_PKG_VERSION"))
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn ssh_connect(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    host: String,
    port: u16,
    username: String,
    password: String,
    cols: u32,
    rows: u32,
    on_data: Channel<InvokeResponseBody>,
    on_event: Channel<TermEvent>,
) -> Result<u64, String> {
    let known_path = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("known_hosts");

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
            password,
            cols,
            rows,
            term: "xterm-256color".into(),
        },
        KnownHosts::new(known_path),
        verifier,
        ev_tx,
    )
    .await
    .map_err(|e| e.to_string())?;

    let id = state.next_id();
    state.sessions.lock().unwrap().insert(id, shell);
    Ok(id)
}

fn get_shell(state: &AppState, id: u64) -> Result<Shell, String> {
    state
        .sessions
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| "sessione inesistente".to_string())
}

#[tauri::command]
async fn ssh_write(state: State<'_, Arc<AppState>>, id: u64, data: Vec<u8>) -> Result<(), String> {
    get_shell(&state, id)?
        .write(data)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn ssh_resize(
    state: State<'_, Arc<AppState>>,
    id: u64,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    get_shell(&state, id)?
        .resize(cols, rows)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn ssh_close(state: State<'_, Arc<AppState>>, id: u64) -> Result<(), String> {
    let shell = state.sessions.lock().unwrap().remove(&id);
    if let Some(s) = shell {
        s.close().await;
    }
    Ok(())
}

#[tauri::command]
fn hostkey_reply(state: State<'_, Arc<AppState>>, req_id: u64, accept: bool) {
    if let Some(tx) = state.pending_hostkeys.lock().unwrap().remove(&req_id) {
        let _ = tx.send(accept);
    }
}

pub fn run() {
    tauri::Builder::default()
        .manage(Arc::new(AppState::default()))
        .invoke_handler(tauri::generate_handler![
            app_info,
            ssh_connect,
            ssh_write,
            ssh_resize,
            ssh_close,
            hostkey_reply
        ])
        .run(tauri::generate_context!())
        .expect("errore nell'avvio di PiCoXterm");
}
