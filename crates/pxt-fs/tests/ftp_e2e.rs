//! Prove del backend FTP/FTPS contro il server di `testenv/` (servizio `ftp`, pure-ftpd con TLS
//! esplicito e certificato autofirmato). `docker compose up -d ftp`, poi:
//! `cargo test -p pxt-fs --test ftp_e2e -- --ignored --test-threads=1`

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use pxt_fs::transfer::{
    self, plan_download, plan_upload, ConflictPolicy, Control, JobState, Progress, TransferError,
    PART_SUFFIX,
};
use pxt_fs::{EntryKind, FsError, FtpFs, FtpParams, FtpSecurity, RemoteFs};

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

fn params(security: FtpSecurity, pinned: Option<String>) -> FtpParams {
    FtpParams {
        host: "127.0.0.1".into(),
        port: 2121,
        username: "test".into(),
        password: "test".into(),
        security,
        pinned_cert: pinned,
        max_connections: 4,
    }
}

/// Server FTP con una cartella di lavoro remota e una locale, ripulite alla fine.
struct Env {
    fs: FtpFs,
    remote: String,
    local: PathBuf,
}

async fn setup(security: FtpSecurity) -> Env {
    let pinned = if security == FtpSecurity::Plain {
        None
    } else {
        Some(untrusted_fingerprint().await)
    };
    let fs = FtpFs::connect(params(security, pinned))
        .await
        .expect("connessione FTP");
    let remote = format!("/pxt-ftp-{}", nanos());
    fs.mkdir(&remote).await.unwrap();
    let local = std::env::temp_dir().join(format!("pxt-ftp-local-{}", nanos()));
    std::fs::create_dir_all(&local).unwrap();
    Env { fs, remote, local }
}

