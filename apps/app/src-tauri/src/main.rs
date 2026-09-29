//! The OurStick desktop app (stage 1, docs/gui-spec.md): a thin shell like the
//! CLI. The UI calls the commands in `commands`, which call only
//! `ourstick-core`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod dialog;
mod drive;
mod inhibit;
#[cfg(target_os = "macos")]
mod menu;
mod qr;
mod serving;
mod share;
mod state;
mod tray;
mod windows;

use tauri::Manager;

fn main() {
    // WebKitGTK draws text in a scrolling list on its own compositing layer,
    // with other antialiasing: the file names turn bold once the list
    // scrolls (seen on Ubuntu). Without compositing all text looks the same;
    // the window is simple enough not to need it. Set before any thread
    // starts; a value the user set is kept.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none() {
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }
    // GNOME on Wayland lets no app raise its own window without a token
    // from a click in that app, and a click on the tray icon gives none:
    // "Open OurStick" left the window behind. Through X11 (XWayland) it can
    // be raised. A value the user set is kept.
    #[cfg(target_os = "linux")]
    if std::env::var_os("GDK_BACKEND").is_none() {
        std::env::set_var("GDK_BACKEND", "x11");
    }

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

    let builder = tauri::Builder::default();
    // macOS: our own app menu, whose ⌘Q asks before quitting. Elsewhere the
    // windows have no menu bar.
    #[cfg(target_os = "macos")]
    let builder = builder.menu(menu::build).on_menu_event(|app, event| {
        if event.id.as_ref() == menu::QUIT {
            if let Err(error) = dialog::open(app, dialog::Mode::Quit) {
                tracing::warn!(error = %format!("{error:#}"), "cannot ask before quitting");
            }
        }
    });
    builder
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
            if let tauri::RunEvent::ExitRequested { api, .. } = &event {
                // Quit is always confirmed (docs/gui-spec.md): from the Dock,
                // with ⌘Q or from the app menu too, not only from the tray.
                if !dialog::QUIT_CONFIRMED.load(std::sync::atomic::Ordering::SeqCst) {
                    api.prevent_exit();
                    if let Err(error) = dialog::open(app, dialog::Mode::Quit) {
                        tracing::warn!(error = %format!("{error:#}"), "cannot ask before quitting");
                    }
                }
            }
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
