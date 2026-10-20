//! Avvio del client RDP del sistema (D-08, fase 1): si genera un file `.rdp` temporaneo e si lancia
//! `mstsc` (Windows), Windows App (macOS) o `xfreerdp` (Linux). Nessun emulatore RDP qui dentro.
//!
//! **Segreti.** Il file `.rdp` non contiene mai la password. Su Windows la password passa dal
//! Credential Manager con una voce `TERMSRV/<host>` di sessione (sparisce comunque al logoff) che si
//! rimuove poco dopo l'avvio; non compare mai su una riga di comando né in un log.

use std::path::PathBuf;
use std::time::Duration;

#[cfg(windows)]
mod credman;
mod engine;

pub use engine::{
    engine_path, RdpEvent, RdpHandle, RdpInput, RdpSession, SessionParams, ENGINE_NAME,
};

#[derive(Debug, thiserror::Error)]
pub enum RdpError {
    #[error("valore non valido per {0}: caratteri di controllo non ammessi")]
    InvalidValue(&'static str),
    #[error("risoluzione non valida (atteso LARGHEZZAxALTEZZA, per esempio 1920x1080)")]
    InvalidResolution,
    #[error("nessun client RDP trovato: {0}")]
    NoClient(String),
    #[error("impossibile avviare il client RDP: {0}")]
    Launch(String),
    #[error("credenziali: {0}")]
    Credentials(String),
    #[error("il motore RDP non si trova: {0}")]
    EngineMissing(String),
    #[error("connessione RDP: {0}")]
    Connect(String),
    #[error("autenticazione RDP rifiutata o fallita: {0}")]
    Auth(String),
    /// Il certificato del server non è verificabile e non è fissato: decide l'utente.
    #[error("certificato del server non verificabile (SHA-256 {fingerprint})")]
    UntrustedCertificate { fingerprint: String, changed: bool },
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, RdpError>;

/// Client da usare; `Auto` sceglie quello abituale del sistema operativo.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Client {
    #[default]
    Auto,
    Mstsc,
    WindowsApp,
    FreeRdp,
}

#[derive(Debug, Clone, Default)]
pub struct RdpParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Dominio; se presente l'utente diventa `DOMINIO\utente`.
    pub domain: Option<String>,
    pub client: Client,
    pub fullscreen: bool,
    pub multimon: bool,
    /// `LARGHEZZAxALTEZZA`; vuoto = come la finestra.
    pub resolution: Option<String>,
    pub redirect_clipboard: bool,
    /// Condivide le unità del computer con il server: disattivato di norma.
    pub redirect_drives: bool,
    pub gateway: Option<String>,
}

/// Niente a capo né caratteri di controllo: un valore con `\n` potrebbe aggiungere righe al file `.rdp`
/// (per esempio `alternate shell`, che esegue un programma).
fn clean(name: &'static str, v: &str) -> Result<()> {
    if v.chars().any(char::is_control) {
        Err(RdpError::InvalidValue(name))
    } else {
        Ok(())
    }
}

fn parse_resolution(r: &str) -> Result<(u32, u32)> {
    let (w, h) = r
        .trim()
        .split_once(['x', 'X'])
        .ok_or(RdpError::InvalidResolution)?;
    let (w, h): (u32, u32) = (
        w.trim().parse().map_err(|_| RdpError::InvalidResolution)?,
        h.trim().parse().map_err(|_| RdpError::InvalidResolution)?,
    );
    if (200..=16384).contains(&w) && (200..=16384).contains(&h) {
        Ok((w, h))
    } else {
        Err(RdpError::InvalidResolution)
    }
}

