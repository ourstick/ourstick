//! The OurStick desktop app (stage 1, docs/gui-spec.md): a thin shell like the
//! CLI. The UI calls the commands in `commands`, which call only
//! `ourstick-core`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod about;
mod busy;
mod commands;
mod dialog;
mod drive;
mod inhibit;
mod keys;
mod local_network;
mod login;
mod making;
#[cfg(target_os = "macos")]
mod menu;
mod problems;
mod qr;
mod receiving;
mod refresh;
#[cfg(any(windows, test))]
mod registry;
mod serving;
mod share;
mod sharing;
mod state;
mod stick_commands;
mod stick_drop;
mod stick_life_commands;
#[cfg(test)]
mod testing;
mod tray;
mod windows;
mod words;

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
    // Also before any thread starts: few malloc arenas, few threads per
    // blob store, and the app's own runtime (Tauri's) small too.
    let worker_threads = ourstick_core::threads::limit_store_threads();
    let runtime = match ourstick_core::threads::program_runtime(worker_threads) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("OurStick cannot start: {error}");
            std::process::exit(1);
        }
    };
    tauri::async_runtime::set(runtime.handle().clone());

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
        if event.id.as_ref() == menu::ABOUT {
            // Not in the menu handler itself, as the tray's windows.
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = about::open(&app) {
                    tracing::warn!(error = %format!("{error:#}"), "About did not open");
                }
            });
        }
    });
    builder
        // A second launch shows the running app instead: one app per user,
        // which shares its sticks (each locked, see ourstick_core::locks).
        // The login entry's start finds the app running: nothing to show.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !login::autostarted(&args) {
                tray::show_main(app);
            }
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .manage(problems::Problems::default())
        .manage(receiving::Receivers::default())
        // The main window's ✕ asks first: Minimize, or Quit, which stops
        // sharing (see dialog). The window is only ever minimized, never
        // hidden, so that it stays in sight (and on Linux the tray can bring
        // it back, see windows).
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    dialog::close_requested(window.app_handle());
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
            // Its endpoints wait for the window's Local Network step, if
            // due (ADR 0020): macOS asks right after it.
            // The keys' move comes first, in the window (ADR 0021): nothing
            // reads a key before it is done or put off.
            let due = crate::keys::move_due(store).unwrap_or_else(|error| {
                tracing::warn!(error = %format!("{error:#}"), "whether the keys must move is not known");
                None
            });
            if due == Some(0) {
                // No key yet: nothing for macOS to ask about.
                if let Err(error) = crate::keys::move_now(store, &state.data_dir) {
                    tracing::warn!(%error, "the empty keychain was not moved");
                }
            }
            // Started at login (ADR 0024): no window, unless it has
            // something to ask first.
            let notice = store
                .settings()
                .is_ok_and(|settings| settings.start_at_login && !settings.start_at_login_told);
            let ask_first =
                due.is_some_and(|keys| keys > 0) || !*state.network_open.borrow() || notice;
            if !login::autostarted(&std::env::args().collect::<Vec<_>>()) || ask_first {
                if let Some(window) = app.get_webview_window("main") {
                    window.show()?;
                }
            }
            // The login entry made to match the choice, off the main thread.
            let settings_store = state.store.clone();
            tauri::async_runtime::spawn_blocking(move || {
                login::sync_at_start(settings_store.as_ref())
            });
            if due.is_some_and(|keys| keys > 0) {
                state
                    .keys_wait
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            } else if pending {
                sharing::start_app(app.handle());
            }
            sharing::watch_mount(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_state,
            local_network::local_network_explained,
            local_network::open_local_network_settings,
            commands::move_keys,
            commands::plug_in,
            stick_commands::stick,
            stick_life_commands::sticks,
            stick_life_commands::new_link,
            stick_life_commands::new_stick,
            stick_life_commands::rename,
            problems::app_problem,
            receiving::commands::received,
            receiving::preview::preview_link,
            receiving::clipboard::link_on_clipboard,
            receiving::commands::choose_folder,
            receiving::commands::add_received,
            receiving::commands::pause_received,
            receiving::commands::resume_received,
            receiving::commands::forget_received_stick,
            receiving::commands::open_received_folder,
            receiving::choice::received_contents,
            receiving::ticks::choice_tick,
            receiving::ticks::choice_ticks,
            receiving::preview::preview_inside,
            receiving::preview::preview_needs,
            serving::readers::readers,
            serving::readers::remove_readers,
            receiving::reports::receiver_name,
            receiving::reports::set_receiver_name,
            receiving::choice::choose_what_to_get,
            problems::dismiss_app_problem,
            stick_life_commands::remove,
            stick_drop::add_paths,
            stick_drop::choose_for_stick,
            stick_commands::link_paths,
            stick_commands::remove_entry,
            share::open_share,
            share::share_view,
            share::copy_link,
            share::copy_qr_image,
            dialog::confirm_view,
            dialog::open_dialog,
            dialog::stop_sharing,
            dialog::start_sharing_again,
            dialog::quit,
            dialog::minimize_main,
            commands::eject,
            commands::mount,
            commands::open_in_file_manager,
            about::about_view,
            about::open_about,
            about::open_about_link,
            login::commands::start_at_login,
            login::commands::set_start_at_login,
            login::commands::start_at_login_told,
        ])
        .build(tauri::generate_context!())
        .expect("the app could not start its window")
        .run(|app, event| {
            // macOS: a click on the Dock icon brings the window back.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                tray::show_main(app);
            }
            if let tauri::RunEvent::ExitRequested { api, .. } = &event {
                // Quit is always confirmed (docs/gui-spec.md): from the Dock,
                // with ⌘Q or from the app menu too, not only from the tray.
                if !dialog::QUIT_CONFIRMED.load(std::sync::atomic::Ordering::SeqCst) {
                    api.prevent_exit();
                    dialog::open_later(app, dialog::Mode::Quit);
                }
            }
            if let tauri::RunEvent::Exit = event {
                // Close the owner's stores cleanly and release the stick's locks.
                let state = app.state::<state::AppState>();
                tauri::async_runtime::block_on(sharing::stop_all(&state));
                // What came so far stays; each goes on where it was.
                let receivers = app.state::<receiving::Receivers>();
                tauri::async_runtime::block_on(receivers.stop_all());
            }
        });
}
