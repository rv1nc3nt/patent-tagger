mod automation;
mod backup;
mod commands;
mod db;
mod import_worker;
mod library;
mod model;
mod platform;
mod retrain;
mod retrieval_worker;
mod review;
mod settings;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_fs::init())
    .plugin(tauri_plugin_opener::init())
    .invoke_handler(tauri::generate_handler![
      commands::import_numbers,
      commands::save_ops_credentials,
      commands::test_ops_connection,
      commands::run_import_jobs,
      commands::retry_document,
      commands::list_tags,
      commands::create_tag,
      commands::archive_tag,
      commands::review_queue,
      commands::document_detail,
      commands::validate_document,
      commands::skip_document,
      commands::retrain_now,
      commands::tag_metrics,
      commands::enable_automatic_mode,
      commands::disable_automatic_mode,
      commands::tag_eligibility,
      commands::run_retrieval_jobs,
      commands::retrieve_fulltext_now,
      commands::retrieve_drawings_now,
      settings::get_settings,
      settings::update_settings,
      settings::data_directory,
      settings::create_backup,
      settings::restore_backup,
      settings::export_tag_schema,
      settings::import_tag_schema,
      library::library_search,
      library::similar_documents,
      library::export_csv,
      library::export_json,
      library::export_tag_list,
      library::export_documents,
      library::bulk_retrieve,
    ])
    .setup(|app| {
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }
      app.manage(db::open().map_err(|e| e.to_string())?);
      app.manage(model::Model(
        embed_lib::BgeSmallEmbedder::load().map_err(|e| e.to_string())?,
      ));
      Ok(())
    })
    .run(tauri::generate_context!())
    // Invariant: a failure here means the webview/window could not be created at all;
    // there is no usable state to recover into, so aborting is correct.
    .expect("error while running tauri application");
}
