// Sembunyikan console window di rilis Windows. Di debug ia dipertahankan
// karena log ke stderr adalah cara tercepat melihat apa yang terjadi.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod events;
mod state;
mod views;

use tauri::Manager;

use state::AppState;

fn main() {
    let paths =
        hub_core::paths::AppPaths::resolve().expect("tidak dapat menentukan direktori data");
    let _log_guard = init_logging(&paths);

    tracing::info!(version = hub_core::LAUNCHER_VERSION, "Studio Hub mulai");

    tauri::Builder::default()
        // Dua instance yang menulis DB bersamaan = korupsi (PRD §11.2).
        // Instance kedua mengaktifkan window pertama lalu keluar.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = AppState::initialize()?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::catalog_get,
            commands::catalog_status,
            commands::library_list,
            commands::updates_list,
            commands::install_plan,
            commands::install_start,
            commands::update_all_start,
            commands::rollback_start,
            commands::uninstall_start,
            commands::job_cancel,
            commands::daw_running,
            commands::reveal_in_explorer,
            commands::plugin_icon,
            commands::cache_images,
            commands::cache_clear,
            commands::logs_open,
            commands::open_external,
            commands::prefs_get,
            commands::prefs_set,
            commands::telemetry_reset_id,
            commands::version_skip,
            commands::launcher_update_required,
            commands::launcher_update_check,
            commands::launcher_update_install,
            commands::diagnostics_summary,
        ])
        .run(tauri::generate_context!())
        .expect("gagal menjalankan Studio Hub");
}

/// Logging terstruktur ke file, rolling harian (PRD §18.1).
///
/// Guard yang dikembalikan harus tetap hidup selama proses berjalan; menjatuh-
/// kannya menghentikan penulisan ke file.
fn init_logging(paths: &hub_core::paths::AppPaths) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::prelude::*;

    let _ = std::fs::create_dir_all(&paths.logs_dir);
    let file_appender = tracing_appender::rolling::daily(&paths.logs_dir, "hub.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = tracing_subscriber::EnvFilter::try_from_env("STUDIO_HUB_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(non_blocking)
        .with_ansi(false);

    let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stderr_layer)
        .try_init();

    prune_old_logs(&paths.logs_dir, 14);
    guard
}

/// Retensi 14 hari (PRD §18.1). Log yang menumpuk tanpa batas adalah cara
/// pelan-pelan mengisi disk pengguna.
fn prune_old_logs(dir: &std::path::Path, keep_days: u64) {
    let max_age = std::time::Duration::from_secs(keep_days * 24 * 3600);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let too_old = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().map(|e| e > max_age).unwrap_or(false))
            .unwrap_or(false);
        if too_old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
