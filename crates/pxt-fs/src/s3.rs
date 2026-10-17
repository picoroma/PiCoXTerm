//! Backend S3 (e compatibili: MinIO, RustFS, Ceph...) su `aws-sdk-s3`, con TLS `rustls` e backend
//! crittografico `ring` (D-11: niente `aws-lc`, niente NASM).
//!
//! **Percorsi.** `/` è l'elenco dei bucket dell'account, `/bucket` la radice di un bucket,
//! `/bucket/cartella/oggetto` un oggetto. Le "cartelle" di S3 sono prefissi: esistono solo se contengono
//! qualcosa o se c'è un oggetto segnaposto `cartella/` (lo crea `mkdir`).
//!
//! **Scrittura.** Un oggetto compare solo a caricamento completo: sotto 8 MiB con una sola richiesta,
//! sopra con il caricamento a parti (`multipart`), che tiene in memoria al massimo qualche parte alla
//! volta (mai il file intero, regola 7). Un caricamento interrotto viene annullato e non lascia nulla.
//!
//! **Segreti.** La chiave segreta resta in memoria solo per firmare le richieste e non compare mai in
//! `Debug` né nei messaggi d'errore.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::config::Region;
use aws_sdk_s3::error::{ProvideErrorMetadata, SdkError};
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{
    CompletedMultipartUpload, CompletedPart, MetadataDirective, StorageClass, Tag, Tagging,
};
use aws_sdk_s3::Client;
use aws_smithy_http_client::tls;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::OnceCell;
use tokio::task::JoinHandle;

use crate::{Entry, EntryKind, FsCapabilities, FsError, RemoteFs, Result};

/// Sotto questa dimensione un oggetto si carica con una sola richiesta.
const PART_SIZE: usize = 8 * 1024 * 1024;
/// Dopo mille parti si passa a parti più grandi: S3 ne ammette al massimo 10.000.
const BIG_PART_SIZE: usize = 64 * 1024 * 1024;
const PARTS_BEFORE_BIG: i32 = 1000;
/// Parti in volo insieme per un oggetto: tiene la memoria sotto (MAX_INFLIGHT + 1) × dimensione parte.
const MAX_INFLIGHT: usize = 3;

pub struct S3Params {
    /// Indirizzo del servizio (MinIO, RustFS...); `None` per AWS.
    pub endpoint: Option<String>,
    pub region: String,
    /// Chiavi di accesso. Se mancano si usa il profilo (o la catena di credenziali standard di AWS).
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
    pub session_token: Option<String>,
    /// Profilo AWS (`~/.aws/config`), anche con accesso SSO.
    pub profile: Option<String>,
    /// Indirizzi del tipo `endpoint/bucket` (necessario per MinIO e simili) invece di `bucket.endpoint`.
    pub path_style: bool,
    /// "Bucket esterno": l'account non può elencare i bucket, si lavora su uno solo.
    pub bucket: Option<String>,
}

