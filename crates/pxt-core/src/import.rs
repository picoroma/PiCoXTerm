//! Importazione di sessioni da altri programmi (PuTTY, MobaXterm). Qui si legge solo la struttura:
//! host, porta, utente, file della chiave, cartelle. Nessuna password passa da questo modulo.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;
use uuid::Uuid;

use crate::model::{Auth, AuthMethod, Folder, Protocol, Session, SessionNode};

/// Una sessione trovata nel file sorgente, pronta per l'anteprima.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    /// Cartelle in cui sta nel programma di origine, dalla più esterna.
    pub folder: Vec<String>,
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub key_path: Option<String>,
}

/// Una voce che non si può importare, con il motivo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub candidates: Vec<Candidate>,
    pub skipped: Vec<Skipped>,
}

/// Testo di un file di un altro programma: UTF-16 LE con BOM (i `.reg`), UTF-8 con o senza BOM,
/// altrimenti si legge come UTF-8 sostituendo i byte illeggibili.
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

/// Decodifica `%XX` (PuTTY codifica così i nomi delle sessioni).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------- PuTTY ----------------

/// Valore di una riga `"Nome"="testo"` o `"Nome"=dword:0000001a` di un file `.reg`.
fn reg_value(raw: &str) -> Option<String> {
    if let Some(hex) = raw.strip_prefix("dword:") {
        return u32::from_str_radix(hex.trim(), 16)
            .ok()
            .map(|n| n.to_string());
    }
    let inner = raw.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Sessioni PuTTY da un file `.reg` (esportato dal ramo `...\PuTTY\Sessions` o da tutto `...\PuTTY`).
pub fn parse_putty_reg(text: &str) -> Scan {
    let mut sessions: Vec<(String, HashMap<String, String>)> = Vec::new();
    let mut current: Option<usize> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = None;
            let lower = header.to_ascii_lowercase();
            if let Some(i) = lower.find("\\putty\\sessions\\") {
                let name = &header[i + "\\putty\\sessions\\".len()..];
                if !name.contains('\\') && !name.is_empty() {
                    sessions.push((percent_decode(name), HashMap::new()));
                    current = Some(sessions.len() - 1);
                }
            }
        } else if let (Some(idx), Some((k, v))) = (current, line.split_once("\"=")) {
            if let (Some(key), Some(val)) = (k.strip_prefix('"'), reg_value(v)) {
                sessions[idx].1.insert(key.to_string(), val);
            }
        }
    }
    putty_scan(sessions)
}

/// Sessioni PuTTY di una cartella `~/.putty/sessions` (un file per sessione, righe `Chiave=Valore`).
pub fn parse_putty_dir(dir: &Path) -> std::io::Result<Scan> {
    let mut sessions = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if !e.path().is_file() {
            continue;
        }
        let name = percent_decode(&e.file_name().to_string_lossy());
        let text = decode_text(&std::fs::read(e.path())?);
        let props = text
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim_end_matches('\r').to_string()))
            .collect();
        sessions.push((name, props));
    }
    Ok(putty_scan(sessions))
}

fn putty_scan(sessions: Vec<(String, HashMap<String, String>)>) -> Scan {
    let mut scan = Scan::default();
    for (name, p) in sessions {
        if name == "Default Settings" {
            continue;
        }
        let protocol = p.get("Protocol").map(String::as_str).unwrap_or("ssh");
        if protocol != "ssh" {
            scan.skipped.push(Skipped {
                name,
                reason: format!("protocol \"{protocol}\" is not supported"),
            });
            continue;
        }
        let mut host = p.get("HostName").cloned().unwrap_or_default();
        let mut username = p.get("UserName").cloned().unwrap_or_default();
        if let Some((u, h)) = host.clone().split_once('@') {
            if username.is_empty() {
                username = u.to_string();
            }
            host = h.to_string();
        }
        if host.trim().is_empty() {
            scan.skipped.push(Skipped {
                name,
                reason: "no host name".into(),
            });
            continue;
        }
        let port = p
            .get("PortNumber")
            .and_then(|s| s.parse().ok())
            .filter(|&n| n != 0)
            .unwrap_or(22);
        let key_path = p.get("PublicKeyFile").filter(|s| !s.is_empty()).cloned();
        scan.candidates.push(Candidate {
            folder: Vec::new(),
            name,
            protocol: Protocol::Ssh,
            host: host.trim().to_string(),
            port,
            username,
            key_path,
        });
    }
    scan
}

