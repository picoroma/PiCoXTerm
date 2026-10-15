#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("connessione fallita: {0}")]
    Connect(String),
    #[error("autenticazione rifiutata dal server")]
    AuthFailed,
    #[error("la chiave privata è cifrata: serve la passphrase")]
    PassphraseRequired,
    #[error("chiave privata non utilizzabile (passphrase errata o formato non valido): {0}")]
    Key(String),
    #[error("nessun agent SSH raggiungibile (Pageant o agent OpenSSH non in esecuzione)")]
    AgentUnavailable,
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
    #[error("il server non consente l'inoltro TCP verso il salto successivo (AllowTcpForwarding)")]
    ForwardingDenied,
    #[error("shell non supportata per il tracciamento della directory: {0}")]
    UnsupportedShell(String),
    #[error("nessuna risposta entro {0} secondi")]
    Timeout(u64),
    #[error("{host}: {source}")]
    Target { host: String, source: Box<SshError> },
    #[error("jump host {host}: {source}")]
    Jump { host: String, source: Box<SshError> },
    #[error("errore di I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("errore SSH: {0}")]
    Ssh(#[from] russh::Error),
    #[error("il canale è chiuso")]
    ChannelClosed,
}
