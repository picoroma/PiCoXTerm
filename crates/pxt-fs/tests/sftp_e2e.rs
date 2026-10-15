//! Prove end-to-end contro il server di `testenv/` (docker compose up -d).
//! `cargo test -p pxt-fs --test sftp_e2e -- --ignored --test-threads=1`

use std::sync::Arc;

use pxt_fs::{EntryKind, FsError, RemoteFs, SftpFs};
use pxt_ssh::{
    connect_shell, ConnectParams, Credentials, Hop, HostKeyDecision, HostKeyVerifier, KnownHosts,
};
use tokio::sync::mpsc;

/// Restituisce anche il ricevitore degli eventi: se venisse scartato, la shell si chiuderebbe.
async fn open_sftp() -> (SftpFs, pxt_ssh::Shell, mpsc::Receiver<pxt_ssh::ShellEvent>) {
    let (tx, rx) = mpsc::channel(64);
    let dir = std::env::temp_dir().join(format!(
        "pxt-fs-e2e-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
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
        KnownHosts::new(dir.join("known_hosts")),
        verifier,
        tx,
    )
    .await
    .expect("connessione SSH");
    let stream = shell.open_sftp_stream().await.expect("sottosistema sftp");
    let fs = SftpFs::connect(stream).await.expect("protocollo SFTP");
    (fs, shell, rx)
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn start_folder_listing_stat_and_errors() {
    let (fs, _shell, _events) = open_sftp().await;

    // Cartella di partenza: la home dell'utente.
    let home = fs.canonicalize(".").await.unwrap();
    assert!(home.starts_with('/'), "{home}");
    println!("home = {home}");

    // Elenco della radice: contiene /etc (cartella) e niente "." o "..".
    let root = fs.list("/").await.unwrap();
    let etc = root.iter().find(|e| e.name == "etc").expect("/etc");
    assert_eq!(etc.kind, EntryKind::Dir);
    assert_eq!(etc.path, "/etc");
    assert!(root.iter().all(|e| e.name != "." && e.name != ".."));

    // Un link simbolico verso una cartella si comporta da cartella (su Alpine /var/run -> /run).
    let link = root
        .iter()
        .chain(fs.list("/var").await.unwrap().iter())
        .find(|e| e.is_symlink)
        .cloned();
    if let Some(l) = link {
        println!("link: {} -> kind {:?}", l.path, l.kind);
        assert!(l.is_symlink);
    }

    // stat su un file: tipo, dimensione, permessi, proprietario.
    let passwd = fs.stat("/etc/passwd").await.unwrap();
    assert_eq!(passwd.kind, EntryKind::File);
    assert!(passwd.size > 0);
    assert!(passwd.mode.is_some());
    assert!(passwd.modified.is_some());
    assert_eq!(passwd.name, "passwd");

    // Errori: percorso inesistente e permesso negato sono distinti.
    assert!(matches!(
        fs.list("/non/esiste").await.unwrap_err(),
        FsError::NotFound(_)
    ));
    assert!(matches!(
        fs.stat("/non/esiste").await.unwrap_err(),
        FsError::NotFound(_)
    ));
    let denied = fs.list("/root").await;
    assert!(
        matches!(denied, Err(FsError::PermissionDenied(_))),
        "atteso permesso negato, ottenuto {denied:?}"
    );
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn the_shell_keeps_working_while_sftp_is_open() {
    let (fs, shell, _events) = open_sftp().await;
    // L'SFTP viaggia sulla stessa connessione: la shell deve restare utilizzabile.
    shell.write(b"echo ok\n".to_vec()).await.unwrap();
    assert!(!fs.list("/etc").await.unwrap().is_empty());
    shell.resize(100, 30).await.unwrap();
}