// ---------------- MobaXterm ----------------

/// Sessioni da `MobaXterm.ini` o da un export `.mxtsessions`: sezioni `[Bookmarks]`, `[Bookmarks_1]`…
/// con `SubRep=` (la cartella) e righe `nome=#icona#tipo%host%porta%utente%…#…`.
pub fn parse_mobaxterm_ini(text: &str) -> Scan {
    let mut scan = Scan::default();
    let mut in_bookmarks = false;
    let mut folder: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(sec) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_bookmarks = sec == "Bookmarks" || sec.starts_with("Bookmarks_");
            folder.clear();
            continue;
        }
        if !in_bookmarks {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key == "SubRep" {
            folder = value
                .split('\\')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            continue;
        }
        if key == "ImgNum" || !value.starts_with('#') {
            continue;
        }
        match mobaxterm_entry(key, &folder, value) {
            Ok(c) => scan.candidates.push(c),
            Err(s) => scan.skipped.push(s),
        }
    }
    scan
}

/// Una riga di sessione: `#icona#tipo%host%porta%utente%…#impostazioni#…`.
fn mobaxterm_entry(name: &str, folder: &[String], value: &str) -> Result<Candidate, Skipped> {
    let skip = |reason: String| Skipped {
        name: folder
            .iter()
            .map(String::as_str)
            .chain([name])
            .collect::<Vec<_>>()
            .join("/"),
        reason,
    };
    let fields: Vec<&str> = value.split('#').nth(2).unwrap_or("").split('%').collect();
    let kind = fields.first().copied().unwrap_or("");
    let (protocol, default_port) = match kind {
        "0" => (Protocol::Ssh, 22),
        "4" => (Protocol::Rdp, 3389),
        "5" => (Protocol::Vnc, 5900),
        "6" => (Protocol::Ftp, 21),
        "7" => (Protocol::Sftp, 22),
        other => {
            let what = match other {
                "1" => "Telnet",
                "2" => "Rsh",
                "3" => "XDMCP",
                "8" => "serial",
                "9" => "file",
                "10" => "local shell",
                "11" => "browser",
                "12" => "Cygwin",
                "13" => "Mosh",
                "14" => "S3",
                "15" => "WSL",
                _ => "unknown type",
            };
            return Err(skip(format!("{what} sessions are not imported")));
        }
    };
    let host = fields.get(1).map(|s| s.trim()).unwrap_or("");
    if host.is_empty() {
        return Err(skip("no host name".into()));
    }
    let port = fields
        .get(2)
        .and_then(|s| s.trim().parse().ok())
        .filter(|&n| n != 0)
        .unwrap_or(default_port);
    let username = fields
        .get(3)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != "<default>")
        .unwrap_or("")
        .to_string();
    // Il file della chiave sta in una posizione che cambia con la versione: si cerca il campo che ha
    // l'aspetto di un file di chiave.
    let key_path = if matches!(protocol, Protocol::Ssh | Protocol::Sftp) {
        fields.iter().skip(4).find_map(|f| {
            let l = f.to_ascii_lowercase();
            let looks = l.ends_with(".ppk")
                || l.ends_with(".pem")
                || l.ends_with(".key")
                || l.rsplit(['\\', '/'])
                    .next()
                    .is_some_and(|n| n.starts_with("id_"));
            looks.then(|| f.to_string())
        })
    } else {
        None
    };
    Ok(Candidate {
        folder: folder.to_vec(),
        name: name.to_string(),
        protocol,
        host: host.to_string(),
        port,
        username,
        key_path,
    })
}

// ---------------- Costruzione dei nodi ----------------

