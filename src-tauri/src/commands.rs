//! Command Tauri (PRD §12.2).
//!
//! Command bersifat **task-oriented**, bukan primitif: tidak ada `read_file`
//! atau `http_get`; yang ada adalah `install_plugin`. Frontend tidak memiliki
//! kapabilitas filesystem, jaringan, atau shell (ADR-5), jadi setiap kemampuan
//! yang dibutuhkannya harus muncul sebagai command bertipe di daftar ini — dan
//! backend yang memvalidasi inputnya.

use std::sync::Arc;

use hub_core::catalog::fetch::CacheStatus;
use hub_core::install::plan::{InstallPlan, PlanInput};
use hub_core::install::{self, InstallContext};
use hub_core::paths::InstallScope;
use hub_core::prefs::Prefs;
use hub_core::telemetry::Event;
use hub_core::HubError;
use serde::Deserialize;
use tauri::{AppHandle, Manager, State};

use crate::events::{self, AppEvent};
use crate::state::AppState;
use crate::views::{self, CatalogView, DiagnosticsSummary, LibraryEntry, UpdateSummary};

type CmdResult<T> = std::result::Result<T, HubError>;

// ── Katalog ──────────────────────────────────────────────────────────────

/// Ambil katalog. `force = true` mengabaikan TTL cache (FR-1.1).
#[tauri::command]
pub async fn catalog_get(
    app: AppHandle,
    state: State<'_, AppState>,
    force: bool,
) -> CmdResult<CatalogView> {
    let started = std::time::Instant::now();
    let url = hub_core::prefs::CATALOG_URL;

    let outcome = state.fetcher.fetch(url, force).await?;
    let stale = outcome.is_stale();
    let catalog = outcome.catalog().clone();

    let cache_hit = matches!(
        outcome,
        hub_core::catalog::fetch::FetchOutcome::CacheFresh(_)
            | hub_core::catalog::fetch::FetchOutcome::NotModified(_)
    );
    state.telemetry.lock().await.record(Event::CatalogFetched {
        cache_hit,
        duration_ms: started.elapsed().as_millis() as u64,
    });

    let status = state.fetcher.status();
    *state.catalog.lock().await = Some(catalog.clone());

    let locale = state.prefs.lock().await.locale.clone();
    let db = state.db.lock().await;
    let view = views::build_catalog_view(
        &catalog,
        &db,
        stale,
        status.last_success_at.clone(),
        &locale,
    );
    let update_count = views::build_update_summary(&catalog, &db).items.len();
    drop(db);

    if stale {
        events::emit_app(
            &app,
            AppEvent::CatalogStale {
                last_success_at: status.last_success_at,
            },
        );
    } else {
        events::emit_app(
            &app,
            AppEvent::CatalogUpdated {
                plugin_count: view.plugins.len(),
                update_count,
            },
        );
    }

    Ok(view)
}

/// Metadata kesegaran cache untuk indikator UI.
#[tauri::command]
pub async fn catalog_status(state: State<'_, AppState>) -> CmdResult<CacheStatus> {
    Ok(state.fetcher.status())
}

// ── State terpasang ──────────────────────────────────────────────────────

/// Daftar plugin terpasang, sudah direkonsiliasi dengan filesystem.
///
/// Dipanggil lebih dulu oleh UI, sebelum katalog tiba — jaringan lambat tidak
/// boleh membuat aplikasi terasa rusak (PRD §8.1 prinsip 1, NFR-1.5).
#[tauri::command]
pub async fn library_list(state: State<'_, AppState>) -> CmdResult<Vec<LibraryEntry>> {
    let mut db = state.db.lock().await;
    let catalog = state.catalog.lock().await;
    let known = catalog
        .as_ref()
        .map(hub_core::registry::reconcile::known_bundles);

    let scan_dirs = hub_core::registry::reconcile::default_scan_dirs(None);
    let report = hub_core::registry::reconcile::reconcile(&mut db, &scan_dirs, known.as_ref());
    if !report.marked_missing.is_empty()
        || !report.adopted.is_empty()
        || !report.restored.is_empty()
        || !report.disowned.is_empty()
    {
        db.save(&state.paths.installed_db())?;
    }

    Ok(views::build_library(catalog.as_ref(), &db))
}

