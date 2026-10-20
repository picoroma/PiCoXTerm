//! Sessione VNC (RFB) per PiCoXterm. Il disegno e l'input li fa noVNC nell'interfaccia; qui c'è la
//! connessione TCP (solo in uscita, nessuna porta in ascolto) e l'**autenticazione**, fatta dal backend
//! perché la password VNC non arrivi mai al codice JavaScript (regola 2): il backend si autentica con il
//! server vero e a noVNC presenta un server che non chiede nulla ("security type None"). Dopo la
//! negoziazione i byte passano così come sono.
//!
//! **Sicurezza del protocollo.** VNC classico non cifra niente: la password dà solo l'accesso (e con una
//! sfida DES a 8 caratteri) e lo schermo, i tasti e gli appunti viaggiano in chiaro. L'interfaccia lo
//! segnala; per un server esposto va usato un tunnel (SSH).
//!
//! Supportati: RFB 3.7 e 3.8 con "None" e "VNC Authentication". Altri tipi (VeNCrypt, Tight, RSA-AES...)
//! sono rifiutati con un messaggio chiaro.

use std::time::Duration;

use des::cipher::{Array, BlockCipherEncrypt, KeyInit};
use des::Des;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

#[derive(Debug, thiserror::Error)]
pub enum VncError {
    #[error("connessione VNC: {0}")]
    Connect(String),
    #[error("il server richiede una password VNC")]
    PasswordRequired,
    #[error("password VNC rifiutata dal server{}", if .0.is_empty() { String::new() } else { format!(": {}", .0) })]
    AuthFailed(String),
    #[error("il server VNC non è supportato: {0}")]
    Unsupported(String),
    #[error("{0}")]
    Protocol(String),
}

pub type Result<T> = std::result::Result<T, VncError>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
const CHUNK: usize = 64 * 1024;

/// Ciò che noVNC crede di ricevere dal "server", in **due messaggi separati**: la versione 3.8, poi un solo
/// tipo di sicurezza (None) e l'esito positivo. noVNC elabora un passo alla volta e dopo la versione aspetta
/// il messaggio successivo: uniti, il resto resterebbe fermo nel buffer.
const FAKE_VERSION: &[u8] = b"RFB 003.008\n";
const FAKE_SECURITY: &[u8] = &[
    1, 1, // un tipo di sicurezza: 1 = None
    0, 0, 0, 0, // SecurityResult: OK
];
/// Quanti byte noVNC manda in risposta: versione (12) + scelta del tipo di sicurezza (1).
const CLIENT_HANDSHAKE_BYTES: usize = 13;

/// Un valore della versione RFB: `3.7` o `3.8` (`3.3` non è supportato).
fn parse_version(banner: &[u8; 12]) -> Result<u8> {
    let text = std::str::from_utf8(banner)
        .map_err(|_| VncError::Protocol("versione RFB illeggibile".into()))?;
    if !text.starts_with("RFB ") || !text.ends_with('\n') {
        return Err(VncError::Protocol(
            "il servizio non risponde come un server VNC (nessun banner RFB)".into(),
        ));
    }
    match &text[4..11] {
        "003.003" => Err(VncError::Unsupported("RFB 3.3 è troppo vecchio".into())),
        "003.007" => Ok(7),
        // 3.8 e le versioni successive (3.889 di Apple, 4.x): si parla 3.8.
        _ => Ok(8),
    }
}

/// Risposta alla sfida di "VNC Authentication": la password (8 byte, completata con zeri) è la chiave DES
/// con i bit di ogni byte invertiti; si cifrano i due blocchi da 8 byte della sfida.
pub fn vnc_auth_response(password: &str, challenge: &[u8; 16]) -> [u8; 16] {
    let mut key = [0u8; 8];
    for (k, b) in key.iter_mut().zip(password.bytes()) {
        *k = b.reverse_bits();
    }
    let cipher = Des::new(&Array::from(key));
    let mut out = [0u8; 16];
    for (i, chunk) in challenge.as_chunks::<8>().0.iter().enumerate() {
        let mut block = Array::from(*chunk);
        cipher.encrypt_block(&mut block);
        out[i * 8..i * 8 + 8].copy_from_slice(&block);
    }
    out
}

async fn read_exact_timeout(s: &mut TcpStream, buf: &mut [u8]) -> Result<()> {
    tokio::time::timeout(HANDSHAKE_TIMEOUT, s.read_exact(buf))
        .await
        .map_err(|_| VncError::Protocol("il server non risponde durante la negoziazione".into()))?
        .map(|_| ())
        .map_err(|e| VncError::Protocol(format!("connessione chiusa durante la negoziazione: {e}")))
}

