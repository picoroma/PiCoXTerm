//! Prove end-to-end contro il server di `testenv/` (docker compose up -d).
//! Si eseguono con: `cargo test -p pxt-ssh --test auth_e2e -- --ignored --test-threads=1`
//! (in serie: OpenSSH penalizza gli accessi falliti ripetuti dallo stesso indirizzo).
//! Le chiavi di prova si generano in `testenv/keys/` (vedi `testenv/README.md`).

use std::path::PathBuf;
use std::sync::Arc;

use pxt_ssh::{connect_shell, ConnectParams, Credentials, HostKeyVerifier, KnownHosts, SshError};
use tokio::sync::mpsc;

fn key(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testenv/keys")
        .join(name)
}

async fn try_connect(creds: Credentials) -> Result<(), SshError> {
    let (tx, _rx) = mpsc::channel(16);
    let known =
        KnownHosts::new(std::env::temp_dir().join(format!("pxt-e2e-kh-{}", std::process::id())));
    let verifier: HostKeyVerifier = Arc::new(|_| Box::pin(async { true }));
    let shell = connect_shell(
        ConnectParams {
            host: "127.0.0.1".into(),
            port: 2222,
            username: "test".into(),
            auth: creds,
            cols: 80,
            rows: 24,
            term: "xterm".into(),
        },
        known,
        verifier,
        tx,
    )
    .await?;
    shell.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn password_ok_and_wrong() {
    try_connect(Credentials::Password("test".into()))
        .await
        .unwrap();
    let err = try_connect(Credentials::Password("sbagliata".into()))
        .await
        .unwrap_err();
    assert!(matches!(err, SshError::AuthFailed), "{err:?}");
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn unencrypted_key_ok() {
    try_connect(Credentials::Key {
        path: key("id_plain"),
        passphrase: None,
    })
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn encrypted_key_needs_and_accepts_passphrase() {
    let err = try_connect(Credentials::Key {
        path: key("id_enc"),
        passphrase: None,
    })
    .await
    .unwrap_err();
    assert!(matches!(err, SshError::PassphraseRequired), "{err:?}");

    let err = try_connect(Credentials::Key {
        path: key("id_enc"),
        passphrase: Some("errata".into()),
    })
    .await
    .unwrap_err();
    assert!(matches!(err, SshError::Key(_)), "{err:?}");

    try_connect(Credentials::Key {
        path: key("id_enc"),
        passphrase: Some("testpass".into()),
    })
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn missing_key_file_is_reported() {
    let err = try_connect(Credentials::Key {
        path: key("non_esiste"),
        passphrase: None,
    })
    .await
    .unwrap_err();
    assert!(matches!(err, SshError::Key(_)), "{err:?}");
}
