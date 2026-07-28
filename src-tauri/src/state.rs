//! State aplikasi yang dikelola Tauri (PRD §11.1).

use std::collections::HashMap;
use std::sync::Arc;

use hub_core::catalog::fetch::CatalogFetcher;
use hub_core::catalog::Catalog;
use hub_core::download::CancellationToken;
use hub_core::paths::AppPaths;
use hub_core::prefs::Prefs;
use hub_core::registry::InstalledDb;
use hub_core::telemetry::Telemetry;
use hub_core::{HubError, Result};
use tokio::sync::Mutex;

/// Job yang sedang berjalan, agar `job_cancel` punya sesuatu untuk dibatalkan.
#[derive(Default)]
pub struct JobRegistry {
    jobs: HashMap<String, CancellationToken>,
}

impl JobRegistry {
    pub fn register(&mut self, job_id: &str) -> CancellationToken {
        let token = CancellationToken::new();
        self.jobs.insert(job_id.to_string(), token.clone());
        token
    }

    pub fn cancel(&self, job_id: &str) -> bool {
        match self.jobs.get(job_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    pub fn finish(&mut self, job_id: &str) {
        self.jobs.remove(job_id);
    }
}

pub struct AppState {
    pub paths: AppPaths,
    /// Mutex tunggal untuk DB. Dua job yang menulis bersamaan akan merusak
    /// `installed.json`; `tauri-plugin-single-instance` menutup kasus dua
    /// proses, mutex ini menutup kasus dua job dalam satu proses.
    pub db: Mutex<InstalledDb>,
    pub prefs: Mutex<Prefs>,
    pub catalog: Mutex<Option<Catalog>>,
    pub fetcher: Arc<CatalogFetcher>,
    pub jobs: Mutex<JobRegistry>,
    pub telemetry: Mutex<Telemetry>,
    /// Diisi saat sesi elevasi aktif, agar "Update all" ke lokasi sistem hanya
    /// memunculkan satu UAC prompt (PRD §13.7).
    pub elevated: Mutex<Option<Arc<hub_elevate::ElevatedSession>>>,
}

impl AppState {
    pub fn initialize() -> Result<Self> {
        let paths = AppPaths::resolve()?;
        paths.ensure_all()?;

        let prefs = Prefs::load(&paths.prefs());
        let mut db = InstalledDb::load(&paths.installed_db());

        // PRD §13.8: bersihkan sisa job yang terputus sebelum apa pun yang lain
        // menyentuh filesystem.
        if let Err(e) = hub_core::install::cleanup_after_crash(&paths, &mut db) {
            tracing::warn!(error = %e, "pembersihan pasca-crash tidak lengkap");
        }

        // FR-2.2 / FR-2.3: DB diverifikasi terhadap disk sebelum ditampilkan.
        let scan_dirs = hub_core::registry::reconcile::default_scan_dirs(None);
        // Katalog belum dimuat saat startup, jadi adopsi belum bisa dijalankan
        // — kita tidak tahu bundle mana yang milik kita. `library_list`
        // menjalankannya lagi begitu katalog tiba.
        let report = hub_core::registry::reconcile::reconcile(&mut db, &scan_dirs, None);
        if !report.marked_missing.is_empty() || !report.restored.is_empty() {
            tracing::info!(
                missing = ?report.marked_missing,
                restored = ?report.restored,
                "rekonsiliasi state"
            );
            let _ = db.save(&paths.installed_db());
        }

        let fetcher = Arc::new(CatalogFetcher::new(paths.cache_dir.clone())?);
        let telemetry = Telemetry::new(
            prefs.telemetry_enabled,
            prefs.install_id.clone(),
            telemetry_endpoint(),
        );

        Ok(AppState {
            paths,
            db: Mutex::new(db),
            prefs: Mutex::new(prefs),
            catalog: Mutex::new(None),
            fetcher,
            jobs: Mutex::new(JobRegistry::default()),
            telemetry: Mutex::new(telemetry),
            elevated: Mutex::new(None),
        })
    }

    /// Katalog yang sudah ada di memori, atau error yang dapat ditampilkan.
    pub async fn catalog_or_err(&self) -> Result<Catalog> {
        self.catalog
            .lock()
            .await
            .clone()
            .ok_or_else(|| HubError::NetworkUnreachable {
                retryable: true,
                detail: "katalog belum dimuat".into(),
            })
    }

    /// Mulai (atau pakai ulang) sesi elevasi.
    ///
    /// Memakai ulang sesi yang masih hidup adalah yang membuat "Update all" ke
    /// lokasi sistem = satu UAC prompt, bukan satu per plugin.
    pub async fn elevated_session(&self) -> Result<Arc<hub_elevate::ElevatedSession>> {
        let mut slot = self.elevated.lock().await;
        if let Some(session) = slot.as_ref() {
            return Ok(session.clone());
        }
        let helper = hub_elevate::default_helper_path()?;
        let session = Arc::new(hub_elevate::ElevatedSession::start(&helper)?);
        *slot = Some(session.clone());
        Ok(session)
    }
}

/// Endpoint telemetry, dikompilasi dari env saat build. Tidak ada default:
/// build tanpa variabel ini menghasilkan launcher yang tidak dapat mengirim
/// apa pun, yang merupakan default yang benar (PRD §17.1).
fn telemetry_endpoint() -> Option<String> {
    option_env!("STUDIO_HUB_TELEMETRY_ENDPOINT").map(str::to_string)
}
