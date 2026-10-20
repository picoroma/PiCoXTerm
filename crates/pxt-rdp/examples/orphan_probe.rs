//! Prova a mano: apre una sessione RDP (server di prova) e poi muore di colpo, senza chiusura
//! ordinata, per verificare che il motore non resti in giro. Uso: vedi `testenv/README.md`.
use pxt_rdp::{RdpError, RdpSession, SessionParams};

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
        keyboard_layout: 0x410,
        clipboard: false,
    }
}

#[tokio::main]
async fn main() {
    let fp = match RdpSession::connect(params(None)).await {
        Err(RdpError::UntrustedCertificate { fingerprint, .. }) => fingerprint,
        other => panic!("{:?}", other.err()),
    };
    let s = RdpSession::connect(params(Some(fp.clone())))
        .await
        .expect("connessione");
    // Una seconda sessione aperta mentre la prima è viva, e la prima chiusa: come con più schede.
    let mut s2 = RdpSession::connect(params(Some(fp)))
        .await
        .expect("seconda connessione");
    s.close();
    drop(s);
    // Si lascia scorrere un po' di grafica, come in una sessione vera.
    for _ in 0..8 {
        let _ = tokio::time::timeout(std::time::Duration::from_millis(300), s2.events.recv()).await;
    }
    println!("sessione aperta, muoio di colpo");
    std::process::abort();
}
