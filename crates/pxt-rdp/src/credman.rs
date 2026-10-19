//! Credential Manager di Windows: voce `TERMSRV/<host>` che `mstsc` usa per accedere senza chiedere
//! la password. Tipo "domain password" (quello che legge mstsc), persistenza "di sessione": sparisce al
//! logoff anche se la rimozione non avviene.

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredWriteW, CREDENTIALW, CRED_PERSIST_SESSION, CRED_TYPE_DOMAIN_PASSWORD,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn write(target: &str, user: &str, password: &str) -> Result<(), String> {
    let mut target_w = wide(target);
    let mut user_w = wide(user);
    // Il blob è la password in UTF-16, senza terminatore.
    let mut blob: Vec<u8> = password.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let cred = CREDENTIALW {
        Type: CRED_TYPE_DOMAIN_PASSWORD,
        TargetName: PWSTR(target_w.as_mut_ptr()),
        UserName: PWSTR(user_w.as_mut_ptr()),
        CredentialBlobSize: u32::try_from(blob.len()).map_err(|_| "password troppo lunga")?,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_SESSION,
        ..Default::default()
    };
    // SAFETY: `cred` punta a buffer che restano vivi per tutta la chiamata.
    let r = unsafe { CredWriteW(&cred, 0) };
    // Il blob contiene la password: si azzera prima di rilasciarlo.
    blob.iter_mut().for_each(|b| *b = 0);
    r.map_err(|e| e.to_string())
}

pub fn delete(target: &str) -> Result<(), String> {
    let t = wide(target);
    // SAFETY: `t` è terminata da NUL e vive per tutta la chiamata.
    unsafe { CredDeleteW(PCWSTR(t.as_ptr()), CRED_TYPE_DOMAIN_PASSWORD, None) }
        .map_err(|e| e.to_string())
}

/// C'è una voce per questo bersaglio? (solo per i test)
#[cfg(test)]
pub fn exists(target: &str) -> bool {
    use windows::Win32::Security::Credentials::{CredFree, CredReadW, CREDENTIALW};
    let t = wide(target);
    let mut out: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: `out` riceve un buffer allocato dal sistema, liberato con `CredFree`.
    unsafe {
        if CredReadW(
            PCWSTR(t.as_ptr()),
            CRED_TYPE_DOMAIN_PASSWORD,
            None,
            &mut out,
        )
        .is_ok()
        {
            CredFree(out.cast());
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_delete_roundtrip_leaves_nothing_behind() {
        let target = "TERMSRV/pxt-test.invalid";
        write(target, "utente-prova", "segreto-di-prova").unwrap();
        assert!(exists(target));
        delete(target).unwrap();
        assert!(!exists(target));
    }
}
