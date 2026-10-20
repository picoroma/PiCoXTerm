//! Protocollo a messaggi fra l'app (`pxt-rdp`) e il motore RDP (`picoxterm-rdp`), che girano in due
//! processi collegati da pipe (stdin/stdout del motore). Nessuna rete, nessuna porta in ascolto.
//!
//! Ogni messaggio è `[lunghezza u32 LE][tipo u8][contenuto]`, dove la lunghezza conta tipo e contenuto.
//! I messaggi rari sono JSON; la grafica (`Update`) è binaria per non pagare la codifica.
//!
//! La password viaggia solo nel messaggio `Connect`, sulla pipe: mai su una riga di comando.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

/// Dimensione massima di un messaggio: un desktop 8192x8192 RGBA più l'intestazione.
pub const MAX_MESSAGE: usize = 8192 * 8192 * 4 + 64;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connect {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub domain: Option<String>,
    pub width: u16,
    pub height: u16,
    pub pinned_cert: Option<String>,
    /// Layout di tastiera del client (KLID, per esempio `0x410` italiano); 0 = quello predefinito del server.
    /// Serve perché si mandano i tasti fisici: il server li interpreta con questo layout.
    #[serde(default)]
    pub keyboard_layout: u32,
}

impl std::fmt::Debug for Connect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connect")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"<nascosta>")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Input {
    MouseMove { x: u16, y: u16 },
    MouseButton { button: u8, pressed: bool },
    Wheel { vertical: bool, units: i16 },
    Key { scancode: u16, pressed: bool },
    ReleaseAll,
}