/// Contenuto del file `.rdp`. `credentials_stored`: la password è già nel Credential Manager, quindi
/// il client non deve chiederla.
pub fn build_rdp_file(p: &RdpParams, credentials_stored: bool) -> Result<String> {
    clean("host", &p.host)?;
    clean("utente", &p.username)?;
    if let Some(d) = &p.domain {
        clean("dominio", d)?;
    }
    if let Some(g) = &p.gateway {
        clean("gateway", g)?;
    }
    let mut out = String::new();
    let port = if p.port == 0 { 3389 } else { p.port };
    out.push_str(&format!("full address:s:{}:{port}\r\n", p.host.trim()));
    let user = p.username.trim();
    if !user.is_empty() {
        match p.domain.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            Some(d) if !user.contains('\\') && !user.contains('@') => {
                out.push_str(&format!("username:s:{d}\\{user}\r\n"));
            }
            _ => out.push_str(&format!("username:s:{user}\r\n")),
        }
    }
    out.push_str(&format!(
        "screen mode id:i:{}\r\n",
        if p.fullscreen { 2 } else { 1 }
    ));
    out.push_str(&format!("use multimon:i:{}\r\n", u8::from(p.multimon)));
    if let Some(r) = p.resolution.as_deref().filter(|r| !r.trim().is_empty()) {
        let (w, h) = parse_resolution(r)?;
        out.push_str(&format!("desktopwidth:i:{w}\r\ndesktopheight:i:{h}\r\n"));
    }
    out.push_str(&format!(
        "redirectclipboard:i:{}\r\n",
        u8::from(p.redirect_clipboard)
    ));
    if p.redirect_drives {
        out.push_str("drivestoredirect:s:*\r\n");
    }
    if let Some(g) = p
        .gateway
        .as_deref()
        .map(str::trim)
        .filter(|g| !g.is_empty())
    {
        out.push_str(&format!(
            "gatewayhostname:s:{g}\r\ngatewayusagemethod:i:1\r\n"
        ));
    }
    // NLA/CredSSP attivi e verifica del certificato del server a cura del client.
    out.push_str("enablecredsspsupport:i:1\r\nauthentication level:i:2\r\n");
    out.push_str(&format!(
        "prompt for credentials:i:{}\r\n",
        u8::from(!credentials_stored)
    ));
    Ok(out)
}

/// Un'apertura in corso: tiene il file temporaneo e la voce delle credenziali, e li toglie con `finish`
/// (o, in ogni caso, quando viene lasciata cadere).
pub struct Launched {
    dir: Option<PathBuf>,
    #[cfg(windows)]
    credential: Option<String>,
}

impl Launched {
    /// Rimuove il file `.rdp` temporaneo e la voce `TERMSRV/<host>`.
    pub fn finish(&mut self) {
        if let Some(dir) = self.dir.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
        #[cfg(windows)]
        if let Some(target) = self.credential.take() {
            let _ = credman::delete(&target);
        }
    }
}

