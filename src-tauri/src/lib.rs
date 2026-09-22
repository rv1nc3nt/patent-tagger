mod commands;
mod db;
mod platform;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![commands::import_numbers])
    .setup(|app| {
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }
      app.manage(db::open().map_err(|e| e.to_string())?);
      Ok(())
    })
    .run(tauri::generate_context!())
    // Invariant: a failure here means the webview/window could not be created at all;
    // there is no usable state to recover into, so aborting is correct.
    .expect("error while running tauri application");
}
