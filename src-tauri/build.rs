use std::process::Command;

/// Esegue `git` e restituisce l'output ripulito, oppure `None` se git non c'è o il comando fallisce.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    // Informazioni mostrate nella finestra "Informazioni su PiCoXterm".
    let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "sconosciuto".into());
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    println!(
        "cargo:rustc-env=PXT_GIT_COMMIT={commit}{}",
        if dirty { "+modifiche" } else { "" }
    );
    let date = git(&["log", "-1", "--format=%cd", "--date=format:%Y-%m-%d"])
        .unwrap_or_else(|| "sconosciuta".into());
    println!("cargo:rustc-env=PXT_BUILD_DATE={date}");
    // Il commit cambia a ogni `git commit`: si ricalcola quando si muove HEAD.
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/logs/HEAD");

    tauri_build::build()
}
