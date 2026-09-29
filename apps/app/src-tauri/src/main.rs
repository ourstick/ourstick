//! The OurStick desktop app (stage 1, docs/gui-spec.md): a thin shell like the
//! CLI. The UI calls the commands in `commands`, which call only
//! `ourstick-core`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod dialog;
mod drive;
mod qr;
mod serving;
mod share;
mod state;
mod tray;

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
            tray::show_main(app);
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        // Closing the main window hides it in the tray: sharing goes on
        // until "Stop sharing" or Quit.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            tray::create(app.handle())?;
            // A stick from an earlier run is shared again at once.
            let has_stick = app
                .state::<state::AppState>()
                .drive_stick()
                .map(|stick| stick.is_some())
                .unwrap_or(false);
            if has_stick {
                // Like plugging a USB stick back in.
                state::plug_in_and_share(app.handle());
            }
            state::watch_mount(app.handle());
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
            dialog::confirm_view,
            dialog::open_dialog,
            dialog::stop_sharing,
            dialog::start_sharing_again,
            dialog::quit,
            commands::eject,
            commands::mount,
            commands::open_in_file_manager,
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
