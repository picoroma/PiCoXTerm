//! Prove del backend S3 contro il server di `testenv/` (servizio `s3`, RustFS su 127.0.0.1:9000).
//! `docker compose up -d s3`, poi:
//! `cargo test -p pxt-fs --test s3_e2e -- --ignored --test-threads=1`

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use pxt_fs::transfer::{
    self, plan_download, plan_upload, ConflictPolicy, Control, JobState, Progress,
};
use pxt_fs::{EntryKind, FsError, ObjectHeaders, RemoteFs, S3Fs, S3Params};

const ACCESS: &str = "pxtaccess";
const SECRET: &str = "pxtsecret-1234";

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

fn params(secret: &str, bucket: Option<String>) -> S3Params {
    S3Params {
        endpoint: Some("http://127.0.0.1:9000".into()),
        region: "us-east-1".into(),
        access_key: Some(ACCESS.into()),
        secret_key: Some(secret.into()),
        session_token: None,
        profile: None,
        path_style: true,
        bucket,
    }
}

struct Env {
    fs: S3Fs,
    bucket: String,
    local: PathBuf,
}

async fn setup() -> Env {
    let fs = S3Fs::connect(params(SECRET, None))
        .await
        .expect("connessione S3");
    let bucket = format!("pxt-{}", nanos());
    fs.mkdir(&format!("/{bucket}")).await.unwrap();
    let local = std::env::temp_dir().join(format!("pxt-s3-local-{}", nanos()));
    std::fs::create_dir_all(&local).unwrap();
    Env { fs, bucket, local }
}

async fn remove_tree(fs: &S3Fs, path: &str) {
    if let Ok(entries) = fs.list(path).await {
        for e in entries {
            if e.kind == EntryKind::Dir {
                Box::pin(remove_tree(fs, &e.path)).await;
                let _ = fs.remove_dir(&e.path).await;
            } else {
                let _ = fs.remove_file(&e.path).await;
            }
        }
    }
}

impl Env {
    fn p(&self, rest: &str) -> String {
        format!("/{}/{rest}", self.bucket)
    }
    async fn cleanup(self) {
        let root = format!("/{}", self.bucket);
        remove_tree(&self.fs, &root).await;
        let _ = self.fs.remove_dir(&root).await;
        let _ = std::fs::remove_dir_all(&self.local);
    }
}

fn write_pseudo_random(path: &Path, mb: usize, seed: u64) {
    let mut f = std::fs::File::create(path).unwrap();
    let mut x = seed | 1;
    let mut block = vec![0u8; 1024 * 1024];
    for _ in 0..mb {
        for chunk in block.chunks_mut(8) {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let b = x.to_le_bytes();
            chunk.copy_from_slice(&b[..chunk.len()]);
        }
        f.write_all(&block).unwrap();
    }
}

fn same_content(a: &Path, b: &Path) -> bool {
    let (mut fa, mut fb) = (
        std::fs::File::open(a).unwrap(),
        std::fs::File::open(b).unwrap(),
    );
    if fa.metadata().unwrap().len() != fb.metadata().unwrap().len() {
        return false;
    }
    let (mut ba, mut bb) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    loop {
        let na = fa.read(&mut ba).unwrap();
        if na == 0 {
            return true;
        }
        fb.read_exact(&mut bb[..na]).unwrap();
        if ba[..na] != bb[..na] {
            return false;
        }
    }
}

async fn run(
    fs: &S3Fs,
    plan: &transfer::Plan,
    control: &Control,
) -> Result<transfer::Summary, transfer::TransferError> {
    let mut none = |_: &Progress| {};
    transfer::run(
        fs,
        plan,
        ConflictPolicy::Overwrite,
        control,
        &JobState::new(plan),
        4,
        &mut none,
    )
    .await
}

#[tokio::test]
#[ignore = "richiede il servizio s3 di testenv"]
async fn wrong_secret_is_rejected_and_never_echoed() {
    let err = match S3Fs::connect(params("segreto-sbagliato-xyz", None)).await {
        Err(e) => e,
        Ok(_) => panic!("le credenziali sbagliate non devono essere accettate"),
    };
    let text = err.to_string();
    assert!(
        !text.contains("segreto-sbagliato-xyz") && !text.contains(SECRET),
        "{text}"
    );
}

