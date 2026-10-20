//! Sessione RDP su IronRDP: connessione con NLA/CredSSP, ciclo di lettura che decodifica la grafica e la
//! restituisce a rettangoli RGBA, invio di mouse e tastiera.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ironrdp::cliprdr::pdu::{ClipboardFormat, ClipboardFormatId, OwnedFormatDataResponse};
use ironrdp::cliprdr::{Client, Cliprdr, CliprdrSvcMessages};
use ironrdp::connector::{self, ClientConnector, Credentials, DesktopSize};
use ironrdp::input::{Database, MouseButton, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{
    connect_begin, connect_finalize, mark_as_upgraded, FramedWrite as _, TokioFramed,
};
use pxt_rdp_proto::{Connect, FromEngine, Input};
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::clipboard::{decode_text, encode_text, ClipEvent, TextBackend, MAX_TEXT_BYTES};
use crate::tls::{self, SeenCert};

/// Tempo massimo per TCP, TLS, NLA e negoziazione.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const MIN_SIZE: u16 = 200;
const MAX_SIZE: u16 = 8192;

#[derive(Debug)]
pub enum ConnectError {
    /// Certificato non verificabile e non fissato: decide l'utente.
    Untrusted {
        fingerprint: String,
        changed: bool,
    },
    Connect(String),
    Auth(String),
}

type TlsFramed = TokioFramed<tokio_rustls::client::TlsStream<TcpStream>>;

pub struct Established {
    pub width: u16,
    pub height: u16,
    framed: TlsFramed,
    stage: ActiveStage,
    image: DecodedImage,
    /// Eventi degli appunti condivisi (se attivi per questa sessione).
    clip_rx: Option<mpsc::UnboundedReceiver<ClipEvent>>,
}

fn config(p: &Connect, width: u16, height: u16) -> connector::Config {
    connector::Config {
        credentials: Credentials::UsernamePassword {
            username: p.username.clone(),
            password: p.password.clone(),
        },
        domain: p.domain.clone().filter(|d| !d.trim().is_empty()),
        enable_tls: true,
        enable_credssp: true,
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_layout: p.keyboard_layout,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: DesktopSize { width, height },
        bitmap: Some(connector::BitmapConfig {
            lossy_compression: true,
            color_depth: 32,
            codecs: ironrdp_pdu::rdp::capability_sets::client_codecs_capabilities(&[])
                .unwrap_or_default(),
        }),
        client_build: 0,
        client_name: "PiCoXterm".into(),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".into(),
        platform: MajorPlatformType::WINDOWS,
        enable_server_pointer: true,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        compression_type: None,
        // Il puntatore viene disegnato nell'immagine: niente gestione separata nella scheda.
        pointer_software_rendering: true,
        multitransport_flags: None,
        performance_flags: PerformanceFlags::default(),
        desktop_scale_factor: 0,
        hardware_id: None,
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
    }
}

fn text(e: &impl fmt::Display) -> String {
    e.to_string()
}

/// Si collega. Un certificato rifiutato dal TLS diventa `ConnectError::Untrusted` con l'impronta.
pub async fn connect(params: Connect) -> Result<Established, ConnectError> {
    let seen: SeenCert = Arc::new(Mutex::new(None));
    let pinned = params.pinned_cert.clone();
    let outcome = tokio::time::timeout(CONNECT_TIMEOUT, connect_inner(params, seen.clone())).await;
    match outcome {
        Err(_) => Err(ConnectError::Connect(
            "nessuna risposta dal server entro 30 secondi".into(),
        )),
        Ok(Ok(s)) => Ok(s),
        Ok(Err(e)) => {
            let fp = seen.lock().unwrap().take();
            match fp {
                Some(fingerprint) => Err(ConnectError::Untrusted {
                    changed: pinned.is_some(),
                    fingerprint,
                }),
                None => Err(e),
            }
        }
    }
}

async fn connect_inner(params: Connect, seen: SeenCert) -> Result<Established, ConnectError> {
    let width = params.width.clamp(MIN_SIZE, MAX_SIZE);
    let height = params.height.clamp(MIN_SIZE, MAX_SIZE);
    let port = if params.port == 0 { 3389 } else { params.port };
    let host = params.host.trim().to_string();
    if host.is_empty() || host.chars().any(char::is_control) {
        return Err(ConnectError::Connect("host non valido".into()));
    }

    let tcp = TcpStream::connect((host.as_str(), port))
        .await
        .map_err(|e| ConnectError::Connect(format!("{host}:{port}: {e}")))?;
    let client_addr = tcp
        .local_addr()
        .map_err(|e| ConnectError::Connect(text(&e)))?;
    let mut framed = TokioFramed::new(tcp);
    let mut connector = ClientConnector::new(config(&params, width, height), client_addr);
    let mut clip_rx = None;
    if params.clipboard {
        let (tx, rx) = mpsc::unbounded_channel();
        connector.attach_static_channel(Cliprdr::<Client>::new(Box::new(TextBackend::new(tx))));
        clip_rx = Some(rx);
    }

    let should_upgrade = connect_begin(&mut framed, &mut connector)
        .await
        .map_err(|e| ConnectError::Connect(text(&e)))?;

    // Passaggio a TLS con il nostro verificatore (pin dell'impronta).
    let tcp = framed.into_inner_no_leftover();
    let tls_config =
        tls::client_config(params.pinned_cert.clone(), seen).map_err(ConnectError::Connect)?;
    let name = ServerName::try_from(host.clone())
        .map_err(|e| ConnectError::Connect(format!("nome del server non valido: {e}")))?;
    let tls_stream = tokio_rustls::TlsConnector::from(Arc::new(tls_config))
        .connect(name, tcp)
        .await
        .map_err(|e| ConnectError::Connect(format!("TLS: {e}")))?;
    let server_public_key = {
        use x509_cert::der::Decode as _;
        let der = tls_stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|c| c.first())
            .ok_or_else(|| {
                ConnectError::Connect("il server non ha mostrato un certificato".into())
            })?;
        let cert = x509_cert::Certificate::from_der(der.as_ref())
            .map_err(|e| ConnectError::Connect(format!("certificato illeggibile: {e}")))?;
        cert.tbs_certificate
            .subject_public_key_info
            .subject_public_key
            .as_bytes()
            .ok_or_else(|| ConnectError::Connect("chiave pubblica del server illeggibile".into()))?
            .to_vec()
    };

    let upgraded = mark_as_upgraded(should_upgrade, &mut connector);
    let mut framed = TokioFramed::new(tls_stream);
    let mut network = ReqwestNetworkClient::new();
    let result = connect_finalize(
        upgraded,
        connector,
        &mut framed,
        &mut network,
        host.clone().into(),
        server_public_key,
        None,
    )
    .await
    .map_err(|e| ConnectError::Auth(text(&e)))?;

    let (w, h) = (result.desktop_size.width, result.desktop_size.height);
    let image = DecodedImage::new(
        ironrdp_graphics::image_processing::PixelFormat::RgbA32,
        w,
        h,
    );
    let stage = ActiveStageBuilder {
        static_channels: result.static_channels,
        user_channel_id: result.user_channel_id,
        io_channel_id: result.io_channel_id,
        message_channel_id: result.message_channel_id,
        share_id: result.share_id,
        compression_type: result.compression_type,
        enable_server_pointer: result.enable_server_pointer,
        pointer_software_rendering: result.pointer_software_rendering,
    }
    .build();
    Ok(Established {
        width: w,
        height: h,
        framed,
        stage,
        image,
        clip_rx,
    })
}

