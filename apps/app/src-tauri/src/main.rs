//! The OurStick desktop app (stage 1, docs/gui-spec.md): a thin shell like the
//! CLI. The UI calls the commands in `commands`, which call only
//! `ourstick-core`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod drive;
mod state;

use tauri::Manager;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let state = match state::AppState::open() {
        Ok(state) => state,
        Err(error) => {
            // Nothing can work without the data folder; say why and stop.
            eprintln!("OurStick cannot start: {error:#}");
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        // A second launch shows the running app instead: two would serve with
        // one node identity (see device::ShareLock).
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::app_state,
            commands::plug_in,
        ])
        .run(tauri::generate_context!())
        .expect("the app could not start its window");
}
