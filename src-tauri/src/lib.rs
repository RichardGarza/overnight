//! Overnight: the app shell. Commands live in the modules; this file wires them up.

mod paths;

#[tauri::command]
fn ping() -> &'static str {
    "overnight"
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![ping])
        .run(tauri::generate_context!())
        .expect("error while running Overnight");
}
