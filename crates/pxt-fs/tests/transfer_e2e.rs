//! Prove del motore di trasferimento contro il server di `testenv/` (docker compose up -d).
//! `cargo test -p pxt-fs --test transfer_e2e -- --ignored --test-threads=1`

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use pxt_fs::transfer::{
    self, discard_partials, plan_download, plan_upload, ConflictPolicy, Control, Direction,
    JobState, Plan, Progress, Summary, TransferError, PART_SUFFIX,
};
use pxt_fs::{EntryKind, RemoteFs, SftpFs};
use pxt_ssh::{
    connect_shell, ConnectParams, Credentials, Hop, HostKeyDecision, HostKeyVerifier, KnownHosts,
};
use tokio::sync::mpsc;

struct Session {
    fs: SftpFs,
    _shell: pxt_ssh::Shell,
    _events: mpsc::Receiver<pxt_ssh::ShellEvent>,
    /// Cartella remota di lavoro, creata per ogni prova.
    remote: String,
    /// Cartella locale di lavoro.
    local: PathBuf,
}

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

async fn setup() -> Session {
    let (tx, rx) = mpsc::channel(64);
    let known = std::env::temp_dir().join(format!("pxt-tr-kh-{}", nanos()));
    let verifier: HostKeyVerifier =
        Arc::new(|_| Box::pin(async { HostKeyDecision::AcceptAndSave }));
    let shell = connect_shell(
        ConnectParams {
            hops: vec![Hop {
                host: "127.0.0.1".into(),
                port: 2222,
                username: "test".into(),
                auth: Credentials::Password("test".into()),
            }],
            cols: 80,
            rows: 24,
            term: "xterm".into(),
        },
        KnownHosts::new(known),
        verifier,
        tx,
    )
    .await
    .expect("connessione SSH");
    let fs = SftpFs::connect(shell.open_sftp_stream().await.unwrap())
        .await
        .unwrap();
    let remote = format!("/tmp/pxt-e2e-{}", nanos());
    fs.mkdir(&remote).await.unwrap();
    let local = std::env::temp_dir().join(format!("pxt-e2e-local-{}", nanos()));
    std::fs::create_dir_all(&local).unwrap();
    Session {
        fs,
        _shell: shell,
        _events: rx,
        remote,
        local,
    }
}

/// Elimina ricorsivamente una cartella remota.
async fn remote_rm_rf(fs: &SftpFs, path: &str) {
    if let Ok(entries) = fs.list(path).await {
        for e in entries {
            if e.kind == EntryKind::Dir && !e.is_symlink {
                Box::pin(remote_rm_rf(fs, &e.path)).await;
            } else {
                let _ = fs.remove_file(&e.path).await;
            }
        }
    }
    let _ = fs.remove_dir(path).await;
}

impl Session {
    async fn cleanup(self) {
        remote_rm_rf(&self.fs, &self.remote).await;
        let _ = std::fs::remove_dir_all(&self.local);
    }
}

/// Scrive `mb` MiB di dati pseudo-casuali riproducibili, a blocchi (senza tenerli tutti in memoria).
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

/// Confronto byte per byte, a blocchi.
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

fn no_progress() -> impl FnMut(&Progress) + Send {
    |_| {}
}

