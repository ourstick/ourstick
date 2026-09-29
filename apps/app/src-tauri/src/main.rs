//! The OurStick desktop app (stage 1, docs/gui-spec.md): a thin shell like the
//! CLI. The UI calls the commands in `commands`, which call only
//! `ourstick-core`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod drive;
mod qr;
mod serving;
mod share;
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
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .setup(|app| {
            // A stick from an earlier run is shared again at once.
            let has_stick = app
                .state::<state::AppState>()
                .drive_stick()
                .map(|stick| stick.is_some())
                .unwrap_or(false);
            if has_stick {
                state::start_sharing(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_state,
            commands::plug_in,
            commands::stick,
            commands::add_paths,
            commands::remove_entry,
            share::open_share,
            share::share_view,
            share::copy_link,
            share::copy_qr_image,
            share::send_by_email,
        ])
        .build(tauri::generate_context!())
        .expect("the app could not start its window")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // Close the cache cleanly and release the share lock.
                let state = app.state::<state::AppState>();
                tauri::async_runtime::block_on(async {
                    let sharing =
                        std::mem::replace(&mut *state.sharing.lock().await, state::Sharing::Off);
                    if let state::Sharing::On(serving) = sharing {
                        serving.stop().await;
                    }
                });
            }
        });
}
