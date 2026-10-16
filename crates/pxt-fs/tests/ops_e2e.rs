//! Prove delle operazioni di modifica contro il server di `testenv/` (docker compose up -d).
//! `cargo test -p pxt-fs --test ops_e2e -- --ignored --test-threads=1`

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use pxt_fs::ops::{chmod, remove_recursive, tree_info};
use pxt_fs::{EntryKind, FsError, RemoteFs, SftpFs};
use pxt_ssh::{
    connect_shell, ConnectParams, Credentials, Hop, HostKeyDecision, HostKeyVerifier, KnownHosts,
};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

struct Env {
    fs: SftpFs,
    shell: pxt_ssh::Shell,
    _events: mpsc::Receiver<pxt_ssh::ShellEvent>,
    root: String,
}

async fn setup() -> Env {
    let (tx, rx) = mpsc::channel(256);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let verifier: HostKeyVerifier =
        Arc::new(|_| Box::pin(async { HostKeyDecision::AcceptAndSave }));
    let shell = connect_shell(
        ConnectParams {
            hops: vec![Hop {
                host: "127.0.0.1".into(),
                port: 2222,
                username: "test".into(),
                auth: Credentials::Password("test".into()),
            }],
            cols: 80,
            rows: 24,
            term: "xterm".into(),
        },
        KnownHosts::new(std::env::temp_dir().join(format!("pxt-ops-kh-{nanos}"))),
        verifier,
        tx,
    )
    .await
    .expect("connessione SSH");
    let fs = SftpFs::connect(shell.open_sftp_stream().await.unwrap())
        .await
        .unwrap();
    let root = format!("/tmp/pxt-ops-{nanos}");
    fs.mkdir(&root).await.unwrap();
    Env {
        fs,
        shell,
        _events: rx,
        root,
    }
}

impl Env {
    async fn put(&self, rel: &str, content: &str) {
        let mut w = self
            .fs
            .open_write(&format!("{}/{rel}", self.root))
            .await
            .unwrap();
        w.write_all(content.as_bytes()).await.unwrap();
        w.shutdown().await.unwrap();
    }

    async fn dir(&self, rel: &str) {
        self.fs
            .mkdir(&format!("{}/{rel}", self.root))
            .await
            .unwrap();
    }

    fn p(&self, rel: &str) -> String {
        format!("{}/{rel}", self.root)
    }

    /// Esegue un comando nella shell (per creare link simbolici, che SFTP qui non espone).
    async fn sh(&self, cmd: &str) {
        self.shell
            .write(format!("{cmd}\n").into_bytes())
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    }

    async fn exists(&self, rel: &str) -> bool {
        self.fs.lstat(&self.p(rel)).await.is_ok()
    }

    async fn cleanup(self) {
        // Si ripristinano i permessi eventualmente ridotti dalle prove, poi si elimina tutto.
        let _ = chmod(&self.fs, &self.root, 0o755, true, &AtomicBool::new(false)).await;
        let _ = remove_recursive(&self.fs, &self.root, &AtomicBool::new(false)).await;
    }
}

fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn tree_info_counts_files_dirs_and_bytes() {
    let e = setup().await;
    e.dir("a").await;
    e.dir("a/b").await;
    e.put("uno.txt", "1").await;
    e.put("a/due.txt", "22").await;
    e.put("a/b/tre.txt", "333").await;

    let info = tree_info(&e.fs, &e.root, &no_cancel()).await.unwrap();
    assert_eq!(
        (info.files, info.dirs, info.bytes, info.partial),
        (3, 3, 6, false)
    );

    // Su un file singolo.
    let single = tree_info(&e.fs, &e.p("a/b/tre.txt"), &no_cancel())
        .await
        .unwrap();
    assert_eq!((single.files, single.dirs, single.bytes), (1, 0, 3));
    e.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn remove_recursive_deletes_a_whole_tree() {
    let e = setup().await;
    e.dir("x").await;
    e.dir("x/y").await;
    e.put("x/f1", "a").await;
    e.put("x/y/f2", "b").await;
    e.put("fuori.txt", "c").await;

    let report = remove_recursive(&e.fs, &e.p("x"), &no_cancel())
        .await
        .unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(report.done, 4, "2 file + 2 cartelle");
    assert!(!e.exists("x").await);
    assert!(
        e.exists("fuori.txt").await,
        "ciò che sta fuori non va toccato"
    );
    e.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn symlinks_are_never_followed_when_deleting() {
    let e = setup().await;
    e.dir("keep").await;
    e.put("keep/prezioso.txt", "non toccare").await;
    e.dir("victim").await;
    e.put("victim/f", "x").await;
    // Un link dentro la cartella da eliminare punta a un'altra cartella che deve restare intatta.
    e.sh(&format!("ln -s {} {}", e.p("keep"), e.p("victim/link")))
        .await;
    assert!(e.exists("victim/link").await, "il link doveva esistere");

    // lstat non segue il link; stat sì.
    let l = e.fs.lstat(&e.p("victim/link")).await.unwrap();
    assert!(l.is_symlink && l.kind == EntryKind::Other);
    assert_eq!(
        e.fs.stat(&e.p("victim/link")).await.unwrap().kind,
        EntryKind::Dir
    );

    let report = remove_recursive(&e.fs, &e.p("victim"), &no_cancel())
        .await
        .unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(!e.exists("victim").await);
    assert!(
        e.exists("keep/prezioso.txt").await,
        "il contenuto dietro il link è stato toccato!"
    );

    // Eliminare il link stesso toglie solo il link.
    e.dir("v2").await;
    e.sh(&format!("ln -s {} {}", e.p("keep"), e.p("v2/lnk")))
        .await;
    let report = remove_recursive(&e.fs, &e.p("v2/lnk"), &no_cancel())
        .await
        .unwrap();
    assert_eq!(report.done, 1);
    assert!(!e.exists("v2/lnk").await);
    assert!(e.exists("keep/prezioso.txt").await);
    e.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn dangerous_paths_are_refused() {
    let e = setup().await;
    for bad in ["/", "//", "", "."] {
        let r = remove_recursive(&e.fs, bad, &no_cancel()).await;
        assert!(
            matches!(r, Err(FsError::Sftp(_))),
            "{bad:?} doveva essere rifiutato: {r:?}"
        );
        let r = chmod(&e.fs, bad, 0o777, true, &no_cancel()).await;
        assert!(r.is_err(), "chmod su {bad:?} doveva essere rifiutato");
    }
    e.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn chmod_applies_to_one_item_or_to_the_whole_tree() {
    let e = setup().await;
    e.dir("d").await;
    e.dir("d/sub").await;
    e.put("d/f", "x").await;
    e.put("d/sub/g", "y").await;
    let mode_of = |rel: String| {
        let fs = &e.fs;
        async move { fs.stat(&rel).await.unwrap().mode.unwrap() }
    };

    // Solo la cartella: i figli non cambiano.
    let before_child = mode_of(e.p("d/f")).await;
    let r = chmod(&e.fs, &e.p("d"), 0o750, false, &no_cancel())
        .await
        .unwrap();
    assert_eq!(r.done, 1);
    assert_eq!(mode_of(e.p("d")).await, 0o750);
    assert_eq!(mode_of(e.p("d/f")).await, before_child);

    // Ricorsivo: tutto con lo stesso valore.
    let r = chmod(&e.fs, &e.p("d"), 0o700, true, &no_cancel())
        .await
        .unwrap();
    assert_eq!((r.done, r.failed.len()), (4, 0), "{:?}", r.failed);
    for rel in ["d", "d/sub", "d/f", "d/sub/g"] {
        assert_eq!(mode_of(e.p(rel)).await, 0o700, "{rel}");
    }
    e.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn undeletable_entries_are_reported_without_stopping_the_rest() {
    let e = setup().await;
    e.dir("lotto").await;
    e.put("lotto/libero.txt", "ok").await;
    e.dir("lotto/ro").await;
    e.put("lotto/ro/bloccato.txt", "no").await;
    // Cartella non scrivibile: dentro non si può eliminare nulla (l'utente non è root).
    chmod(&e.fs, &e.p("lotto/ro"), 0o555, false, &no_cancel())
        .await
        .unwrap();

    let report = remove_recursive(&e.fs, &e.p("lotto"), &no_cancel())
        .await
        .unwrap();
    assert!(
        !report.failed.is_empty(),
        "doveva segnalare almeno un errore"
    );
    assert!(
        report
            .failed
            .iter()
            .any(|f| f.path.ends_with("bloccato.txt")),
        "{:?}",
        report.failed
    );
    assert!(
        !e.exists("lotto/libero.txt").await,
        "il file libero doveva essere eliminato"
    );
    assert!(e.exists("lotto/ro/bloccato.txt").await);
    e.cleanup().await;
}
