//! TLS per FTPS (rustls con il backend `ring`, D-11): verifica standard con i certificati del
//! sistema e, in alternativa, il **pin** dell'impronta del certificato del server. Un certificato che
//! non passa la verifica standard e non corrisponde al pin non viene mai accettato in silenzio:
//! la connessione fallisce e riporta l'impronta, perché sia l'utente a decidere (come per le host key).

use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    ClientConfig, DigitallySignedStruct, Error as TlsError, RootCertStore, SignatureScheme,
};
use sha2::{Digest, Sha256};

/// Impronta SHA-256 di un certificato, come `AA:BB:...`.
pub fn fingerprint(cert: &CertificateDer<'_>) -> String {
    Sha256::digest(cert.as_ref())
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Impronta dell'ultimo certificato rifiutato, per mostrarla all'utente.
pub type SeenCert = Arc<Mutex<Option<String>>>;

#[derive(Debug)]
struct PinVerifier {
    standard: Arc<WebPkiServerVerifier>,
    /// Impronta che l'utente ha già accettato per questo server.
    pinned: Option<String>,
    seen: SeenCert,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let fp = fingerprint(end_entity);
        // Un certificato già accettato dall'utente vale anche se è autofirmato o scaduto.
        if self.pinned.as_deref() == Some(fp.as_str()) {
            return Ok(ServerCertVerified::assertion());
        }
        match self.standard.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(ok) => Ok(ok),
            Err(e) => {
                *self.seen.lock().unwrap() = Some(fp);
                Err(e)
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.standard.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.standard.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.standard.supported_verify_schemes()
    }
}

/// Configurazione TLS: certificati del sistema più l'eventuale pin. Il certificato rifiutato finisce
/// in `seen`.
pub fn client_config(pinned: Option<String>, seen: SeenCert) -> Result<ClientConfig, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().certs {
        // Un certificato di sistema che rustls non capisce non deve impedire di collegarsi.
        let _ = roots.add(cert);
    }
    let standard = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .map_err(|e| e.to_string())?;
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinVerifier {
            standard,
            pinned,
            seen,
        }))
        .with_no_client_auth();
    Ok(config)
}
