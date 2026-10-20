//! Il motore non deve sopravvivere all'app che lo ha avviato. Di norma basta la chiusura delle pipe, ma
//! per non dipendere da quella (un handle copiato altrove la terrebbe aperta) si guarda anche il processo
//! padre: quando termina, il motore termina.

/// Termina il motore quando finisce il processo `pid` (se `pid` è 0 non fa nulla).
#[cfg(windows)]
pub fn exit_with(pid: u32) {
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, INFINITE};
    // SYNCHRONIZE: diritto di attendere la fine del processo.
    const SYNCHRONIZE: u32 = 0x0010_0000;
    if pid == 0 {
        return;
    }
    std::thread::spawn(move || {
        // SAFETY: chiamate di sola attesa su un handle ottenuto qui e mai usato altrove.
        unsafe {
            let handle = OpenProcess(SYNCHRONIZE, 0, pid);
            // Il padre non esiste già più: nessuno ha più bisogno di me.
            if handle.is_null() {
                std::process::exit(0);
            }
            if WaitForSingleObject(handle, INFINITE) == WAIT_OBJECT_0 {
                std::process::exit(0);
            }
        }
    });
}

#[cfg(not(windows))]
pub fn exit_with(_pid: u32) {}
