//! Windows: i motori RDP stanno in un *job object* "kill on close". Se l'app finisce di colpo (chiusa con
//! il Task Manager, crash, `taskkill /F`) il sistema chiude il job e termina con lui tutti i motori:
//! nessun `picoxterm-rdp.exe` resta in giro, qualunque cosa sia successa alle pipe.

use std::os::windows::io::AsRawHandle;
use std::process::Child;
use std::sync::OnceLock;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

/// Il job vive quanto il processo: la sua chiusura è proprio ciò che fa terminare i motori.
static JOB: OnceLock<Option<isize>> = OnceLock::new();

fn job_handle() -> Option<HANDLE> {
    let raw = JOB.get_or_init(|| {
        // SAFETY: chiamate di creazione e configurazione senza puntatori che sopravvivono alla chiamata.
        unsafe {
            let job = CreateJobObjectW(None, windows::core::PCWSTR::null()).ok()?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                u32::try_from(std::mem::size_of_val(&info)).ok()?,
            )
            .ok()?;
            Some(job.0 as isize)
        }
    });
    raw.map(|r| HANDLE(r as *mut core::ffi::c_void))
}

/// Mette il processo figlio nel job. `false` se non è riuscito (il figlio funziona comunque, ma non
/// muore da solo se l'app viene uccisa).
pub fn assign(child: &Child) -> bool {
    let Some(job) = job_handle() else {
        return false;
    };
    // SAFETY: l'handle del figlio è valido finché `child` esiste.
    unsafe { AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())).is_ok() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use windows::Win32::System::JobObjects::IsProcessInJob;

    #[test]
    fn a_child_assigned_to_the_job_is_in_it() {
        let mut child = Command::new("ping")
            .args(["-n", "20", "127.0.0.1"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        assert!(assign(&child));
        let job = job_handle().unwrap();
        let mut inside = windows::core::BOOL(0);
        // SAFETY: handle validi per tutta la chiamata.
        unsafe {
            IsProcessInJob(HANDLE(child.as_raw_handle()), Some(job), &mut inside).unwrap();
        }
        assert!(inside.as_bool());
        let _ = child.kill();
        let _ = child.wait();
    }
}