/// Dall'app al motore.
#[derive(Debug)]
pub enum ToEngine {
    Connect(Connect),
    Input(Input),
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connected {
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertUntrusted {
    pub fingerprint: String,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Failed {
    /// `connect` (rete, TLS, protocollo) o `auth` (credenziali rifiutate).
    pub kind: String,
    pub message: String,
}

/// Dal motore all'app.
#[derive(Debug)]
pub enum FromEngine {
    Connected(Connected),
    CertUntrusted(CertUntrusted),
    Failed(Failed),
    Update {
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        rgba: Vec<u8>,
    },
    Disconnected(String),
}

const T_CONNECT: u8 = 0x01;
const T_INPUT: u8 = 0x02;
const T_CLOSE: u8 = 0x03;
const T_CONNECTED: u8 = 0x10;
const T_CERT: u8 = 0x11;
const T_FAILED: u8 = 0x12;
const T_UPDATE: u8 = 0x20;
const T_DISCONNECTED: u8 = 0x21;

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn json<T: Serialize>(v: &T) -> io::Result<Vec<u8>> {
    serde_json::to_vec(v).map_err(|e| bad(&e.to_string()))
}

fn from_json<T: for<'de> Deserialize<'de>>(b: &[u8]) -> io::Result<T> {
    serde_json::from_slice(b).map_err(|e| bad(&e.to_string()))
}

fn put(out: &mut impl Write, tag: u8, body: &[u8]) -> io::Result<()> {
    let len = u32::try_from(body.len() + 1).map_err(|_| bad("messaggio troppo grande"))?;
    // Un'unica scrittura: i messaggi non si mescolano se due thread scrivono.
    let mut buf = Vec::with_capacity(body.len() + 5);
    buf.extend_from_slice(&len.to_le_bytes());
    buf.push(tag);
    buf.extend_from_slice(body);
    out.write_all(&buf)?;
    out.flush()
}

pub fn write_to_engine(out: &mut impl Write, m: &ToEngine) -> io::Result<()> {
    match m {
        ToEngine::Connect(c) => put(out, T_CONNECT, &json(c)?),
        ToEngine::Input(i) => put(out, T_INPUT, &json(i)?),
        ToEngine::Close => put(out, T_CLOSE, &[]),
    }
}

pub fn write_from_engine(out: &mut impl Write, m: &FromEngine) -> io::Result<()> {
    match m {
        FromEngine::Connected(c) => put(out, T_CONNECTED, &json(c)?),
        FromEngine::CertUntrusted(c) => put(out, T_CERT, &json(c)?),
        FromEngine::Failed(f) => put(out, T_FAILED, &json(f)?),
        FromEngine::Update {
            x,
            y,
            width,
            height,
            rgba,
        } => {
            let mut body = Vec::with_capacity(8 + rgba.len());
            for v in [x, y, width, height] {
                body.extend_from_slice(&v.to_le_bytes());
            }
            body.extend_from_slice(rgba);
            put(out, T_UPDATE, &body)
        }
        FromEngine::Disconnected(r) => put(out, T_DISCONNECTED, r.as_bytes()),
    }
}

/// Legge un messaggio grezzo; `Ok(None)` a fine flusso.
fn read_raw(input: &mut impl Read) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut len = [0u8; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > MAX_MESSAGE {
        return Err(bad("lunghezza del messaggio non valida"));
    }
    let mut tag = [0u8; 1];
    input.read_exact(&mut tag)?;
    let mut body = vec![0u8; len - 1];
    input.read_exact(&mut body)?;
    Ok(Some((tag[0], body)))
}

pub fn read_to_engine(input: &mut impl Read) -> io::Result<Option<ToEngine>> {
    let Some((tag, body)) = read_raw(input)? else {
        return Ok(None);
    };
    Ok(Some(match tag {
        T_CONNECT => ToEngine::Connect(from_json(&body)?),
        T_INPUT => ToEngine::Input(from_json(&body)?),
        T_CLOSE => ToEngine::Close,
        _ => return Err(bad("tipo di messaggio sconosciuto")),
    }))
}

pub fn read_from_engine(input: &mut impl Read) -> io::Result<Option<FromEngine>> {
    let Some((tag, body)) = read_raw(input)? else {
        return Ok(None);
    };
    Ok(Some(match tag {
        T_CONNECTED => FromEngine::Connected(from_json(&body)?),
        T_CERT => FromEngine::CertUntrusted(from_json(&body)?),
        T_FAILED => FromEngine::Failed(from_json(&body)?),
        T_UPDATE => {
            if body.len() < 8 {
                return Err(bad("aggiornamento troncato"));
            }
            let n = |i: usize| u16::from_le_bytes([body[i], body[i + 1]]);
            let (x, y, width, height) = (n(0), n(2), n(4), n(6));
            if body.len() - 8 != usize::from(width) * usize::from(height) * 4 {
                return Err(bad("dimensioni dell'aggiornamento incoerenti"));
            }
            FromEngine::Update {
                x,
                y,
                width,
                height,
                rgba: body[8..].to_vec(),
            }
        }
        T_DISCONNECTED => FromEngine::Disconnected(String::from_utf8_lossy(&body).into_owned()),
        _ => return Err(bad("tipo di messaggio sconosciuto")),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connect() -> Connect {
        Connect {
            host: "h".into(),
            port: 3389,
            username: "u".into(),
            password: "segreto-di-prova".into(),
            domain: Some("D".into()),
            width: 1280,
            height: 800,
            pinned_cert: None,
            keyboard_layout: 0x410,
        }
    }

    #[test]
    fn messages_to_the_engine_roundtrip() {
        let mut buf = Vec::new();
        write_to_engine(&mut buf, &ToEngine::Connect(connect())).unwrap();
        write_to_engine(&mut buf, &ToEngine::Input(Input::MouseMove { x: 5, y: 6 })).unwrap();
        write_to_engine(
            &mut buf,
            &ToEngine::Input(Input::Key {
                scancode: 0xE05B,
                pressed: true,
            }),
        )
        .unwrap();
        write_to_engine(&mut buf, &ToEngine::Close).unwrap();
        let mut r = buf.as_slice();
        match read_to_engine(&mut r).unwrap().unwrap() {
            ToEngine::Connect(c) => {
                assert_eq!((c.host.as_str(), c.port, c.width), ("h", 3389, 1280));
                assert_eq!(c.password, "segreto-di-prova");
            }
            m => panic!("{m:?}"),
        }
        assert!(matches!(
            read_to_engine(&mut r).unwrap().unwrap(),
            ToEngine::Input(Input::MouseMove { x: 5, y: 6 })
        ));
        assert!(matches!(
            read_to_engine(&mut r).unwrap().unwrap(),
            ToEngine::Input(Input::Key {
                scancode: 0xE05B,
                pressed: true
            })
        ));
        assert!(matches!(
            read_to_engine(&mut r).unwrap().unwrap(),
            ToEngine::Close
        ));
        assert!(read_to_engine(&mut r).unwrap().is_none());
    }

    #[test]
    fn messages_from_the_engine_roundtrip() {
        let mut buf = Vec::new();
        write_from_engine(
            &mut buf,
            &FromEngine::Connected(Connected {
                width: 3,
                height: 2,
            }),
        )
        .unwrap();
        write_from_engine(
            &mut buf,
            &FromEngine::CertUntrusted(CertUntrusted {
                fingerprint: "AA:BB".into(),
                changed: true,
            }),
        )
        .unwrap();
        write_from_engine(
            &mut buf,
            &FromEngine::Update {
                x: 1,
                y: 2,
                width: 2,
                height: 1,
                rgba: vec![1, 2, 3, 4, 5, 6, 7, 8],
            },
        )
        .unwrap();
        write_from_engine(&mut buf, &FromEngine::Disconnected("fine".into())).unwrap();
        let mut r = buf.as_slice();
        assert!(matches!(
            read_from_engine(&mut r).unwrap().unwrap(),
            FromEngine::Connected(Connected {
                width: 3,
                height: 2
            })
        ));
        match read_from_engine(&mut r).unwrap().unwrap() {
            FromEngine::CertUntrusted(c) => assert!(c.changed && c.fingerprint == "AA:BB"),
            m => panic!("{m:?}"),
        }
        match read_from_engine(&mut r).unwrap().unwrap() {
            FromEngine::Update {
                x,
                y,
                width,
                height,
                rgba,
            } => {
                assert_eq!((x, y, width, height), (1, 2, 2, 1));
                assert_eq!(rgba, vec![1, 2, 3, 4, 5, 6, 7, 8]);
            }
            m => panic!("{m:?}"),
        }
        match read_from_engine(&mut r).unwrap().unwrap() {
            FromEngine::Disconnected(s) => assert_eq!(s, "fine"),
            m => panic!("{m:?}"),
        }
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        // Lunghezza enorme.
        let mut r: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF, 0x20];
        assert!(read_from_engine(&mut r).is_err());
        // Aggiornamento con dimensioni incoerenti.
        let mut buf = Vec::new();
        put(&mut buf, T_UPDATE, &[1, 0, 0, 0, 9, 0, 9, 0, 1, 2]).unwrap();
        assert!(read_from_engine(&mut buf.as_slice()).is_err());
        // Tipo sconosciuto.
        let mut buf = Vec::new();
        put(&mut buf, 0x7F, b"x").unwrap();
        assert!(read_from_engine(&mut buf.as_slice()).is_err());
        assert!(read_to_engine(&mut buf.as_slice()).is_err());
    }

    #[test]
    fn the_password_is_hidden_in_debug_output() {
        assert!(!format!("{:?}", connect()).contains("segreto-di-prova"));
    }
}