impl std::fmt::Debug for S3Params {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Params")
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("access_key", &self.access_key)
            .field(
                "secret_key",
                &self.secret_key.as_ref().map(|_| "<nascosta>"),
            )
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct S3Fs {
    client: Client,
    /// Se impostato, `/` elenca solo questo bucket.
    fixed_bucket: Option<String>,
}

/// Dettagli di un oggetto (schede "Proprietà", "Intestazioni" e "Tag").
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectInfo {
    pub size: u64,
    pub etag: Option<String>,
    pub modified: Option<i64>,
    pub storage_class: Option<String>,
    pub content_type: Option<String>,
    pub cache_control: Option<String>,
    pub content_disposition: Option<String>,
    pub content_encoding: Option<String>,
    /// Metadati dell'utente (`x-amz-meta-*`), senza il prefisso.
    pub metadata: HashMap<String, String>,
    pub version_id: Option<String>,
    pub encryption: Option<String>,
    pub tags: Vec<(String, String)>,
}

/// Intestazioni modificabili di un oggetto.
#[derive(Debug, Clone, Default)]
pub struct ObjectHeaders {
    pub content_type: Option<String>,
    pub cache_control: Option<String>,
    pub content_disposition: Option<String>,
    pub content_encoding: Option<String>,
    pub metadata: HashMap<String, String>,
}

/// Divide `/bucket/chiave` in bucket e chiave (senza `/` iniziale né finale).
fn split_path(path: &str) -> (Option<String>, String) {
    let parts: Vec<&str> = path
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    // `..` toglie il livello precedente.
    let mut stack: Vec<&str> = Vec::new();
    for p in parts {
        if p == ".." {
            stack.pop();
        } else {
            stack.push(p);
        }
    }
    match stack.split_first() {
        None => (None, String::new()),
        Some((bucket, rest)) => (Some((*bucket).to_string()), rest.join("/")),
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

fn map_err<E>(e: SdkError<E>, path: &str) -> FsError
where
    E: ProvideErrorMetadata + std::error::Error + Send + Sync + 'static,
{
    let code = e.code().unwrap_or_default().to_string();
    let message = e.message().map(str::to_string);
    match code.as_str() {
        "NoSuchKey" | "NoSuchBucket" | "NotFound" | "NoSuchUpload" | "NoSuchVersion" => {
            FsError::NotFound(path.to_string())
        }
        "AccessDenied" | "Forbidden" | "AllAccessDisabled" => {
            FsError::PermissionDenied(path.to_string())
        }
        "InvalidAccessKeyId" | "SignatureDoesNotMatch" | "ExpiredToken" | "InvalidToken" => {
            FsError::S3(format!("credenziali rifiutate dal servizio ({code})"))
        }
        _ => {
            // Il testo dell'SDK non contiene la chiave segreta; si tiene il codice, che dice cosa è successo.
            let detail = message.unwrap_or_else(|| {
                if code.is_empty() {
                    // Errori di rete e simili.
                    format!("{e}")
                } else {
                    code.clone()
                }
            });
            FsError::S3(format!("{path}: {detail}"))
        }
    }
}

fn secs(t: &aws_sdk_s3::primitives::DateTime) -> i64 {
    t.secs()
}

fn dir_entry(path: String, name: String, modified: Option<i64>) -> Entry {
    Entry {
        name,
        path,
        kind: EntryKind::Dir,
        is_symlink: false,
        size: 0,
        modified,
        mode: None,
        owner: None,
        group: None,
        uid: None,
        gid: None,
    }
}

impl S3Fs {
    /// Crea il client e verifica che le credenziali funzionino (elenco dei bucket, o accesso al bucket
    /// indicato per i "bucket esterni").
    pub async fn connect(params: S3Params) -> Result<S3Fs> {
        // TLS con `ring`: lo stesso backend del resto del progetto (D-11).
        let http = aws_smithy_http_client::Builder::new()
            .tls_provider(tls::Provider::rustls(
                tls::rustls_provider::CryptoMode::Ring,
            ))
            .build_https();
        let mut loader = aws_config::defaults(BehaviorVersion::latest())
            .http_client(http)
            .region(Region::new(params.region.clone()));
        match (&params.access_key, &params.secret_key) {
            (Some(access), Some(secret)) => {
                loader = loader.credentials_provider(Credentials::new(
                    access,
                    secret,
                    params.session_token.clone(),
                    None,
                    "picoxterm",
                ));
            }
            _ => {
                if let Some(profile) = &params.profile {
                    loader = loader.profile_name(profile);
                }
            }
        }
        let shared = loader.load().await;
        let mut builder =
            aws_sdk_s3::config::Builder::from(&shared).force_path_style(params.path_style);
        if let Some(endpoint) = &params.endpoint {
            builder = builder.endpoint_url(endpoint);
        }
        let fs = S3Fs {
            client: Client::from_conf(builder.build()),
            fixed_bucket: params.bucket.clone().filter(|b| !b.is_empty()),
        };
        // Prova di accesso.
        match &fs.fixed_bucket {
            Some(bucket) => {
                fs.client
                    .head_bucket()
                    .bucket(bucket)
                    .send()
                    .await
                    .map_err(|e| map_err(e, &format!("/{bucket}")))?;
            }
            None => {
                fs.client
                    .list_buckets()
                    .send()
                    .await
                    .map_err(|e| map_err(e, "/"))?;
            }
        }
        Ok(fs)
    }

    async fn list_buckets(&self) -> Result<Vec<Entry>> {
        if let Some(b) = &self.fixed_bucket {
            return Ok(vec![dir_entry(format!("/{b}"), b.clone(), None)]);
        }
        let out = self
            .client
            .list_buckets()
            .send()
            .await
            .map_err(|e| map_err(e, "/"))?;
        Ok(out
            .buckets()
            .iter()
            .filter_map(|b| {
                let name = b.name()?.to_string();
                Some(dir_entry(
                    format!("/{name}"),
                    name,
                    b.creation_date().map(secs),
                ))
            })
            .collect())
    }

    /// Voci "dirette" di un prefisso: cartelle e oggetti, senza scendere.
    async fn list_prefix(&self, bucket: &str, key: &str) -> Result<Vec<Entry>> {
        let prefix = if key.is_empty() {
            String::new()
        } else {
            format!("{key}/")
        };
        let base = if key.is_empty() {
            format!("/{bucket}")
        } else {
            format!("/{bucket}/{key}")
        };
        let mut out = Vec::new();
        let mut pages = self
            .client
            .list_objects_v2()
            .bucket(bucket)
            .prefix(&prefix)
            .delimiter("/")
            .into_paginator()
            .send();
        while let Some(page) = pages.next().await {
            let page = page.map_err(|e| map_err(e, &base))?;
            for cp in page.common_prefixes() {
                let Some(p) = cp.prefix() else { continue };
                let name = p[prefix.len()..].trim_end_matches('/');
                if !name.is_empty() {
                    out.push(dir_entry(join(&base, name), name.to_string(), None));
                }
            }
            for obj in page.contents() {
                let Some(k) = obj.key() else { continue };
                let name = &k[prefix.len()..];
                // Il segnaposto della cartella stessa (`cartella/`) non è una voce.
                if name.is_empty() || name.ends_with('/') {
                    continue;
                }
                out.push(Entry {
                    name: name.to_string(),
                    path: join(&base, name),
                    kind: EntryKind::File,
                    is_symlink: false,
                    size: obj.size().map_or(0, |s| u64::try_from(s).unwrap_or(0)),
                    modified: obj.last_modified().map(secs),
                    mode: None,
                    owner: None,
                    group: None,
                    uid: None,
                    gid: None,
                });
            }
        }
        Ok(out)
    }

    /// Tutte le chiavi sotto un prefisso (senza delimitatore), per copiare o spostare una cartella.
    async fn keys_under(&self, bucket: &str, key: &str) -> Result<Vec<String>> {
        let prefix = format!("{key}/");
        let mut keys = Vec::new();
        let mut pages = self
            .client
            .list_objects_v2()
            .bucket(bucket)
            .prefix(&prefix)
            .into_paginator()
            .send();
        while let Some(page) = pages.next().await {
            let page = page.map_err(|e| map_err(e, &format!("/{bucket}/{key}")))?;
            keys.extend(
                page.contents()
                    .iter()
                    .filter_map(|o| o.key().map(str::to_string)),
            );
        }
        Ok(keys)
    }

    async fn copy_key(&self, bucket: &str, from: &str, to: &str) -> Result<()> {
        // L'origine va nel formato `bucket/chiave`, con la chiave codificata.
        let source = format!("{bucket}/{}", urlencode_key(from));
        self.client
            .copy_object()
            .bucket(bucket)
            .copy_source(source)
            .key(to)
            .send()
            .await
            .map_err(|e| map_err(e, &format!("/{bucket}/{from}")))?;
        Ok(())
    }

    // ---- Operazioni specifiche di S3 (schede dell'oggetto) ----

    /// Proprietà, intestazioni, metadati e tag di un oggetto.
    pub async fn object_info(&self, path: &str) -> Result<ObjectInfo> {
        let (Some(bucket), key) = split_path(path) else {
            return Err(FsError::NotRegularFile(path.to_string()));
        };
        if key.is_empty() {
            return Err(FsError::NotRegularFile(path.to_string()));
        }
        let head = self
            .client
            .head_object()
            .bucket(&bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| map_err(e, path))?;
        // I tag si leggono a parte; se il servizio non li supporta si mostra l'oggetto senza.
        let tags = match self
            .client
            .get_object_tagging()
            .bucket(&bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(t) => t
                .tag_set()
                .iter()
                .map(|t| (t.key().to_string(), t.value().to_string()))
                .collect(),
            Err(_) => Vec::new(),
        };
        Ok(ObjectInfo {
            size: head
                .content_length()
                .map_or(0, |s| u64::try_from(s).unwrap_or(0)),
            etag: head.e_tag().map(|e| e.trim_matches('"').to_string()),
            modified: head.last_modified().map(secs),
            storage_class: head.storage_class().map(|s| s.as_str().to_string()),
            content_type: head.content_type().map(str::to_string),
            cache_control: head.cache_control().map(str::to_string),
            content_disposition: head.content_disposition().map(str::to_string),
            content_encoding: head.content_encoding().map(str::to_string),
            metadata: head.metadata().cloned().unwrap_or_default(),
            version_id: head.version_id().map(str::to_string),
            encryption: head
                .server_side_encryption()
                .map(|s| s.as_str().to_string()),
            tags,
        })
    }

    /// Cambia intestazioni e metadati copiando l'oggetto su sé stesso (S3 non li modifica sul posto).
    /// I tag e la classe di storage restano quelli attuali.
    pub async fn set_headers(&self, path: &str, headers: &ObjectHeaders) -> Result<()> {
        let (Some(bucket), key) = split_path(path) else {
            return Err(FsError::NotRegularFile(path.to_string()));
        };
        let current = self.object_info(path).await?;
        let mut req = self
            .client
            .copy_object()
            .bucket(&bucket)
            .key(&key)
            .copy_source(format!("{bucket}/{}", urlencode_key(&key)))
            .metadata_directive(MetadataDirective::Replace)
            .set_metadata(Some(headers.metadata.clone()))
            .set_content_type(headers.content_type.clone())
            .set_cache_control(headers.cache_control.clone())
            .set_content_disposition(headers.content_disposition.clone())
            .set_content_encoding(headers.content_encoding.clone());
        if let Some(class) = current.storage_class.as_deref() {
            req = req.storage_class(StorageClass::from(class));
        }
        req.send().await.map_err(|e| map_err(e, path))?;
        Ok(())
    }

    /// Sostituisce i tag dell'oggetto.
    pub async fn set_tags(&self, path: &str, tags: &[(String, String)]) -> Result<()> {
        let (Some(bucket), key) = split_path(path) else {
            return Err(FsError::NotRegularFile(path.to_string()));
        };
        let set = tags
            .iter()
            .map(|(k, v)| Tag::builder().key(k).value(v).build())
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| FsError::S3(e.to_string()))?;
        let tagging = Tagging::builder()
            .set_tag_set(Some(set))
            .build()
            .map_err(|e| FsError::S3(e.to_string()))?;
        self.client
            .put_object_tagging()
            .bucket(&bucket)
            .key(&key)
            .tagging(tagging)
            .send()
            .await
            .map_err(|e| map_err(e, path))?;
        Ok(())
    }

    /// Cambia la classe di storage (copia l'oggetto su sé stesso con la nuova classe).
    pub async fn set_storage_class(&self, path: &str, class: &str) -> Result<()> {
        let (Some(bucket), key) = split_path(path) else {
            return Err(FsError::NotRegularFile(path.to_string()));
        };
        self.client
            .copy_object()
            .bucket(&bucket)
            .key(&key)
            .copy_source(format!("{bucket}/{}", urlencode_key(&key)))
            .metadata_directive(MetadataDirective::Copy)
            .storage_class(StorageClass::from(class))
            .send()
            .await
            .map_err(|e| map_err(e, path))?;
        Ok(())
    }

    /// Indirizzo temporaneo per scaricare l'oggetto senza credenziali (valido `expires`).
    pub async fn presigned_url(&self, path: &str, expires: Duration) -> Result<String> {
        let (Some(bucket), key) = split_path(path) else {
            return Err(FsError::NotRegularFile(path.to_string()));
        };
        let cfg = PresigningConfig::expires_in(expires).map_err(|e| FsError::S3(e.to_string()))?;
        let req = self
            .client
            .get_object()
            .bucket(&bucket)
            .key(&key)
            .presigned(cfg)
            .await
            .map_err(|e| map_err(e, path))?;
        Ok(req.uri().to_string())
    }
}

/// Codifica una chiave per l'intestazione `x-amz-copy-source` (tutto tranne le `/`).
fn urlencode_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for b in key.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---- Lettura e scrittura a flusso ----

/// Oggetto letto a flusso (opzionalmente da un certo punto, con `Range`).
pub struct S3Reader(Pin<Box<dyn AsyncRead + Send>>);

impl AsyncRead for S3Reader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.0.as_mut().poll_read(cx, buf)
    }
}

