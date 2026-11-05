//! The OurStick app (stage 1, docs/gui-spec.md; phones in ADR 0027): a thin
//! shell like the CLI. The UI calls the commands in `commands`, which call only
//! `ourstick-core`.

mod about;
#[cfg(desktop)]
mod arrival;
mod busy;
mod commands;
mod dialog;
#[cfg(any(target_os = "android", test))]
mod documents;
mod drive;
mod found;
mod inhibit;
mod keys;
mod local_network;
mod login;
#[cfg(target_os = "macos")]
mod menu;
#[cfg(target_os = "android")]
mod phone;
mod pick;
mod problems;
mod receiving;
mod refresh;
#[cfg(any(windows, test))]
mod registry;
mod send;
mod serving;
mod share;
mod sharing;
mod state;
mod stick_commands;
mod stick_life_commands;
mod still_open;
#[cfg(test)]
mod testing;
mod tray;
#[cfg(windows)]
mod unkept_clipboard;
mod windows;
mod words;

use tauri::Manager;

/// Runs the app until it quits. `main.rs` on a desktop; the phones' entry
/// point (ADR 0027).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
        .with_env_filter(log_filter())
        .with_writer(std::io::stderr)
        .init();

    // ourstick:receive (ADR 0030): the running app, if any, may come to
    // the front, before this start hands it over and exits.
    #[cfg(desktop)]
    arrival::before_start();

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
    // A second launch shows the running app instead: one app per user,
    // which shares its sticks (each locked, see ourstick_core::locks).
    // The login entry's start finds the app running: nothing to show. A
    // phone starts one app by itself.
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if receiving::scheme::asked(&args) {
                if arrival::arrived(app) {
                    tray::show_main(app);
                }
            } else if !login::autostarted(&args) {
                tray::show_main(app);
            }
        }))
        .manage(receiving::scheme::Arrivals::default());
    // A sharing link opened from another app (ADR 0027, decision 6).
    #[cfg(target_os = "android")]
    let builder = builder.plugin(phone::plugin());
    builder
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(problems::Problems::default())
        .manage(tray::DotShown::default())
        .manage(receiving::Receivers::default())
        .manage(receiving::opened::OpenedLink::default())
        .manage(receiving::clipboard::LastSeen::default())
        .manage(serving::looked::LastLooked::default())
        // The main window's ✕ asks first: Minimize, or Quit, which stops
        // sharing (see dialog). The window is only ever minimized, never
        // hidden, so that it stays in sight (and on Linux the tray can bring
        // it back, see windows).
        .on_window_event(|window, event| {
            // The main window came back after it lost focus (ADR 0030 §4).
            #[cfg(desktop)]
            if let tauri::WindowEvent::Focused(focused) = event {
                if window.label() == "main" {
                    arrival::focus_changed(window.app_handle(), *focused);
                }
            }
            // A phone looks at what it sends again when it comes back to
            // the front (ADR 0029 decision 4).
            #[cfg(mobile)]
            if let tauri::WindowEvent::Resumed = event {
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    stick_commands::look_at_all(&app.state::<state::AppState>()).await;
                });
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    dialog::close_requested(window.app_handle());
                }
            }
        })
        .setup(|app| {
            // A desktop's state, opened only once this start is the one
            // that runs (a second start hands over and exits before it, and
            // never opens nor migrates the database of the running one).
            // Before the window can call a command: the event loop has not
            // started yet.
            #[cfg(desktop)]
            match ourstick_core::device::data_dir()
                .map_err(anyhow::Error::from)
                .and_then(state::AppState::open)
            {
                Ok(state) => {
                    app.manage(state);
                }
                Err(error) => {
                    // Nothing can work without the data folder; say why and stop.
                    eprintln!("OurStick cannot start: {error:#}");
                    std::process::exit(1);
                }
            }
            #[cfg(target_os = "ios")]
            app.manage(state::AppState::open(state::phone_data_dir(app.handle())?)?);
            // What the user chooses to send is read through Android (ADR
            // 0029), once the window can call into it.
            #[cfg(target_os = "android")]
            {
                let documents = std::sync::Arc::new(documents::DocumentFiles::default());
                let data_dir = state::phone_data_dir(app.handle())?;
                app.manage(state::AppState::open_with(data_dir, documents.clone())?);
                match app.get_webview_window("main") {
                    Some(window) => documents::start(&window, documents)?,
                    None => tracing::warn!("no window: chosen files cannot be read"),
                }
                let handle = app.handle().clone();
                let store = app.state::<state::AppState>().store.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Err(error) = phone::tidy_grants(&handle, store.as_ref()) {
                        tracing::warn!(error = %format!("{error:#}"), "read grants were not tidied");
                    }
                });
            }
            tray::create(app.handle())?;
            #[cfg(target_os = "linux")]
            arrival::linux::serve(app.handle());
            #[cfg(target_os = "macos")]
            login::no_relaunch_at_login();
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
            #[cfg(desktop)]
            let asked = arrival::asked_at_start();
            #[cfg(mobile)]
            let asked = false;
            if asked {
                // The window, once it listens, finds Receive waiting.
                #[cfg(desktop)]
                let _ = arrival::arrived(app.handle());
            }
            let autostarted = login::autostarted(
                &std::env::args_os()
                    .map(|argument| argument.to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
            );
            if asked || !autostarted || ask_first {
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
            stick_commands::stick,
            stick_life_commands::sticks,
            stick_life_commands::new_link,
            stick_life_commands::rename,
            problems::app_problem,
            receiving::list::received,
            receiving::preview::preview_link,
            receiving::clipboard::link_on_clipboard,
            receiving::clipboard::new_link_on_clipboard,
            receiving::opened::link_opened,
            receiving::opened::take_opened_link,
            receiving::commands::choose_folder,
            receiving::commands::add_received,
            receiving::commands::pause_received,
            receiving::commands::resume_received,
            receiving::commands::forget_received_stick,
            receiving::moving::move_received,
            receiving::waiting::wait_for_link,
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
            pick::choose_paths,
            stick_commands::link_paths,
            stick_commands::show_entry,
            send::check_send,
            send::measure,
            send::send,
            stick_commands::remove_entry,
            stick_commands::look_again,
            found::find_entry,
            share::copy_link,
            share::share_link,
            share::sends_in_background,
            dialog::confirm_view,
            dialog::open_dialog,
            dialog::stop_sharing,
            dialog::start_sharing_again,
            dialog::quit,
            dialog::minimize_main,
            still_open::still_open_view,
            still_open::still_open_ok,
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
            // macOS: an address opened with the app (ADR 0030).
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = &event {
                arrival::opened(app, urls);
            }
            if let tauri::RunEvent::ExitRequested { api, .. } = &event {
                // Quit is always confirmed (docs/gui-spec.md): from the Dock,
                // with ⌘Q or from the app menu too, not only from the tray.
                if !dialog::QUIT_CONFIRMED.load(std::sync::atomic::Ordering::SeqCst) {
                    api.prevent_exit();
                    // Already stopping ("Quitting… please wait."): it goes
                    // by itself; no second dialog.
                    if !dialog::quitting() {
                        dialog::open_later(app, dialog::Mode::Quit);
                    }
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

/// The log filter: `RUST_LOG`, with `tao` never below `debug`, since at
/// `trace` it logs the addresses the app was opened with, and one may hold
/// a key (ADR 0030).
fn log_filter() -> tracing_subscriber::EnvFilter {
    let filter = tracing_subscriber::EnvFilter::from_default_env();
    // Without RUST_LOG only errors are logged: nothing to cap, and a cap
    // would let tao's debug lines through.
    if std::env::var_os("RUST_LOG").is_none() {
        return filter;
    }
    match "tao=debug".parse() {
        Ok(cap) => filter.add_directive(cap),
        Err(_) => filter,
    }
}