/// Esegue un piano da zero (stato nuovo, quattro file per volta).
async fn run(
    fs: &SftpFs,
    plan: &Plan,
    policy: ConflictPolicy,
    control: &Control,
    on_progress: &mut (dyn FnMut(&Progress) + Send),
) -> Result<Summary, TransferError> {
    transfer::run(
        fs,
        plan,
        policy,
        control,
        &JobState::new(plan),
        4,
        on_progress,
    )
    .await
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn large_file_roundtrip_is_byte_identical_and_reports_progress() {
    let s = setup().await;
    let src = s.local.join("grande.bin");
    write_pseudo_random(&src, 60, 0x1234_5678_9abc_def1);

    let plan = plan_upload(&s.fs, std::slice::from_ref(&src), &s.remote)
        .await
        .unwrap();
    assert_eq!((plan.file_count, plan.conflicts), (1, 0));
    assert_eq!(plan.total_bytes, 60 * 1024 * 1024);

    let cancel = Control::new();
    let mut reports = Vec::new();
    let t0 = std::time::Instant::now();
    let mut on_progress = |p: &Progress| reports.push(p.bytes_done);
    let sum = run(
        &s.fs,
        &plan,
        ConflictPolicy::Overwrite,
        &cancel,
        &mut on_progress,
    )
    .await
    .unwrap();
    let up = t0.elapsed();
    assert_eq!(
        (sum.files_done, sum.failed.len()),
        (1, 0),
        "{:?}",
        sum.failed
    );
    assert!(reports.len() > 2, "avanzamento troppo rado: {reports:?}");
    assert!(
        reports.windows(2).all(|w| w[0] <= w[1]),
        "avanzamento non monotono"
    );
    assert_eq!(*reports.last().unwrap(), 60 * 1024 * 1024);
    // Nessun file temporaneo rimasto.
    let remote_list = s.fs.list(&s.remote).await.unwrap();
    assert!(remote_list.iter().all(|e| !e.name.ends_with(PART_SUFFIX)));

    // Scarica in un'altra cartella e confronta.
    let dl_dir = s.local.join("scaricati");
    std::fs::create_dir_all(&dl_dir).unwrap();
    let remote_file = format!("{}/grande.bin", s.remote);
    let plan = plan_download(&s.fs, &[remote_file], &dl_dir).await.unwrap();
    let t1 = std::time::Instant::now();
    let sum = run(
        &s.fs,
        &plan,
        ConflictPolicy::Overwrite,
        &cancel,
        &mut no_progress(),
    )
    .await
    .unwrap();
    let down = t1.elapsed();
    assert_eq!(sum.files_done, 1);
    assert!(
        same_content(&src, &dl_dir.join("grande.bin")),
        "contenuto diverso"
    );
    println!(
        "60 MiB: upload {:.1} s ({:.0} MiB/s), download {:.1} s ({:.0} MiB/s)",
        up.as_secs_f64(),
        60.0 / up.as_secs_f64(),
        down.as_secs_f64(),
        60.0 / down.as_secs_f64()
    );
    s.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn folder_tree_roundtrip_keeps_structure_mtime_and_empty_dirs() {
    let s = setup().await;
    let root = s.local.join("albero");
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    std::fs::create_dir_all(root.join("vuota")).unwrap();
    for (rel, text) in [
        ("uno.txt", "uno"),
        ("a/due.txt", "due"),
        ("a/b/tre.txt", "tre"),
        ("con spazi.txt", "spazi"),
    ] {
        std::fs::write(root.join(rel), text).unwrap();
    }
    let old = UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    std::fs::OpenOptions::new()
        .write(true)
        .open(root.join("uno.txt"))
        .unwrap()
        .set_modified(old)
        .unwrap();

    let cancel = Control::new();
    let plan = plan_upload(&s.fs, std::slice::from_ref(&root), &s.remote)
        .await
        .unwrap();
    assert_eq!(plan.file_count, 4);
    let sum = run(
        &s.fs,
        &plan,
        ConflictPolicy::Overwrite,
        &cancel,
        &mut no_progress(),
    )
    .await
    .unwrap();
    assert!(sum.failed.is_empty(), "{:?}", sum.failed);

    // Sul server: struttura e cartella vuota.
    let remote_root = format!("{}/albero", s.remote);
    let names: Vec<String> =
        s.fs.list(&remote_root)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
    for n in ["uno.txt", "a", "vuota", "con spazi.txt"] {
        assert!(names.contains(&n.to_string()), "{n} manca: {names:?}");
    }
    let uno = s.fs.stat(&format!("{remote_root}/uno.txt")).await.unwrap();
    assert_eq!(
        uno.modified,
        Some(1_600_000_000),
        "la data di modifica va conservata"
    );

    // Ritorno: scarica tutta la cartella e confronta.
    let dl = s.local.join("ritorno");
    std::fs::create_dir_all(&dl).unwrap();
    let plan = plan_download(&s.fs, &[remote_root], &dl).await.unwrap();
    let sum = run(
        &s.fs,
        &plan,
        ConflictPolicy::Overwrite,
        &cancel,
        &mut no_progress(),
    )
    .await
    .unwrap();
    assert!(sum.failed.is_empty(), "{:?}", sum.failed);
    let back = dl.join("albero");
    assert!(back.join("vuota").is_dir());
    assert_eq!(
        std::fs::read_to_string(back.join("a/b/tre.txt")).unwrap(),
        "tre"
    );
    assert_eq!(
        std::fs::read_to_string(back.join("con spazi.txt")).unwrap(),
        "spazi"
    );
    let mtime = std::fs::metadata(back.join("uno.txt"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        mtime.duration_since(UNIX_EPOCH).unwrap().as_secs(),
        1_600_000_000
    );
    s.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn conflict_policies_skip_overwrite_rename_and_if_newer() {
    let s = setup().await;
    let cancel = Control::new();
    let file = s.local.join("a.txt");
    std::fs::write(&file, "versione 1").unwrap();
    let plan = plan_upload(&s.fs, std::slice::from_ref(&file), &s.remote)
        .await
        .unwrap();
    run(
        &s.fs,
        &plan,
        ConflictPolicy::Overwrite,
        &cancel,
        &mut no_progress(),
    )
    .await
    .unwrap();

    // Contenuto diverso, stessa destinazione: il piano segnala il conflitto.
    std::fs::write(&file, "versione 2 più lunga").unwrap();
    let plan = plan_upload(&s.fs, std::slice::from_ref(&file), &s.remote)
        .await
        .unwrap();
    assert_eq!(plan.conflicts, 1);
    let remote_a = format!("{}/a.txt", s.remote);
    let read_remote = |name: String| {
        let fs = &s.fs;
        async move {
            use tokio::io::AsyncReadExt;
            let mut r = fs.open_read(&name).await.unwrap();
            let mut out = String::new();
            r.read_to_string(&mut out).await.unwrap();
            out
        }
    };

    // Salta: resta la versione 1.
    let sum = run(
        &s.fs,
        &plan,
        ConflictPolicy::Skip,
        &cancel,
        &mut no_progress(),
    )
    .await
    .unwrap();
    assert_eq!((sum.files_done, sum.files_skipped), (0, 1));
    assert_eq!(read_remote(remote_a.clone()).await, "versione 1");

    // Rinomina: nasce "a (1).txt", l'originale resta.
    let sum = run(
        &s.fs,
        &plan,
        ConflictPolicy::Rename,
        &cancel,
        &mut no_progress(),
    )
    .await
    .unwrap();
    assert_eq!(sum.files_done, 1);
    assert_eq!(read_remote(remote_a.clone()).await, "versione 1");
    assert_eq!(
        read_remote(format!("{}/a (1).txt", s.remote)).await,
        "versione 2 più lunga"
    );

    // Sovrascrivi: l'originale cambia.
    let sum = run(
        &s.fs,
        &plan,
        ConflictPolicy::Overwrite,
        &cancel,
        &mut no_progress(),
    )
    .await
    .unwrap();
    assert_eq!(sum.files_done, 1);
    assert_eq!(read_remote(remote_a.clone()).await, "versione 2 più lunga");

    // Sovrascrivi se più recente: la sorgente (data vecchia) non lo è, quindi salta.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(1_500_000_000))
        .unwrap();
    std::fs::write(&file, "ancora diversa").unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(1_500_000_000))
        .unwrap();
    let plan = plan_upload(&s.fs, std::slice::from_ref(&file), &s.remote)
        .await
        .unwrap();
    let sum = run(
        &s.fs,
        &plan,
        ConflictPolicy::OverwriteIfNewer,
        &cancel,
        &mut no_progress(),
    )
    .await
    .unwrap();
    assert_eq!((sum.files_done, sum.files_skipped), (0, 1));
    assert_eq!(read_remote(remote_a).await, "versione 2 più lunga");
    s.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn cancelling_stops_quickly_and_leaves_no_partial_files() {
    let s = setup().await;
    let big = s.local.join("enorme.bin");
    write_pseudo_random(&big, 200, 99);
    let plan = plan_upload(&s.fs, &[big], &s.remote).await.unwrap();

    let cancel = Arc::new(Control::new());
    let flag = Arc::clone(&cancel);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        flag.cancel();
    });
    let t0 = std::time::Instant::now();
    let result = run(
        &s.fs,
        &plan,
        ConflictPolicy::Overwrite,
        &cancel,
        &mut no_progress(),
    )
    .await;
    assert!(
        matches!(result, Err(TransferError::Cancelled)),
        "{result:?}"
    );
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "annullamento troppo lento"
    );

    let names: Vec<String> =
        s.fs.list(&s.remote)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
    assert!(names.is_empty(), "restano file sul server: {names:?}");

    // Anche in download: nessun .pxt-part locale.
    let small = s.local.join("p.bin");
    write_pseudo_random(&small, 80, 7);
    let plan = plan_upload(&s.fs, &[small], &s.remote).await.unwrap();
    run(
        &s.fs,
        &plan,
        ConflictPolicy::Overwrite,
        &Control::new(),
        &mut no_progress(),
    )
    .await
    .unwrap();
    let dl = s.local.join("dl");
    std::fs::create_dir_all(&dl).unwrap();
    let plan = plan_download(&s.fs, &[format!("{}/p.bin", s.remote)], &dl)
        .await
        .unwrap();
    let cancel = Arc::new(Control::new());
    let flag = Arc::clone(&cancel);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        flag.cancel();
    });
    let result = run(
        &s.fs,
        &plan,
        ConflictPolicy::Overwrite,
        &cancel,
        &mut no_progress(),
    )
    .await;
    assert!(
        matches!(result, Err(TransferError::Cancelled)) || result.is_ok(),
        "{result:?}"
    );
    let leftovers: Vec<_> = std::fs::read_dir(&dl)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(
        leftovers
            .iter()
            .all(|n| !n.to_string_lossy().ends_with(PART_SUFFIX)),
        "file temporanei locali rimasti: {leftovers:?}"
    );
    s.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn a_failing_file_does_not_stop_the_rest() {
    let s = setup().await;
    let ok = s.local.join("ok.txt");
    std::fs::write(&ok, "ok").unwrap();
    let cancel = Control::new();

    // /proc non è scrivibile: il file fallisce, ma il lotto prosegue e riporta l'errore.
    let mut plan = plan_upload(&s.fs, std::slice::from_ref(&ok), &s.remote)
        .await
        .unwrap();
    let mut bad = plan.items[0].clone();
    bad.dst = "/proc/non-scrivibile.txt".into();
    plan.items.insert(0, bad);
    plan.file_count = 2;
    let sum = run(
        &s.fs,
        &plan,
        ConflictPolicy::Overwrite,
        &cancel,
        &mut no_progress(),
    )
    .await
    .unwrap();
    assert_eq!(sum.files_done, 1);
    assert_eq!(sum.failed.len(), 1, "{:?}", sum.failed);
    assert!(sum.failed[0].path.ends_with("ok.txt"));
    let names: Vec<String> =
        s.fs.list(&s.remote)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
    assert_eq!(names, vec!["ok.txt".to_string()]);
    assert_eq!(plan.direction, Direction::Upload);
    s.cleanup().await;
}

