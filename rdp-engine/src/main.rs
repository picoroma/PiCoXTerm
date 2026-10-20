//! `picoxterm-rdp`: il motore RDP di PiCoXterm. Non si lancia a mano: lo avvia l'app e ci parla con il
//! protocollo `pxt-rdp-proto` su stdin/stdout. Niente porte in ascolto; i log vanno su stderr e non
//! contengono mai la password.

mod clipboard;
mod parent;
mod session;
mod tls;

use std::io::{self, BufReader, BufWriter, Write};

use pxt_rdp_proto::{
    read_to_engine, write_from_engine, CertUntrusted, Connected, Failed, FromEngine, ToEngine,
};
use tokio::sync::mpsc;

use crate::session::{Command, ConnectError};

fn main() {
    use std::io::IsTerminal as _;
    if io::stdin().is_terminal() {
        // Avviato a mano (doppio clic, terminale): non serve a nulla da solo.
        eprintln!("picoxterm-rdp è il motore RDP di PiCoXterm: lo avvia il programma, non va lanciato a mano.");
        eprintln!("Premi Invio per chiudere.");
        let _ = io::stdin().read_line(&mut String::new());
        return;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let code = real_main().await;
        // Il lettore di stdin può essere bloccato in una lettura: si esce senza aspettarlo.
        std::process::exit(code);
    });
}

async fn real_main() -> i32 {
    // Uscita verso l'app: un thread scrive i messaggi in ordine su stdout.
    let (out_tx, mut out_rx) = mpsc::channel::<FromEngine>(64);
    std::thread::spawn(move || {
        let mut out = BufWriter::new(io::stdout().lock());
        while let Some(m) = out_rx.blocking_recv() {
            if write_from_engine(&mut out, &m).is_err() || out.flush().is_err() {
                break;
            }
        }
    });

    // Ingresso dall'app: un thread legge i messaggi; la fine del flusso (l'app è finita) chiude tutto.
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Command>();
    let (connect_tx, connect_rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut input = BufReader::new(io::stdin().lock());
        let mut connect_tx = Some(connect_tx);
        loop {
            match read_to_engine(&mut input) {
                Ok(Some(ToEngine::Connect(c))) => {
                    if let Some(tx) = connect_tx.take() {
                        let _ = tx.send(c);
                    }
                }
                Ok(Some(ToEngine::Input(i))) => {
                    let _ = cmd_tx.send(Command::Input(i));
                }
                Ok(Some(ToEngine::Repaint)) => {
                    let _ = cmd_tx.send(Command::Repaint);
                }
                Ok(Some(ToEngine::Resize { width, height })) => {
                    let _ = cmd_tx.send(Command::Resize { width, height });
                }
                Ok(Some(ToEngine::SetClipboard(t))) => {
                    let _ = cmd_tx.send(Command::SetClipboard(t));
                }
                Ok(Some(ToEngine::Close)) | Ok(None) | Err(_) => {
                    let _ = cmd_tx.send(Command::Close);
                    break;
                }
            }
        }
    });

    let Ok(params) = connect_rx.await else {
        return 0;
    };
    // Se l'app che mi ha avviato finisce, finisco anch'io (anche se le pipe non si chiudessero).
    parent::exit_with(params.parent_pid);
    match session::connect(params).await {
        Err(e) => {
            let msg = match e {
                ConnectError::Untrusted {
                    fingerprint,
                    changed,
                } => FromEngine::CertUntrusted(CertUntrusted {
                    fingerprint,
                    changed,
                }),
                ConnectError::Connect(message) => FromEngine::Failed(Failed {
                    kind: "connect".into(),
                    message,
                }),
                ConnectError::Auth(message) => FromEngine::Failed(Failed {
                    kind: "auth".into(),
                    message,
                }),
            };
            let _ = out_tx.send(msg).await;
            // Lascia al thread di uscita il tempo di svuotare la coda.
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            0
        }
        Ok(est) => {
            let _ = out_tx
                .send(FromEngine::Connected(Connected {
                    width: est.width,
                    height: est.height,
                }))
                .await;
            let reason = session::run(est, out_tx.clone(), cmd_rx).await;
            let _ = out_tx.send(FromEngine::Disconnected(reason)).await;
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            0
        }
    }
}