type PartResult = Result<CompletedPart>;
type Finishing = Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>;

/// Oggetto scritto a flusso: si accumulano parti da `PART_SIZE`, che partono in parallelo.
pub struct S3Writer {
    client: Client,
    bucket: String,
    key: String,
    buf: Vec<u8>,
    /// Identificatore del caricamento a parti, creato alla prima parte.
    upload_id: Arc<OnceCell<String>>,
    next_part: i32,
    inflight: VecDeque<JoinHandle<PartResult>>,
    done: Vec<CompletedPart>,
    finishing: Option<Finishing>,
    finished: bool,
}

fn part_target(next_part: i32) -> usize {
    if next_part > PARTS_BEFORE_BIG {
        BIG_PART_SIZE
    } else {
        PART_SIZE
    }
}

fn io_err(e: FsError) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

impl S3Writer {
    fn new(client: Client, bucket: String, key: String) -> Self {
        S3Writer {
            client,
            bucket,
            key,
            buf: Vec::new(),
            upload_id: Arc::default(),
            next_part: 1,
            inflight: VecDeque::new(),
            done: Vec::new(),
            finishing: None,
            finished: false,
        }
    }

    /// Avvia l'invio del contenuto accumulato come una parte.
    fn start_part(&mut self) {
        let body = std::mem::take(&mut self.buf);
        let number = self.next_part;
        self.next_part += 1;
        let (client, bucket, key) = (self.client.clone(), self.bucket.clone(), self.key.clone());
        let upload_id = Arc::clone(&self.upload_id);
        self.inflight.push_back(tokio::spawn(async move {
            let path = format!("/{bucket}/{key}");
            let id = upload_id
                .get_or_try_init(|| async {
                    client
                        .create_multipart_upload()
                        .bucket(&bucket)
                        .key(&key)
                        .send()
                        .await
                        .map_err(|e| map_err(e, &path))?
                        .upload_id()
                        .map(str::to_string)
                        .ok_or_else(|| {
                            FsError::S3("il servizio non ha dato l'id del caricamento".into())
                        })
                })
                .await?
                .clone();
            let out = client
                .upload_part()
                .bucket(&bucket)
                .key(&key)
                .upload_id(&id)
                .part_number(number)
                .body(ByteStream::from(body))
                .send()
                .await
                .map_err(|e| map_err(e, &path))?;
            Ok(CompletedPart::builder()
                .part_number(number)
                .set_e_tag(out.e_tag().map(str::to_string))
                .build())
        }));
    }