impl Drop for Launched {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Quanto aspettare prima di togliere le credenziali: il client le legge nei primi secondi.
pub const CLEANUP_DELAY: Duration = Duration::from_secs(30);

fn temp_dir() -> Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("picoxterm-rdp-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Avvia il client RDP. `password` è la password dell'utente, se nota. Restituisce un oggetto da
/// tenere finché il client può ancora leggere file e credenziali (poi `finish`, vedi `CLEANUP_DELAY`).
pub fn launch(p: &RdpParams, password: Option<&str>) -> Result<Launched> {
    #[cfg(windows)]
    {
        launch_windows(p, password)
    }
    #[cfg(not(windows))]
    {
        launch_unix(p, password)
    }
}

#[cfg(windows)]
fn launch_windows(p: &RdpParams, password: Option<&str>) -> Result<Launched> {
    let target = format!("TERMSRV/{}", p.host.trim());
    let mut stored = false;
    let mut credential = None;
    if let (Some(pw), false) = (password, p.username.trim().is_empty()) {
        let user = match p.domain.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            Some(d) if !p.username.contains('\\') && !p.username.contains('@') => {
                format!("{d}\\{}", p.username.trim())
            }
            _ => p.username.trim().to_string(),
        };
        credman::write(&target, &user, pw).map_err(RdpError::Credentials)?;
        credential = Some(target);
        stored = true;
    }
    let content = build_rdp_file(p, stored);
    let dir = match content.and_then(|c| temp_dir().map(|d| (c, d))) {
        Ok((c, d)) => {
            std::fs::write(d.join("session.rdp"), c)?;
            d
        }
        Err(e) => {
            if let Some(t) = &credential {
                let _ = credman::delete(t);
            }
            return Err(e);
        }
    };
    let mut launched = Launched {
        dir: Some(dir.clone()),
        credential,
    };
    let file = dir.join("session.rdp");
    let program = match p.client {
        Client::FreeRdp => "wfreerdp.exe",
        _ => "mstsc.exe",
    };
    let result = if p.client == Client::FreeRdp {
        std::process::Command::new(program)
            .arg(format!(
                "/v:{}:{}",
                p.host.trim(),
                if p.port == 0 { 3389 } else { p.port }
            ))
            .spawn()
    } else {
        std::process::Command::new(program).arg(&file).spawn()
    };
    match result {
        Ok(_) => Ok(launched),
        Err(e) => {
            launched.finish();
            Err(RdpError::NoClient(format!("{program}: {e}")))
        }
    }
}

#[cfg(not(windows))]
fn launch_unix(p: &RdpParams, _password: Option<&str>) -> Result<Launched> {
    // Su macOS/Linux non c'è un modo sicuro e uniforme di passare la password: il client la chiede.
    let content = build_rdp_file(p, false)?;
    let dir = temp_dir()?;
    let file = dir.join("session.rdp");
    std::fs::write(&file, content)?;
    let mut launched = Launched { dir: Some(dir) };
    let port = if p.port == 0 { 3389 } else { p.port };
    let spawned = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&file).spawn()
    } else {
        let mut c = std::process::Command::new(
            if std::process::Command::new("sh")
                .args(["-c", "command -v xfreerdp >/dev/null"])
                .status()
                .is_ok_and(|s| s.success())
            {
                "xfreerdp"
            } else {
                "wlfreerdp"
            },
        );
        c.arg(format!("/v:{}:{port}", p.host.trim()));
        if !p.username.trim().is_empty() {
            c.arg(format!("/u:{}", p.username.trim()));
        }
        if let Some(d) = p.domain.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            c.arg(format!("/d:{d}"));
        }
        if p.fullscreen {
            c.arg("/f");
        }
        if p.redirect_clipboard {
            c.arg("+clipboard");
        }
        c.spawn()
    };
    match spawned {
        Ok(_) => Ok(launched),
        Err(e) => {
            launched.finish();
            Err(RdpError::NoClient(e.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> RdpParams {
        RdpParams {
            host: "srv.example.com".into(),
            port: 3389,
            username: "mario".into(),
            redirect_clipboard: true,
            ..Default::default()
        }
    }

    #[test]
    fn builds_a_minimal_file_without_any_password() {
        let f = build_rdp_file(&base(), true).unwrap();
        assert!(f.contains("full address:s:srv.example.com:3389\r\n"));
        assert!(f.contains("username:s:mario\r\n"));
        assert!(f.contains("screen mode id:i:1\r\n"));
        assert!(f.contains("redirectclipboard:i:1\r\n"));
        assert!(f.contains("prompt for credentials:i:0\r\n"));
        assert!(!f.to_lowercase().contains("password"));
        assert!(!f.contains("drivestoredirect"));
    }

    #[test]
    fn prompts_when_no_credentials_are_stored() {
        assert!(build_rdp_file(&base(), false)
            .unwrap()
            .contains("prompt for credentials:i:1\r\n"));
    }

    #[test]
    fn domain_prefixes_the_user_unless_already_qualified() {
        let mut p = base();
        p.domain = Some("CORP".into());
        assert!(build_rdp_file(&p, true)
            .unwrap()
            .contains("username:s:CORP\\mario\r\n"));
        p.username = "mario@corp.local".into();
        assert!(build_rdp_file(&p, true)
            .unwrap()
            .contains("username:s:mario@corp.local\r\n"));
    }

    #[test]
    fn options_map_to_rdp_keys() {
        let mut p = base();
        p.fullscreen = true;
        p.multimon = true;
        p.resolution = Some("1920x1080".into());
        p.redirect_drives = true;
        p.gateway = Some("gw.example.com".into());
        p.port = 0;
        let f = build_rdp_file(&p, true).unwrap();
        assert!(f.contains("full address:s:srv.example.com:3389\r\n"));
        assert!(f.contains("screen mode id:i:2\r\n"));
        assert!(f.contains("use multimon:i:1\r\n"));
        assert!(f.contains("desktopwidth:i:1920\r\ndesktopheight:i:1080\r\n"));
        assert!(f.contains("drivestoredirect:s:*\r\n"));
        assert!(f.contains("gatewayhostname:s:gw.example.com\r\ngatewayusagemethod:i:1\r\n"));
        assert!(f.contains("enablecredsspsupport:i:1\r\n"));
    }

    #[test]
    fn values_that_could_inject_rdp_lines_are_refused() {
        for evil in ["h\nalternate shell:s:calc.exe", "h\r\nx", "h\0"] {
            let mut p = base();
            p.host = evil.into();
            assert!(build_rdp_file(&p, true).is_err(), "{evil:?}");
        }
        let mut p = base();
        p.username = "u\nalternate shell:s:calc.exe".into();
        assert!(build_rdp_file(&p, true).is_err());
        let mut p = base();
        p.domain = Some("d\ne".into());
        assert!(build_rdp_file(&p, true).is_err());
        let mut p = base();
        p.gateway = Some("g\ne".into());
        assert!(build_rdp_file(&p, true).is_err());
    }

    #[test]
    fn resolution_is_validated() {
        for bad in ["abc", "1920", "1920x", "x1080", "10x10", "99999x99999"] {
            let mut p = base();
            p.resolution = Some(bad.into());
            assert!(build_rdp_file(&p, true).is_err(), "{bad}");
        }
        let mut p = base();
        p.resolution = Some("  ".into());
        assert!(build_rdp_file(&p, true).is_ok());
    }
}