/// Nodi da salvare: tutto sotto una cartella nuova (niente sovrascritture), con le cartelle di origine
/// ricreate sotto di essa.
pub fn build_nodes(candidates: &[Candidate], root_name: &str) -> Vec<SessionNode> {
    let root = Folder {
        id: Uuid::new_v4(),
        name: root_name.to_string(),
        parent_id: None,
    };
    let mut out = vec![SessionNode::Folder(root.clone())];
    let mut folders: HashMap<Vec<String>, Uuid> = HashMap::new();
    for c in candidates {
        let mut parent = root.id;
        for depth in 1..=c.folder.len() {
            let path = c.folder[..depth].to_vec();
            parent = *folders.entry(path).or_insert_with(|| {
                let id = Uuid::new_v4();
                out.push(SessionNode::Folder(Folder {
                    id,
                    name: c.folder[depth - 1].clone(),
                    parent_id: Some(parent),
                }));
                id
            });
        }
        let mut s = Session::new_ssh(&c.name, &c.host, c.port, &c.username);
        s.parent_id = Some(parent);
        s.protocol = c.protocol;
        if let Some(k) = &c.key_path {
            s.auth = Auth {
                method: AuthMethod::Key,
                key_path: Some(k.clone()),
            };
        }
        out.push(SessionNode::Session(Box::new(s)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const REG: &str = "Windows Registry Editor Version 5.00\r\n\r\n\
[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\Default%20Settings]\r\n\"HostName\"=\"\"\r\n\r\n\
[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\Web%20srv%20%231]\r\n\"HostName\"=\"admin@10.0.0.5\"\r\n\"Protocol\"=\"ssh\"\r\n\"PortNumber\"=dword:000008ae\r\n\"PublicKeyFile\"=\"C:\\\\keys\\\\web.ppk\"\r\n\r\n\
[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\Switch]\r\n\"HostName\"=\"sw1\"\r\n\"Protocol\"=\"telnet\"\r\n\r\n\
[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\SshHostKeys]\r\n\"rsa2@22:x\"=\"0x1\"\r\n";

    #[test]
    fn putty_reg_sessions() {
        let s = parse_putty_reg(REG);
        assert_eq!(s.candidates.len(), 1);
        let c = &s.candidates[0];
        assert_eq!(c.name, "Web srv #1");
        assert_eq!(
            (c.host.as_str(), c.port, c.username.as_str()),
            ("10.0.0.5", 2222, "admin")
        );
        assert_eq!(c.key_path.as_deref(), Some("C:\\keys\\web.ppk"));
        assert_eq!(s.skipped.len(), 1);
        assert_eq!(s.skipped[0].name, "Switch");
    }

    #[test]
    fn utf16_reg_is_decoded() {
        let mut bytes = vec![0xFF, 0xFE];
        for u in "[a]".encode_utf16() {
            bytes.extend(u.to_le_bytes());
        }
        assert_eq!(decode_text(&bytes), "[a]");
    }

    const INI: &str = "[Misc]\r\nx=1\r\n[Bookmarks]\r\nSubRep=\r\nImgNum=42\r\nrouter=#109#0%192.168.1.1%22%admin%%-1%-1#MobaXterm#0#\r\n\
[Bookmarks_1]\r\nSubRep=Clienti\\Acme\r\nImgNum=41\r\ndb=#109#0%db.acme.it%2200%dba%%-1%-1%%%%%0%0%0%C:\\Users\\me\\.ssh\\id_rsa.ppk%%1#x#0#\r\nwin=#91#4%10.1.1.9%3389%Administrator%%-1#x#\r\nold=#98#1%host%23%#x#\r\n\
[Passwords]\r\nadmin@192.168.1.1=secret\r\n";

    #[test]
    fn mobaxterm_sessions_and_folders() {
        let s = parse_mobaxterm_ini(INI);
        assert_eq!(s.candidates.len(), 3);
        assert_eq!(s.candidates[0].name, "router");
        assert!(s.candidates[0].folder.is_empty());
        let db = &s.candidates[1];
        assert_eq!(db.folder, vec!["Clienti", "Acme"]);
        assert_eq!((db.port, db.username.as_str()), (2200, "dba"));
        assert_eq!(
            db.key_path.as_deref(),
            Some("C:\\Users\\me\\.ssh\\id_rsa.ppk")
        );
        assert_eq!(s.candidates[2].protocol, Protocol::Rdp);
        assert_eq!(s.skipped.len(), 1);
        assert!(s.skipped[0].reason.contains("Telnet"));
    }

    #[test]
    fn nodes_recreate_folders_once() {
        let s = parse_mobaxterm_ini(INI);
        let nodes = build_nodes(&s.candidates, "Imported");
        let folders = nodes
            .iter()
            .filter(|n| matches!(n, SessionNode::Folder(_)))
            .count();
        assert_eq!(folders, 3); // Imported, Clienti, Acme
        let db = nodes
            .iter()
            .find_map(|n| match n {
                SessionNode::Session(s) if s.name == "db" => Some(s),
                _ => None,
            })
            .unwrap();
        assert_eq!(db.auth.method, AuthMethod::Key);
        assert!(db.credential_ref.is_none());
    }
}