#[tokio::test]
#[ignore = "richiede il servizio s3 di testenv"]
async fn buckets_prefixes_and_basic_operations() {
    let env = setup().await;
    let fs = &env.fs;

    // Il bucket compare nell'elenco della radice.
    let root = fs.list("/").await.unwrap();
    assert!(root
        .iter()
        .any(|e| e.name == env.bucket && e.kind == EntryKind::Dir));

    // Cartella (segnaposto), file e rinomina.
    fs.mkdir(&env.p("docs")).await.unwrap();
    let src = env.local.join("ciao.txt");
    std::fs::write(&src, "ciao dal client").unwrap();
    let plan = plan_upload(fs, std::slice::from_ref(&src), &env.p("docs"))
        .await
        .unwrap();
    let sum = run(fs, &plan, &Control::new()).await.unwrap();
    assert_eq!((sum.files_done, sum.failed.len()), (1, 0));

    let list = fs.list(&env.p("docs")).await.unwrap();
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!((list[0].name.as_str(), list[0].size), ("ciao.txt", 15));
    assert_eq!(fs.stat(&env.p("docs")).await.unwrap().kind, EntryKind::Dir);
    assert!(matches!(
        fs.stat(&env.p("non/esiste")).await,
        Err(FsError::NotFound(_))
    ));

    fs.rename(&env.p("docs/ciao.txt"), &env.p("docs/nuovo nome è.txt"))
        .await
        .unwrap();
    let names: Vec<_> = fs
        .list(&env.p("docs"))
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["nuovo nome è.txt"]);

    // Una cartella si sposta con tutto il contenuto.
    fs.rename(&env.p("docs"), &env.p("archivio")).await.unwrap();
    assert!(fs.stat(&env.p("archivio/nuovo nome è.txt")).await.is_ok());
    assert!(fs.stat(&env.p("docs/nuovo nome è.txt")).await.is_err());

    // Non si rinomina fra bucket diversi.
    assert!(fs
        .rename(&env.p("archivio"), "/altro-bucket/x")
        .await
        .is_err());

    fs.remove_file(&env.p("archivio/nuovo nome è.txt"))
        .await
        .unwrap();
    assert!(fs.list(&env.p("archivio")).await.unwrap().is_empty());
    env.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il servizio s3 di testenv"]
