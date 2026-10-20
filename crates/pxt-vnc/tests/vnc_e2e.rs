//! Prove contro il server di `testenv/` (servizio `vnc`, TigerVNC su 127.0.0.1:5901, password `pxtvnc12`):
//! `docker compose up -d vnc`, poi `cargo test -p pxt-vnc --test vnc_e2e -- --ignored`.

use std::time::Duration;

use pxt_vnc::{VncError, VncEvent, VncSession};

async fn collect(s: &mut VncSession, at_least: usize) -> Vec<u8> {
    let mut all = Vec::new();
    while all.len() < at_least {
        match tokio::time::timeout(Duration::from_secs(8), s.events.recv()).await {
            Ok(Some(VncEvent::Data(d))) => all.extend_from_slice(&d),
            other => panic!("{other:?} dopo {} byte", all.len()),
        }
    }
    all
}

#[tokio::test]
#[ignore = "richiede il servizio vnc di testenv"]
async fn authenticates_and_delivers_the_server_init() {
    let mut s = VncSession::connect("127.0.0.1", 5901, Some("pxtvnc12"))
        .await
        .expect("connessione");
    // Finto handshake per noVNC: 12 + 2 + 4 byte.
    let hello = collect(&mut s, 18).await;
    assert_eq!(&hello[..12], b"RFB 003.008\n");
    // noVNC risponde: versione, scelta della sicurezza, poi ClientInit (condiviso). Al server arriva solo l'ultimo.
    let mut from_novnc = b"RFB 003.008\n".to_vec();
    from_novnc.extend_from_slice(&[1, 1]);
    assert!(s.handle.send(from_novnc).await);
    // ServerInit: larghezza, altezza (16 bit), formato pixel (16 byte), lunghezza del nome (32 bit), nome.
    let init = collect(&mut s, 24).await;
    let (w, h) = (
        u16::from_be_bytes([init[0], init[1]]),
        u16::from_be_bytes([init[2], init[3]]),
    );
    assert_eq!((w, h), (1280, 800), "{init:?}");
    s.handle.close();
}

#[tokio::test]
#[ignore = "richiede il servizio vnc di testenv"]
async fn a_wrong_or_missing_password_is_refused_without_leaking_it() {
    match VncSession::connect("127.0.0.1", 5901, Some("sbagliata")).await {
        Err(e @ VncError::AuthFailed(_)) => {
            let text = format!("{e} {e:?}");
            assert!(!text.contains("sbagliata"), "{text}");
        }
        other => panic!("{:?}", other.err()),
    }
    assert!(matches!(
        VncSession::connect("127.0.0.1", 5901, None).await,
        Err(VncError::PasswordRequired)
    ));
}
