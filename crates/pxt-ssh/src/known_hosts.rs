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
}