// ---- Parallelismo, pausa e ripresa ----

/// Esegue `plan` con uno stato dato e `concurrency` file per volta.
async fn run_with(
    s: &Session,
    plan: &Plan,
    control: &Control,
    state: &JobState,
    concurrency: usize,
) -> Result<Summary, TransferError> {
    transfer::run(
        &s.fs,
        plan,
        ConflictPolicy::Overwrite,
        control,
        state,
        concurrency,
        &mut no_progress(),
    )
    .await
}

/// Elenco dei file `.pxt-part` in una cartella remota.
async fn remote_parts(s: &Session, dir: &str) -> Vec<String> {
    s.fs.list(dir)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .filter(|n| n.ends_with(PART_SUFFIX))
        .collect()
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn many_small_files_in_parallel_arrive_intact() {
    let s = setup().await;
    let root = s.local.join("tanti");
    std::fs::create_dir_all(&root).unwrap();
    for sub in 0..5 {
        std::fs::create_dir_all(root.join(format!("d{sub}"))).unwrap();
        for i in 0..40 {
            std::fs::write(
                root.join(format!("d{sub}/f{i}.txt")),
                format!("contenuto {sub}-{i}"),
            )
            .unwrap();
        }
    }
    let plan = plan_upload(&s.fs, std::slice::from_ref(&root), &s.remote)
        .await
        .unwrap();
    assert_eq!(plan.file_count, 200);

    let t0 = std::time::Instant::now();
    let seq = run_with(&s, &plan, &Control::new(), &JobState::new(&plan), 1)
        .await
        .unwrap();
    let t_seq = t0.elapsed();
    assert_eq!(
        (seq.files_done, seq.failed.len()),
        (200, 0),
        "{:?}",
        seq.failed
    );

    // Stesso lotto con 8 file per volta, sovrascrivendo: stesso risultato, in meno tempo.
    let t1 = std::time::Instant::now();
    let par = run_with(&s, &plan, &Control::new(), &JobState::new(&plan), 8)
        .await
        .unwrap();
    let t_par = t1.elapsed();
    assert_eq!(
        (par.files_done, par.failed.len()),
        (200, 0),
        "{:?}",
        par.failed
    );
    println!(
        "200 file piccoli: 1 alla volta {:.2} s, 8 alla volta {:.2} s",
        t_seq.as_secs_f64(),
        t_par.as_secs_f64()
    );

    // Controllo del contenuto di alcuni file e assenza di parziali.
    let dl = s.local.join("ritorno");
    std::fs::create_dir_all(&dl).unwrap();
    let plan = plan_download(&s.fs, &[format!("{}/tanti", s.remote)], &dl)
        .await
        .unwrap();
    let sum = run_with(&s, &plan, &Control::new(), &JobState::new(&plan), 8)
        .await
        .unwrap();
    assert_eq!(
        (sum.files_done, sum.failed.len()),
        (200, 0),
        "{:?}",
        sum.failed
    );
    for (sub, i) in [(0, 0), (2, 17), (4, 39)] {
        assert_eq!(
            std::fs::read_to_string(dl.join(format!("tanti/d{sub}/f{i}.txt"))).unwrap(),
            format!("contenuto {sub}-{i}")
        );
    }
    assert!(remote_parts(&s, &format!("{}/tanti/d0", s.remote))
        .await
        .is_empty());
    s.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn paused_upload_resumes_from_where_it_stopped() {
    let s = setup().await;
    let big = s.local.join("grande.bin");
    write_pseudo_random(&big, 120, 4242);
    let plan = plan_upload(&s.fs, std::slice::from_ref(&big), &s.remote)
        .await
        .unwrap();
    let state = JobState::new(&plan);

    // Prima esecuzione: si mette in pausa dopo poco.
    let control = Arc::new(Control::new());
    let c = Arc::clone(&control);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(250)).await;
        c.pause();
    });
    let first = run_with(&s, &plan, &control, &state, 4).await;
    assert!(matches!(first, Err(TransferError::Paused)), "{first:?}");
    let parts = remote_parts(&s, &s.remote).await;
    assert_eq!(parts.len(), 1, "il parziale deve restare: {parts:?}");
    let part = format!("{}/{}", s.remote, parts[0]);
    let sent = s.fs.stat(&part).await.unwrap().size;
    assert!(
        sent > 0 && sent < 120 * 1024 * 1024,
        "pausa troppo tardi o troppo presto: {sent}"
    );
    assert_eq!(state.remaining(), 1);

    // Ripresa: i byte già inviati non si rimandano e il risultato è identico all'originale.
    let mut reports = Vec::new();
    let mut on_progress = |p: &Progress| reports.push(p.bytes_done);
    let sum = transfer::run(
        &s.fs,
        &plan,
        ConflictPolicy::Overwrite,
        &Control::new(),
        &state,
        4,
        &mut on_progress,
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
        reports.first().copied().unwrap_or(0) >= sent,
        "la ripresa deve ripartire dai byte già inviati ({sent}): {reports:?}"
    );
    assert!(remote_parts(&s, &s.remote).await.is_empty());
    assert_eq!(state.remaining(), 0);

    let dl = s.local.join("verifica");
    std::fs::create_dir_all(&dl).unwrap();
    let plan = plan_download(&s.fs, &[format!("{}/grande.bin", s.remote)], &dl)
        .await
        .unwrap();
    run_with(&s, &plan, &Control::new(), &JobState::new(&plan), 4)
        .await
        .unwrap();
    assert!(
        same_content(&big, &dl.join("grande.bin")),
        "contenuto diverso"
    );
    s.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn paused_download_resumes_and_a_stale_partial_is_not_reused() {
    let s = setup().await;
    let big = s.local.join("sorgente.bin");
    write_pseudo_random(&big, 100, 77);
    let up = plan_upload(&s.fs, std::slice::from_ref(&big), &s.remote)
        .await
        .unwrap();
    run_with(&s, &up, &Control::new(), &JobState::new(&up), 4)
        .await
        .unwrap();

    let dl = s.local.join("dl");
    std::fs::create_dir_all(&dl).unwrap();
    let plan = plan_download(&s.fs, &[format!("{}/sorgente.bin", s.remote)], &dl)
        .await
        .unwrap();
    let state = JobState::new(&plan);
    let control = Arc::new(Control::new());
    let c = Arc::clone(&control);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        c.pause();
    });
    let first = run_with(&s, &plan, &control, &state, 4).await;
    if first.is_ok() {
        // Rete troppo veloce: il file è finito prima della pausa, niente da riprendere.
        s.cleanup().await;
        return;
    }
    assert!(matches!(first, Err(TransferError::Paused)), "{first:?}");
    let part: PathBuf = std::fs::read_dir(&dl)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(PART_SUFFIX))
        .expect("parziale locale");
    let len = std::fs::metadata(&part).unwrap().len();
    assert!(len > 0 && len < 100 * 1024 * 1024);

    // Si rovina la coda del parziale: la ripresa non deve fidarsene e ricomincia da zero.
    {
        use std::io::Seek;
        let mut f = std::fs::OpenOptions::new().write(true).open(&part).unwrap();
        let from = len.saturating_sub(1000);
        f.seek(std::io::SeekFrom::Start(from)).unwrap();
        f.write_all(&vec![0xAB; (len - from) as usize]).unwrap();
    }
    let sum = run_with(&s, &plan, &Control::new(), &state, 4)
        .await
        .unwrap();
    assert_eq!(
        (sum.files_done, sum.failed.len()),
        (1, 0),
        "{:?}",
        sum.failed
    );
    assert!(
        same_content(&big, &dl.join("sorgente.bin")),
        "un parziale rovinato è stato riutilizzato"
    );
    s.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn discarding_partials_removes_them_and_finished_files_stay() {
    let s = setup().await;
    let root = s.local.join("misto");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a_piccolo.txt"), "piccolo").unwrap();
    write_pseudo_random(&root.join("z_grande.bin"), 150, 5);
    let plan = plan_upload(&s.fs, std::slice::from_ref(&root), &s.remote)
        .await
        .unwrap();
    let state = JobState::new(&plan);
    let control = Arc::new(Control::new());
    let c = Arc::clone(&control);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        c.pause();
    });
    // Un solo file per volta: il piccolo finisce, il grande resta a metà.
    let first = run_with(&s, &plan, &control, &state, 1).await;
    assert!(matches!(first, Err(TransferError::Paused)), "{first:?}");
    let dir = format!("{}/misto", s.remote);
    assert_eq!(remote_parts(&s, &dir).await.len(), 1);
    assert!(s.fs.stat(&format!("{dir}/a_piccolo.txt")).await.is_ok());

    let removed = discard_partials(&s.fs, Direction::Upload, &state).await;
    assert_eq!(removed, 1);
    assert!(remote_parts(&s, &dir).await.is_empty());
    s.cleanup().await;
}