    /// Aspetta la parte più vecchia; `Pending` se non è ancora finita.
    fn poll_oldest(&mut self, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let Some(front) = self.inflight.front_mut() else {
            return Poll::Ready(Ok(()));
        };
        match Pin::new(front).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(joined) => {
                self.inflight.pop_front();
                match joined {
                    Ok(Ok(part)) => {
                        self.done.push(part);
                        Poll::Ready(Ok(()))
                    }
                    Ok(Err(e)) => Poll::Ready(Err(io_err(e))),
                    Err(e) => Poll::Ready(Err(std::io::Error::other(e.to_string()))),
                }
            }
        }
    }
}

impl AsyncWrite for S3Writer {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.finishing.is_some() || self.finished {
            return Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "scrittura S3 già conclusa",
            )));
        }
        // Se ci sono troppe parti in volo si aspetta la più vecchia (così la memoria resta limitata).
        while self.inflight.len() >= MAX_INFLIGHT {
            match self.poll_oldest(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(())) => {}
            }
        }
        let target = part_target(self.next_part);
        let room = target - self.buf.len();
        let n = data.len().min(room);
        self.buf.extend_from_slice(&data[..n]);
        if self.buf.len() >= target {
            self.start_part();
        }
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        // Le parti partono da sole quando sono piene: il resto si invia alla chiusura.
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.finished {
            return Poll::Ready(Ok(()));
        }
        if self.finishing.is_none() {
            let (client, bucket, key) =
                (self.client.clone(), self.bucket.clone(), self.key.clone());
            let path = format!("/{bucket}/{key}");
            if self.next_part == 1 {
                // Piccolo: una sola richiesta, niente caricamento a parti.
                let body = std::mem::take(&mut self.buf);
                self.finishing = Some(Box::pin(async move {
                    client
                        .put_object()
                        .bucket(&bucket)
                        .key(&key)
                        .body(ByteStream::from(body))
                        .send()
                        .await
                        .map_err(|e| io_err(map_err(e, &path)))?;
                    Ok(())
                }));
            } else {
                if !self.buf.is_empty() {
                    self.start_part();
                }
                let handles: Vec<_> = self.inflight.drain(..).collect();
                let mut parts = std::mem::take(&mut self.done);
                let upload_id = Arc::clone(&self.upload_id);
                self.finishing = Some(Box::pin(async move {
                    for h in handles {
                        match h.await {
                            Ok(Ok(p)) => parts.push(p),
                            Ok(Err(e)) => return Err(io_err(e)),
                            Err(e) => return Err(std::io::Error::other(e.to_string())),
                        }
                    }
                    parts.sort_by_key(|p| p.part_number());
                    let id = upload_id
                        .get()
                        .cloned()
                        .ok_or_else(|| std::io::Error::other("caricamento a parti non avviato"))?;
                    client
                        .complete_multipart_upload()
                        .bucket(&bucket)
                        .key(&key)
                        .upload_id(&id)
                        .multipart_upload(
                            CompletedMultipartUpload::builder()
                                .set_parts(Some(parts))
                                .build(),
                        )
                        .send()
                        .await
                        .map_err(|e| io_err(map_err(e, &path)))?;
                    Ok(())
                }));
            }
        }
        let fut = self.finishing.as_mut().expect("in corso");
        match fut.as_mut().poll(cx) {
            Poll::Ready(r) => {
                self.finishing = None;
                self.finished = r.is_ok();
                Poll::Ready(r)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for S3Writer {
    fn drop(&mut self) {
        // Scrittura interrotta: si annulla il caricamento a parti, così non resta nulla (né costi).
        if self.finished {
            return;
        }
        for h in &self.inflight {
            h.abort();
        }
        let Some(id) = self.upload_id.get().cloned() else {
            return;
        };
        let (client, bucket, key) = (self.client.clone(), self.bucket.clone(), self.key.clone());
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                let _ = client
                    .abort_multipart_upload()
                    .bucket(bucket)
                    .key(key)
                    .upload_id(id)
                    .send()
                    .await;
            });
        }
    }
}

