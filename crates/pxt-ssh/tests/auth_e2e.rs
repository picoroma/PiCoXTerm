//! Prove end-to-end contro il server di `testenv/` (docker compose up -d).
//! Si eseguono con: `cargo test -p pxt-ssh --test auth_e2e -- --ignored --test-threads=1`
//! (in serie: OpenSSH penalizza gli accessi falliti ripetuti dallo stesso indirizzo).
//! Le chiavi di prova si generano in `testenv/keys/` (vedi `testenv/README.md`).

use std::path::PathBuf;
use std::sync::Arc;

use pxt_ssh::{
    connect_shell, ConnectParams, Credentials, Hop, HostKeyVerifier, KnownHosts, SshError,
};
use tokio::sync::mpsc;

fn key(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testenv/keys")
        .join(name)
}

fn hop(creds: Credentials) -> Hop {
    Hop {
        host: "127.0.0.1".into(),
        port: 2222,
        username: "test".into(),
        auth: creds,
    }
}

async fn try_chain(hops: Vec<Hop>) -> Result<String, SshError> {
    let (tx, mut rx) = mpsc::channel(64);
    let known =
        KnownHosts::new(std::env::temp_dir().join(format!("pxt-e2e-kh-{}", std::process::id())));
    let verifier: HostKeyVerifier = Arc::new(|_| Box::pin(async { true }));
    let shell = connect_shell(
        ConnectParams {
            hops,
            cols: 80,
            rows: 24,
            term: "xterm".into(),
        },
        known,
        verifier,
        tx,
    )
    .await?;
    // Esegue un comando e legge l'output, per provare che la shell finale risponde davvero.
    shell.write(b"echo PXT-$((20+22))\n".to_vec()).await?;
    let mut seen = String::new();
    while let Ok(Some(ev)) =
        tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await
    {
        if let pxt_ssh::ShellEvent::Data(d) = ev {
            seen.push_str(&String::from_utf8_lossy(&d));
            if seen.contains("PXT-42") {
                break;
            }
        }
    }
    shell.close().await;
    Ok(seen)
}

async fn try_connect(creds: Credentials) -> Result<(), SshError> {
    try_chain(vec![hop(creds)]).await.map(|_| ())
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn jump_host_chain_with_key_then_password() {
    // Il server di prova fa da jump host verso se stesso (127.0.0.1:2222 visto dall'interno).
    let out = try_chain(vec![
        hop(Credentials::Key {
            path: key("id_enc"),
            passphrase: Some("testpass".into()),
        }),
        hop(Credentials::Password("test".into())),
    ])
    .await
    .unwrap();
    assert!(out.contains("PXT-42"), "{out}");
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn jump_host_failure_names_the_hop() {
    let err = try_chain(vec![
        hop(Credentials::Password("sbagliata".into())),
        hop(Credentials::Password("test".into())),
    ])
    .await
    .unwrap_err();
    assert!(matches!(err, SshError::Jump { .. }), "{err:?}");
    assert!(err.to_string().contains("127.0.0.1:2222"), "{err}");
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn three_hop_chain() {
    let out = try_chain(vec![
        hop(Credentials::Password("test".into())),
        hop(Credentials::Password("test".into())),
        hop(Credentials::Password("test".into())),
    ])
    .await
    .unwrap();
    assert!(out.contains("PXT-42"), "{out}");
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

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn unencrypted_key_with_stray_passphrase() {
    // Succede quando una sessione passa da password a chiave e il vecchio segreto resta salvato.
    let r = try_connect(Credentials::Key {
        path: key("id_plain"),
        passphrase: Some("test".into()),
    })
    .await;
    println!("risultato: {r:?}");
    assert!(r.is_ok(), "{r:?}");
}
