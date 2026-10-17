//! Backend del filesystem locale: serve al pannello locale del file browser a doppio pannello.
//! I percorsi sono quelli nativi del sistema (su Windows `C:\Users\...`, con la lista dei dischi come
//! radice); i trasferimenti verso un filesystem remoto restano quelli del motore (`transfer`), che
//! legge e scrive già i file locali in streaming.

use std::io::SeekFrom;
use std::path::{Path, PathBuf};

use tokio::io::AsyncSeekExt;

use crate::{Entry, EntryKind, FsCapabilities, FsError, RemoteFs, Result};

#[derive(Debug, Clone, Copy, Default)]
pub struct LocalFs;

fn io_err(e: std::io::Error, path: &str) -> FsError {
    use std::io::ErrorKind;
    match e.kind() {
        ErrorKind::NotFound => FsError::NotFound(path.to_string()),
        ErrorKind::PermissionDenied => FsError::PermissionDenied(path.to_string()),
        _ => FsError::Local(format!("{path}: {e}")),
    }
}

/// Toglie il prefisso `\\?\` che `canonicalize` mette ai percorsi di Windows.
fn plain(path: &Path) -> String {
    let s = path.to_string_lossy().into_owned();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => rest.to_string(),
        _ => s,
    }
}

fn secs(t: std::time::SystemTime) -> Option<i64> {
    t.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
}

#[cfg(unix)]
fn mode_of(meta: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn mode_of(_: &std::fs::Metadata) -> Option<u32> {
    None
}

#[cfg(unix)]
fn ids_of(meta: &std::fs::Metadata) -> (Option<u32>, Option<u32>) {
    use std::os::unix::fs::MetadataExt;
    (Some(meta.uid()), Some(meta.gid()))
}

#[cfg(not(unix))]
fn ids_of(_: &std::fs::Metadata) -> (Option<u32>, Option<u32>) {
    (None, None)
}

/// Voce a partire dal percorso. Un link simbolico si comporta come la cosa a cui punta; se non si
/// risolve resta "altro".
fn entry_for(path: &Path) -> std::io::Result<Entry> {
    let meta = std::fs::symlink_metadata(path)?;
    let is_symlink = meta.file_type().is_symlink();
    let target = if is_symlink {
        std::fs::metadata(path).ok()
    } else {
        Some(meta.clone())
    };
    let kind = match &target {
        Some(m) if m.is_dir() => EntryKind::Dir,
        Some(m) if m.is_file() => EntryKind::File,
        _ => EntryKind::Other,
    };
    let shown = target.as_ref().unwrap_or(&meta);
    let (uid, gid) = ids_of(&meta);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| plain(path));
    Ok(Entry {
        name,
        path: plain(path),
        kind,
        is_symlink,
        size: if kind == EntryKind::Dir {
            0
        } else {
            shown.len()
        },
        modified: meta.modified().ok().and_then(secs),
        mode: mode_of(&meta),
        owner: None,
        group: None,
        uid,
        gid,
    })
}

/// Cartella di partenza del pannello locale: Documenti dell'utente se esiste, altrimenti la home.
pub fn start_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)?;
    let docs = home.join("Documents");
    Some(if docs.is_dir() { docs } else { home })
}

/// Dischi presenti (solo Windows): `A:\` … `Z:\`.
#[cfg(windows)]
fn drives() -> Vec<Entry> {
    ('A'..='Z')
        .map(|c| format!("{c}:\\"))
        .filter(|p| Path::new(p).exists())
        .map(|p| Entry {
            name: p.trim_end_matches('\\').to_string(),
            path: p,
            kind: EntryKind::Dir,
            is_symlink: false,
            size: 0,
            modified: None,
            mode: None,
            owner: None,
            group: None,
            uid: None,
            gid: None,
        })
        .collect()
}

#[cfg(not(windows))]
fn drives() -> Vec<Entry> {
    Vec::new()
}

impl RemoteFs for LocalFs {
    type Reader = tokio::fs::File;
    type Writer = tokio::fs::File;