// ---- Criteri di accettazione di M2 (lenti: `-- --ignored --test-threads=1 accept_`) ----
// Per la prova completa da 2 GiB: `PXT_BIG_MIB=2048`.

#[tokio::test]
#[ignore = "lenta: richiede il container di testenv e molto spazio su disco"]
async fn accept_large_file_pause_resume_and_stable_memory() {
    let mib: usize = std::env::var("PXT_BIG_MIB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let s = setup().await;
    let big = s.local.join("enorme.bin");
    write_pseudo_random(&big, mib, 2024);
    let plan = plan_upload(&s.fs, std::slice::from_ref(&big), &s.remote)
        .await
        .unwrap();
    let state = JobState::new(&plan);

    // Si ferma dopo un po' (circa un quarto del lavoro), poi si riprende fino in fondo.
    let control = Arc::new(Control::new());
    let c = Arc::clone(&control);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(3)).await;
        c.pause();
    });
    let t0 = std::time::Instant::now();
    let first = run_with(&s, &plan, &control, &state, 4).await;
    assert!(matches!(first, Err(TransferError::Paused)), "{first:?}");
    let parts = remote_parts(&s, &s.remote).await;
    assert_eq!(parts.len(), 1);
    let sent =
        s.fs.stat(&format!("{}/{}", s.remote, parts[0]))
            .await
            .unwrap()
            .size;
    println!(
        "{mib} MiB: pausa dopo {:.1} s a {:.0} MiB",
        t0.elapsed().as_secs_f64(),
        sent as f64 / 1048576.0
    );
    println!("PXT_PHASE resumed-upload");
    let t1 = std::time::Instant::now();
    let sum = run_with(&s, &plan, &Control::new(), &state, 4)
        .await
        .unwrap();
    assert_eq!(
        (sum.files_done, sum.failed.len()),
        (1, 0),
        "{:?}",
        sum.failed
    );
    println!("ripresa completata in {:.1} s", t1.elapsed().as_secs_f64());

    println!("PXT_PHASE download");
    let dl = s.local.join("scaricato");
    std::fs::create_dir_all(&dl).unwrap();
    let plan = plan_download(&s.fs, &[format!("{}/enorme.bin", s.remote)], &dl)
        .await
        .unwrap();
    run_with(&s, &plan, &Control::new(), &JobState::new(&plan), 4)
        .await
        .unwrap();
    assert!(
        same_content(&big, &dl.join("enorme.bin")),
        "contenuto diverso dopo pausa e ripresa"
    );
    println!("PXT_PHASE done");
    s.cleanup().await;
}