async fn write_all(s: &mut TcpStream, data: &[u8]) -> Result<()> {
    s.write_all(data)
        .await
        .map_err(|e| VncError::Protocol(format!("scrittura: {e}")))
}

/// Negozia versione e sicurezza con il server vero, autenticandosi se serve. Al ritorno il server aspetta
/// il `ClientInit`.
async fn handshake(s: &mut TcpStream, password: Option<&str>) -> Result<()> {
    let mut banner = [0u8; 12];
    read_exact_timeout(s, &mut banner).await?;
    let version = parse_version(&banner)?;
    write_all(
        s,
        if version == 7 {
            b"RFB 003.007\n"
        } else {
            b"RFB 003.008\n"
        },
    )
    .await?;

    // Tipi di sicurezza offerti.
    let mut count = [0u8; 1];
    read_exact_timeout(s, &mut count).await?;
    if count[0] == 0 {
        // Il server rifiuta subito, con un motivo.
        let mut len = [0u8; 4];
        read_exact_timeout(s, &mut len).await?;
        let n = (u32::from_be_bytes(len) as usize).min(1024);
        let mut reason = vec![0u8; n];
        read_exact_timeout(s, &mut reason).await?;
        return Err(VncError::Protocol(format!(
            "il server rifiuta la connessione: {}",
            String::from_utf8_lossy(&reason)
        )));
    }
    let mut types = vec![0u8; usize::from(count[0])];
    read_exact_timeout(s, &mut types).await?;

    // Si preferisce la password se il server la offre e ce n'è una; altrimenti nessuna autenticazione.
    let chosen = if types.contains(&2) && (password.is_some() || !types.contains(&1)) {
        2u8
    } else if types.contains(&1) {
        1
    } else {
        let names: Vec<String> = types.iter().map(|t| security_name(*t)).collect();
        return Err(VncError::Unsupported(format!(
            "tipi di sicurezza offerti: {} (sono supportati solo None e VNC Authentication)",
            names.join(", ")
        )));
    };
    write_all(s, &[chosen]).await?;

    if chosen == 2 {
        let Some(pw) = password else {
            return Err(VncError::PasswordRequired);
        };
        let mut challenge = [0u8; 16];
        read_exact_timeout(s, &mut challenge).await?;
        write_all(s, &vnc_auth_response(pw, &challenge)).await?;
    }
    // Esito: sempre con VNC Authentication; con None solo da RFB 3.8.
    if chosen == 2 || version == 8 {
        let mut result = [0u8; 4];
        read_exact_timeout(s, &mut result).await?;
        if u32::from_be_bytes(result) != 0 {
            let mut reason = String::new();
            if version == 8 {
                let mut len = [0u8; 4];
                if read_exact_timeout(s, &mut len).await.is_ok() {
                    let n = (u32::from_be_bytes(len) as usize).min(1024);
                    let mut buf = vec![0u8; n];
                    if read_exact_timeout(s, &mut buf).await.is_ok() {
                        reason = String::from_utf8_lossy(&buf).into_owned();
                    }
                }
            }
            return Err(VncError::AuthFailed(reason));
        }
    }
    Ok(())
}

fn security_name(t: u8) -> String {
    match t {
        1 => "None".into(),
        2 => "VNC Authentication".into(),
        5 => "RA2".into(),
        6 => "RA2ne".into(),
        16 => "Tight".into(),
        18 => "TLS".into(),
        19 => "VeNCrypt".into(),
        129 => "Apple Remote Desktop".into(),
        n => format!("tipo {n}"),
    }
}

/// Cosa arriva dalla connessione.
#[derive(Debug)]
pub enum VncEvent {
    /// Byte per noVNC (prima il finto handshake, poi il flusso del server).
    Data(Vec<u8>),
    /// La connessione è finita.
    Closed(String),
}

/// Il lato "comandi": invio dei byte di noVNC al server e chiusura.
#[derive(Clone)]
pub struct VncHandle {
    to_server: mpsc::Sender<Vec<u8>>,
    close: mpsc::Sender<()>,
}

impl VncHandle {
    /// Manda al server i byte di noVNC. `false` se la connessione è finita.
    pub async fn send(&self, data: Vec<u8>) -> bool {
        self.to_server.send(data).await.is_ok()
    }

    pub fn close(&self) {
        let _ = self.close.try_send(());
    }
}

pub struct VncSession {
    pub events: mpsc::Receiver<VncEvent>,
    pub handle: VncHandle,
}

