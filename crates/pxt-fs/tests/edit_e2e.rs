//! Prove dell'editor integrato contro il server di `testenv/` (docker compose up -d).
//! `cargo test -p pxt-fs --test edit_e2e -- --ignored --test-threads=1`

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use pxt_fs::edit::{read_text, write_text, Encoding, Eol, SaveOutcome, Stamp, MAX_EDIT_BYTES};
use pxt_fs::ops::remove_recursive;
use pxt_fs::{FsError, RemoteFs, SftpFs};
use pxt_ssh::{
    connect_shell, ConnectParams, Credentials, Hop, HostKeyDecision, HostKeyVerifier, KnownHosts,
};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

struct Env {
    fs: SftpFs,
    _shell: pxt_ssh::Shell,
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
        KnownHosts::new(std::env::temp_dir().join(format!("pxt-edit-kh-{nanos}"))),
        verifier,
        tx,
    )
    .await
    .expect("connessione SSH");
    let fs = SftpFs::connect(shell.open_sftp_stream().await.unwrap())
        .await
        .unwrap();
    let root = format!("/tmp/pxt-edit-{nanos}");
    fs.mkdir(&root).await.unwrap();
    Env {
        fs,
        _shell: shell,
        _events: rx,
        root,
    }
}

impl Env {
    fn p(&self, rel: &str) -> String {
        format!("{}/{rel}", self.root)
    }

    async fn put(&self, rel: &str, bytes: &[u8]) {
        let mut w = self.fs.open_write(&self.p(rel)).await.unwrap();
        w.write_all(bytes).await.unwrap();
        w.shutdown().await.unwrap();
    }

    async fn cleanup(self) {
        let _ = remove_recursive(&self.fs, &self.root, &AtomicBool::new(false)).await;
    }
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn read_edit_save_keeps_encoding_eol_and_mode() {
    let env = setup().await;
    env.put("conf.txt", b"uno\r\ndue \xE8\r\n").await;
    env.fs
        .set_permissions(&env.p("conf.txt"), 0o640)
        .await
        .unwrap();

    let f = read_text(&env.fs, &env.p("conf.txt"), MAX_EDIT_BYTES)
        .await
        .unwrap();
    assert_eq!(f.content, "uno\ndue è\n");
    assert_eq!((f.encoding, f.eol), (Encoding::Latin1, Eol::Crlf));

    let base = Stamp {
        size: f.size,
        modified: f.modified,
    };
    let out = write_text(
        &env.fs,
        &env.p("conf.txt"),
        "uno\ndue è\ntre\n",
        f.encoding,
        f.eol,
        Some(base),
        MAX_EDIT_BYTES,
    )
    .await
    .unwrap();
    assert!(matches!(out, SaveOutcome::Saved { .. }));

    let again = read_text(&env.fs, &env.p("conf.txt"), MAX_EDIT_BYTES)
        .await
        .unwrap();
    assert_eq!(again.content, "uno\ndue è\ntre\n");
    // Permessi invariati: la scrittura è sul posto.
    let st = env.fs.stat(&env.p("conf.txt")).await.unwrap();
    assert_eq!(st.mode.unwrap() & 0o777, 0o640);
    env.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn concurrent_change_is_detected_and_overwrite_works() {
    let env = setup().await;
    env.put("a.txt", b"originale\n").await;
    let f = read_text(&env.fs, &env.p("a.txt"), MAX_EDIT_BYTES)
        .await
        .unwrap();
    let base = Stamp {
        size: f.size,
        modified: f.modified,
    };

    // Qualcun altro modifica il file dopo l'apertura.
    env.put("a.txt", b"modificato da un altro utente\n").await;

    let out = write_text(
        &env.fs,
        &env.p("a.txt"),
        "la mia versione\n",
        f.encoding,
        f.eol,
        Some(base),
        MAX_EDIT_BYTES,
    )
    .await
    .unwrap();
    assert!(matches!(out, SaveOutcome::Conflict { missing: false, .. }));
    let now = read_text(&env.fs, &env.p("a.txt"), MAX_EDIT_BYTES)
        .await
        .unwrap();
    assert_eq!(now.content, "modificato da un altro utente\n");

    // Sovrascrittura voluta: senza `base` si scrive comunque.
    let out = write_text(
        &env.fs,
        &env.p("a.txt"),
        "la mia versione\n",
        f.encoding,
        f.eol,
        None,
        MAX_EDIT_BYTES,
    )
    .await
    .unwrap();
    assert!(matches!(out, SaveOutcome::Saved { .. }));

    // File sparito: conflitto "missing".
    env.fs.remove_file(&env.p("a.txt")).await.unwrap();
    let out = write_text(
        &env.fs,
        &env.p("a.txt"),
        "x\n",
        f.encoding,
        f.eol,
        Some(base),
        MAX_EDIT_BYTES,
    )
    .await
    .unwrap();
    assert!(matches!(out, SaveOutcome::Conflict { missing: true, .. }));
    env.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn refuses_binary_large_and_directories() {
    let env = setup().await;
    env.put("bin.dat", b"\x7fELF\0\0\0").await;
    assert!(matches!(
        read_text(&env.fs, &env.p("bin.dat"), MAX_EDIT_BYTES).await,
        Err(FsError::NotText)
    ));

    env.put("big.txt", &vec![b'a'; 2048]).await;
    assert!(matches!(
        read_text(&env.fs, &env.p("big.txt"), 1024).await,
        Err(FsError::TooLarge { .. })
    ));

    assert!(matches!(
        read_text(&env.fs, &env.root, MAX_EDIT_BYTES).await,
        Err(FsError::NotRegularFile(_))
    ));
    env.cleanup().await;
}
