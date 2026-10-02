//! The OurStick desktop app (stage 1, docs/gui-spec.md): a thin shell like the
//! CLI. The UI calls the commands in `commands`, which call only
//! `ourstick-core`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod busy;
mod commands;
mod dialog;
mod drive;
mod inhibit;
mod making;
#[cfg(target_os = "macos")]
mod menu;
mod problems;
mod qr;
mod receiving;
mod serving;
mod share;
mod sharing;
mod state;
mod stick_commands;
mod stick_life_commands;
#[cfg(test)]
mod testing;
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
    // The tray's activation tokens, for raising windows (see windows).
    #[cfg(target_os = "linux")]
    let builder = builder.manage(windows::Activation::default());
    // macOS: our own app menu, whose ⌘Q asks before quitting. Elsewhere the
    // windows have no menu bar.
    #[cfg(target_os = "macos")]
    let builder = builder.menu(menu::build).on_menu_event(|app, event| {
        if event.id.as_ref() == menu::QUIT {
            dialog::open_later(app, dialog::Mode::Quit);
        }
    });
    builder
        // A second launch shows the running app instead: one app per user,
        // which shares its sticks (each locked, see ourstick_core::locks).
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app);
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .manage(problems::Problems::default())
        .manage(receiving::Receivers::default())
        // Closing the main window keeps the app in the tray: sharing goes
        // on until "Stop sharing" or Quit. Linux minimizes it instead of
        // hiding it, so that the tray can bring it back (see windows).
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    if cfg!(target_os = "linux") {
                        let _ = window.minimize();
                    } else {
                        let _ = window.hide();
                    }
                }
            }
        })
        .setup(|app| {
            tray::create(app.handle())?;
            // Sticks from an earlier run are plugged in and shared again at
            // once, like USB sticks plugged back in; a removal or a new link
            // stopped halfway is finished even when no stick is left to see.
            let state = app.state::<state::AppState>();
            let store = state.store.as_ref();
            let pending = state.app_sticks().is_ok_and(|sticks| !sticks.is_empty())
                || store
                    .removing_sticks()
                    .is_ok_and(|sticks| !sticks.is_empty())
                || store.retired_sticks().is_ok_and(|ids| !ids.is_empty())
                || store.joined_sticks().is_ok_and(|sticks| {
                    sticks
                        .iter()
                        .any(|stick| stick.kind == ourstick_core::StickKind::App)
                })
                || store
                    .forgetting_joined_sticks()
                    .is_ok_and(|sticks| !sticks.is_empty());
            if pending {
                sharing::start_app(app.handle());
            }
            sharing::watch_mount(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_state,
            commands::plug_in,
            stick_commands::stick,
            stick_life_commands::sticks,
            stick_life_commands::new_link,
            stick_life_commands::new_stick,
            stick_life_commands::rename,
            problems::app_problem,
            receiving::commands::received,
            receiving::preview::preview_link,
            receiving::commands::choose_folder,
            receiving::commands::add_received,
            receiving::commands::pause_received,
            receiving::commands::resume_received,
            receiving::commands::forget_received_stick,
            receiving::commands::open_received_folder,
            problems::dismiss_app_problem,
            stick_life_commands::remove,
            stick_commands::add_paths,
            stick_commands::link_paths,
            stick_commands::remove_entry,
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
                    dialog::open_later(app, dialog::Mode::Quit);
                }
            }
            if let tauri::RunEvent::Exit = event {
                // Close the suitcases cleanly and release the stick's locks.
                let state = app.state::<state::AppState>();
                tauri::async_runtime::block_on(sharing::stop_all(&state));
                // What came so far stays; each goes on where it was.
                let receivers = app.state::<receiving::Receivers>();
                tauri::async_runtime::block_on(receivers.stop_all());
            }
        });
}
