#[cfg(test)]
mod bench_decode;
mod commands;
/// Harness-owned storage isolation (task 5.3): compiled ONLY in
/// `test-support` builds; the DRIFT_TEST_* environment variables do not
/// exist in production builds (Req 12.10).
#[cfg(feature = "test-support")]
mod harness;
mod keyboard;
mod rdp;
mod renderer;
mod store;
mod utils;
#[cfg(target_os = "linux")]
mod webkit_exit;

use std::sync::Arc;
use tauri::Manager;

use rdp::session::SessionManager;
use store::library::LibraryStore;
use store::settings::SettingsStore;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Install the ring crypto provider for rustls (required for TLS in spawned tasks)
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // A GUI build has no visible stderr, so record panics in the app log
    // before the default hook runs. Without this an unwinding main-thread
    // panic ends the app with no trace.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        log::error!(
            target: "panic",
            "thread '{}' panicked: {info}\n{}",
            thread.name().unwrap_or("<unnamed>"),
            std::backtrace::Backtrace::force_capture()
        );
        default_hook(info);
    }));

    let app = tauri::Builder::default()
        // First, so a second launch hands over its command line and exits.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            commands::launch::forward(app, argv)
        }))
        .manage(commands::launch::LaunchArgs::from_env())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(commands::updater::UpdateState::default())
        .manage(commands::credential_check::CredentialChecks::default())
        .manage(commands::folder_share::FolderShareDrafts::default())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(commands::native_clipboard::NativeClipboardState::default());
    // The folder picker on Linux and macOS (rdp-folder-sharing task 7.5);
    // Windows uses the shell's dialog directly.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let app = app.plugin(tauri_plugin_dialog::init());
    let app = app
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .filter(rdp::credential_probe::allow_diagnostic)
                .level_for("wgpu_core", log::LevelFilter::Warn)
                .level_for("wgpu_hal", log::LevelFilter::Warn)
                .level_for("naga", log::LevelFilter::Warn)
                .level_for("tao", log::LevelFilter::Warn)
                // The 40 KB KeepOne default discards the previous run's
                // lines, including the last ones before a crash.
                .max_file_size(2_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
                .build(),
        )
        .setup(|app| {
            // Resolve the app data directory for persistent storage.
            //
            // Task 5.3 (Req 12.7): test-support builds honor a harness-owned
            // data-dir override so E2E connection files/settings never touch
            // user data. Production builds resolve the real app-data dir
            // only — the environment variable does not exist there (Req 12.10).
            #[cfg(feature = "test-support")]
            let data_dir = harness::harness_data_dir().unwrap_or_else(|| {
                app.path()
                    .app_data_dir()
                    .expect("Failed to resolve app data directory")
            });
            #[cfg(not(feature = "test-support"))]
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to resolve app data directory");

            // Ensure the data directory exists
            std::fs::create_dir_all(&data_dir).expect("Failed to create app data directory");

            log::info!("App data directory: {:?}", data_dir);

            // Initialize stores
            // Task 6.6 (Req 10.10/10.12 + design "Logical Data Model"
            // option A′): the v2 LibraryStore is THE connection store —
            // one `connections.json` v2 document behind one lock. The
            // transitional `libraries.json` a 6.4/6.5 build may have left
            // behind is merged (v1 connections into the existing default
            // library) or adopted once at open, fail-closed into
            // write-blocked recovery; the v1 ConnectionStore is retired.
            let library_store = LibraryStore::open_unified(
                data_dir.join("connections.json"),
                data_dir.join("libraries.json"),
            );
            app.manage(rdp::peer_identity::pins::PinStore::new(
                data_dir.join("server-pins.json"),
            ));
            // rdp-folder-sharing 3.1: grants live outside the exported library
            // and are revoked inside its edit and deletion transactions.
            let share_grants = Arc::new(store::share_grants::ShareGrantStore::new(
                data_dir.join("share-grants.json"),
            ));
            library_store.attach_share_grants(Arc::clone(&share_grants));
            app.manage(share_grants);
            let settings_store = SettingsStore::new(data_dir);
            let renderer_setting = settings_store.renderer_at_startup();
            let session_manager = SessionManager::new();
            // Task 5.3 (Req 12.7): test-support builds may redirect
            // credentials into a harness-owned namespace; production builds
            // always use the production keyring service (the environment
            // variable is compiled out there, Req 12.10).
            let credential_store = {
                #[cfg(feature = "test-support")]
                {
                    harness::harness_credential_store().unwrap_or_default()
                }
                #[cfg(not(feature = "test-support"))]
                {
                    store::credentials::CredentialStore::new()
                }
            };

            // Register managed state
            app.manage(library_store);
            app.manage(settings_store);
            app.manage(session_manager);
            app.manage(credential_store);

            // Spec rdp-native-renderer: native layer or web canvas. Decided
            // once here, before the main window exists, because only the
            // native path makes the window transparent; only a failure
            // changes it later.
            let (path, reason) = renderer::display_path::decide(&renderer_setting);
            let display_path = renderer::display_path::DisplayPathState::new(path, reason);
            let main_window = {
                let config = app
                    .config()
                    .app
                    .windows
                    .first()
                    .ok_or("tauri.conf.json defines no window")?
                    .clone();
                let builder = tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?;
                // On Windows this also makes WebView2's default background
                // transparent, so the layer below shows (amendment 5a). On
                // macOS the window stays opaque: the layer sits inside it,
                // and only the web view stops painting its background.
                #[cfg(any(target_os = "linux", windows))]
                let builder = builder.transparent(path == renderer::display_path::Path::Native);
                builder.build()?
            };
            let frame_stores = Arc::new(renderer::frame_stores::FrameStores::default());
            let render_shared = renderer::gpu::RenderShared::new(Arc::clone(&frame_stores));
            #[cfg(any(target_os = "linux", windows, target_os = "macos"))]
            if path == renderer::display_path::Path::Native {
                let started = renderer::native_layer::attach(&main_window, frame_stores.wake())
                    .and_then(|target| {
                        let handle = app.handle().clone();
                        renderer::gpu::spawn(
                            target,
                            Arc::clone(&render_shared),
                            Box::new(move |error| {
                                renderer::display_path::runtime_fallback(&handle, error);
                            }),
                        )
                        .map_err(|e| e.to_string())
                    });
                if let Err(error) = started {
                    log::warn!("Native layer unavailable, using the web canvas: {error}");
                    // On Windows, building the layer is the DirectComposition part;
                    // the GPU surface is created on the render thread.
                    #[cfg(windows)]
                    let reason = renderer::display_path::Reason::CompositionFailed(error);
                    #[cfg(not(windows))]
                    let reason = renderer::display_path::Reason::SurfaceFailed(error);
                    display_path.fall_back(reason);
                }
            }
            #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
            let _ = &main_window;
            app.manage(display_path);
            // Spec rdp-keyboard-layout: follow the local layout (Wayland), or
            // remember the main thread for Windows and macOS detection.
            #[cfg(target_os = "linux")]
            keyboard::detect::watch_wayland();
            keyboard::detect::remember_ui_thread();
            app.manage(frame_stores);
            app.manage(render_shared);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            renderer::display_path::get_display_path,
            renderer::gpu::native_view,
            keyboard::detect::detect_keyboard_layout,
            keyboard::detect::keyboard_layouts,
            commands::session::send_text,
            renderer::gpu::native_frames,
            // Connection commands (task 6.6: unified v2 store)
            commands::connection::list_connections,
            commands::connection::get_connection,
            commands::connection::create_connection,
            commands::connection::update_connection,
            commands::connection::delete_connection,
            commands::connection::duplicate_connection,
            commands::peer_identity::review_server_identity,
            commands::credential_check::test_saved_credentials,
            commands::peer_identity::confirm_server_identity,
            commands::peer_identity::cancel_server_identity,
            commands::folder_share::get_folder_share,
            commands::folder_share::select_share_folder,
            commands::folder_share::confirm_folder_share,
            commands::folder_share::cancel_folder_share,
            commands::folder_share::revoke_folder_share,
            commands::connection::import_rdp_file,
            commands::connection::test_connection,
            commands::connection::get_connection_password,
            commands::connection::has_saved_password,
            commands::launch::take_launch_args,
            commands::launch::copy_text,
            commands::peer_identity::confirmed_fingerprint,
            commands::connection::measure_latency,
            commands::connection::import_ssh_config,
            // Clipboard commands
            commands::clipboard::clipboard_write,
            commands::clipboard::clipboard_read,
            commands::clipboard::clipboard_sync,
            commands::clipboard::clipboard_context,
            commands::clipboard::clipboard_paste,
            commands::native_clipboard::clipboard_local_acquire,
            commands::native_clipboard::clipboard_local_release,
            commands::native_clipboard::clipboard_local_read,
            commands::native_clipboard::clipboard_local_write,
            // Library commands (tasks 6.4/6.5/7.2/7.3, Req 10.1/10.2/10.3/10.5/10.8/10.15/10.16/10.17/10.18)
            commands::library::create_library,
            commands::library::rename_library,
            commands::library::create_folder,
            commands::library::rename_folder,
            commands::library::delete_folder,
            commands::library::delete_library,
            commands::library::move_node,
            commands::library::list_tree,
            commands::library::list_descendants,
            commands::library::get_library_status,
            commands::library::reload_library_store,
            // Session commands
            commands::session::connect,
            commands::session::frame_consumed,
            commands::session::pause_display,
            commands::session::resume_display,
            commands::session::session_view_returned,
            commands::session::disconnect,
            commands::session::list_sessions,
            commands::session::get_session_info,
            commands::session::send_key_event,
            commands::session::send_mouse_event,
            commands::session::resize_session,
            commands::session::get_frame,
            commands::session::cancel_reconnect,
            commands::updater::check_for_updates,
            commands::updater::install_update,
            // Settings commands
            commands::settings::get_settings,
            commands::settings::update_settings,
            // Task 5.3 (Req 12.2/12.7/12.10): E2E test-support commands.
            // The cfg attributes compile these match arms out of production
            // builds, so the commands are not registered there at all.
            #[cfg(feature = "test-support")]
            commands::test_support::debug_seed_mock_connections,
            #[cfg(feature = "test-support")]
            commands::test_support::debug_drop_mock_connection,
            #[cfg(feature = "test-support")]
            commands::test_support::debug_cleanup_test_credentials,
            // Task 5.4 (Req 12.4/12.10): frame-kind spy commands, gated the
            // same way — production builds register none of them.
            #[cfg(feature = "test-support")]
            commands::test_support::debug_frame_spy_snapshot,
            #[cfg(feature = "test-support")]
            commands::test_support::debug_frame_spy_reset,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_, event| {
        // End WebKit's web process before its own exit crashes (see
        // webkit_exit.rs): when the main window is about to close, and when
        // the app is asked to exit some other way.
        #[cfg(target_os = "linux")]
        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. }
                | tauri::RunEvent::WindowEvent {
                    event: tauri::WindowEvent::CloseRequested { .. },
                    ..
                }
        ) {
            webkit_exit::end_web_processes();
        }
        #[cfg(not(target_os = "linux"))]
        let _ = event;
    });
}
