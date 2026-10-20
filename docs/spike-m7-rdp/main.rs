use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use ironrdp::connector::{self, ClientConnector, Credentials, DesktopSize};
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStageBuilder, ActiveStageOutput};
use ironrdp_pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{connect_begin, connect_finalize, mark_as_upgraded, FramedWrite as _, TokioFramed};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_rustls::rustls;
use tokio_rustls::rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{DigitallySignedStruct, SignatureScheme};

/// Verificatore del certificato: in M7 sarà il nostro PinVerifier (impronta SHA-256 confermata
/// dall'utente). Nello spike accetta tutto ma stampa l'impronta.
#[derive(Debug)]
struct PrintFingerprint;

impl ServerCertVerifier for PrintFingerprint {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = Sha256::digest(end_entity.as_ref());
        println!("certificato del server, SHA-256: {}", hex(&fp));
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(":")
}

fn config(user: &str, pass: &str) -> connector::Config {
    connector::Config {
        credentials: Credentials::UsernamePassword {
            username: user.into(),
            password: pass.into(),
        },
        domain: None,
        enable_tls: true,
        enable_credssp: true,
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: DesktopSize { width: 1280, height: 800 },
        bitmap: Some(connector::BitmapConfig {
            lossy_compression: true,
            color_depth: 16,
            codecs: ironrdp_pdu::rdp::capability_sets::client_codecs_capabilities(&[]).unwrap(),
        }),
        client_build: 0,
        client_name: "picoxterm-spike".into(),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".into(),
        platform: MajorPlatformType::WINDOWS,
        enable_server_pointer: true,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        compression_type: None,
        pointer_software_rendering: true,
        multitransport_flags: None,
        performance_flags: PerformanceFlags::default(),
        desktop_scale_factor: 0,
        hardware_id: None,
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let host = args.get(1).map_or("127.0.0.1", String::as_str);
    let port: u16 = args.get(2).and_then(|p| p.parse().ok()).unwrap_or(3390);
    let user = args.get(3).map_or("pxtuser", String::as_str);
    let pass = args.get(4).map_or("pxtpass-1234", String::as_str);

    let t0 = Instant::now();
    let tcp = TcpStream::connect((host, port)).await.context("TCP")?;
    let client_addr = tcp.local_addr()?;
    let mut framed = TokioFramed::new(tcp);
    let mut connector = ClientConnector::new(config(user, pass), client_addr);

    let should_upgrade = connect_begin(&mut framed, &mut connector).await.context("begin")?;
    println!("negoziazione X.224 fatta in {:?}", t0.elapsed());

    // Passaggio a TLS con il nostro codice (rustls + ring), non con ironrdp-tls.
    let tcp = framed.into_inner_no_leftover();
    let mut tls_cfg = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PrintFingerprint))
        .with_no_client_auth();
    // CredSSP non supporta la ripresa delle sessioni TLS.
    tls_cfg.resumption = rustls::client::Resumption::disabled();
    let name = ServerName::try_from(host.to_owned())?;
    let mut tls = tokio_rustls::TlsConnector::from(Arc::new(tls_cfg)).connect(name, tcp).await.context("TLS")?;
    tokio::io::AsyncWriteExt::flush(&mut tls).await?;
    let (_, conn) = tls.get_ref();
    println!(
        "TLS: {:?} {:?}",
        conn.protocol_version(),
        conn.negotiated_cipher_suite().map(|s| s.suite())
    );
    let server_public_key = {
        use x509_cert::der::Decode as _;
        let der = conn.peer_certificates().and_then(|c| c.first()).context("certificato mancante")?;
        let cert = x509_cert::Certificate::from_der(der.as_ref())?;
        cert.tbs_certificate
            .subject_public_key_info
            .subject_public_key
            .as_bytes()
            .context("chiave pubblica")?
            .to_vec()
    };

    let upgraded = mark_as_upgraded(should_upgrade, &mut connector);
    let mut framed = TokioFramed::new(tls);
    let mut net = ReqwestNetworkClient::new();
    let result = connect_finalize(
        upgraded,
        connector,
        &mut framed,
        &mut net,
        host.to_owned().into(),
        server_public_key,
        None,
    )
    .await
    .context("finalize (NLA/CredSSP)")?;
    println!(
        "connesso in {:?}: desktop {}x{}",
        t0.elapsed(),
        result.desktop_size.width,
        result.desktop_size.height
    );

    let mut image = DecodedImage::new(
        ironrdp_graphics::image_processing::PixelFormat::RgbA32,
        result.desktop_size.width,
        result.desktop_size.height,
    );
    let mut stage = ActiveStageBuilder {
        static_channels: result.static_channels,
        user_channel_id: result.user_channel_id,
        io_channel_id: result.io_channel_id,
        message_channel_id: result.message_channel_id,
        share_id: result.share_id,
        compression_type: result.compression_type,
        enable_server_pointer: result.enable_server_pointer,
        pointer_software_rendering: result.pointer_software_rendering,
    }
    .build();
    let deadline = Instant::now() + Duration::from_secs(12);
    let (mut updates, mut bytes) = (0usize, 0usize);
    'outer: while Instant::now() < deadline {
        let read = tokio::time::timeout(Duration::from_secs(2), framed.read_pdu()).await;
        let (action, payload) = match read {
            Err(_) => continue,
            Ok(r) => r.context("lettura")?,
        };
        bytes += payload.len();
        for out in stage.process(&mut image, action, &payload)? {
            match out {
                ActiveStageOutput::ResponseFrame(f) => framed.write_all(&f).await?,
                ActiveStageOutput::GraphicsUpdate(_) => updates += 1,
                ActiveStageOutput::Terminate(r) => {
                    println!("terminato: {r:?}");
                    break 'outer;
                }
                _ => {}
            }
        }
    }
    println!("aggiornamenti grafici: {updates}, byte ricevuti: {bytes}");
    let img: image::ImageBuffer<image::Rgba<u8>, _> =
        image::ImageBuffer::from_raw(u32::from(image.width()), u32::from(image.height()), image.data())
            .context("immagine")?;
    let non_black = img.pixels().filter(|p| p.0[0] | p.0[1] | p.0[2] != 0).count();
    println!("pixel non neri: {non_black} su {}", img.pixels().len());
    img.save("spike.png")?;
    Ok(())
}