impl VncSession {
    /// Si collega, si autentica e prepara il relay. `password`: la password VNC (se il server la chiede).
    pub async fn connect(host: &str, port: u16, password: Option<&str>) -> Result<VncSession> {
        let addr = format!("{}:{port}", host.trim());
        let mut stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr))
            .await
            .map_err(|_| VncError::Connect(format!("{addr}: nessuna risposta entro 15 secondi")))?
            .map_err(|e| VncError::Connect(format!("{addr}: {e}")))?;
        let _ = stream.set_nodelay(true);
        handshake(&mut stream, password).await?;

        let (ev_tx, ev_rx) = mpsc::channel::<VncEvent>(64);
        let (in_tx, mut in_rx) = mpsc::channel::<Vec<u8>>(256);
        let (close_tx, mut close_rx) = mpsc::channel::<()>(1);
        let (mut read_half, mut write_half) = stream.into_split();

        // noVNC riceve subito il finto handshake, in due messaggi.
        for part in [FAKE_VERSION, FAKE_SECURITY] {
            ev_tx
                .send(VncEvent::Data(part.to_vec()))
                .await
                .map_err(|_| VncError::Protocol("interrotto".into()))?;
        }

        // Dal server verso noVNC.
        let ev_reader = ev_tx.clone();
        let reader = tokio::spawn(async move {
            let mut buf = vec![0u8; CHUNK];
            let reason = loop {
                match read_half.read(&mut buf).await {
                    Ok(0) => break "il server ha chiuso la connessione".to_string(),
                    Ok(n) => {
                        if ev_reader
                            .send(VncEvent::Data(buf[..n].to_vec()))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(e) => break format!("connessione interrotta: {e}"),
                }
            };
            let _ = ev_reader.send(VncEvent::Closed(reason)).await;
        });

        // Da noVNC verso il server, scartando la sua parte di handshake (già fatta con il server vero).
        tokio::spawn(async move {
            let mut to_skip = CLIENT_HANDSHAKE_BYTES;
            loop {
                tokio::select! {
                    data = in_rx.recv() => {
                        let Some(mut data) = data else { break };
                        if to_skip > 0 {
                            let n = to_skip.min(data.len());
                            data.drain(..n);
                            to_skip -= n;
                        }
                        if !data.is_empty() && write_half.write_all(&data).await.is_err() {
                            break;
                        }
                    }
                    _ = close_rx.recv() => break,
                }
            }
            let _ = write_half.shutdown().await;
            reader.abort();
        });

        Ok(VncSession {
            events: ev_rx,
            handle: VncHandle {
                to_server: in_tx,
                close: close_tx,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// Il caso classico della sfida VNC: password "password", sfida di soli zeri.
    /// Valore ottenuto con una implementazione indipendente: `openssl enc -des-ecb -K 0e86ceceeef64e26 -nopad`
    /// (la chiave è "password" con i bit di ogni byte invertiti).
    #[test]
    fn vnc_auth_matches_an_independent_des_implementation() {
        let out = vnc_auth_response("password", &[0u8; 16]);
        assert_eq!(
            out,
            [
                0xff, 0x97, 0x50, 0x2e, 0x94, 0x22, 0xf0, 0x89, 0xff, 0x97, 0x50, 0x2e, 0x94, 0x22,
                0xf0, 0x89
            ]
        );
    }

    #[test]
    fn long_passwords_use_only_the_first_eight_characters() {
        let c: [u8; 16] = core::array::from_fn(|i| u8::try_from(i * 7).unwrap());
        assert_eq!(
            vnc_auth_response("12345678", &c),
            vnc_auth_response("12345678XYZ", &c)
        );
        assert_ne!(
            vnc_auth_response("12345678", &c),
            vnc_auth_response("1234567", &c)
        );
    }

    #[test]
    fn banners_are_parsed() {
        assert_eq!(parse_version(b"RFB 003.008\n").unwrap(), 8);
        assert_eq!(parse_version(b"RFB 003.007\n").unwrap(), 7);
        assert_eq!(parse_version(b"RFB 003.889\n").unwrap(), 8);
        assert!(matches!(
            parse_version(b"RFB 003.003\n"),
            Err(VncError::Unsupported(_))
        ));
        assert!(matches!(
            parse_version(b"SSH-2.0-open"),
            Err(VncError::Protocol(_))
        ));
    }

    /// Un finto server che segue il protocollo, per provare la negoziazione senza Docker.
    async fn fake_server(types: Vec<u8>, password: &'static str, banner: &'static [u8; 12]) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            s.write_all(banner).await.unwrap();
            let mut v = [0u8; 12];
            s.read_exact(&mut v).await.unwrap();
            s.write_all(&[u8::try_from(types.len()).unwrap()])
                .await
                .unwrap();
            s.write_all(&types).await.unwrap();
            let mut choice = [0u8; 1];
            s.read_exact(&mut choice).await.unwrap();
            if choice[0] == 2 {
                let challenge = [9u8; 16];
                s.write_all(&challenge).await.unwrap();
                let mut resp = [0u8; 16];
                s.read_exact(&mut resp).await.unwrap();
                if resp != vnc_auth_response(password, &challenge) {
                    s.write_all(&1u32.to_be_bytes()).await.unwrap();
                    let reason = b"password errata";
                    s.write_all(&u32::try_from(reason.len()).unwrap().to_be_bytes())
                        .await
                        .unwrap();
                    s.write_all(reason).await.unwrap();
                    return;
                }
                s.write_all(&0u32.to_be_bytes()).await.unwrap();
            } else if &v == b"RFB 003.008\n" {
                s.write_all(&0u32.to_be_bytes()).await.unwrap();
            }
            // Dopo la sicurezza: il server aspetta il ClientInit e risponde con una ServerInit finta.
            let mut init = [0u8; 1];
            if s.read_exact(&mut init).await.is_ok() {
                s.write_all(b"SERVERINIT").await.unwrap();
            }
        });
        port
    }

    async fn next_data(s: &mut VncSession) -> Vec<u8> {
        match tokio::time::timeout(Duration::from_secs(5), s.events.recv()).await {
            Ok(Some(VncEvent::Data(d))) => d,
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn authenticates_with_a_password_and_relays_the_stream() {
        let port = fake_server(vec![2], "segreto1", b"RFB 003.008\n").await;
        let mut s = VncSession::connect("127.0.0.1", port, Some("segreto1"))
            .await
            .unwrap();
        // noVNC riceve il finto handshake (versione 3.8, None, OK) e nessuna traccia della password.
        assert_eq!(next_data(&mut s).await, FAKE_VERSION);
        assert_eq!(next_data(&mut s).await, FAKE_SECURITY);
        // La parte di handshake di noVNC (13 byte) non arriva al server; il ClientInit sì.
        let mut from_novnc = b"RFB 003.008\n".to_vec();
        from_novnc.push(1); // scelta del tipo di sicurezza
        from_novnc.push(1); // ClientInit (shared)
        assert!(s.handle.send(from_novnc).await);
        assert_eq!(next_data(&mut s).await, b"SERVERINIT");
    }

    #[tokio::test]
    async fn a_wrong_password_is_reported_with_the_reason() {
        let port = fake_server(vec![2], "segreto1", b"RFB 003.008\n").await;
        match VncSession::connect("127.0.0.1", port, Some("sbagliata")).await {
            Err(VncError::AuthFailed(r)) => assert_eq!(r, "password errata"),
            other => panic!("{:?}", other.err()),
        }
    }

    #[tokio::test]
    async fn a_missing_password_asks_for_one() {
        let port = fake_server(vec![2], "x", b"RFB 003.008\n").await;
        assert!(matches!(
            VncSession::connect("127.0.0.1", port, None).await,
            Err(VncError::PasswordRequired)
        ));
    }

    #[tokio::test]
    async fn a_server_without_authentication_works_without_a_password() {
        let port = fake_server(vec![1], "", b"RFB 003.008\n").await;
        let mut s = VncSession::connect("127.0.0.1", port, None).await.unwrap();
        assert_eq!(next_data(&mut s).await, FAKE_VERSION);
    }

    #[tokio::test]
    async fn unsupported_security_types_are_named() {
        let port = fake_server(vec![19, 18], "", b"RFB 003.008\n").await;
        match VncSession::connect("127.0.0.1", port, Some("x")).await {
            Err(VncError::Unsupported(m)) => {
                assert!(m.contains("VeNCrypt") && m.contains("TLS"), "{m}")
            }
            other => panic!("{:?}", other.err()),
        }
    }

    #[tokio::test]
    async fn a_closed_port_fails_fast_and_a_non_vnc_service_is_recognised() {
        assert!(matches!(
            VncSession::connect("127.0.0.1", 1, None).await,
            Err(VncError::Connect(_))
        ));
        let port = fake_server(vec![1], "", b"SSH-2.0-open").await;
        assert!(matches!(
            VncSession::connect("127.0.0.1", port, None).await,
            Err(VncError::Protocol(_))
        ));
    }
}
