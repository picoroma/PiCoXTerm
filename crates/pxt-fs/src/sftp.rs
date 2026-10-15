//! Backend SFTP su russh-sftp.

use russh_sftp::client::fs::Metadata;
use russh_sftp::client::SftpSession;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{Entry, EntryKind, FsCapabilities, FsError, RemoteFs, Result};

pub struct SftpFs {
    session: SftpSession,
}

impl SftpFs {
    /// Avvia il protocollo SFTP su un flusso già aperto verso il sottosistema `sftp` del server.
    pub async fn connect<S>(stream: S) -> Result<Self>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let session = SftpSession::new(stream)
            .await
            .map_err(|e| FsError::Sftp(e.to_string()))?;
        Ok(Self { session })
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Costruisce una voce dai metadati. `target` è l'esito di `stat` su un link simbolico.
fn entry_from(name: String, path: String, meta: &Metadata, target: Option<&Metadata>) -> Entry {
    let is_symlink = meta.is_symlink();
    // Un link si comporta come la cosa a cui punta; se non si risolve resta "altro".
    let effective = if is_symlink { target } else { Some(meta) };
    let kind = match effective {
        Some(m) if m.is_dir() => EntryKind::Dir,
        Some(m) if m.is_regular() => EntryKind::File,
        _ => EntryKind::Other,
    };
    let shown = effective.unwrap_or(meta);
    Entry {
        name,
        path,
        kind,
        is_symlink,
        size: shown.len(),
        modified: meta.mtime.map(i64::from),
        mode: meta.permissions.map(|p| p & 0o7777),
        owner: meta.user.clone(),
        group: meta.group.clone(),
        uid: meta.uid,
        gid: meta.gid,
    }
}

impl RemoteFs for SftpFs {
    type Reader = russh_sftp::client::fs::File;
    type Writer = russh_sftp::client::fs::File;

    async fn list(&self, path: &str) -> Result<Vec<Entry>> {
        let dir = self
            .session
            .read_dir(path)
            .await
            .map_err(|e| FsError::from_sftp(e, path))?;
        let mut out = Vec::new();
        for item in dir {
            let name = item.file_name();
            let full = join(path, &name);
            let meta = item.metadata();
            let target = if meta.is_symlink() {
                self.session.metadata(full.clone()).await.ok()
            } else {
                None
            };
            out.push(entry_from(name, full, &meta, target.as_ref()));
        }
        Ok(out)
    }

    async fn stat(&self, path: &str) -> Result<Entry> {
        let meta = self
            .session
            .metadata(path)
            .await
            .map_err(|e| FsError::from_sftp(e, path))?;
        let name = path.rsplit('/').find(|s| !s.is_empty()).unwrap_or("/");
        Ok(entry_from(name.to_string(), path.to_string(), &meta, None))
    }

    async fn canonicalize(&self, path: &str) -> Result<String> {
        self.session
            .canonicalize(path)
            .await
            .map_err(|e| FsError::from_sftp(e, path))
    }

    async fn mkdir(&self, path: &str) -> Result<()> {
        self.session
            .create_dir(path)
            .await
            .map_err(|e| FsError::from_sftp(e, path))
    }

    async fn remove_file(&self, path: &str) -> Result<()> {
        self.session
            .remove_file(path)
            .await
            .map_err(|e| FsError::from_sftp(e, path))
    }

    async fn remove_dir(&self, path: &str) -> Result<()> {
        self.session
            .remove_dir(path)
            .await
            .map_err(|e| FsError::from_sftp(e, path))
    }

    async fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.session
            .rename(from, to)
            .await
            .map_err(|e| FsError::from_sftp(e, from))
    }

    async fn set_permissions(&self, path: &str, mode: u32) -> Result<()> {
        let attrs = Metadata {
            permissions: Some(mode & 0o7777),
            ..Metadata::default()
        };
        self.session
            .set_metadata(path, attrs)
            .await
            .map_err(|e| FsError::from_sftp(e, path))
    }

    async fn set_modified(&self, path: &str, secs: i64) -> Result<()> {
        let t = u32::try_from(secs.max(0)).unwrap_or(u32::MAX);
        let attrs = Metadata {
            atime: Some(t),
            mtime: Some(t),
            ..Metadata::default()
        };
        self.session
            .set_metadata(path, attrs)
            .await
            .map_err(|e| FsError::from_sftp(e, path))
    }

    async fn open_read(&self, path: &str) -> Result<Self::Reader> {
        self.session
            .open(path)
            .await
            .map_err(|e| FsError::from_sftp(e, path))
    }

    async fn open_write(&self, path: &str) -> Result<Self::Writer> {
        self.session
            .create(path)
            .await
            .map_err(|e| FsError::from_sftp(e, path))
    }

    fn capabilities(&self) -> FsCapabilities {
        FsCapabilities {
            permissions: true,
            symlinks: true,
            owner_group: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh_sftp::client::fs::Metadata;

    fn attrs(permissions: u32, size: u64) -> Metadata {
        Metadata {
            size: Some(size),
            permissions: Some(permissions),
            mtime: Some(1_700_000_000),
            user: Some("root".into()),
            ..Metadata::default()
        }
    }

    #[test]
    fn join_handles_trailing_slash() {
        assert_eq!(join("/usr", "bin"), "/usr/bin");
        assert_eq!(join("/", "etc"), "/etc");
    }

    #[test]
    fn regular_file_and_directory_kinds() {
        let file = entry_from("a".into(), "/a".into(), &attrs(0o100644, 12), None);
        assert_eq!(file.kind, EntryKind::File);
        assert_eq!(
            (file.size, file.mode, file.is_symlink),
            (12, Some(0o644), false)
        );
        assert_eq!(file.owner.as_deref(), Some("root"));
        assert_eq!(file.modified, Some(1_700_000_000));

        let dir = entry_from("d".into(), "/d".into(), &attrs(0o040755, 4096), None);
        assert_eq!(dir.kind, EntryKind::Dir);
        assert_eq!(dir.mode, Some(0o755));
    }

    #[test]
    fn symlink_follows_its_target_kind_and_broken_links_are_other() {
        let link = attrs(0o120777, 9);
        let to_dir = attrs(0o040755, 4096);
        let e = entry_from("bin".into(), "/bin".into(), &link, Some(&to_dir));
        assert_eq!((e.kind, e.is_symlink), (EntryKind::Dir, true));

        let broken = entry_from("x".into(), "/x".into(), &link, None);
        assert_eq!((broken.kind, broken.is_symlink), (EntryKind::Other, true));
    }
}
