use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Dir,
    /// Dispositivi, socket, FIFO e link simbolici interrotti.
    Other,
}

/// Una voce di una cartella remota.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    /// Percorso completo, con `/` come separatore.
    pub path: String,
    /// Tipo effettivo: un link simbolico verso una cartella risulta `Dir`.
    pub kind: EntryKind,
    pub is_symlink: bool,
    pub size: u64,
    /// Ultima modifica, in secondi dal 1970.
    pub modified: Option<i64>,
    /// Permessi (bit `0o7777`), se il server li comunica.
    pub mode: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

/// Cosa sa fare il backend: la UI nasconde ciò che non è supportato (per esempio chmod su S3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsCapabilities {
    pub permissions: bool,
    pub symlinks: bool,
    pub owner_group: bool,
    /// Un file scritto compare solo a scrittura completa (S3): non servono file temporanei né rinomina,
    /// e un'interruzione non lascia un oggetto a metà.
    pub atomic_writes: bool,
}