/// Il server FTPS usa un certificato autofirmato: la prima connessione senza pin fallisce e dà l'impronta.
async fn untrusted_fingerprint() -> String {
    match FtpFs::connect(params(FtpSecurity::Explicit, None)).await {
        Err(FsError::UntrustedCertificate {
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

async fn remove_tree(fs: &FtpFs, path: &str) {
    if let Ok(entries) = fs.list(path).await {
        for e in entries {
            if e.kind == EntryKind::Dir && !e.is_symlink {
                Box::pin(remove_tree(fs, &e.path)).await;
            } else {
                let _ = fs.remove_file(&e.path).await;
            }
        }
    }
    let _ = fs.remove_dir(path).await;
}

impl Env {
    async fn cleanup(self) {
        remove_tree(&self.fs, &self.remote).await;
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
        let mut got = 0;
        while got < na {
            let n = fb.read(&mut bb[got..na]).unwrap();
            assert!(n > 0, "il secondo file è più corto");
            got += n;
        }
        if na == 0 {
            return true;
        }
        if ba[..na] != bb[..na] {
            return false;
        }
    }
}

async fn run(
    fs: &FtpFs,
    plan: &transfer::Plan,
    control: &Control,
    state: &JobState,
    concurrency: usize,
) -> Result<transfer::Summary, TransferError> {
    let mut none = |_: &Progress| {};
    transfer::run(
        fs,
        plan,
        ConflictPolicy::Overwrite,
        control,
        state,
        concurrency,
        &mut none,
    )
    .await
}

#[tokio::test]
#[ignore = "richiede il servizio ftp di testenv"]
async fn plain_ftp_basic_operations() {
    let env = setup(FtpSecurity::Plain).await;
    let fs = &env.fs;

    let home = fs.canonicalize(".").await.unwrap();
    assert!(home.starts_with('/'), "{home}");
    assert_eq!(fs.canonicalize("/a/b/../c").await.unwrap(), "/a/c");

    // Carica un file con il motore, poi elenco, stat, rinomina ed eliminazione.
    let src = env.local.join("ciao.txt");
    std::fs::write(&src, "ciao dal client").unwrap();
    let plan = plan_upload(fs, std::slice::from_ref(&src), &env.remote)
        .await
        .unwrap();
    let sum = run(fs, &plan, &Control::new(), &JobState::new(&plan), 4)
        .await
        .unwrap();
    assert_eq!(
        (sum.files_done, sum.failed.len()),
        (1, 0),
        "{:?}",
        sum.failed
    );

    let list = fs.list(&env.remote).await.unwrap();
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(
        (list[0].name.as_str(), list[0].kind, list[0].size),
        ("ciao.txt", EntryKind::File, 15)
    );
    let st = fs.stat(&format!("{}/ciao.txt", env.remote)).await.unwrap();
    assert_eq!((st.size, st.kind), (15, EntryKind::File));
    assert!(matches!(
        fs.stat(&format!("{}/manca.txt", env.remote)).await,
        Err(FsError::NotFound(_))
    ));
    assert!(matches!(
        fs.list("/non-esiste").await,
        Err(FsError::NotFound(_) | FsError::Ftp(_))
    ));

    fs.mkdir(&format!("{}/sotto", env.remote)).await.unwrap();
    fs.rename(
        &format!("{}/ciao.txt", env.remote),
        &format!("{}/sotto/nuovo nome.txt", env.remote),
    )
    .await
    .unwrap();
    let inner = fs.list(&format!("{}/sotto", env.remote)).await.unwrap();
    assert_eq!(inner[0].name, "nuovo nome.txt");
    fs.remove_file(&inner[0].path).await.unwrap();
    fs.remove_dir(&format!("{}/sotto", env.remote))
        .await
        .unwrap();
    assert!(fs.list(&env.remote).await.unwrap().is_empty());
    env.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il servizio ftp di testenv"]
async fn ftps_needs_a_pin_for_a_self_signed_certificate_and_detects_a_changed_one() {
    // Prima connessione: nessun pin, certificato autofirmato -> rifiutato con l'impronta.
    let fp = untrusted_fingerprint().await;
    assert_eq!(fp.split(':').count(), 32, "impronta SHA-256: {fp}");

    // Con il pin accettato dall'utente si collega e funziona.
    let fs = FtpFs::connect(params(FtpSecurity::Explicit, Some(fp.clone())))
        .await
        .expect("con il pin deve collegarsi");
    let list = fs.list("/").await.unwrap();
    println!("FTPS: {} voci nella radice", list.len());
    drop(fs);

    // Un pin diverso (certificato cambiato): rifiutato, e lo dice.
    let wrong = "00:".repeat(31) + "00";
    match FtpFs::connect(params(FtpSecurity::Explicit, Some(wrong))).await {
        Err(FsError::UntrustedCertificate {
            fingerprint,
            changed,
        }) => {
            assert!(changed);
            assert_eq!(fingerprint, fp);
        }
        other => panic!("atteso certificato cambiato, ottenuto {:?}", other.err()),
    }
}

#[tokio::test]
#[ignore = "richiede il servizio ftp di testenv"]
async fn password_errors_do_not_leak_the_password() {
    let mut p = params(FtpSecurity::Plain, None);
    p.password = "sbagliata-ma-segreta".into();
    let err = FtpFs::connect(p)
        .await
        .err()
        .expect("autenticazione rifiutata");
    let msg = err.to_string();
    assert!(!msg.contains("sbagliata-ma-segreta"), "{msg}");
    println!("errore di autenticazione: {msg}");
}

#[tokio::test]
#[ignore = "richiede il servizio ftp di testenv"]
async fn large_file_roundtrip_over_ftp_and_ftps() {
    for security in [FtpSecurity::Plain, FtpSecurity::Explicit] {
        let env = setup(security).await;
        let src = env.local.join("grande.bin");
        write_pseudo_random(&src, 40, 0xfeed_beef);
        let plan = plan_upload(&env.fs, std::slice::from_ref(&src), &env.remote)
            .await
            .unwrap();
        let t0 = std::time::Instant::now();
        let sum = run(&env.fs, &plan, &Control::new(), &JobState::new(&plan), 4)
            .await
            .unwrap();
        assert_eq!(
            (sum.files_done, sum.failed.len()),
            (1, 0),
            "{security:?}: {:?}",
            sum.failed
        );
        let up = t0.elapsed();
        let names: Vec<_> = env
            .fs
            .list(&env.remote)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(
            names,
            vec!["grande.bin".to_string()],
            "restano parziali? {names:?}"
        );

        let dl = env.local.join("scaricati");
        std::fs::create_dir_all(&dl).unwrap();
        let plan = plan_download(&env.fs, &[format!("{}/grande.bin", env.remote)], &dl)
            .await
            .unwrap();
        let t1 = std::time::Instant::now();
        let sum = run(&env.fs, &plan, &Control::new(), &JobState::new(&plan), 4)
            .await
            .unwrap();
        assert_eq!(
            (sum.files_done, sum.failed.len()),
            (1, 0),
            "{security:?}: {:?}",
            sum.failed
        );
        assert!(
            same_content(&src, &dl.join("grande.bin")),
            "{security:?}: contenuto diverso"
        );
        println!(
            "{security:?}: 40 MiB, upload {:.1} s, download {:.1} s",
            up.as_secs_f64(),
            t1.elapsed().as_secs_f64()
        );
        env.cleanup().await;
    }
}

#[tokio::test]
#[ignore = "richiede il servizio ftp di testenv"]
async fn many_files_in_parallel_reuse_the_connections() {
    let env = setup(FtpSecurity::Explicit).await;
    let root = env.local.join("tanti");
    for d in 0..4 {
        std::fs::create_dir_all(root.join(format!("d{d}"))).unwrap();
        for i in 0..30 {
            std::fs::write(root.join(format!("d{d}/f{i}.txt")), format!("{d}-{i}")).unwrap();
        }
    }
    let plan = plan_upload(&env.fs, std::slice::from_ref(&root), &env.remote)
        .await
        .unwrap();
    assert_eq!(plan.file_count, 120);
    let t0 = std::time::Instant::now();
    let sum = run(&env.fs, &plan, &Control::new(), &JobState::new(&plan), 4)
        .await
        .unwrap();
    println!(
        "120 file piccoli su FTPS: {:.1} s",
        t0.elapsed().as_secs_f64()
    );
    assert_eq!(
        (sum.files_done, sum.failed.len()),
        (120, 0),
        "{:?}",
        sum.failed
    );

    let dl = env.local.join("ritorno");
    std::fs::create_dir_all(&dl).unwrap();
    let plan = plan_download(&env.fs, &[format!("{}/tanti", env.remote)], &dl)
        .await
        .unwrap();
    let sum = run(&env.fs, &plan, &Control::new(), &JobState::new(&plan), 4)
        .await
        .unwrap();
    assert_eq!(
        (sum.files_done, sum.failed.len()),
        (120, 0),
        "{:?}",
        sum.failed
    );
    assert_eq!(
        std::fs::read_to_string(dl.join("tanti/d3/f29.txt")).unwrap(),
        "3-29"
    );
    env.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il servizio ftp di testenv"]
async fn paused_transfers_resume_or_restart_cleanly_over_ftp() {
    let env = setup(FtpSecurity::Plain).await;
    let big = env.local.join("grande.bin");
    write_pseudo_random(&big, 120, 31337);
    let plan = plan_upload(&env.fs, std::slice::from_ref(&big), &env.remote)
        .await
        .unwrap();
    let state = JobState::new(&plan);
    let control = Arc::new(Control::new());
    let c = Arc::clone(&control);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        c.pause();
    });
    let first = run(&env.fs, &plan, &control, &state, 4).await;
    if first.is_ok() {
        println!("rete troppo veloce: finito prima della pausa");
        env.cleanup().await;
        return;
    }
    assert!(matches!(first, Err(TransferError::Paused)), "{first:?}");
    let parts: Vec<_> = env
        .fs
        .list(&env.remote)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.name.ends_with(PART_SUFFIX))
        .collect();
    assert_eq!(parts.len(), 1, "il parziale deve restare");
    println!("pausa a {} byte", parts[0].size);

    // Ripresa (se il server sa riprendere) o ripartenza da zero: in ogni caso il file è identico.
    let sum = run(&env.fs, &plan, &Control::new(), &state, 4)
        .await
        .unwrap();
    assert_eq!(
        (sum.files_done, sum.failed.len()),
        (1, 0),
        "{:?}",
        sum.failed
    );
    let dl = env.local.join("verifica");
    std::fs::create_dir_all(&dl).unwrap();
    let plan = plan_download(&env.fs, &[format!("{}/grande.bin", env.remote)], &dl)
        .await
        .unwrap();
    run(&env.fs, &plan, &Control::new(), &JobState::new(&plan), 4)
        .await
        .unwrap();
    assert!(
        same_content(&big, &dl.join("grande.bin")),
        "contenuto diverso dopo la ripresa"
    );
    env.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il servizio ftp di testenv"]
async fn paused_download_resumes_with_rest_over_ftps() {
    let env = setup(FtpSecurity::Explicit).await;
    let big = env.local.join("sorgente.bin");
    write_pseudo_random(&big, 400, 99);
    let up = plan_upload(&env.fs, std::slice::from_ref(&big), &env.remote)
        .await
        .unwrap();
    run(&env.fs, &up, &Control::new(), &JobState::new(&up), 4)
        .await
        .unwrap();

    let dl = env.local.join("dl");
    std::fs::create_dir_all(&dl).unwrap();
    let plan = plan_download(&env.fs, &[format!("{}/sorgente.bin", env.remote)], &dl)
        .await
        .unwrap();
    let state = JobState::new(&plan);
    let control = Arc::new(Control::new());
    let c = Arc::clone(&control);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(250)).await;
        c.pause();
    });
    let first = run(&env.fs, &plan, &control, &state, 4).await;
    if first.is_ok() {
        println!("rete troppo veloce: scaricato prima della pausa");
        env.cleanup().await;
        return;
    }
    assert!(matches!(first, Err(TransferError::Paused)), "{first:?}");
    let part = std::fs::read_dir(&dl)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(PART_SUFFIX))
        .expect("parziale locale");
    let len = std::fs::metadata(&part).unwrap().len();
    println!("pausa a {len} byte su {}", 400u64 * 1024 * 1024);
    assert!(len > 0 && len < 400 * 1024 * 1024);

    // La ripresa riparte da `len` con REST: il risultato è identico all'originale.
    let mut reports = Vec::new();
    let mut on = |p: &Progress| reports.push(p.bytes_done);
    let sum = transfer::run(
        &env.fs,
        &plan,
        ConflictPolicy::Overwrite,
        &Control::new(),
        &state,
        4,
        &mut on,
    )
    .await
    .unwrap();
    assert_eq!(
        (sum.files_done, sum.failed.len()),
        (1, 0),
        "{:?}",
        sum.failed
    );
    assert!(
        reports.first().copied().unwrap_or(0) >= len,
        "non è ripartito dal punto raggiunto: {reports:?}"
    );
    assert!(
        same_content(&big, &dl.join("sorgente.bin")),
        "contenuto diverso dopo la ripresa"
    );
    env.cleanup().await;
}