impl RemoteFs for S3Fs {
    type Reader = S3Reader;
    type Writer = S3Writer;

    async fn list(&self, path: &str) -> Result<Vec<Entry>> {
        match split_path(path) {
            (None, _) => self.list_buckets().await,
            (Some(bucket), key) => self.list_prefix(&bucket, &key).await,
        }
    }

    async fn stat(&self, path: &str) -> Result<Entry> {
        let (bucket, key) = split_path(path);
        let Some(bucket) = bucket else {
            return Ok(dir_entry("/".into(), "/".into(), None));
        };
        let name = key
            .rsplit('/')
            .next()
            .filter(|n| !n.is_empty())
            .unwrap_or(&bucket)
            .to_string();
        let full = if key.is_empty() {
            format!("/{bucket}")
        } else {
            format!("/{bucket}/{key}")
        };
        if key.is_empty() {
            self.client
                .head_bucket()
                .bucket(&bucket)
                .send()
                .await
                .map_err(|e| map_err(e, path))?;
            return Ok(dir_entry(full, name, None));
        }
        match self
            .client
            .head_object()
            .bucket(&bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(h) => Ok(Entry {
                name,
                path: full,
                kind: EntryKind::File,
                is_symlink: false,
                size: h
                    .content_length()
                    .map_or(0, |s| u64::try_from(s).unwrap_or(0)),
                modified: h.last_modified().map(secs),
                mode: None,
                owner: None,
                group: None,
                uid: None,
                gid: None,
            }),
            Err(e) => {
                let mapped = map_err(e, path);
                if !matches!(mapped, FsError::NotFound(_)) {
                    return Err(mapped);
                }
                // Non è un oggetto: potrebbe essere una cartella (un prefisso con qualcosa dentro).
                let probe = self
                    .client
                    .list_objects_v2()
                    .bucket(&bucket)
                    .prefix(format!("{key}/"))
                    .max_keys(1)
                    .send()
                    .await
                    .map_err(|e| map_err(e, path))?;
                if probe.key_count().unwrap_or(0) > 0 {
                    Ok(dir_entry(full, name, None))
                } else {
                    Err(FsError::NotFound(path.to_string()))
                }
            }
        }
    }

