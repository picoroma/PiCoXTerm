//! Esportazione e importazione delle sessioni in JSON, senza segreti (sezione 8.4).

use std::collections::HashMap;

use uuid::Uuid;

use crate::error::Result;
use crate::local::migrate;
use crate::model::{Folder, SessionNode, SessionsFile};

/// JSON delle sessioni. I riferimenti alle credenziali vengono tolti: sono validi solo su questa macchina.
pub fn export_json(nodes: &[SessionNode]) -> Result<String> {
    let nodes = nodes
        .iter()
        .cloned()
        .map(|mut n| {
            if let SessionNode::Session(s) = &mut n {
                s.credential_ref = None;
            }
            n
        })
        .collect();
    Ok(serde_json::to_string_pretty(&SessionsFile {
        nodes,
        ..SessionsFile::default()
    })?)
}

/// Prepara i nodi da importare: nuovi id (nessuna sovrascrittura di ciò che esiste già), tutto sotto
/// una cartella nuova chiamata `folder_name`, riferimenti ai jump host rimappati, nessuna credenziale.
pub fn import_json(text: &str, folder_name: &str) -> Result<Vec<SessionNode>> {
    let file = migrate(serde_json::from_str(text)?)?;
    let remap: HashMap<Uuid, Uuid> = file
        .nodes
        .iter()
        .map(|n| (n.id(), Uuid::new_v4()))
        .collect();
    let root = Folder {
        id: Uuid::new_v4(),
        name: folder_name.to_string(),
        parent_id: None,
    };
    let parent_of = |old: Option<Uuid>| old.and_then(|p| remap.get(&p).copied()).or(Some(root.id));

    let mut out = vec![SessionNode::Folder(root.clone())];
    for node in file.nodes {
        out.push(match node {
            SessionNode::Folder(mut f) => {
                f.id = remap[&f.id];
                f.parent_id = parent_of(f.parent_id);
                SessionNode::Folder(f)
            }
            SessionNode::Session(mut s) => {
                s.id = remap[&s.id];
                s.parent_id = parent_of(s.parent_id);
                s.credential_ref = None;
                s.jump_hosts = s
                    .jump_hosts
                    .iter()
                    .filter_map(|j| remap.get(j).copied())
                    .collect();
                SessionNode::Session(s)
            }
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CredentialRef, Session};

    fn sample() -> Vec<SessionNode> {
        let folder = Folder {
            id: Uuid::new_v4(),
            name: "Clienti".into(),
            parent_id: None,
        };
        let mut jump = Session::new_ssh("jump", "10.0.0.2", 22, "adm");
        jump.parent_id = Some(folder.id);
        jump.credential_ref = Some(CredentialRef::new_local());
        let mut s = Session::new_ssh("app", "10.0.0.1", 22, "root");
        s.parent_id = Some(folder.id);
        s.jump_hosts = vec![jump.id];
        s.credential_ref = Some(CredentialRef::new_local());
        vec![
            SessionNode::Folder(folder),
            SessionNode::Session(Box::new(jump)),
            SessionNode::Session(Box::new(s)),
        ]
    }

    #[test]
    fn export_has_no_credential_references() {
        let json = export_json(&sample()).unwrap();
        assert!(!json.contains("credentialRef\": {"));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        for n in v["nodes"].as_array().unwrap() {
            if n["type"] == "session" {
                assert!(n["credentialRef"].is_null());
            }
        }
    }

    #[test]
    fn import_remaps_ids_parents_and_jump_hosts() {
        let original = sample();
        let json = export_json(&original).unwrap();
        let imported = import_json(&json, "Importate").unwrap();
        assert_eq!(imported.len(), 4);

        let SessionNode::Folder(root) = &imported[0] else {
            panic!("la prima voce deve essere la cartella di import")
        };
        assert_eq!(root.name, "Importate");
        assert_eq!(root.parent_id, None);

        let old_ids: Vec<Uuid> = original.iter().map(|n| n.id()).collect();
        for n in &imported {
            assert!(!old_ids.contains(&n.id()), "gli id devono essere nuovi");
        }
        let SessionNode::Folder(clienti) = &imported[1] else {
            panic!()
        };
        assert_eq!(clienti.parent_id, Some(root.id));
        let (SessionNode::Session(jump), SessionNode::Session(app)) = (&imported[2], &imported[3])
        else {
            panic!()
        };
        assert_eq!(jump.parent_id, Some(clienti.id));
        assert_eq!(app.jump_hosts, vec![jump.id]);
        assert!(app.credential_ref.is_none() && jump.credential_ref.is_none());
    }

    #[test]
    fn import_drops_dangling_jump_hosts_and_rejects_garbage() {
        let mut nodes = sample();
        if let SessionNode::Session(s) = &mut nodes[2] {
            s.jump_hosts.push(Uuid::new_v4());
        }
        let imported = import_json(&export_json(&nodes).unwrap(), "x").unwrap();
        let SessionNode::Session(app) = &imported[3] else {
            panic!()
        };
        assert_eq!(app.jump_hosts.len(), 1);

        assert!(import_json("non è json", "x").is_err());
        assert!(import_json(r#"{"schemaVersion": 99, "nodes": []}"#, "x").is_err());
    }
}
