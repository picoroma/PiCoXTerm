#[tauri::command]
fn app_info() -> String {
    format!("{} {}", pxt_core::APP_NAME, env!("CARGO_PKG_VERSION"))
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_info])
        .run(tauri::generate_context!())
        .expect("errore nell'avvio di PiCoXterm");
}