    async fn lstat(&self, path: &str) -> Result<Entry> {
        self.stat(path).await
    }

    async fn canonicalize(&self, path: &str) -> Result<String> {
        if path == "." || path.is_empty() {
            return Ok("/".into());
        }
        let (bucket, key) = split_path(path);
        Ok(match bucket {
            None => "/".into(),
            Some(b) if key.is_empty() => format!("/{b}"),
            Some(b) => format!("/{b}/{key}"),
        })
    }

    async fn mkdir(&self, path: &str) -> Result<()> {
        match split_path(path) {
            (None, _) => Err(FsError::S3("non si può creare la radice".into())),
            (Some(bucket), key) if key.is_empty() => {
                self.client
                    .create_bucket()
                    .bucket(&bucket)
                    .send()
                    .await
                    .map_err(|e| map_err(e, path))?;
                Ok(())
            }
            // Una "cartella" è un oggetto vuoto con la chiave che finisce con `/`.
            (Some(bucket), key) => {
                self.client
                    .put_object()
                    .bucket(&bucket)
                    .key(format!("{key}/"))
                    .body(ByteStream::from_static(b""))
                    .send()
                    .await
                    .map_err(|e| map_err(e, path))?;
                Ok(())
            }
        }
    }

    async fn remove_file(&self, path: &str) -> Result<()> {
        let (Some(bucket), key) = split_path(path) else {
            return Err(FsError::S3("percorso non valido".into()));
        };
        if key.is_empty() {
            return Err(FsError::S3("un bucket si elimina come cartella".into()));
        }
        self.client
            .delete_object()
            .bucket(&bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| map_err(e, path))?;
        Ok(())
    }

