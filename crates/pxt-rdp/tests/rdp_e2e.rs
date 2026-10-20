//! Prove della sessione RDP integrata contro il server di `testenv/` (servizio `rdp`, xrdp su
//! 127.0.0.1:3390) e il motore `picoxterm-rdp`.
//!
//! ```text
//! docker compose up -d rdp                          (in testenv/)
//! cargo build --manifest-path rdp-engine/Cargo.toml
//! cargo test -p pxt-rdp --test rdp_e2e -- --ignored --test-threads=1
//! ```
//!
//! Il motore si cerca in `PXT_RDP_ENGINE`, altrimenti in `rdp-engine/target/debug`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pxt_rdp::{RdpError, RdpEvent, RdpInput, RdpSession, SessionParams};

fn use_dev_engine() {
    if std::env::var_os("PXT_RDP_ENGINE").is_none() {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../rdp-engine/target/debug")
            .join(pxt_rdp::ENGINE_NAME);
        std::env::set_var("PXT_RDP_ENGINE", p);
    }
}

fn params(pinned: Option<String>) -> SessionParams {
    SessionParams {
        host: "127.0.0.1".into(),
        port: 3390,
        username: "pxtuser".into(),
        password: "pxtpass-1234".into(),
        domain: None,
        width: 1024,
        height: 768,
        pinned_cert: pinned,
    }
}

async fn fingerprint() -> String {
    match RdpSession::connect(params(None)).await {
        Err(RdpError::UntrustedCertificate {
            fingerprint,
            changed,
        }) => {
            assert!(!changed);
            fingerprint
        }
        Err(e) => panic!("atteso un certificato non attendibile, ottenuto: {e}"),
        Ok(_) => panic!("il certificato autofirmato non deve essere accettato senza pin"),
    }
}

/// Raccoglie gli aggiornamenti per un po' e riporta (numero, pixel non neri nel primo, ultimo evento).
async fn drain(s: &mut RdpSession, how_long: Duration) -> (usize, Option<String>) {
    let end = Instant::now() + how_long;
    let mut updates = 0;
    let mut last = None;
    while let Ok(Some(ev)) = tokio::time::timeout_at(end.into(), s.events.recv()).await {
        match ev {
            RdpEvent::Update { .. } => updates += 1,
            RdpEvent::Disconnected(r) => {
                last = Some(r);
                break;
            }
            _ => {}
        }
    }
    (updates, last)
}

#[tokio::test]
#[ignore = "richiede il servizio rdp di testenv e il motore compilato"]
async fn an_unknown_certificate_is_never_accepted_silently() {
    use_dev_engine();
    let fp = fingerprint().await;
    assert_eq!(fp.split(':').count(), 32, "{fp}");

    // Con un'impronta diversa da quella del server: "cambiato".
    let wrong = "00:".repeat(31) + "00";
    match RdpSession::connect(params(Some(wrong))).await {
        Err(RdpError::UntrustedCertificate {
            fingerprint,
            changed,
        }) => {
            assert!(changed);
            assert_eq!(fingerprint, fp);
        }
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("un'impronta diversa non deve essere accettata"),
    }
}

#[tokio::test]
#[ignore = "richiede il servizio rdp di testenv e il motore compilato"]
async fn connects_with_nla_after_pinning_and_receives_the_desktop() {
    use_dev_engine();
    let fp = fingerprint().await;
    let mut s = RdpSession::connect(params(Some(fp)))
        .await
        .expect("connessione");
    assert_eq!((s.width, s.height), (1024, 768));

    // Compare la schermata di login di xrdp: molti aggiornamenti, e non è tutta nera.
    let mut updates = 0;
    let mut painted = false;
    let end = Instant::now() + Duration::from_secs(8);
    while Instant::now() < end && updates < 20 {
        match tokio::time::timeout(Duration::from_secs(2), s.events.recv()).await {
            Ok(Some(RdpEvent::Update {
                rgba,
                width,
                height,
                ..
            })) => {
                assert_eq!(rgba.len(), usize::from(width) * usize::from(height) * 4);
                painted |= rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|p| p[0] | p[1] | p[2] != 0);
                updates += 1;
            }
            Ok(Some(RdpEvent::Disconnected(r))) => panic!("disconnesso: {r}"),
            Ok(Some(_)) => {}
            _ => break,
        }
    }
    assert!(updates >= 5, "troppo pochi aggiornamenti: {updates}");
    assert!(painted, "tutti i pixel sono neri");
    s.close();
}

#[tokio::test]
#[ignore = "richiede il servizio rdp di testenv e il motore compilato"]
async fn keyboard_and_mouse_reach_the_server_and_close_is_orderly() {
    use_dev_engine();
    let fp = fingerprint().await;
    let mut s = RdpSession::connect(params(Some(fp)))
        .await
        .expect("connessione");
    // Aspetta che la schermata di login sia disegnata.
    let (first, _) = drain(&mut s, Duration::from_secs(3)).await;
    assert!(first > 0);

    // Scrive nel campo della password: ogni tasto fa disegnare un asterisco.
    assert!(s.send(RdpInput::MouseMove { x: 500, y: 400 }));
    for sc in [0x1Eu16, 0x30, 0x2E] {
        assert!(s.send(RdpInput::Key {
            scancode: sc,
            pressed: true
        }));
        assert!(s.send(RdpInput::Key {
            scancode: sc,
            pressed: false
        }));
    }
    let (after, ended) = drain(&mut s, Duration::from_secs(3)).await;
    assert!(ended.is_none(), "la sessione non doveva finire: {ended:?}");
    assert!(after > 0, "nessun aggiornamento dopo i tasti");

    // Chiusura ordinata: arriva "Disconnected".
    s.close();
    let (_, reason) = drain(&mut s, Duration::from_secs(5)).await;
    assert!(reason.is_some(), "manca la notifica di fine sessione");
}

#[tokio::test]
#[ignore = "richiede il motore compilato"]
async fn errors_are_clear_and_never_contain_the_password() {
    use_dev_engine();
    // Porta chiusa.
    let mut p = params(None);
    p.port = 1;
    p.password = "password-molto-segreta-xyz".into();
    let started = Instant::now();
    let err = match RdpSession::connect(p).await {
        Err(e) => e,
        Ok(_) => panic!("non doveva collegarsi"),
    };
    assert!(matches!(err, RdpError::Connect(_)), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(20));
    let text = format!("{err} {err:?}");
    assert!(!text.contains("password-molto-segreta-xyz"), "{text}");

    // Motore mancante.
    std::env::set_var("PXT_RDP_ENGINE", "C:\\non\\esiste\\picoxterm-rdp.exe");
    assert!(matches!(
        RdpSession::connect(params(None)).await,
        Err(RdpError::EngineMissing(_))
    ));
    std::env::remove_var("PXT_RDP_ENGINE");
}