#[tokio::test]
#[ignore = "richiede il container di testenv"]
async fn accept_one_thousand_small_files() {
    let s = setup().await;
    let root = s.local.join("mille");
    for d in 0..10 {
        std::fs::create_dir_all(root.join(format!("cartella{d}"))).unwrap();
        for i in 0..100 {
            std::fs::write(
                root.join(format!("cartella{d}/file{i:03}.txt")),
                format!("{d}/{i} {}", "x".repeat(i * 7)),
            )
            .unwrap();
        }
    }
    let plan = plan_upload(&s.fs, std::slice::from_ref(&root), &s.remote)
        .await
        .unwrap();
    assert_eq!(plan.file_count, 1000);
    let t0 = std::time::Instant::now();
    let sum = run_with(&s, &plan, &Control::new(), &JobState::new(&plan), 4)
        .await
        .unwrap();
    println!("1000 file: upload {:.1} s", t0.elapsed().as_secs_f64());
    assert_eq!(
        (sum.files_done, sum.failed.len()),
        (1000, 0),
        "{:?}",
        sum.failed
    );

    let dl = s.local.join("indietro");
    std::fs::create_dir_all(&dl).unwrap();
    let plan = plan_download(&s.fs, &[format!("{}/mille", s.remote)], &dl)
        .await
        .unwrap();
    assert_eq!(plan.file_count, 1000);
    let t1 = std::time::Instant::now();
    let sum = run_with(&s, &plan, &Control::new(), &JobState::new(&plan), 4)
        .await
        .unwrap();
    println!("1000 file: download {:.1} s", t1.elapsed().as_secs_f64());
    assert_eq!(
        (sum.files_done, sum.failed.len()),
        (1000, 0),
        "{:?}",
        sum.failed
    );
    for d in 0..10 {
        for i in [0usize, 37, 99] {
            let a = std::fs::read(root.join(format!("cartella{d}/file{i:03}.txt"))).unwrap();
            let b = std::fs::read(dl.join(format!("mille/cartella{d}/file{i:03}.txt"))).unwrap();
            assert_eq!(a, b);
        }
    }
    s.cleanup().await;
}