fn to_operations(input: &Input) -> Vec<Operation> {
    match *input {
        Input::MouseMove { x, y } => vec![Operation::MouseMove(MousePosition { x, y })],
        Input::MouseButton { button, pressed } => {
            let b = match button {
                0 => MouseButton::Left,
                1 => MouseButton::Middle,
                2 => MouseButton::Right,
                3 => MouseButton::X1,
                _ => MouseButton::X2,
            };
            vec![if pressed {
                Operation::MouseButtonPressed(b)
            } else {
                Operation::MouseButtonReleased(b)
            }]
        }
        Input::Wheel { vertical, units } => vec![Operation::WheelRotations(WheelRotations {
            is_vertical: vertical,
            rotation_units: units,
        })],
        Input::Key { scancode, pressed } => {
            let sc = Scancode::from_u16(scancode);
            vec![if pressed {
                Operation::KeyPressed(sc)
            } else {
                Operation::KeyReleased(sc)
            }]
        }
        Input::ReleaseAll => Vec::new(),
    }
}

/// Copia il rettangolo aggiornato dal buffer del desktop.
fn extract(
    image: &DecodedImage,
    rect: &ironrdp_pdu::geometry::InclusiveRectangle,
) -> Option<FromEngine> {
    let (w, h) = (image.width(), image.height());
    let left = rect.left.min(w.saturating_sub(1));
    let top = rect.top.min(h.saturating_sub(1));
    let right = rect.right.min(w.saturating_sub(1));
    let bottom = rect.bottom.min(h.saturating_sub(1));
    if right < left || bottom < top {
        return None;
    }
    let (rw, rh) = (usize::from(right - left) + 1, usize::from(bottom - top) + 1);
    let stride = usize::from(w) * 4;
    let data = image.data();
    let mut rgba = Vec::with_capacity(rw * rh * 4);
    for row in usize::from(top)..usize::from(top) + rh {
        let start = row * stride + usize::from(left) * 4;
        rgba.extend_from_slice(data.get(start..start + rw * 4)?);
    }
    Some(FromEngine::Update {
        x: left,
        y: top,
        width: u16::try_from(rw).ok()?,
        height: u16::try_from(rh).ok()?,
        rgba,
    })
}