/// Gabungan katalog + terpasang: apa yang punya update.
#[tauri::command]
pub async fn updates_list(state: State<'_, AppState>) -> CmdResult<UpdateSummary> {
    let catalog = state.catalog_or_err().await?;
    let db = state.db.lock().await;
    let summary = views::build_update_summary(&catalog, &db);

    let telemetry = state.telemetry.lock().await;
    for item in &summary.items {
        telemetry.record(Event::UpdateOffered {
            plugin_id: item.plugin_id.clone(),
            from: item.from_version.clone(),
            to: item.to_version.clone(),
            breaking: item.breaking,
        });
    }
    Ok(summary)
}

// ── Perencanaan (dry-run) ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanArgs {
    pub plugin_id: String,
    /// `None` = versi latest.
    pub version: Option<String>,
    pub scope: Option<InstallScope>,
}

/// Hitung apa yang akan terjadi tanpa melakukan apa pun (FR-3.2).
#[tauri::command]
pub async fn install_plan(state: State<'_, AppState>, args: PlanArgs) -> CmdResult<InstallPlan> {
    let catalog = state.catalog_or_err().await?;
    let db = state.db.lock().await;
    let prefs = state.prefs.lock().await;

    let scope = args.scope.unwrap_or_else(|| prefs.resolved_scope());

    install::plan::build_plan(PlanInput {
        catalog: &catalog,
        db: &db,
        paths: &state.paths,
        plugin_id: &args.plugin_id,
        version: args.version.as_deref(),
        scope,
    })
}

// ── Eksekusi ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartArgs {
    pub plugin_id: String,
    pub version: Option<String>,
    pub scope: Option<InstallScope>,
}

/// Mulai job instalasi. Mengembalikan `job_id`; progres dikirim via event.
///
/// Perhatikan bahwa yang diterima adalah *argumen*, bukan `InstallPlan` dari
/// frontend. Plan dibangun ulang di backend: plan yang melintasi IPC kehilangan
/// `expected_digest` (field `#[serde(skip)]`), dan menerimanya kembali berarti
/// mempercayai frontend soal apa yang akan dipasang.
#[tauri::command]
pub async fn install_start(
    app: AppHandle,
    state: State<'_, AppState>,
    args: StartArgs,
) -> CmdResult<String> {
    let job_id = uuid::Uuid::new_v4().to_string();
    let cancel = state.jobs.lock().await.register(&job_id);

    let catalog = state.catalog_or_err().await?;
    let scope = match args.scope {
        Some(scope) => scope,
        None => state.prefs.lock().await.resolved_scope(),
    };

    let plan = {
        let db = state.db.lock().await;
        install::plan::build_plan(PlanInput {
            catalog: &catalog,
            db: &db,
            paths: &state.paths,
            plugin_id: &args.plugin_id,
            version: args.version.as_deref(),
            scope,
        })?
    };

    let telemetry = state.telemetry.lock().await.clone();
    telemetry.record(Event::InstallStarted {
        plugin_id: plan.plugin_id.clone(),
        plugin_version: plan.to_version.clone(),
        scope: plan.target.scope.as_str().to_string(),
    });

    let elevator: Option<Arc<hub_elevate::ElevatedSession>> = if plan.target.requires_elevation {
        match state.elevated_session().await {
            Ok(session) => Some(session),
            Err(HubError::ElevationDenied) => {
                telemetry.record(Event::ElevationDenied {
                    scope: plan.target.scope.as_str().to_string(),
                });
                // §8.8: UI menawarkan instalasi per-user, bukan sekadar menolak.
                events::emit_job(
                    &app,
                    &job_id,
                    install::JobEvent::Failed {
                        error: HubError::ElevationDenied,
                    },
                );
                state.jobs.lock().await.finish(&job_id);
                return Err(HubError::ElevationDenied);
            }
            Err(e) => return Err(e),
        }
    } else {
        None
    };

    let app_for_job = app.clone();
    let job_id_for_job = job_id.clone();

    // Job berjalan di background agar UI tidak blocking (NFR-1.2).
    tauri::async_runtime::spawn(async move {
        let state = app_for_job.state::<AppState>();
        let started = std::time::Instant::now();
        let from_version = plan.from_version.clone();

        let mut db = state.db.lock().await;
        let ctx = InstallContext {
            paths: &state.paths,
            job_id: job_id_for_job.clone(),
            cancel,
            known_daws: &catalog.daw_processes,
            elevator: elevator.as_deref().map(|s| s as &dyn install::Elevator),
        };

        let app_for_events = app_for_job.clone();
        let job_for_events = job_id_for_job.clone();
        let result = install::execute(&plan, &ctx, &mut db, move |event| {
            events::emit_job(&app_for_events, &job_for_events, event);
        })
        .await;
        drop(db);

        match &result {
            Ok(entry) => {
                telemetry.record(Event::InstallCompleted {
                    plugin_id: entry.plugin_id.clone(),
                    plugin_version: entry.version.clone(),
                    scope: entry.scope.as_str().to_string(),
                    duration_ms: started.elapsed().as_millis() as u64,
                    from_version,
                });
                events::emit_app(&app_for_job, AppEvent::LibraryChanged);
            }
            Err(error) => {
                telemetry.record(Event::InstallFailed {
                    plugin_id: plan.plugin_id.clone(),
                    plugin_version: plan.to_version.clone(),
                    error_code: error_code(error),
                    stage: "install".into(),
                });
                if let HubError::FileLocked { holders, .. } = error {
                    for holder in holders {
                        if let Some(name) = &holder.name {
                            telemetry.record(Event::DawConflictDetected {
                                daw_name: name.clone(),
                            });
                        }
                    }
                }
            }
        }

        state.jobs.lock().await.finish(&job_id_for_job);
    });

    Ok(job_id)
}

