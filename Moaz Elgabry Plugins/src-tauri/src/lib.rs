mod catalog;
mod installer;
mod models;

use tauri::Manager;

#[tauri::command]
async fn dashboard_state() -> Result<models::DashboardState, String> {
    catalog::build_dashboard_state()
        .await
        .map_err(|error| models::UiError::from_error("dashboard", &error).to_json_string())
}

#[tauri::command]
async fn apply_plugin_action(
    plugin_id: String,
    action: String,
    target_version: Option<String>,
) -> Result<models::PluginOperationResult, String> {
    installer::apply_plugin_action(&plugin_id, &action, target_version.as_deref())
        .await
        .map_err(|error| models::UiError::from_error("plugin_action", &error).to_json_string())
}

pub fn run() {
    let mut builder = tauri::Builder::default().plugin(tauri_plugin_process::init());

    if let Some(pubkey) = option_env!("MEPM_UPDATER_PUBKEY") {
        builder = builder.plugin(
            tauri_plugin_updater::Builder::new()
                .pubkey(pubkey)
                .build(),
        );
    }

    builder
        .invoke_handler(tauri::generate_handler![dashboard_state, apply_plugin_action])
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                window.show()?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Moaz Elgabry Plugins");
}
