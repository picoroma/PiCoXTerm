#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("connessione fallita: {0}")]
    Connect(String),
    #[error("autenticazione rifiutata dal server")]
    AuthFailed,
    #[error("host key rifiutata dall'utente")]
    HostKeyRejected,
    #[error(
        "HOST KEY CAMBIATA per {host}:{port}. Atteso {expected}, ricevuto {got}. \
         Connessione bloccata: rimuovere a mano la voce da known_hosts solo se il cambio è legittimo."
    )]
    HostKeyChanged {
        host: String,
        port: u16,
        expected: String,
        got: String,
    },
    #[error("tipo di host key non supportato (certificati non ancora gestiti)")]
    UnsupportedHostKey,
    #[error("errore di I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("errore SSH: {0}")]
    Ssh(#[from] russh::Error),
    #[error("il canale è chiuso")]
    ChannelClosed,
}