/// FR-4.6: "Update all" memasang semua update non-breaking secara berurutan.
/// Update breaking dikecualikan dan harus dikonfirmasi satu per satu.
#[tauri::command]
pub async fn update_all_start(
    app: AppHandle,
    state: State<'_, AppState>,
    include_breaking: bool,
) -> CmdResult<Vec<String>> {
    let catalog = state.catalog_or_err().await?;
    let summary = {
        let db = state.db.lock().await;
        views::build_update_summary(&catalog, &db)
    };

    let mut job_ids = Vec::new();
    for item in summary.items {
        if item.breaking && !include_breaking {
            continue;
        }
        // Job di-antrikan, bukan dijalankan paralel: setiap job memegang
        // `state.db` selama seluruh eksekusinya, jadi mutex itu yang
        // menyerialkan penulisan. Dua instalasi yang menulis ke direktori VST3
        // yang sama pada saat bersamaan adalah cara termudah membuat bundle
        // korup.
        let job_id = install_start(
            app.clone(),
            state.clone(),
            StartArgs {
                plugin_id: item.plugin_id,
                version: None,
                scope: None,
            },
        )
        .await?;
        job_ids.push(job_id);
    }
    Ok(job_ids)
}

#[tauri::command]
pub async fn rollback_start(
    app: AppHandle,
    state: State<'_, AppState>,
    plugin_id: String,
) -> CmdResult<String> {
    let mut db = state.db.lock().await;
    let from = db.get(&plugin_id).map(|e| e.version.clone());
    let entry = install::rollback(&mut db, &state.paths, &plugin_id)?;
    drop(db);

    state
        .telemetry
        .lock()
        .await
        .record(Event::RollbackPerformed {
            plugin_id: plugin_id.clone(),
            from: from.unwrap_or_default(),
            to: entry.version.clone(),
        });
    events::emit_app(&app, AppEvent::LibraryChanged);
    Ok(entry.version)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallArgs {
    pub plugin_id: String,
    /// Default `false` (FR-5.2). Dialog di UI yang menentukan nilainya.
    pub remove_user_data: bool,
}

#[tauri::command]
pub async fn uninstall_start(
    app: AppHandle,
    state: State<'_, AppState>,
    args: UninstallArgs,
) -> CmdResult<Vec<String>> {
    let user_data: Vec<String> = {
        let catalog = state.catalog.lock().await;
        catalog
            .as_ref()
            .and_then(|c| c.plugin(&args.plugin_id))
            .map(|p| {
                p.user_data
                    .preset_paths
                    .iter()
                    .chain(p.user_data.config_paths.iter())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    };

    let mut db = state.db.lock().await;
    let failures = install::uninstall(
        &mut db,
        &state.paths,
        &args.plugin_id,
        args.remove_user_data,
        &user_data,
    )?;
    drop(db);

    events::emit_app(&app, AppEvent::LibraryChanged);
    Ok(failures)
}

#[tauri::command]
pub async fn job_cancel(state: State<'_, AppState>, job_id: String) -> CmdResult<bool> {
    Ok(state.jobs.lock().await.cancel(&job_id))
}

// ── Prasyarat & sistem ───────────────────────────────────────────────────

#[tauri::command]
pub async fn daw_running(
    state: State<'_, AppState>,
) -> CmdResult<Vec<hub_core::error::ProcessHolder>> {
    let catalog = state.catalog.lock().await;
    let known = catalog
        .as_ref()
        .map(|c| c.daw_processes.clone())
        .unwrap_or_default();
    drop(catalog);
    Ok(hub_core::daw::detect_running_daws(&known))
}

/// Buka lokasi plugin di Explorer.
///
/// Frontend tidak punya `shell:` scope; path yang dibuka berasal dari DB kami
/// sendiri, bukan dari argumen frontend — yang dikirim frontend hanya id plugin.
#[tauri::command]
pub async fn reveal_in_explorer(state: State<'_, AppState>, plugin_id: String) -> CmdResult<()> {
    let db = state.db.lock().await;
    let entry = db.get(&plugin_id).ok_or_else(|| HubError::NotInstalled {
        plugin_id: plugin_id.clone(),
    })?;
    let dir = entry.install_dir.clone();
    drop(db);

    open_in_file_manager(&dir)
}

/// Path lokal ikon plugin, siap dipakai `convertFileSrc` di frontend.
///
/// `None` berarti plugin tidak punya ikon atau unduhannya gagal — bukan error.
/// Ikon adalah hiasan; ketiadaannya tidak boleh menggagalkan render daftar.
#[tauri::command]
pub async fn plugin_icon(
    state: State<'_, AppState>,
    plugin_id: String,
) -> CmdResult<Option<String>> {
    let icon_url = {
        let catalog = state.catalog.lock().await;
        catalog
            .as_ref()
            .and_then(|c| c.plugin(&plugin_id))
            .and_then(|p| p.icon_url.clone())
    };

    let path =
        hub_core::icons::ensure_cached(&state.paths.icons_dir(), &plugin_id, icon_url.as_deref())
            .await;

    Ok(path.map(|p| p.to_string_lossy().to_string()))
}

/// Batas gambar yang di-cache per permintaan. README yang menyebut ratusan
/// gambar tidak boleh membuat launcher mengunduh tanpa henti.
const MAX_README_IMAGES: usize = 24;

/// Cache gambar README dan kembalikan peta `URL asal → path lokal`.
///
/// Frontend hanya merender `<img>` untuk URL yang ada di peta ini. Gambar yang
/// gagal diunduh atau ditolak validasi tidak muncul sama sekali — bukan sebagai
/// ikon rusak, dan yang lebih penting, bukan sebagai request jaringan dari
/// dalam WebView (PRD §14.5).
#[tauri::command]
pub async fn cache_images(
    state: State<'_, AppState>,
    urls: Vec<String>,
) -> CmdResult<std::collections::HashMap<String, String>> {
    let dir = state.paths.icons_dir();
    let mut out = std::collections::HashMap::new();

    for url in urls.into_iter().take(MAX_README_IMAGES) {
        if let Some(path) = hub_core::icons::ensure_url_cached(&dir, &url).await {
            out.insert(url, path.to_string_lossy().to_string());
        }
    }
    Ok(out)
}

/// Kosongkan cache.
///
/// `full = false` membuang katalog dan gambar saja — itu yang dilakukan tombol
/// Refresh. `full = true` membuang seluruh direktori cache termasuk artefak
/// yang sudah diunduh.
///
/// Tidak ada yang hilang permanen: semuanya diambil ulang, dan artefak tetap
/// diverifikasi SHA-256 saat diunduh lagi.
#[tauri::command]
pub async fn cache_clear(state: State<'_, AppState>, full: bool) -> CmdResult<()> {
    if full {
        state.paths.clear_all_cache()?;
    } else {
        state.paths.clear_catalog_cache()?;
    }
    // Katalog di memori ikut dibuang, kalau tidak UI masih menyajikan data yang
    // barusan dinyatakan basi.
    *state.catalog.lock().await = None;
    tracing::info!(full, "cache dikosongkan");
    Ok(())
}

#[tauri::command]
pub async fn logs_open(state: State<'_, AppState>) -> CmdResult<()> {
    open_in_file_manager(&state.paths.logs_dir)
}

/// Buka URL di browser default.
///
/// Frontend tidak diberi kemampuan membuka URL sendiri (ADR-5) — ia memanggil
/// command ini, dan backend yang memutuskan apakah URL-nya layak dibuka.
/// Bedanya nyata: `homepage_url` di katalog adalah teks yang datang dari
/// jaringan, dan tanpa gerbang ini sebuah katalog yang di-tamper dapat
/// mengarahkan pengguna ke `file://` atau ke situs phishing lewat satu klik
/// yang terlihat sah.
#[tauri::command]
pub async fn open_external(app: AppHandle, url: String) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;

    let parsed = url::Url::parse(&url).map_err(|e| HubError::CatalogInvalid {
        detail: format!("URL tidak valid: {e}"),
    })?;
    if parsed.scheme() != "https" {
        return Err(HubError::CatalogInvalid {
            detail: format!("hanya https yang dibuka, bukan {}", parsed.scheme()),
        });
    }

    // Selain allowlist unduhan, satu host Microsoft diizinkan: halaman unduhan
    // VC++ Redistributable yang ditawarkan `prereq.rs` saat prasyarat hilang.
    let host = parsed.host_str().unwrap_or_default();
    let allowed = hub_core::host_is_allowed(host)
        || host == "aka.ms"
        || host == "learn.microsoft.com"
        || host == "support.microsoft.com";
    if !allowed {
        tracing::warn!(host, "permintaan membuka URL di luar allowlist ditolak");
        return Err(HubError::CatalogInvalid {
            detail: format!("host tidak diizinkan: {host}"),
        });
    }

    app.opener()
        .open_url(parsed.to_string(), None::<&str>)
        .map_err(|e| HubError::internal(format!("gagal membuka URL: {e}")))
}

fn open_in_file_manager(path: &std::path::Path) -> CmdResult<()> {
    #[cfg(windows)]
    {
        std::process::Command::new("explorer.exe")
            .arg(path)
            .spawn()
            .map_err(|e| HubError::internal(format!("gagal membuka Explorer: {e}")))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(HubError::internal("hanya didukung di Windows"))
    }
}

// ── Preferensi ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn prefs_get(state: State<'_, AppState>) -> CmdResult<Prefs> {
    Ok(state.prefs.lock().await.clone())
}

#[tauri::command]
pub async fn prefs_set(state: State<'_, AppState>, patch: serde_json::Value) -> CmdResult<Prefs> {
    let mut prefs = state.prefs.lock().await;

    // Merge patch ke prefs yang ada agar frontend dapat mengirim satu field
    // tanpa perlu mengirim ulang seluruh objek.
    let mut current = serde_json::to_value(&*prefs)
        .map_err(|e| HubError::internal(format!("serialisasi prefs: {e}")))?;
    if let (Some(base), Some(patch)) = (current.as_object_mut(), patch.as_object()) {
        for (key, value) in patch {
            // `installId` tidak dapat diubah lewat patch biasa; ia punya
            // command sendiri agar reset-nya eksplisit dan tercatat.
            if key == "installId" {
                continue;
            }
            base.insert(key.clone(), value.clone());
        }
    }

    let mut updated: Prefs = serde_json::from_value(current)
        .map_err(|e| HubError::internal(format!("prefs tidak valid: {e}")))?;
    updated.telemetry_prompt_shown = true;

    updated.save(&state.paths.prefs())?;
    let telemetry_changed = prefs.telemetry_enabled != updated.telemetry_enabled;
    *prefs = updated.clone();
    drop(prefs);

    if telemetry_changed {
        *state.telemetry.lock().await = hub_core::telemetry::Telemetry::new(
            updated.telemetry_enabled,
            updated.install_id.clone(),
            option_env!("STUDIO_HUB_TELEMETRY_ENDPOINT").map(str::to_string),
        );
    }

    Ok(updated)
}

/// Reset `install_id` (PRD §17.2). Terpisah dari `prefs_set` agar aksi ini
/// eksplisit dan tidak terjadi sebagai efek samping patch lain.
#[tauri::command]
pub async fn telemetry_reset_id(state: State<'_, AppState>) -> CmdResult<String> {
    let mut prefs = state.prefs.lock().await;
    prefs.install_id = uuid::Uuid::new_v4().to_string();
    prefs.save(&state.paths.prefs())?;
    Ok(prefs.install_id.clone())
}

/// FR-4.7: tolak sebuah versi secara permanen.
#[tauri::command]
pub async fn version_skip(
    app: AppHandle,
    state: State<'_, AppState>,
    plugin_id: String,
    version: String,
) -> CmdResult<()> {
    let mut db = state.db.lock().await;
    db.skip_version(&plugin_id, &version);
    db.save(&state.paths.installed_db())?;
    drop(db);

    state
        .telemetry
        .lock()
        .await
        .record(Event::UpdateSkipped { plugin_id, version });
    events::emit_app(&app, AppEvent::LibraryChanged);
    Ok(())
}

// ── Self-update ──────────────────────────────────────────────────────────

/// Versi minimum yang dituntut katalog, kalau launcher saat ini belum memenuhinya.
///
/// `None` berarti launcher sudah cukup baru. `Some(v)` berarti ada build di
/// katalog yang menolak dipasang sampai launcher mencapai `v` (FR-7.3).
async fn catalog_required_version(state: &AppState) -> Option<String> {
    let catalog = state.catalog.lock().await.clone()?;
    let current = hub_core::LAUNCHER_VERSION;

    let mut required: Option<String> = None;
    for plugin in &catalog.plugins {
        let Some(min) = &plugin.latest.min_launcher_version else {
            continue;
        };
        if hub_core::version::satisfies_minimum(current, min) {
            continue;
        }
        // Ambil tuntutan tertinggi, bukan yang pertama ditemui: memperbarui ke
        // versi terendah yang menuntut hanya akan memblokir lagi di plugin
        // berikutnya.
        let higher = match &required {
            Some(existing) => !hub_core::version::satisfies_minimum(existing, min),
            None => true,
        };
        if higher {
            required = Some(min.clone());
        }
    }
    required
}

/// Periksa apakah katalog menuntut launcher yang lebih baru (FR-7.3).
///
/// Ini menjawab "apakah instalasi plugin diblokir?", bukan "apakah ada rilis
/// baru?". Untuk yang kedua, lihat `launcher_update_check`.
#[tauri::command]
pub async fn launcher_update_required(
    state: State<'_, AppState>,
) -> CmdResult<Option<crate::views::LauncherUpdate>> {
    let Some(catalog) = state.catalog.lock().await.clone() else {
        return Ok(None);
    };
    let current = hub_core::LAUNCHER_VERSION;
    let required = catalog_required_version(&state).await;

    let latest = catalog.launcher.latest_version.clone();
    let has_newer = latest
        .as_deref()
        .map(|v| !hub_core::version::satisfies_minimum(current, v))
        .unwrap_or(false);

    if required.is_none() && !has_newer {
        return Ok(None);
    }

    Ok(Some(crate::views::LauncherUpdate {
        current_version: current.to_string(),
        available_version: latest
            .unwrap_or_else(|| required.clone().unwrap_or_else(|| current.to_string())),
        notes: String::new(),
        security: false,
        required: required.is_some(),
    }))
}

/// Tanya endpoint updater apakah ada rilis launcher yang lebih baru.
///
/// Pemeriksaan, unduhan, dan verifikasi dijalankan `tauri-plugin-updater` di
/// sisi Rust, bukan di frontend. Frontend tidak punya kapabilitas jaringan
/// (ADR-5), dan menaruhnya di backend berarti tidak ada dependensi npm baru
/// hanya untuk memanggil satu endpoint.
///
/// Signature Ed25519 diverifikasi terhadap `pubkey` yang tertanam di binary,
/// dan plugin tidak menyediakan cara mematikannya. Itulah yang membuat manifest
/// yang dipalsukan tidak dapat memasang apa pun.
#[tauri::command]
pub async fn launcher_update_check(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<crate::views::LauncherUpdate>> {
    use tauri_plugin_updater::UpdaterExt;

    let current = hub_core::LAUNCHER_VERSION.to_string();
    let required = catalog_required_version(&state).await;

    // Jaringan mati, GitHub down, atau manifest belum terbit bukan kondisi
    // error yang perlu dilempar ke pengguna: mereka tidak meminta apa pun dan
    // tidak dapat berbuat apa pun. Dicatat ke log, lalu diperlakukan sebagai
    // "tidak ada update".
    let found = match app.updater() {
        Ok(updater) => match updater.check().await {
            Ok(found) => found,
            Err(err) => {
                tracing::warn!(error = %err, "pemeriksaan update launcher gagal");
                None
            }
        },
        Err(err) => {
            tracing::warn!(error = %err, "updater tidak tersedia");
            None
        }
    };

    let Some(update) = found else {
        // Tidak ada rilis baru. Tuntutan katalog tetap dilaporkan supaya
        // pengguna tahu kenapa instalasi plugin diblokir, alih-alih menghadapi
        // tombol yang mati tanpa penjelasan.
        return Ok(required.map(|min| crate::views::LauncherUpdate {
            current_version: current,
            available_version: min,
            notes: String::new(),
            security: false,
            required: true,
        }));
    };

    Ok(Some(crate::views::LauncherUpdate {
        current_version: current,
        available_version: update.version.clone(),
        notes: update.body.clone().unwrap_or_default(),
        security: false,
        required: required.is_some(),
    }))
}

/// Unduh dan pasang update launcher, lalu jalankan ulang aplikasi.
///
/// `check()` dipanggil ulang di sini alih-alih menyimpan hasil pemeriksaan
/// sebelumnya. Satu permintaan HTTP tambahan jauh lebih murah daripada
/// menyimpan state yang bisa basi: pengguna bisa membiarkan jendela terbuka
/// berjam-jam sebelum menekan tombolnya.
#[tauri::command]
pub async fn launcher_update_install(app: AppHandle) -> CmdResult<()> {
    use tauri_plugin_updater::UpdaterExt;

    let updater = app
        .updater()
        .map_err(|err| HubError::internal(format!("updater tidak tersedia: {err}")))?;

    let Some(update) = updater
        .check()
        .await
        .map_err(|err| HubError::internal(format!("gagal memeriksa update: {err}")))?
    else {
        // Rilis sudah ditarik antara pemeriksaan dan klik. Bukan kegagalan.
        return Ok(());
    };

    update
        .download_and_install(|_downloaded, _total| {}, || {})
        .await
        .map_err(|err| HubError::internal(format!("gagal memasang update: {err}")))?;

    // Installer sudah berjalan; proses ini harus keluar agar berkasnya dapat
    // ditimpa. `restart` tidak pernah kembali.
    app.restart();
}

// ── Diagnostik ───────────────────────────────────────────────────────────

/// Ringkasan yang ditampilkan **sebelum** pengguna menyimpan ekspor diagnostik.
///
/// Menampilkan isinya lebih dulu bukan sekadar sopan santun: pengguna yang tahu
/// apa yang akan mereka kirim lebih mungkin bersedia mengirimnya, dan itu yang
/// membuat debugging jarak jauh mungkin (PRD §18.2).
#[tauri::command]
pub async fn diagnostics_summary(state: State<'_, AppState>) -> CmdResult<DiagnosticsSummary> {
    let db = state.db.lock().await;
    let catalog = state.catalog.lock().await;
    let known = catalog
        .as_ref()
        .map(|c| c.daw_processes.clone())
        .unwrap_or_default();

    Ok(DiagnosticsSummary {
        launcher_version: hub_core::LAUNCHER_VERSION.to_string(),
        os_build: hub_core::prereq::os_build_number(),
        arch: std::env::consts::ARCH.to_string(),
        vc_redist: hub_core::prereq::vc_redist_installed(),
        installed_count: db.entries.len(),
        catalog_generated_at: catalog.as_ref().map(|c| c.generated_at.clone()),
        // Nama saja, bukan path (PRD §18.2).
        detected_daws: hub_core::daw::detect_running_daws(&known)
            .into_iter()
            .filter_map(|d| d.name)
            .collect(),
        logs_dir: state.paths.logs_dir.to_string_lossy().to_string(),
    })
}

fn error_code(error: &HubError) -> String {
    serde_json::to_value(error)
        .ok()
        .and_then(|v| v.get("code").and_then(|c| c.as_str()).map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_matches_the_serde_tag() {
        // Frontend melakukan `switch` pada nilai ini, dan telemetry
        // mengelompokkan kegagalan dengannya — keduanya rusak diam-diam kalau
        // tag berubah.
        assert_eq!(
            error_code(&HubError::IntegrityMismatch {
                expected: "a".into(),
                actual: "b".into()
            }),
            "integrity_mismatch"
        );
        assert_eq!(error_code(&HubError::ElevationDenied), "elevation_denied");
    }
}
