//! Sessione RDP integrata (D-32). Il protocollo RDP lo parla un programma a parte, `picoxterm-rdp`
//! (cartella `rdp-engine/`, con il suo Cargo.lock: IronRDP e `russh` non possono stare nello stesso
//! `Cargo.lock`), collegato a questo processo da pipe. Qui si avvia, gli si manda l'input e si
//! ricevono i rettangoli di grafica. Nessuna porta in ascolto.
//!
//! La password viaggia una volta sola, nel messaggio di connessione sulla pipe: mai su una riga di
//! comando, mai in un log.

use std::io::{BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pxt_rdp_proto::{
    read_from_engine, write_to_engine, Connect, FromEngine, Input as ProtoInput, ToEngine,
};
use tokio::sync::mpsc;

use crate::{RdpError, Result};

/// Cosa arriva dalla sessione (`Update` con i pixel, `Disconnected` alla fine).
pub type RdpEvent = FromEngine;
/// Cosa si può mandare alla sessione: mouse, rotella, tasti.
pub type RdpInput = ProtoInput;

/// Nome del programma del motore.
pub const ENGINE_NAME: &str = if cfg!(windows) {
    "picoxterm-rdp.exe"
} else {
    "picoxterm-rdp"
};

/// Tempo massimo fra l'avvio e la risposta alla connessione (il motore ha il suo limite di 30 s).
const REPLY_TIMEOUT: Duration = Duration::from_secs(45);

pub struct SessionParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Password in chiaro: vive solo il tempo di spedirla al motore.
    pub password: String,
    pub domain: Option<String>,
    /// Dimensione iniziale del desktop.
    pub width: u16,
    pub height: u16,
    /// Impronta SHA-256 del certificato già accettata dall'utente.
    pub pinned_cert: Option<String>,
    /// Layout di tastiera del client (KLID, per esempio `0x410`); 0 = predefinito del server.
    /// Con un layout diverso da quello della tastiera, i tasti fisici producono caratteri sbagliati
    /// (su una tastiera italiana il `-` finirebbe come `/` con un server impostato sul layout US).
    pub keyboard_layout: u32,
    /// Condivide gli appunti (solo testo) con il server.
    pub clipboard: bool,
}

impl std::fmt::Debug for SessionParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionParams")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"<nascosta>")
            .finish_non_exhaustive()
    }
}

/// Layout di tastiera di questo computer come KLID di RDP (per esempio `0x410` per l'italiano, `0x409`
/// per l'inglese americano), o 0 se non si riesce a saperlo. Su Windows è il layout della finestra in
/// primo piano (la nostra, quando si apre una sessione).
pub fn system_keyboard_layout() -> u32 {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId,
        };
        // SAFETY: chiamate di sola lettura senza puntatori da mantenere.
        unsafe {
            let hwnd = GetForegroundWindow();
            let thread = if hwnd.0.is_null() {
                0
            } else {
                GetWindowThreadProcessId(hwnd, None)
            };
            let hkl = GetKeyboardLayout(thread);
            // La parte bassa è l'identificatore della lingua, che per i layout abituali coincide con il KLID.
            (hkl.0 as usize & 0xFFFF) as u32
        }
    }
    #[cfg(not(windows))]
    {
        0
    }
}

/// Dove sta il motore: la variabile `PXT_RDP_ENGINE` (per prove e sviluppo), poi accanto
/// all'eseguibile dell'app.
pub fn engine_path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("PXT_RDP_ENGINE").map(PathBuf::from) {
        return if p.is_file() {
            Ok(p)
        } else {
            Err(RdpError::EngineMissing(p.display().to_string()))
        };
    }
    let exe = std::env::current_exe()?;
    let beside = exe.with_file_name(ENGINE_NAME);
    if beside.is_file() {
        Ok(beside)
    } else {
        Err(RdpError::EngineMissing(beside.display().to_string()))
    }
}

pub struct RdpSession {
    pub width: u16,
    pub height: u16,
    pub events: mpsc::Receiver<RdpEvent>,
    handle: RdpHandle,
}

/// Il lato "comandi" della sessione: input e chiusura. Chiude il motore quando viene lasciato cadere.
pub struct RdpHandle {
    to_engine: std::sync::mpsc::Sender<ToEngine>,
    child: Arc<Mutex<Child>>,
}

#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_window(_: &mut Command) {}