    async fn list(&self, path: &str) -> Result<Vec<Entry>> {
        // Radice di Windows: l'elenco dei dischi.
        if path.is_empty() && cfg!(windows) {
            return Ok(drives());
        }
        let dir = path.to_string();
        tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            for item in std::fs::read_dir(&dir).map_err(|e| io_err(e, &dir))? {
                let item = item.map_err(|e| io_err(e, &dir))?;
                // Una voce che sparisce o non si legge non deve far fallire l'elenco.
                if let Ok(entry) = entry_for(&item.path()) {
                    out.push(entry);
                }
            }
            Ok(out)
        })
        .await
        .map_err(|e| FsError::Local(e.to_string()))?
    }

    async fn stat(&self, path: &str) -> Result<Entry> {
        let p = PathBuf::from(path);
        let meta = tokio::fs::metadata(&p).await.map_err(|e| io_err(e, path))?;
        let mut entry = entry_for(&p).map_err(|e| io_err(e, path))?;
        // `stat` segue i link: tipo e dimensione sono quelli della destinazione.
        entry.kind = if meta.is_dir() {
            EntryKind::Dir
        } else if meta.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        Ok(entry)
    }

    async fn lstat(&self, path: &str) -> Result<Entry> {
        let p = PathBuf::from(path);
        let mut entry = entry_for(&p).map_err(|e| io_err(e, path))?;
        // `lstat` non segue il link: per un link il tipo è `Other`, come nel backend SFTP.
        if entry.is_symlink {
            entry.kind = EntryKind::Other;
        }
        Ok(entry)
    }

    async fn canonicalize(&self, path: &str) -> Result<String> {
        if path == "." || path.is_empty() {
            return start_dir()
                .map(|p| plain(&p))
                .ok_or_else(|| FsError::Local("cartella personale non trovata".into()));
        }
        let p = tokio::fs::canonicalize(path)
            .await
            .map_err(|e| io_err(e, path))?;
        Ok(plain(&p))
    }

    async fn mkdir(&self, path: &str) -> Result<()> {
        tokio::fs::create_dir(path)
            .await
            .map_err(|e| io_err(e, path))
    }

    async fn remove_file(&self, path: &str) -> Result<()> {
        // Su Windows un link simbolico a una cartella si toglie con `remove_dir`.
        let is_dir_link = std::fs::symlink_metadata(path)
            .map(|m| {
                m.file_type().is_symlink() && std::fs::metadata(path).is_ok_and(|t| t.is_dir())
            })
            .unwrap_or(false);
        let r = if is_dir_link && cfg!(windows) {
            tokio::fs::remove_dir(path).await
        } else {
            tokio::fs::remove_file(path).await
        };
        r.map_err(|e| io_err(e, path))
    }

    async fn remove_dir(&self, path: &str) -> Result<()> {
        tokio::fs::remove_dir(path)
            .await
            .map_err(|e| io_err(e, path))
    }

    async fn rename(&self, from: &str, to: &str) -> Result<()> {
        tokio::fs::rename(from, to)
            .await
            .map_err(|e| io_err(e, from))
    }

    #[cfg(unix)]
    async fn set_permissions(&self, path: &str, mode: u32) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777))
            .await
            .map_err(|e| io_err(e, path))
    }

    #[cfg(not(unix))]
    async fn set_permissions(&self, _path: &str, _mode: u32) -> Result<()> {
        Err(FsError::Local(
            "i permessi in stile Unix non esistono su questo sistema".into(),
        ))
    }

    async fn set_modified(&self, path: &str, secs: i64) -> Result<()> {
        let t = std::time::UNIX_EPOCH
            + std::time::Duration::from_secs(u64::try_from(secs).unwrap_or(0));
        let p = path.to_string();
        tokio::task::spawn_blocking(move || {
            std::fs::OpenOptions::new()
                .write(true)
                .open(&p)
                .and_then(|f| f.set_modified(t))
                .map_err(|e| io_err(e, &p))
        })
        .await
        .map_err(|e| FsError::Local(e.to_string()))?
    }

    async fn open_read(&self, path: &str) -> Result<Self::Reader> {
        tokio::fs::File::open(path)
            .await
            .map_err(|e| io_err(e, path))
    }

    async fn open_write(&self, path: &str) -> Result<Self::Writer> {
        tokio::fs::File::create(path)
            .await
            .map_err(|e| io_err(e, path))
    }

    async fn open_read_at(&self, path: &str, offset: u64) -> Result<Self::Reader> {
        let mut f = self.open_read(path).await?;
        f.seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| io_err(e, path))?;
        Ok(f)
    }

    async fn open_write_at(&self, path: &str, offset: u64) -> Result<Self::Writer> {
        let mut f = tokio::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .await
            .map_err(|e| io_err(e, path))?;
        f.seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| io_err(e, path))?;
        Ok(f)
    }

    fn capabilities(&self) -> FsCapabilities {
        FsCapabilities {
            permissions: cfg!(unix),
            symlinks: true,
            owner_group: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn scratch(tag: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("pxt-local-{tag}-{n}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn list_stat_mkdir_rename_remove() {
        let dir = scratch("basic");
        let fs = LocalFs;
        std::fs::write(dir.join("a.txt"), "ciao").unwrap();
        fs.mkdir(&s(&dir.join("sotto"))).await.unwrap();

        let mut names: Vec<_> = fs
            .list(&s(&dir))
            .await
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.kind))
            .collect();
        names.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            names,
            vec![
                ("a.txt".into(), EntryKind::File),
                ("sotto".into(), EntryKind::Dir)
            ]
        );
        let st = fs.stat(&s(&dir.join("a.txt"))).await.unwrap();
        assert_eq!(
            (st.size, st.kind, st.is_symlink),
            (4, EntryKind::File, false)
        );
        assert!(st.modified.is_some());

        fs.rename(&s(&dir.join("a.txt")), &s(&dir.join("b.txt")))
            .await
            .unwrap();
        assert!(matches!(
            fs.stat(&s(&dir.join("a.txt"))).await,
            Err(FsError::NotFound(_))
        ));
        fs.remove_file(&s(&dir.join("b.txt"))).await.unwrap();
        fs.remove_dir(&s(&dir.join("sotto"))).await.unwrap();
        assert!(fs.list(&s(&dir)).await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_shared_operations_work_on_the_local_filesystem() {
        use crate::ops::{remove_recursive, tree_info};
        use std::sync::atomic::AtomicBool;
        let dir = scratch("ops");
        let fs = LocalFs;
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/uno.txt"), "12345").unwrap();
        std::fs::write(dir.join("a/b/due.txt"), "123").unwrap();
        let cancel = AtomicBool::new(false);

        let info = tree_info(&fs, &s(&dir.join("a")), &cancel).await.unwrap();
        assert_eq!((info.files, info.dirs, info.bytes), (2, 2, 8));

        let report = remove_recursive(&fs, &s(&dir.join("a")), &cancel)
            .await
            .unwrap();
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert!(!dir.join("a").exists());

        // Le radici non si eliminano mai.
        for root in ["/", "C:\\", "C:"] {
            assert!(
                remove_recursive(&fs, root, &cancel).await.is_err(),
                "{root}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn remove_dir_refuses_a_non_empty_folder_and_missing_paths_are_not_found() {
        let dir = scratch("nonempty");
        let fs = LocalFs;
        std::fs::write(dir.join("x"), "1").unwrap();
        assert!(fs.remove_dir(&s(&dir)).await.is_err());
        assert!(matches!(
            fs.list(&s(&dir.join("non-esiste"))).await,
            Err(FsError::NotFound(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn read_write_and_offsets_for_resuming() {
        let dir = scratch("rw");
        let fs = LocalFs;
        let f = s(&dir.join("dati.bin"));
        let mut w = fs.open_write(&f).await.unwrap();
        w.write_all(b"0123456789").await.unwrap();
        w.shutdown().await.unwrap();
        drop(w);

        let mut r = fs.open_read_at(&f, 4).await.unwrap();
        let mut got = String::new();
        r.read_to_string(&mut got).await.unwrap();
        assert_eq!(got, "456789");

        // Si riprende una scrittura a metà: i primi byte restano, il resto viene riscritto.
        let mut w = fs.open_write_at(&f, 6).await.unwrap();
        w.write_all(b"abcd").await.unwrap();
        w.shutdown().await.unwrap();
        drop(w);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "012345abcd");
        // `open_write` svuota.
        let w = fs.open_write(&f).await.unwrap();
        drop(w);
        assert_eq!(std::fs::metadata(&f).unwrap().len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn canonicalize_has_no_verbatim_prefix_and_dot_is_the_start_folder() {
        let dir = scratch("canon");
        let fs = LocalFs;
        let c = fs.canonicalize(&s(&dir)).await.unwrap();
        assert!(!c.starts_with(r"\\?\"), "{c}");
        assert!(Path::new(&c).is_dir());
        let home = fs.canonicalize(".").await.unwrap();
        assert!(Path::new(&home).is_dir(), "{home}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn the_empty_path_lists_the_drives_on_windows() {
        let drives = LocalFs.list("").await.unwrap();
        assert!(!drives.is_empty());
        assert!(drives
            .iter()
            .all(|d| d.kind == EntryKind::Dir && d.path.ends_with('\\') && d.name.ends_with(':')));
        // Un disco si apre come una cartella.
        assert!(LocalFs.list(&drives[0].path).await.is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinks_follow_their_target_and_lstat_does_not() {
        let dir = scratch("link");
        let fs = LocalFs;
        std::fs::create_dir(dir.join("reale")).unwrap();
        std::os::unix::fs::symlink(dir.join("reale"), dir.join("collegamento")).unwrap();
        std::os::unix::fs::symlink(dir.join("manca"), dir.join("rotto")).unwrap();
        let by_name = |v: Vec<Entry>, n: &str| v.into_iter().find(|e| e.name == n).unwrap();
        let list = fs.list(&s(&dir)).await.unwrap();
        let l = by_name(list.clone(), "collegamento");
        assert_eq!((l.kind, l.is_symlink), (EntryKind::Dir, true));
        let b = by_name(list, "rotto");
        assert_eq!((b.kind, b.is_symlink), (EntryKind::Other, true));
        let ls = fs.lstat(&s(&dir.join("collegamento"))).await.unwrap();
        assert_eq!((ls.kind, ls.is_symlink), (EntryKind::Other, true));
        fs.remove_file(&s(&dir.join("collegamento"))).await.unwrap();
        assert!(
            dir.join("reale").is_dir(),
            "togliere il link non tocca la destinazione"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