    async fn remove_dir(&self, path: &str) -> Result<()> {
        match split_path(path) {
            (None, _) => Err(FsError::S3("non si può eliminare la radice".into())),
            (Some(bucket), key) if key.is_empty() => {
                // Solo un bucket vuoto: S3 rifiuta gli altri.
                self.client
                    .delete_bucket()
                    .bucket(&bucket)
                    .send()
                    .await
                    .map_err(|e| map_err(e, path))?;
                Ok(())
            }
            (Some(bucket), key) => {
                // Si toglie il segnaposto; se dentro c'è altro la cartella continua a esistere (è un
                // prefisso), per questo l'eliminazione ricorsiva svuota prima il contenuto.
                self.client
                    .delete_object()
                    .bucket(&bucket)
                    .key(format!("{key}/"))
                    .send()
                    .await
                    .map_err(|e| map_err(e, path))?;
                Ok(())
            }
        }
    }

    /// Rinomina o sposta (dentro lo stesso bucket): copia e poi elimina, perché S3 non ha una
    /// rinomina. Una cartella si sposta oggetto per oggetto.
    async fn rename(&self, from: &str, to: &str) -> Result<()> {
        let (Some(bucket), from_key) = split_path(from) else {
            return Err(FsError::S3("percorso non valido".into()));
        };
        let (to_bucket, to_key) = split_path(to);
        if to_bucket.as_deref() != Some(bucket.as_str()) || from_key.is_empty() || to_key.is_empty()
        {
            return Err(FsError::S3(
                "S3 permette di rinominare solo dentro lo stesso bucket".into(),
            ));
        }
        let entry = self.stat(from).await?;
        if entry.kind == EntryKind::File {
            self.copy_key(&bucket, &from_key, &to_key).await?;
            return self.remove_file(from).await;
        }
        let prefix = format!("{from_key}/");
        for key in self.keys_under(&bucket, &from_key).await? {
            let target = format!("{to_key}/{}", &key[prefix.len()..]);
            self.copy_key(&bucket, &key, &target).await?;
            self.client
                .delete_object()
                .bucket(&bucket)
                .key(&key)
                .send()
                .await
                .map_err(|e| map_err(e, from))?;
        }
        Ok(())
    }