async fn multipart_roundtrip_and_resume_of_a_download() {
    let env = setup().await;
    let fs = &env.fs;

    // 21 MiB: tre parti da 8 MiB (l'ultima più corta).
    let big = env.local.join("grande.bin");
    write_pseudo_random(&big, 21, 7);
    let plan = plan_upload(fs, std::slice::from_ref(&big), &env.p(""))
        .await
        .unwrap();
    let sum = run(fs, &plan, &Control::new()).await.unwrap();
    assert_eq!((sum.files_done, sum.failed.len()), (1, 0));
    assert_eq!(
        fs.stat(&env.p("grande.bin")).await.unwrap().size,
        21 * 1024 * 1024
    );

    let back = env.local.join("indietro");
    std::fs::create_dir_all(&back).unwrap();
    let plan = plan_download(fs, &[env.p("grande.bin")], &back)
        .await
        .unwrap();
    run(fs, &plan, &Control::new()).await.unwrap();
    assert!(same_content(&big, &back.join("grande.bin")));

    // Pausa e ripresa di un download: i `Range` permettono di ripartire dal parziale.
    let back2 = env.local.join("indietro2");
    std::fs::create_dir_all(&back2).unwrap();
    let plan = plan_download(fs, &[env.p("grande.bin")], &back2)
        .await
        .unwrap();
    let control = Control::new();
    let state = JobState::new(&plan);
    let mut seen = 0u64;
    let mut on = |p: &Progress| {
        seen = p.bytes_done;
        if seen > 2 * 1024 * 1024 {
            control.pause();
        }
    };
    let first = transfer::run(
        fs,
        &plan,
        ConflictPolicy::Overwrite,
        &control,
        &state,
        1,
        &mut on,
    )
    .await;
    assert!(
        first.is_err() || state.remaining() > 0,
        "la pausa doveva fermare il download"
    );
    let control = Control::new();
    let mut none = |_: &Progress| {};
    transfer::run(
        fs,
        &plan,
        ConflictPolicy::Overwrite,
        &control,
        &state,
        1,
        &mut none,
    )
    .await
    .unwrap();
    assert!(same_content(&big, &back2.join("grande.bin")));
    env.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il servizio s3 di testenv"]
async fn an_aborted_upload_leaves_no_object_behind() {
    use tokio::io::AsyncWriteExt;
    let env = setup().await;
    let fs = &env.fs;
    let mut w = fs.open_write(&env.p("interrotto.bin")).await.unwrap();
    // Abbastanza dati per avviare il caricamento a parti, poi si abbandona senza chiudere.
    w.write_all(&vec![1u8; 9 * 1024 * 1024]).await.unwrap();
    drop(w);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(matches!(
        fs.stat(&env.p("interrotto.bin")).await,
        Err(FsError::NotFound(_))
    ));
    env.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il servizio s3 di testenv"]
async fn folders_headers_tags_storage_class_and_presign() {
    let env = setup().await;
    let fs = &env.fs;

    // Cartella locale con sottocartella: si carica e si riscarica identica.
    let tree = env.local.join("albero");
    std::fs::create_dir_all(tree.join("sotto")).unwrap();
    std::fs::write(tree.join("a.txt"), "a").unwrap();
    std::fs::write(tree.join("sotto").join("b.txt"), "bb").unwrap();
    let plan = plan_upload(fs, std::slice::from_ref(&tree), &env.p(""))
        .await
        .unwrap();
    run(fs, &plan, &Control::new()).await.unwrap();
    assert_eq!(fs.stat(&env.p("albero/sotto/b.txt")).await.unwrap().size, 2);

    // Intestazioni e metadati.
    let obj = env.p("albero/a.txt");
    let mut meta = std::collections::HashMap::new();
    meta.insert("progetto".to_string(), "picoxterm".to_string());
    fs.set_headers(
        &obj,
        &ObjectHeaders {
            content_type: Some("text/plain; charset=utf-8".into()),
            cache_control: Some("max-age=60".into()),
            metadata: meta,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let info = fs.object_info(&obj).await.unwrap();
    assert_eq!(
        info.content_type.as_deref(),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(info.cache_control.as_deref(), Some("max-age=60"));
    assert_eq!(
        info.metadata.get("progetto").map(String::as_str),
        Some("picoxterm")
    );
    assert_eq!(info.size, 1);

    // Tag.
    fs.set_tags(
        &obj,
        &[("env".into(), "test".into()), ("team".into(), "pxt".into())],
    )
    .await
    .unwrap();
    let mut tags = fs.object_info(&obj).await.unwrap().tags;
    tags.sort();
    assert_eq!(
        tags,
        [("env".into(), "test".into()), ("team".into(), "pxt".into())]
    );

    // Classe di storage (STANDARD resta valida su qualunque server).
    fs.set_storage_class(&obj, "STANDARD").await.unwrap();

    // Indirizzo temporaneo: contiene firma e scadenza, e non la chiave segreta.
    let url = fs
        .presigned_url(&obj, Duration::from_secs(300))
        .await
        .unwrap();
    assert!(
        url.contains("X-Amz-Signature") && url.contains("X-Amz-Expires=300"),
        "{url}"
    );
    assert!(!url.contains(SECRET));
    env.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il servizio s3 di testenv"]
async fn external_bucket_lists_only_that_bucket() {
    let env = setup().await;
    let single = S3Fs::connect(params(SECRET, Some(env.bucket.clone())))
        .await
        .unwrap();
    let root = single.list("/").await.unwrap();
    assert_eq!(root.len(), 1);
    assert_eq!(root[0].name, env.bucket);
    env.cleanup().await;
}
