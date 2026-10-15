//! Log di audit (sezione 5.2): una riga JSON per evento, in `logs/audit.log`.
//! Contiene host, utente, protocollo ed esito. Mai credenziali né contenuti di sessione.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use serde::Serialize;

use crate::error::Result;

const DEFAULT_MAX_BYTES: u64 = 1024 * 1024;
const KEEP: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Ok,
    Failed,
}

/// Evento da registrare. `detail` deve essere un messaggio d'errore, mai un segreto.
#[derive(Debug, Clone, Serialize)]
pub struct AuditEvent {
    /// `connect`, `disconnect`, `hostkey_accepted`, `hostkey_changed`…
    pub event: String,
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub outcome: Outcome,
    /// Jump host attraversati, nell'ordine (`host:porta`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub via: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Serialize)]
struct Line<'a> {
    ts: String,
    #[serde(flatten)]
    event: &'a AuditEvent,
}

pub struct AuditLog {
    path: PathBuf,
    max_bytes: u64,
    lock: Mutex<()>,
}

impl AuditLog {
    /// `dir` è la cartella `logs/`; il file è `dir/audit.log`.
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self::with_limit(dir, DEFAULT_MAX_BYTES)
    }

    pub fn with_limit(dir: impl AsRef<Path>, max_bytes: u64) -> Self {
        Self {
            path: dir.as_ref().join("audit.log"),
            max_bytes,
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn record(&self, event: &AuditEvent) -> Result<()> {
        let _g = self.lock.lock().unwrap();
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        if fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0) >= self.max_bytes {
            self.rotate()?;
        }
        let line = serde_json::to_string(&Line {
            ts: Utc::now().to_rfc3339(),
            event,
        })?;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{line}")?;
        Ok(())
    }

    /// `audit.log` -> `audit.log.1` -> … -> `audit.log.5` (il più vecchio viene eliminato).
    fn rotate(&self) -> Result<()> {
        let numbered = |n: usize| {
            let mut s = self.path.clone().into_os_string();
            s.push(format!(".{n}"));
            PathBuf::from(s)
        };
        let oldest = numbered(KEEP);
        if oldest.exists() {
            fs::remove_file(&oldest)?;
        }
        for n in (1..KEEP).rev() {
            if numbered(n).exists() {
                fs::rename(numbered(n), numbered(n + 1))?;
            }
        }
        fs::rename(&self.path, numbered(1))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(detail: Option<&str>) -> AuditEvent {
        AuditEvent {
            event: "connect".into(),
            protocol: "ssh".into(),
            host: "10.0.0.1".into(),
            port: 22,
            user: "root".into(),
            outcome: if detail.is_some() {
                Outcome::Failed
            } else {
                Outcome::Ok
            },
            via: vec!["bastion:22".into()],
            detail: detail.map(String::from),
        }
    }

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("pxt-audit-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn writes_one_json_line_per_event_without_secret_fields() {
        let dir = temp_dir();
        let log = AuditLog::new(&dir);
        log.record(&event(None)).unwrap();
        log.record(&event(Some("autenticazione rifiutata")))
            .unwrap();
        let text = fs::read_to_string(log.path()).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["event"], "connect");
        assert_eq!(lines[0]["outcome"], "ok");
        assert_eq!(lines[0]["via"][0], "bastion:22");
        assert!(lines[0]["ts"].as_str().unwrap().contains('T'));
        assert!(lines[0].get("detail").is_none());
        assert_eq!(lines[1]["outcome"], "failed");
        assert_eq!(lines[1]["detail"], "autenticazione rifiutata");
        for l in &lines {
            for k in ["password", "passphrase", "secret"] {
                assert!(l.get(k).is_none());
            }
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rotates_by_size_and_keeps_five_old_files() {
        let dir = temp_dir();
        let log = AuditLog::with_limit(&dir, 200);
        for _ in 0..40 {
            log.record(&event(None)).unwrap();
        }
        let name = |n: usize| dir.join(format!("audit.log.{n}"));
        for n in 1..=5 {
            assert!(name(n).exists(), "audit.log.{n}");
        }
        assert!(!name(6).exists());
        assert!(log.path().exists());
        let _ = fs::remove_dir_all(dir);
    }
}
