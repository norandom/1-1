mod commands;
/// Harness-owned storage isolation (task 5.3): compiled ONLY in
/// `test-support` builds; the DRIFT_TEST_* environment variables do not
/// exist in production builds (Req 12.10).
#[cfg(feature = "test-support")]
mod harness;
mod rdp;
mod renderer;
mod store;
mod utils;

use std::sync::Arc;
use tauri::{Manager, RunEvent, WindowEvent};

use rdp::session::SessionManager;
use renderer::gpu::GpuRenderer;
use renderer::shared_frame::SharedFrame;
use store::library::LibraryStore;
use store::settings::SettingsStore;

/// Opt-in for the experimental native wgpu renderer (off by default).
const NATIVE_RENDERER_ENV: &str = "ONE_ONE_NATIVE_RENDERER";

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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(commands::updater::UpdateState::default())
        .manage(commands::credential_check::CredentialChecks::default())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(commands::native_clipboard::NativeClipboardState::default())
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
            let shared_frame = SharedFrame::new(1920, 1080);

            // Register managed state
            app.manage(library_store);
            app.manage(settings_store);
            app.manage(session_manager);
            app.manage(credential_store);
            app.manage(shared_frame.clone());

            // The web view's canvas draws remote frames from IPC. The native
            // wgpu renderer drew into the window's own surface under the opaque
            // web view: invisible on Windows, and on Wayland it shared GTK's
            // top-level surface, which explicit-sync compositors reject with
            // protocol error 71 (issue #1). Keep it for experiments only.
            let native_renderer = std::env::var_os(NATIVE_RENDERER_ENV).is_some_and(|v| v == "1");
            if !native_renderer {
                log::info!("Remote frames are drawn by the web view; set {NATIVE_RENDERER_ENV}=1 to try the native renderer.");
                return Ok(());
            }
            let window = app.get_webview_window("main").unwrap();
            let sf = shared_frame.clone();

            match tauri::async_runtime::block_on(GpuRenderer::new(window, sf)) {
                Ok(renderer) => {
                    let renderer = Arc::new(renderer);
                    app.manage(renderer.clone());

                    // Spawn render loop on a DEDICATED OS thread (not tokio)
                    // GPU present() blocks for vsync — must not starve the async runtime
                    let r = renderer.clone();
                    let sf_render = shared_frame.clone();
                    std::thread::Builder::new()
                        .name("gpu-render".into())
                        .spawn(move || {
                            log::info!(
                                "[GPU] Render thread started (thread: {:?})",
                                std::thread::current().id()
                            );
                            let mut first_frame = true;
                            loop {
                                // Sleep until session actor signals a new frame (via condvar)
                                // Times out at 100ms to handle resize/surface-lost recovery
                                sf_render.wait_for_frame(std::time::Duration::from_millis(100));

                                match r.render() {
                                    Ok(()) => {
                                        if first_frame {
                                            log::info!("[GPU] First frame rendered successfully!");
                                            first_frame = false;
                                        }
                                    }
                                    Err(
                                        wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated,
                                    ) => {
                                        log::warn!(
                                            "Surface lost/outdated, waiting for reconfigure"
                                        );
                                        std::thread::sleep(std::time::Duration::from_millis(16));
                                    }
                                    Err(wgpu::SurfaceError::OutOfMemory) => {
                                        log::error!("GPU out of memory, stopping render thread");
                                        break;
                                    }
                                    Err(e) => {
                                        log::warn!("Render error: {}", e);
                                    }
                                }
                            }
                        })
                        .expect("Failed to spawn GPU render thread");

                    log::info!("GPU renderer initialized successfully");
                }
                Err(e) => {
                    log::warn!(
                        "GPU renderer failed to initialize, falling back to IPC frames: {}",
                        e
                    );
                    // No GPU renderer — the webview Canvas fallback handles frames via IPC
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
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
            commands::connection::import_rdp_file,
            commands::connection::test_connection,
            commands::connection::get_connection_password,
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

    app.run(|app_handle, event| {
        if let RunEvent::WindowEvent {
            label: _,
            event: WindowEvent::Resized(size),
            ..
        } = event
        {
            // Resize the GPU surface if renderer exists
            if let Some(renderer) = app_handle.try_state::<Arc<GpuRenderer>>() {
                renderer.resize(size.width, size.height);
            }
        }
    });
}
