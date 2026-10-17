//! Nomi di utenti e gruppi per gli id numerici. Il protocollo SFTP (versione 3, quella di OpenSSH)
//! comunica solo `uid` e `gid`: per mostrare i nomi si leggono `/etc/passwd` e `/etc/group` del server
//! (due file piccoli). Gli utenti che non stanno lì (LDAP, SSSD...) restano numerici.

use std::collections::HashMap;

use serde::Serialize;
use tokio::io::AsyncReadExt;

use crate::RemoteFs;

/// Oltre questa dimensione il file non si legge: non è un file di utenti normale.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct IdNames {
    pub users: HashMap<u32, String>,
    pub groups: HashMap<u32, String>,
}

/// `nome:x:id:...` per riga; `id` è il terzo campo sia in `passwd` (uid) sia in `group` (gid).
fn parse(text: &str) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let mut fields = line.split(':');
        let (Some(name), Some(_), Some(id)) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        if let (false, Ok(id)) = (name.is_empty(), id.parse::<u32>()) {
            // In caso di duplicati vale il primo, come fa il sistema.
            out.entry(id).or_insert_with(|| name.to_string());
        }
    }
    out
}

pub fn parse_passwd(text: &str) -> HashMap<u32, String> {
    parse(text)
}

pub fn parse_group(text: &str) -> HashMap<u32, String> {
    parse(text)
}

async fn read_small<F: RemoteFs>(fs: &F, path: &str) -> Option<String> {
    let size = fs.stat(path).await.ok()?.size;
    if size > MAX_BYTES {
        return None;
    }
    let mut reader = fs.open_read(path).await.ok()?;
    let mut bytes = Vec::new();
    (&mut reader)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .ok()?;
    String::from_utf8(bytes).ok()
}

/// Legge i nomi dal server. Se i file mancano o non si leggono l'elenco corrispondente è vuoto.
pub async fn load<F: RemoteFs>(fs: &F) -> IdNames {
    IdNames {
        users: read_small(fs, "/etc/passwd")
            .await
            .map(|t| parse_passwd(&t))
            .unwrap_or_default(),
        groups: read_small(fs, "/etc/group")
            .await
            .map(|t| parse_group(&t))
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwd_and_group_lines_are_parsed() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\n# commento\nbgpadm:x:1001:79:SAP:/home/bgpadm:/bin/csh\nrotta\n:x:5:5::/:/bin/sh\nx:x:abc:1::\n";
        let users = parse_passwd(passwd);
        assert_eq!(users.get(&0).map(String::as_str), Some("root"));
        assert_eq!(users.get(&1001).map(String::as_str), Some("bgpadm"));
        assert_eq!(users.len(), 2, "{users:?}");
        let groups = parse_group("sapsys:x:79:bgpadm,root\nusers:x:100:\n");
        assert_eq!(groups.get(&79).map(String::as_str), Some("sapsys"));
        assert_eq!(groups.get(&100).map(String::as_str), Some("users"));
    }

    #[test]
    fn the_first_entry_wins_for_duplicate_ids() {
        let users = parse_passwd("root:x:0:0::/:/bin/sh\ntoor:x:0:0::/:/bin/sh\n");
        assert_eq!(users.get(&0).map(String::as_str), Some("root"));
    }
}
