use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::SshError;

/// File `known_hosts` proprio di PiCoXterm.
/// Una riga per voce: `[host]:porta tipo-chiave SHA256:fingerprint`.
#[derive(Debug, Clone)]
pub struct KnownHosts {
    path: PathBuf,
}

/// Una voce salvata, per mostrarla all'utente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownHostEntry {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HostKeyStatus {
    Known,
    Unknown,
    /// Il server è già noto ma con un'altra chiave; contiene la fingerprint attesa.
    Changed(String),
}

impl KnownHosts {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn host_id(host: &str, port: u16) -> String {
        format!("[{host}]:{port}")
    }

    pub fn check(
        &self,
        host: &str,
        port: u16,
        key_type: &str,
        fp: &str,
    ) -> Result<HostKeyStatus, SshError> {
        let content = match fs::read_to_string(&self.path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(HostKeyStatus::Unknown)
            }
            Err(e) => return Err(e.into()),
        };
        let id = Self::host_id(host, port);
        let mut expected = None;
        for line in content.lines() {
            let mut parts = line.split_whitespace();
            let (Some(h), Some(t), Some(f)) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            if h != id {
                continue;
            }
            if t == key_type && f == fp {
                return Ok(HostKeyStatus::Known);
            }
            expected.get_or_insert_with(|| f.to_string());
        }
        Ok(match expected {
            Some(e) => HostKeyStatus::Changed(e),
            None => HostKeyStatus::Unknown,
        })
    }

    /// Tutte le voci salvate.
    pub fn list(&self) -> Result<Vec<KnownHostEntry>, SshError> {
        let content = match fs::read_to_string(&self.path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        Ok(content.lines().filter_map(parse_line).collect())
    }

    /// Elimina tutte le voci di un host. Restituisce quante ne ha tolte.
    pub fn remove(&self, host: &str, port: u16) -> Result<usize, SshError> {
        let content = match fs::read_to_string(&self.path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.into()),
        };
        let id = Self::host_id(host, port);
        let (gone, kept): (Vec<&str>, Vec<&str>) = content
            .lines()
            .partition(|l| l.split_whitespace().next() == Some(id.as_str()));
        if gone.is_empty() {
            return Ok(0);
        }
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let mut out = kept.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        fs::write(&tmp, out)?;
        fs::rename(&tmp, &self.path)?;
        Ok(gone.len())
    }

    pub fn add(&self, host: &str, port: u16, key_type: &str, fp: &str) -> Result<(), SshError> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{} {key_type} {fp}", Self::host_id(host, port))?;
        Ok(())
    }
}

/// `[host]:porta tipo SHA256:...` -> voce.
fn parse_line(line: &str) -> Option<KnownHostEntry> {
    let mut parts = line.split_whitespace();
    let (id, key_type, fp) = (parts.next()?, parts.next()?, parts.next()?);
    let (host, port) = id.strip_prefix('[')?.rsplit_once("]:")?;
    Some(KnownHostEntry {
        host: host.to_string(),
        port: port.parse().ok()?,
        key_type: key_type.to_string(),
        fingerprint: fp.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_then_known_then_changed() {
        let dir = std::env::temp_dir().join(format!("pxt-kh-{}", std::process::id()));
        let kh = KnownHosts::new(dir.join("known_hosts"));
        assert_eq!(
            kh.check("h", 22, "ssh-ed25519", "SHA256:a").unwrap(),
            HostKeyStatus::Unknown
        );
        kh.add("h", 22, "ssh-ed25519", "SHA256:a").unwrap();
        assert_eq!(
            kh.check("h", 22, "ssh-ed25519", "SHA256:a").unwrap(),
            HostKeyStatus::Known
        );
        assert_eq!(
            kh.check("h", 22, "ssh-ed25519", "SHA256:b").unwrap(),
            HostKeyStatus::Changed("SHA256:a".into())
        );
        assert_eq!(
            kh.check("h", 2222, "ssh-ed25519", "SHA256:b").unwrap(),
            HostKeyStatus::Unknown
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn list_and_remove_entries() {
        let dir = std::env::temp_dir().join(format!("pxt-kh-list-{}", std::process::id()));
        let kh = KnownHosts::new(dir.join("known_hosts"));
        assert!(kh.list().unwrap().is_empty());
        assert_eq!(kh.remove("a", 22).unwrap(), 0);
        kh.add("a", 22, "ssh-ed25519", "SHA256:1").unwrap();
        kh.add("a", 22, "ssh-rsa", "SHA256:2").unwrap();
        kh.add("b", 2222, "ssh-ed25519", "SHA256:3").unwrap();
        assert_eq!(kh.list().unwrap().len(), 3);
        assert_eq!(kh.list().unwrap()[2].port, 2222);

        assert_eq!(kh.remove("a", 22).unwrap(), 2);
        let left = kh.list().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].host, "b");
        // dopo la rimozione l'host torna "sconosciuto"
        assert_eq!(
            kh.check("a", 22, "ssh-ed25519", "SHA256:9").unwrap(),
            HostKeyStatus::Unknown
        );
        let _ = fs::remove_dir_all(dir);
    }
}