/// Cosa si chiede al ciclo di una sessione già aperta.
pub enum Command {
    Input(Input),
    SetClipboard(String),
    Close,
}

/// Ciclo della sessione: legge dal server, manda gli aggiornamenti e applica l'input.
/// Restituisce il motivo della fine.
pub async fn run(
    est: Established,
    out: mpsc::Sender<FromEngine>,
    mut commands: mpsc::UnboundedReceiver<Command>,
) -> String {
    let Established {
        mut framed,
        mut stage,
        mut image,
        mut clip_rx,
        ..
    } = est;
    let mut clip = ClipState::default();
    let mut db = Database::new();
    'outer: loop {
        tokio::select! {
            pdu = framed.read_pdu() => {
                let (action, payload) = match pdu {
                    Ok(p) => p,
                    Err(e) => break 'outer format!("connessione interrotta: {e}"),
                };
                let outputs = match stage.process(&mut image, action, &payload) {
                    Ok(o) => o,
                    Err(e) => break 'outer format!("errore del protocollo: {e}"),
                };
                for o in outputs {
                    match o {
                        ActiveStageOutput::ResponseFrame(f) => {
                            if let Err(e) = framed.write_all(&f).await {
                                break 'outer format!("scrittura: {e}");
                            }
                        }
                        ActiveStageOutput::GraphicsUpdate(rect) => {
                            if let Some(ev) = extract(&image, &rect) {
                                if out.send(ev).await.is_err() {
                                    break 'outer "chiusa".to_string();
                                }
                            }
                        }
                        ActiveStageOutput::Terminate(r) => break 'outer r.description(),
                        ActiveStageOutput::DeactivateAll => {
                            break 'outer "il server ha cambiato la risoluzione: non ancora gestito".into();
                        }
                        _ => {}
                    }
                }
            }
            ev = async {
                match clip_rx.as_mut() {
                    Some(r) => r.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(ev) = ev {
                    if std::env::var_os("PXT_RDP_DEBUG").is_some() {
                        eprintln!("appunti: evento {ev:?}");
                    }
                    if let Err(e) = handle_clip(ev, &mut clip, &mut stage, &mut framed, &out).await {
                        break 'outer e;
                    }
                }
            }
            cmd = commands.recv() => {
                match cmd {
                    None | Some(Command::Close) => {
                        if let Ok(outs) = stage.graceful_shutdown() {
                            for o in outs {
                                if let ActiveStageOutput::ResponseFrame(f) = o {
                                    let _ = framed.write_all(&f).await;
                                }
                            }
                        }
                        break 'outer "chiusa".to_string();
                    }
                    Some(Command::SetClipboard(t)) => {
                        if t.len() <= MAX_TEXT_BYTES {
                            clip.local = Some(t);
                            if let Err(e) = advertise(&mut clip, &mut stage, &mut framed).await {
                                break 'outer e;
                            }
                        }
                    }
                    Some(Command::Input(i)) => {
                        let fast = if matches!(i, Input::ReleaseAll) {
                            db.release_all()
                        } else {
                            db.apply(to_operations(&i))
                        };
                        match stage.process_fastpath_input(&mut image, &fast) {
                            Ok(outs) => {
                                for o in outs {
                                    match o {
                                        ActiveStageOutput::ResponseFrame(f) => {
                                            if let Err(e) = framed.write_all(&f).await {
                                                break 'outer format!("scrittura: {e}");
                                            }
                                        }
                                        ActiveStageOutput::GraphicsUpdate(rect) => {
                                            if let Some(ev) = extract(&image, &rect) {
                                                let _ = out.send(ev).await;
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            Err(e) => break 'outer format!("input: {e}"),
                        }
                    }
                }
            }
        }
    }
}

/// Stato degli appunti di una sessione.
#[derive(Default)]
struct ClipState {
    /// Il canale è pronto: si può annunciare e chiedere dati.
    ready: bool,
    /// Ultimo testo copiato sul computer (quello che il server può incollare).
    local: Option<String>,
}

async fn send_clip(
    stage: &mut ActiveStage,
    framed: &mut TlsFramed,
    msgs: CliprdrSvcMessages<Client>,
) -> Result<(), String> {
    let bytes = stage
        .process_svc_processor_messages(msgs)
        .map_err(|e| format!("appunti: {e}"))?;
    framed
        .write_all(&bytes)
        .await
        .map_err(|e| format!("scrittura: {e}"))
}

/// Annuncia al server i formati disponibili sul computer: il testo, se c'è, altrimenti niente.
/// Mandare l'elenco (anche vuoto) all'avvio del canale completa la negoziazione.
async fn advertise(
    clip: &mut ClipState,
    stage: &mut ActiveStage,
    framed: &mut TlsFramed,
) -> Result<(), String> {
    let formats: Vec<ClipboardFormat> = if clip.local.is_some() {
        vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]
    } else {
        Vec::new()
    };
    let Some(c) = stage.get_svc_processor_mut::<Cliprdr<Client>>() else {
        return Ok(());
    };
    // Un errore degli appunti (canale non ancora pronto, per esempio) non deve chiudere la sessione.
    match c.initiate_copy(&formats) {
        Ok(msgs) => send_clip(stage, framed, msgs).await,
        Err(_) => Ok(()),
    }
}

async fn handle_clip(
    ev: ClipEvent,
    clip: &mut ClipState,
    stage: &mut ActiveStage,
    framed: &mut TlsFramed,
    out: &mpsc::Sender<FromEngine>,
) -> Result<(), String> {
    match ev {
        ClipEvent::Ready => {
            clip.ready = true;
            if clip.local.is_some() {
                advertise(clip, stage, framed).await?;
            }
            Ok(())
        }
        ClipEvent::RequestFormatList => advertise(clip, stage, framed).await,
        ClipEvent::RemoteCopy(ids) => {
            if !ids.contains(&ClipboardFormatId::CF_UNICODETEXT) {
                return Ok(());
            }
            let Some(c) = stage.get_svc_processor_mut::<Cliprdr<Client>>() else {
                return Ok(());
            };
            match c.initiate_paste(ClipboardFormatId::CF_UNICODETEXT) {
                Ok(msgs) => send_clip(stage, framed, msgs).await,
                Err(_) => Ok(()),
            }
        }
        ClipEvent::DataResponse(Some(data)) => {
            if data.len() <= MAX_TEXT_BYTES * 2 {
                let text = decode_text(&data);
                if !text.is_empty() {
                    let _ = out.send(FromEngine::ClipboardText(text)).await;
                }
            }
            Ok(())
        }
        ClipEvent::DataResponse(None) => Ok(()),
        ClipEvent::DataRequest(fmt) => {
            let Some(c) = stage.get_svc_processor_mut::<Cliprdr<Client>>() else {
                return Ok(());
            };
            let response = match (&clip.local, fmt == ClipboardFormatId::CF_UNICODETEXT) {
                (Some(t), true) => OwnedFormatDataResponse::new_data(encode_text(t)),
                _ => OwnedFormatDataResponse::new_error(),
            };
            match c.submit_format_data(response) {
                Ok(msgs) => send_clip(stage, framed, msgs).await,
                Err(_) => Ok(()),
            }
        }
    }
}