impl RdpSession {
    /// Avvia il motore e si collega. Un certificato non verificabile e non fissato produce
    /// `RdpError::UntrustedCertificate` con l'impronta: la UI chiede all'utente e riprova con `pinned_cert`.
    pub async fn connect(params: SessionParams) -> Result<RdpSession> {
        let path = engine_path()?;
        let mut cmd = Command::new(&path);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Con PXT_RDP_DEBUG i log del motore (mai la password) vanno al terminale di chi lo avvia.
            .stderr(if std::env::var_os("PXT_RDP_DEBUG").is_some() {
                Stdio::inherit()
            } else {
                Stdio::null()
            });
        no_window(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|e| RdpError::Launch(format!("{}: {e}", path.display())))?;
        // Se l'app muore di colpo, il sistema termina anche il motore.
        #[cfg(windows)]
        let _ = crate::job::assign(&child);
        let stdin = child.stdin.take().expect("stdin del motore");
        let stdout = child.stdout.take().expect("stdout del motore");
        let child = Arc::new(Mutex::new(child));

        // Scrittore: un thread manda i messaggi in ordine.
        let (to_engine, writer_rx) = std::sync::mpsc::channel::<ToEngine>();
        std::thread::spawn(move || {
            let mut out = BufWriter::new(stdin);
            while let Ok(m) = writer_rx.recv() {
                if write_to_engine(&mut out, &m).is_err() || out.flush().is_err() {
                    break;
                }
            }
        });

        // Lettore: un thread legge i messaggi e li passa all'async.
        let (tx, mut rx) = mpsc::channel::<FromEngine>(64);
        std::thread::spawn(move || {
            let mut input = BufReader::new(stdout);
            while let Ok(Some(m)) = read_from_engine(&mut input) {
                if tx.blocking_send(m).is_err() {
                    return;
                }
            }
            let _ = tx.blocking_send(FromEngine::Disconnected("il motore RDP si è chiuso".into()));
        });

        let session_guard = |child: &Arc<Mutex<Child>>| {
            let c = Arc::clone(child);
            move || {
                if let Ok(mut c) = c.lock() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
            }
        };

        let _ = to_engine.send(ToEngine::Connect(Connect {
            host: params.host,
            port: params.port,
            username: params.username,
            password: params.password,
            domain: params.domain,
            width: params.width,
            height: params.height,
            pinned_cert: params.pinned_cert,
            keyboard_layout: params.keyboard_layout,
            clipboard: params.clipboard,
            parent_pid: std::process::id(),
        }));

        let first = tokio::time::timeout(REPLY_TIMEOUT, rx.recv()).await;
        let result = match first {
            Err(_) => Err(RdpError::Connect(
                "nessuna risposta dal motore RDP entro 45 secondi".into(),
            )),
            Ok(None) => Err(RdpError::Connect("il motore RDP si è chiuso".into())),
            Ok(Some(FromEngine::Connected(c))) => {
                return Ok(RdpSession {
                    width: c.width,
                    height: c.height,
                    events: rx,
                    handle: RdpHandle { to_engine, child },
                });
            }
            Ok(Some(FromEngine::CertUntrusted(c))) => Err(RdpError::UntrustedCertificate {
                fingerprint: c.fingerprint,
                changed: c.changed,
            }),
            Ok(Some(FromEngine::Failed(f))) if f.kind == "auth" => Err(RdpError::Auth(f.message)),
            Ok(Some(FromEngine::Failed(f))) => Err(RdpError::Connect(f.message)),
            Ok(Some(FromEngine::Disconnected(r))) => Err(RdpError::Connect(r)),
            Ok(Some(
                FromEngine::Update { .. }
                | FromEngine::ClipboardText(_)
                | FromEngine::Resized { .. },
            )) => Err(RdpError::Connect("risposta inattesa del motore RDP".into())),
        };
        // Connessione non riuscita: il motore va chiuso.
        session_guard(&child)();
        result
    }

    /// Manda un evento di input; `false` se la sessione è già finita.
    pub fn send(&self, input: RdpInput) -> bool {
        self.handle.send(input)
    }

    /// Chiude la sessione (ordinatamente, se il server risponde) e il motore.
    pub fn close(&self) {
        self.handle.close();
    }

    /// Rende incollabile nel server il testo appena copiato sul computer.
    pub fn send_clipboard(&self, text: String) -> bool {
        self.handle.send_clipboard(text)
    }

    /// Chiede al server una nuova risoluzione del desktop; la conferma arriva come `RdpEvent::Resized`.
    pub fn resize(&self, width: u16, height: u16) -> bool {
        self.handle.resize(width, height)
    }

    /// Separa la coda degli eventi dai comandi: gli eventi possono andare a un task che li gira alla
    /// scheda, mentre il resto dell'app tiene l'`RdpHandle` per input e chiusura.
    pub fn split(self) -> (mpsc::Receiver<RdpEvent>, RdpHandle) {
        (self.events, self.handle)
    }
}

impl RdpHandle {
    /// Chiede al server una nuova risoluzione del desktop; la conferma arriva come `RdpEvent::Resized`.
    pub fn resize(&self, width: u16, height: u16) -> bool {
        self.to_engine
            .send(ToEngine::Resize { width, height })
            .is_ok()
    }

    /// Rende incollabile nel server il testo appena copiato sul computer.
    pub fn send_clipboard(&self, text: String) -> bool {
        self.to_engine.send(ToEngine::SetClipboard(text)).is_ok()
    }

    /// Manda un evento di input; `false` se la sessione è già finita.
    pub fn send(&self, input: RdpInput) -> bool {
        self.to_engine.send(ToEngine::Input(input)).is_ok()
    }

    /// Chiude la sessione (ordinatamente, se il server risponde) e il motore.
    pub fn close(&self) {
        let _ = self.to_engine.send(ToEngine::Close);
        let child = Arc::clone(&self.child);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(2));
            if let Ok(mut c) = child.lock() {
                let _ = c.kill();
                let _ = c.wait();
            }
        });
    }
}

impl Drop for RdpHandle {
    fn drop(&mut self) {
        self.close();
    }
}