    async fn set_permissions(&self, _path: &str, _mode: u32) -> Result<()> {
        Err(FsError::S3("S3 non ha permessi in stile Unix".into()))
    }

    async fn set_modified(&self, _path: &str, _secs: i64) -> Result<()> {
        Err(FsError::S3(
            "S3 non permette di cambiare la data di un oggetto".into(),
        ))
    }

    async fn open_read(&self, path: &str) -> Result<Self::Reader> {
        self.open_read_at(path, 0).await
    }

    async fn open_read_at(&self, path: &str, offset: u64) -> Result<Self::Reader> {
        let (Some(bucket), key) = split_path(path) else {
            return Err(FsError::NotRegularFile(path.to_string()));
        };
        let mut req = self.client.get_object().bucket(&bucket).key(&key);
        if offset > 0 {
            req = req.range(format!("bytes={offset}-"));
        }
        let out = req.send().await.map_err(|e| map_err(e, path))?;
        Ok(S3Reader(Box::pin(out.body.into_async_read())))
    }

    async fn open_write(&self, path: &str) -> Result<Self::Writer> {
        let (Some(bucket), key) = split_path(path) else {
            return Err(FsError::S3("percorso non valido".into()));
        };
        if key.is_empty() {
            return Err(FsError::S3("manca il nome dell'oggetto".into()));
        }
        Ok(S3Writer::new(self.client.clone(), bucket, key))
    }

    /// S3 non sa scrivere "da un punto": un oggetto o c'è tutto o non c'è. Chi trasferisce riparte da zero.
    async fn open_write_at(&self, path: &str, offset: u64) -> Result<Self::Writer> {
        if offset > 0 {
            return Err(FsError::S3(
                "S3 non permette di riprendere un caricamento a metà".into(),
            ));
        }
        self.open_write(path).await
    }

    fn capabilities(&self) -> FsCapabilities {
        FsCapabilities {
            permissions: false,
            symlinks: false,
            owner_group: false,
            atomic_writes: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_split_into_bucket_and_key() {
        assert_eq!(split_path("/"), (None, String::new()));
        assert_eq!(split_path(""), (None, String::new()));
        assert_eq!(split_path("/b"), (Some("b".into()), String::new()));
        assert_eq!(split_path("/b/"), (Some("b".into()), String::new()));
        assert_eq!(
            split_path("/b/a/c.txt"),
            (Some("b".into()), "a/c.txt".into())
        );
        assert_eq!(split_path("/b/a/../c"), (Some("b".into()), "c".into()));
        assert_eq!(split_path("b/./x"), (Some("b".into()), "x".into()));
    }

    #[test]
    fn copy_source_keys_are_percent_encoded_but_keep_slashes() {
        assert_eq!(urlencode_key("a b/è.txt"), "a%20b/%C3%A8.txt");
        assert_eq!(urlencode_key("x+y&z"), "x%2By%26z");
    }

    #[test]
    fn the_secret_key_never_appears_in_debug_output() {
        let p = S3Params {
            endpoint: None,
            region: "eu-west-1".into(),
            access_key: Some("AKIAEXAMPLE".into()),
            secret_key: Some("segretissima".into()),
            session_token: Some("token-segreto".into()),
            profile: None,
            path_style: false,
            bucket: None,
        };
        let dbg = format!("{p:?}");
        assert!(
            !dbg.contains("segretissima") && !dbg.contains("token-segreto"),
            "{dbg}"
        );
    }

    #[test]
    fn parts_grow_after_a_thousand() {
        assert_eq!(part_target(1), PART_SIZE);
        assert_eq!(part_target(1000), PART_SIZE);
        assert_eq!(part_target(1001), BIG_PART_SIZE);
    }
}
