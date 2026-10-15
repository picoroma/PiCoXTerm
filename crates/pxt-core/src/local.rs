//! `LocalSessionProvider`: file JSON locale con scrittura atomica, backup rotativo e migrazioni.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use uuid::Uuid;

use crate::error::{CoreError, Result};
use crate::model::{subtree_ids, SessionNode, SessionsFile, SCHEMA_VERSION};
use crate::provider::SessionProvider;

const BACKUPS: usize = 5;

pub struct LocalSessionProvider {
    path: PathBuf,
    /// Serializza i cicli leggi-modifica-scrivi.
    lock: Mutex<()>,
}

impl LocalSessionProvider {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load(&self) -> Result<SessionsFile> {
        let text = match fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SessionsFile::default())
            }
            Err(e) => return Err(e.into()),
        };
        migrate(serde_json::from_str(&text)?)
    }

    fn store(&self, file: &SessionsFile) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = suffixed(&self.path, ".tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(file)?)?;
        // Il file con i dati è già scritto: ruota i backup e poi sostituisce l'originale.
        if self.path.exists() {
            rotate_backups(&self.path)?;
        }
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// `file` -> `file.bak1`, con scorrimento fino a `file.bak5`.
fn rotate_backups(path: &Path) -> Result<()> {
    let last = suffixed(path, &format!(".bak{BACKUPS}"));
    if last.exists() {
        fs::remove_file(&last)?;
    }
    for n in (1..BACKUPS).rev() {
        let from = suffixed(path, &format!(".bak{n}"));
        if from.exists() {
            fs::rename(&from, suffixed(path, &format!(".bak{}", n + 1)))?;
        }
    }
    fs::copy(path, suffixed(path, ".bak1"))?;
    Ok(())
}

/// Porta un file di qualunque versione nota allo schema corrente.
pub(crate) fn migrate(value: serde_json::Value) -> Result<SessionsFile> {
    let found = value
        .get("schemaVersion")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    if found > SCHEMA_VERSION {
        return Err(CoreError::UnsupportedSchema {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    // Le migrazioni future si aggiungono qui, una per versione (found -> found + 1).
    Ok(serde_json::from_value(value)?)
}

impl SessionProvider for LocalSessionProvider {
    fn id(&self) -> &str {
        "local"
    }

    fn is_read_only(&self) -> bool {
        false
    }

    async fn list(&self) -> Result<Vec<SessionNode>> {
        let _g = self.lock.lock().unwrap();
        Ok(self.load()?.nodes)
    }

    async fn save(&self, node: SessionNode) -> Result<()> {
        let _g = self.lock.lock().unwrap();
        let mut file = self.load()?;
        match file.nodes.iter_mut().find(|n| n.id() == node.id()) {
            Some(existing) => *existing = node,
            None => file.nodes.push(node),
        }
        self.store(&file)
    }

    /// Una sola scrittura per tutti i nodi: l'import non consuma i backup rotativi.
    async fn save_many(&self, nodes: Vec<SessionNode>) -> Result<()> {
        let _g = self.lock.lock().unwrap();
        let mut file = self.load()?;
        for node in nodes {
            match file.nodes.iter_mut().find(|n| n.id() == node.id()) {
                Some(existing) => *existing = node,
                None => file.nodes.push(node),
            }
        }
        self.store(&file)
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        let _g = self.lock.lock().unwrap();
        let mut file = self.load()?;
        if !file.nodes.iter().any(|n| n.id() == id) {
            return Err(CoreError::NotFound(id));
        }
        let doomed = subtree_ids(&file.nodes, id);
        file.nodes.retain(|n| !doomed.contains(&n.id()));
        self.store(&file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Folder, Session};

    fn temp_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!("pxt-core-{}", Uuid::new_v4()))
            .join("sessions.json")
    }

    fn cleanup(p: &Path) {
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[tokio::test]
    async fn roundtrip_with_folders_and_no_password_on_disk() {
        let path = temp_path();
        let prov = LocalSessionProvider::new(&path);
        assert!(prov.list().await.unwrap().is_empty());

        let folder = Folder {
            id: Uuid::new_v4(),
            name: "Clienti".into(),
            parent_id: None,
        };
        let mut s = Session::new_ssh("app01", "10.0.0.1", 22, "root");
        s.parent_id = Some(folder.id);
        prov.save(SessionNode::Folder(folder)).await.unwrap();
        prov.save(SessionNode::Session(Box::new(s.clone())))
            .await
            .unwrap();

        // "Riapre l'app": nuovo provider sullo stesso file
        let again = LocalSessionProvider::new(&path);
        let nodes = again.list().await.unwrap();
        assert_eq!(nodes.len(), 2);
        assert!(nodes.contains(&SessionNode::Session(Box::new(s))));

        let raw = fs::read_to_string(&path).unwrap().to_lowercase();
        assert!(!raw.contains("password\":"));
        cleanup(&path);
    }

    #[tokio::test]
    async fn save_updates_in_place_and_delete_removes_subtree() {
        let path = temp_path();
        let prov = LocalSessionProvider::new(&path);
        let root = Folder {
            id: Uuid::new_v4(),
            name: "a".into(),
            parent_id: None,
        };
        let child = Folder {
            id: Uuid::new_v4(),
            name: "b".into(),
            parent_id: Some(root.id),
        };
        let mut s = Session::new_ssh("x", "h", 22, "u");
        s.parent_id = Some(child.id);
        let keep = Session::new_ssh("keep", "h", 22, "u");
        for n in [
            SessionNode::Folder(root.clone()),
            SessionNode::Folder(child),
            SessionNode::Session(Box::new(s)),
            SessionNode::Session(Box::new(keep.clone())),
        ] {
            prov.save(n).await.unwrap();
        }
        let mut renamed = root.clone();
        renamed.name = "rinominata".into();
        prov.save(SessionNode::Folder(renamed)).await.unwrap();
        assert_eq!(prov.list().await.unwrap().len(), 4);

        prov.delete(root.id).await.unwrap();
        let left = prov.list().await.unwrap();
        assert_eq!(left, vec![SessionNode::Session(Box::new(keep))]);
        assert!(matches!(
            prov.delete(Uuid::new_v4()).await,
            Err(CoreError::NotFound(_))
        ));
        cleanup(&path);
    }

    /// Criterio di accettazione M1: 20 sessioni in 3 cartelle, "chiudo e riapro l'app", tutto al suo posto
    /// e nessuna password nel file.
    #[tokio::test]
    async fn twenty_sessions_in_three_folders_survive_a_restart_without_secrets() {
        let path = temp_path();
        let prov = LocalSessionProvider::new(&path);
        let folders: Vec<Folder> = ["Clienti", "Bulgari", "Personali"]
            .iter()
            .map(|n| Folder {
                id: Uuid::new_v4(),
                name: (*n).into(),
                parent_id: None,
            })
            .collect();
        for f in &folders {
            prov.save(SessionNode::Folder(f.clone())).await.unwrap();
        }
        let mut saved = Vec::new();
        for i in 0..20 {
            let mut s = Session::new_ssh(&format!("srv{i:02}"), &format!("10.0.0.{i}"), 22, "root");
            s.parent_id = Some(folders[i % 3].id);
            s.credential_ref = Some(crate::model::CredentialRef::new_local());
            saved.push(s.clone());
            prov.save(SessionNode::Session(Box::new(s))).await.unwrap();
        }

        // "Riapre l'app": un provider nuovo sullo stesso file
        let nodes = LocalSessionProvider::new(&path).list().await.unwrap();
        assert_eq!(nodes.len(), 23);
        for s in &saved {
            let found = nodes
                .iter()
                .find(|n| n.id() == s.id)
                .expect("sessione presente");
            assert_eq!(found.parent_id(), s.parent_id, "stessa cartella");
        }
        let per_folder = |f: &Folder| nodes.iter().filter(|n| n.parent_id() == Some(f.id)).count();
        assert_eq!(
            folders.iter().map(per_folder).collect::<Vec<_>>(),
            vec![7, 7, 6]
        );

        let raw = fs::read_to_string(&path).unwrap().to_lowercase();
        for forbidden in ["password\":", "passphrase", "secret"] {
            assert!(!raw.contains(forbidden), "il file contiene {forbidden}");
        }
        cleanup(&path);
    }

    #[tokio::test]
    async fn keeps_at_most_five_backups() {
        let path = temp_path();
        let prov = LocalSessionProvider::new(&path);
        for i in 0..8 {
            let s = Session::new_ssh(&format!("s{i}"), "h", 22, "u");
            prov.save(SessionNode::Session(Box::new(s))).await.unwrap();
        }
        for n in 1..=5 {
            assert!(suffixed(&path, &format!(".bak{n}")).exists(), "bak{n}");
        }
        assert!(!suffixed(&path, ".bak6").exists());
        assert!(!suffixed(&path, ".tmp").exists());
        cleanup(&path);
    }

    #[tokio::test]
    async fn rejects_newer_schema() {
        let path = temp_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, r#"{"schemaVersion": 99, "nodes": []}"#).unwrap();
        let err = LocalSessionProvider::new(&path).list().await.unwrap_err();
        assert!(matches!(
            err,
            CoreError::UnsupportedSchema { found: 99, .. }
        ));
        cleanup(&path);
    }
}
