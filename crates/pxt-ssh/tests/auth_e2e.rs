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
    let verifier: HostKeyVerifier =
        Arc::new(|_| Box::pin(async { pxt_ssh::HostKeyDecision::AcceptAndSave }));
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

// ---- Host key (SEC-03 / SEC-04) ----

use pxt_ssh::HostKeyDecision;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fresh_known() -> KnownHosts {
    let dir = std::env::temp_dir().join(format!("pxt-e2e-hk-{}", uuid_like()));
    KnownHosts::new(dir.join("known_hosts"))
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{}-{n}", std::process::id())
}

/// Connessione con password e verifier configurabile; conta quante volte il verifier viene interpellato.
async fn connect_with(
    known: KnownHosts,
    decision: HostKeyDecision,
) -> (Result<(), SshError>, usize) {
    let asked = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&asked);
    let verifier: HostKeyVerifier = Arc::new(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { decision })
    });
    let (tx, _rx) = mpsc::channel(16);
    let r = connect_shell(
        ConnectParams {
            hops: vec![hop(Credentials::Password("test".into()))],
            cols: 80,
            rows: 24,
            term: "xterm".into(),
        },
        known,
        verifier,
        tx,
    )
    .await
    .map(|s| {
        let _ = s;
    });
    (r, asked.load(Ordering::SeqCst))
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn host_key_accept_and_save_then_known() {
    let known = fresh_known();
    let (r, asked) = connect_with(known.clone(), HostKeyDecision::AcceptAndSave).await;
    r.unwrap();
    assert_eq!(asked, 1);
    assert_eq!(known.list().unwrap().len(), 1);
    // seconda connessione: la chiave è nota, nessuna domanda
    let (r, asked) = connect_with(known.clone(), HostKeyDecision::Reject).await;
    r.unwrap();
    assert_eq!(asked, 0);
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn host_key_accept_once_is_not_saved() {
    let known = fresh_known();
    let (r, asked) = connect_with(known.clone(), HostKeyDecision::AcceptOnce).await;
    r.unwrap();
    assert_eq!(asked, 1);
    assert!(known.list().unwrap().is_empty());
    // la volta dopo viene chiesto di nuovo
    let (_, asked) = connect_with(known.clone(), HostKeyDecision::AcceptOnce).await;
    assert_eq!(asked, 1);
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn host_key_rejected_blocks_and_saves_nothing() {
    let known = fresh_known();
    let (r, _) = connect_with(known.clone(), HostKeyDecision::Reject).await;
    assert!(matches!(r.unwrap_err(), SshError::HostKeyRejected));
    assert!(known.list().unwrap().is_empty());
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn changed_host_key_is_blocked_without_asking_and_unlocks_only_after_removal() {
    let known = fresh_known();
    known
        .add(
            "127.0.0.1",
            2222,
            "ssh-ed25519",
            "SHA256:chiave-vecchia-diversa",
        )
        .unwrap();
    // Anche un verifier che direbbe sempre sì non deve mai essere interpellato.
    let (r, asked) = connect_with(known.clone(), HostKeyDecision::AcceptAndSave).await;
    assert!(matches!(r.unwrap_err(), SshError::HostKeyChanged { .. }));
    assert_eq!(asked, 0);
    assert_eq!(
        known.list().unwrap().len(),
        1,
        "la voce vecchia non va toccata"
    );

    // Rimossa la voce a mano, l'host torna sconosciuto e si può riaccettare.
    assert_eq!(known.remove("127.0.0.1", 2222).unwrap(), 1);
    let (r, asked) = connect_with(known.clone(), HostKeyDecision::AcceptAndSave).await;
    r.unwrap();
    assert_eq!(asked, 1);
}

// ---- Test connessione ----

async fn run_test(
    hops: Vec<Hop>,
    known: KnownHosts,
    decision: HostKeyDecision,
) -> Result<u64, SshError> {
    let verifier: HostKeyVerifier = Arc::new(move |_| Box::pin(async move { decision }));
    pxt_ssh::test_connection(hops, known, verifier, std::time::Duration::from_secs(5))
        .await
        .map(|r| r.elapsed_ms)
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn test_connection_ok_through_a_jump_host_and_leaves_no_trace() {
    let known = fresh_known();
    let ms = run_test(
        vec![
            hop(Credentials::Password("test".into())),
            hop(Credentials::Password("test".into())),
        ],
        known.clone(),
        HostKeyDecision::AcceptOnce,
    )
    .await
    .unwrap();
    assert!(ms < 5000);
    assert!(
        known.list().unwrap().is_empty(),
        "il test non deve salvare nulla"
    );
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn test_connection_reports_auth_failure_unknown_key_and_timeout() {
    let err = run_test(
        vec![hop(Credentials::Password("sbagliata".into()))],
        fresh_known(),
        HostKeyDecision::AcceptOnce,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SshError::AuthFailed), "{err:?}");

    // Chiave sconosciuta e il test non si fida: si ferma prima di autenticarsi.
    let err = run_test(
        vec![hop(Credentials::Password("test".into()))],
        fresh_known(),
        HostKeyDecision::Reject,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SshError::HostKeyRejected), "{err:?}");

    // Indirizzo non instradabile: deve scadere, non restare appeso.
    let verifier: HostKeyVerifier = Arc::new(|_| Box::pin(async { HostKeyDecision::Reject }));
    let started = std::time::Instant::now();
    let err = pxt_ssh::test_connection(
        vec![Hop {
            host: "10.255.255.1".into(),
            port: 22,
            username: "x".into(),
            auth: Credentials::Password("x".into()),
        }],
        fresh_known(),
        verifier,
        std::time::Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert!(started.elapsed() < std::time::Duration::from_secs(4));
    assert!(
        matches!(err, SshError::Timeout(1) | SshError::Target { .. }),
        "{err:?}"
    );
}

// ---- Tracciamento della directory (OSC 7) ----

/// Collega come `user`, attiva il tracciamento con il rilevamento automatico della shell, esegue i
/// comandi e raccoglie le directory comunicate. Restituisce anche la shell rilevata.
async fn cwd_events_for(user: &str, commands: &[&str]) -> (Result<String, SshError>, Vec<String>) {
    let (tx, mut rx) = mpsc::channel(256);
    let verifier: HostKeyVerifier =
        Arc::new(|_| Box::pin(async { HostKeyDecision::AcceptAndSave }));
    let shell = connect_shell(
        ConnectParams {
            hops: vec![Hop {
                // Server con più shell (testenv, servizio `ssh-shells`)
                host: "127.0.0.1".into(),
                port: 2223,
                username: user.into(),
                auth: Credentials::Password("test".into()),
            }],
            cols: 80,
            rows: 24,
            term: "xterm".into(),
        },
        fresh_known(),
        verifier,
        tx,
    )
    .await
    .unwrap();
    let pause = |ms| tokio::time::sleep(std::time::Duration::from_millis(ms));
    pause(800).await;
    let detected = shell.enable_cwd_tracking().await;
    if detected.is_ok() {
        for c in commands {
            pause(500).await;
            shell
                .write(
                    format!(
                        "{c}
"
                    )
                    .into_bytes(),
                )
                .await
                .unwrap();
        }
    }
    let mut cwds = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while std::time::Instant::now() < deadline {
        if let Ok(Some(pxt_ssh::ShellEvent::Cwd(p))) =
            tokio::time::timeout(std::time::Duration::from_millis(400), rx.recv()).await
        {
            cwds.push(p);
        }
    }
    shell.close().await;
    (detected, cwds)
}

async fn assert_follows(user: &str, expected_shell: &str) {
    let (detected, cwds) = cwd_events_for(user, &["cd /etc", "cd /tmp"]).await;
    let shell = detected.unwrap_or_else(|e| panic!("{user}: {e}"));
    assert!(
        shell.ends_with(expected_shell),
        "{user}: shell rilevata {shell}"
    );
    assert!(cwds.iter().any(|p| p == "/etc"), "{user}: {cwds:?}");
    assert!(cwds.iter().any(|p| p == "/tmp"), "{user}: {cwds:?}");
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn cwd_tracking_follows_cd_in_bash() {
    assert_follows("bashuser", "bash").await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn cwd_tracking_follows_cd_in_zsh() {
    assert_follows("zshuser", "zsh").await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn cwd_tracking_follows_cd_in_tcsh() {
    assert_follows("tcshuser", "tcsh").await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn cwd_tracking_reports_unsupported_shell_clearly() {
    let (detected, cwds) = cwd_events_for("ashuser", &["cd /etc"]).await;
    assert!(
        matches!(&detected, Err(SshError::UnsupportedShell(s)) if s.ends_with("ash")),
        "{detected:?}"
    );
    assert!(cwds.is_empty());
}

// ---- Criteri di accettazione di M2: tempi e cambio utente ----

/// Collega come `user` (servizio `ssh-shells`) con il tracciamento attivo.
async fn shell_with_tracking(
    user: &str,
) -> (pxt_ssh::Shell, mpsc::Receiver<pxt_ssh::ShellEvent>, String) {
    let (tx, rx) = mpsc::channel(256);
    let verifier: HostKeyVerifier =
        Arc::new(|_| Box::pin(async { HostKeyDecision::AcceptAndSave }));
    let shell = connect_shell(
        ConnectParams {
            hops: vec![Hop {
                host: "127.0.0.1".into(),
                port: 2223,
                username: user.into(),
                auth: Credentials::Password("test".into()),
            }],
            cols: 80,
            rows: 24,
            term: "xterm".into(),
        },
        fresh_known(),
        verifier,
        tx,
    )
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    let detected = shell.enable_cwd_tracking().await.unwrap();
    (shell, rx, detected)
}

/// Aspetta un evento `Cwd` con il percorso dato; restituisce dopo quanto è arrivato.
async fn wait_cwd(
    rx: &mut mpsc::Receiver<pxt_ssh::ShellEvent>,
    expected: &str,
    max: std::time::Duration,
) -> Option<std::time::Duration> {
    let t0 = std::time::Instant::now();
    while t0.elapsed() < max {
        if let Ok(Some(pxt_ssh::ShellEvent::Cwd(p))) =
            tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await
        {
            if p == expected {
                return Some(t0.elapsed());
            }
        }
    }
    None
}

async fn cd_latency(user: &str) -> Vec<std::time::Duration> {
    let (shell, mut rx, _) = shell_with_tracking(user).await;
    // Si scarta il percorso iniziale.
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    while rx.try_recv().is_ok() {}
    let mut out = Vec::new();
    for dir in ["/etc", "/usr", "/tmp", "/var"] {
        shell
            .write(format!("cd {dir}\n").into_bytes())
            .await
            .unwrap();
        out.push(
            wait_cwd(&mut rx, dir, std::time::Duration::from_secs(3))
                .await
                .unwrap_or_else(|| panic!("{user}: nessun evento per {dir}")),
        );
    }
    shell.close().await;
    out
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn accept_panel_updates_within_500ms_in_bash_and_tcsh() {
    for user in ["bashuser", "tcshuser"] {
        let lat = cd_latency(user).await;
        println!("{user}: tempi di aggiornamento {lat:?}");
        assert!(
            lat.iter()
                .all(|d| *d < std::time::Duration::from_millis(500)),
            "{user}: troppo lento: {lat:?}"
        );
    }
}

/// Dopo `su - altro_utente` la shell cambia: la re-iniezione deve usare **quella** shell.
async fn reinject_after_su(from: &str, to: &str, expected_shell: &str) {
    let (shell, mut rx, first) = shell_with_tracking(from).await;
    println!("{from}: shell iniziale {first}");
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    shell
        .write(format!("sudo su - {to}\n").into_bytes())
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
    let mut seen = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let pxt_ssh::ShellEvent::Data(d) = ev {
            seen.extend(d);
        }
    }
    println!("terminale dopo su: {:?}", String::from_utf8_lossy(&seen));

    // Come il pulsante "Re-inietta" dell'interfaccia.
    let detected = shell.reinject_cwd_tracking().await;
    println!("dopo su - {to}: rilevata {detected:?}");
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    shell.write(b"cd /etc\n".to_vec()).await.unwrap();
    let got = wait_cwd(&mut rx, "/etc", std::time::Duration::from_secs(3)).await;
    shell.close().await;
    assert!(
        detected
            .as_deref()
            .is_ok_and(|s| s.ends_with(expected_shell)),
        "shell rilevata {detected:?}, attesa {expected_shell}"
    );
    assert!(
        got.is_some(),
        "il tracciamento non è ripartito dopo su - {to}"
    );
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn accept_reinject_after_su_to_another_shell() {
    reinject_after_su("bashuser", "tcshuser", "tcsh").await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn accept_reinject_after_su_to_zsh() {
    reinject_after_su("bashuser", "zshuser", "zsh").await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn reinject_after_su_to_an_unsupported_shell_says_so_instead_of_injecting_the_wrong_one() {
    let (shell, _rx, _) = shell_with_tracking("bashuser").await;
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    shell.write(b"sudo su - ashuser\n".to_vec()).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
    let r = shell.reinject_cwd_tracking().await;
    shell.close().await;
    assert!(
        matches!(&r, Err(SshError::UnsupportedShell(s)) if s.ends_with("ash") || s.ends_with("busybox")),
        "{r:?}"
    );
}
