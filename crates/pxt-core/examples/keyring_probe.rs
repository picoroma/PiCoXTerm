//! Spike M0: salvataggio, lettura ed eliminazione di un segreto nel keyring di sistema.
fn main() {
    let entry = keyring::Entry::new("PiCoXterm-spike", "probe-user").expect("entry");
    entry.set_password("segreto-di-prova").expect("set");
    let got = entry.get_password().expect("get");
    println!(
        "lettura: {}",
        if got == "segreto-di-prova" {
            "OK"
        } else {
            "DIVERSA"
        }
    );
    entry.delete_credential().expect("delete");
    println!(
        "dopo delete: {:?}",
        entry
            .get_password()
            .map(|_| "ancora presente")
            .map_err(|e| e.to_string())
    );
}
