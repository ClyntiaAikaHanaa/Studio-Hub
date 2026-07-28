//! State machine instalasi (PRD §13.1).
//!
//! ```text
//! Planned → Downloading → Verifying → Extracting → PreflightRun
//!         → [Elevating] → BackingUp → Committing → Finalizing → Succeeded
//!                                            └ gagal → RollingBack → Failed
//! ```
//!
//! Setiap transisi ditulis ke `journal.json` di direktori staging, sehingga
//! startup berikutnya dapat menyelesaikan atau membatalkan job yang terputus
//! (§13.8).

pub mod plan;
pub mod vst3;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::download::{CancellationToken, DownloadProgress, Downloader};
use crate::paths::AppPaths;
use crate::registry::{now_rfc3339, BackupRef, Health, InstalledDb, InstalledEntry};
use crate::{HubError, Result};

pub use plan::{Blocker, InstallPlan, PlanInput, Warning};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Planned,
    Downloading,
    Verifying,
    Extracting,
    PreflightRun,
    Elevating,
    BackingUp,
    Committing,
    Finalizing,
    Succeeded,
    RollingBack,
    Failed,
    Cancelled,
}

/// Peristiwa yang diteruskan ke UI (PRD §12.4).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum JobEvent {
    Queued,
    #[serde(rename_all = "camelCase")]
    Downloading {
        received: u64,
        total: u64,
        bytes_per_sec: u64,
    },
    Verifying,
    #[serde(rename_all = "camelCase")]
    Extracting {
        entries_done: usize,
        entries_total: usize,
    },
    Elevating,
    Installing,
    BackingUp,
    /// Job berhenti dan menunggu keputusan pengguna. Lebih baik daripada gagal
    /// langsung: unduhan yang sudah selesai tidak terbuang (PRD §12.4).
    Blocked {
        blocker: Blocker,
    },
    RollingBack {
        reason: String,
    },
    #[serde(rename_all = "camelCase")]
    Succeeded {
        version: String,
        needs_rescan: bool,
    },
    Failed {
        error: HubError,
    },
    Cancelled,
}

/// Journal yang ditulis ke staging setiap transisi (§13.1, §13.8).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    pub job_id: String,
    pub plugin_id: String,
    pub version: String,
    pub stage: Stage,
    pub install_dir: PathBuf,
    pub staged_bundle: Option<PathBuf>,
    pub displaced: Option<PathBuf>,
    pub updated_at: String,
}

impl Journal {
    fn write(&self, staging_dir: &Path) {
        let path = staging_dir.join("journal.json");
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = crate::registry::write_atomic(&path, &bytes);
        }
    }
}

/// Jembatan ke proses helper elevated.
///
/// Didefinisikan sebagai trait di sini, diimplementasikan di `hub-elevate`,
/// sehingga `hub-core` tetap dapat diuji tanpa UAC dan tanpa Windows: test
/// memakai implementasi yang langsung memanggil `std::fs`.
pub trait Elevator: Send + Sync {
    /// Pindahkan direktori dalam volume yang sama, lewat proses elevated.
    fn move_dir(&self, from: &Path, to: &Path) -> Result<()>;
    /// Hapus direktori di bawah direktori VST3 sistem.
    fn remove_dir(&self, path: &Path) -> Result<()>;
    /// Jadwalkan penggantian saat reboot (`MOVEFILE_DELAY_UNTIL_REBOOT`).
    fn schedule_replace_on_reboot(&self, from: &Path, to: &Path) -> Result<()>;
}

pub struct InstallContext<'a> {
    pub paths: &'a AppPaths,
    pub job_id: String,
    pub cancel: CancellationToken,
    pub known_daws: &'a [crate::catalog::DawProcess],
    /// `None` untuk instalasi per-user (jalur paling umum, ADR-3).
    pub elevator: Option<&'a dyn Elevator>,
}

/// Eksekusi rencana instalasi.
///
/// `on_event` dipanggil untuk setiap transisi. Fungsi ini tidak menyentuh UI dan
/// tidak tahu apa-apa tentang Tauri — lapisan `src-tauri` yang memetakan event
/// ke `emit`.
pub async fn execute<F>(
    plan: &InstallPlan,
    ctx: &InstallContext<'_>,
    db: &mut InstalledDb,
    mut on_event: F,
) -> Result<InstalledEntry>
where
    F: FnMut(JobEvent),
{
    if !plan.executable() {
        let blocker = plan.blockers[0].clone();
        on_event(JobEvent::Blocked {
            blocker: blocker.clone(),
        });
        return Err(blocker_to_error(blocker));
    }

    // G4: plan tanpa hash tidak dapat dieksekusi. Ini titik di mana jaminan
    // compile-time bertemu jaminan runtime — `expected_digest` adalah field
    // `#[serde(skip)]`, jadi plan yang dikirim balik frontend tidak akan lolos.
    let expected_digest = plan.digest()?;
    let install_kind = plan
        .install_kind
        .ok_or_else(|| HubError::internal("plan tanpa install_kind"))?;
    if install_kind != crate::catalog::InstallKind::Vst3Bundle {
        return Err(HubError::internal(format!(
            "install_kind {install_kind:?} belum didukung di v1"
        )));
    }

    let target_dir = plan
        .target
        .install_dir
        .parent()
        .ok_or_else(|| HubError::internal("install_dir tanpa parent"))?
        .to_path_buf();
    let staging_dir = ctx.paths.staging_for(&target_dir, &ctx.job_id);
    std::fs::create_dir_all(&staging_dir)?;

    let mut journal = Journal {
        job_id: ctx.job_id.clone(),
        plugin_id: plan.plugin_id.clone(),
        version: plan.to_version.clone(),
        stage: Stage::Planned,
        install_dir: plan.target.install_dir.clone(),
        staged_bundle: None,
        displaced: None,
        updated_at: now_rfc3339(),
    };
    journal.write(&staging_dir);

    let result = run_stages(
        plan,
        ctx,
        db,
        &staging_dir,
        expected_digest,
        &mut journal,
        &mut on_event,
    )
    .await;

    // Staging selalu dibersihkan, sukses maupun gagal. Satu-satunya
    // pengecualian ada di dalam `run_stages`: bundle lama yang sudah dipindah
    // ke backup.
    let _ = std::fs::remove_dir_all(&staging_dir);

    match result {
        Ok(entry) => {
            on_event(JobEvent::Succeeded {
                version: entry.version.clone(),
                // FR-3.10: DAW tidak memantau folder plugin; pengguna harus
                // rescan. Tidak memberi tahu ini adalah penyebab nomor satu
                // "instalasi berhasil tapi plugin tidak muncul".
                needs_rescan: true,
            });
            Ok(entry)
        }
        Err(HubError::Cancelled) => {
            on_event(JobEvent::Cancelled);
            Err(HubError::Cancelled)
        }
        Err(error) => {
            on_event(JobEvent::Failed {
                error: error.clone(),
            });
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_stages<F>(
    plan: &InstallPlan,
    ctx: &InstallContext<'_>,
    db: &mut InstalledDb,
    staging_dir: &Path,
    expected_digest: crate::verify::Sha256Digest,
    journal: &mut Journal,
    on_event: &mut F,
) -> Result<InstalledEntry>
where
    F: FnMut(JobEvent),
{
    macro_rules! stage {
        ($s:expr) => {{
            journal.stage = $s;
            journal.updated_at = now_rfc3339();
            journal.write(staging_dir);
            tracing::info!(job_id = %ctx.job_id, plugin = %plan.plugin_id, stage = ?$s, "transisi");
        }};
    }

    // ── Downloading + Verifying ──────────────────────────────────────────
    stage!(Stage::Downloading);
    on_event(JobEvent::Queued);

    let url = url::Url::parse(&plan.download.url).map_err(|e| HubError::CatalogInvalid {
        detail: format!("URL unduhan tidak valid: {e}"),
    })?;

    let downloader = Downloader::new()?;
    let request = crate::download::DownloadRequest {
        url,
        expected_size: plan.download.size_bytes,
        expected_sha256: expected_digest,
        dest_dir: ctx.paths.downloads_dir(),
    };

    let archive_path = downloader
        .download(&request, &ctx.cancel, |progress| match progress {
            DownloadProgress::Progress {
                received,
                total,
                bytes_per_sec,
            } => on_event(JobEvent::Downloading {
                received,
                total,
                bytes_per_sec,
            }),
            DownloadProgress::Verifying => on_event(JobEvent::Verifying),
            _ => {}
        })
        .await?;

    stage!(Stage::Verifying);
    check_cancelled(&ctx.cancel)?;

    // ── Extracting ───────────────────────────────────────────────────────
    stage!(Stage::Extracting);
    let extract_dir = staging_dir.join("extract");
    let _ = std::fs::remove_dir_all(&extract_dir);
    std::fs::create_dir_all(&extract_dir)?;

    let options = crate::archive::ExtractOptions::new(
        plan.archive_root.clone(),
        plan.max_extract_bytes
            .or(Some(plan.download.size_bytes.saturating_mul(8))),
    );
    let archive_path_owned = archive_path.clone();
    let extract_dir_owned = extract_dir.clone();
    // Ekstraksi memakai I/O blocking; menjalankannya di runtime async akan
    // memblokir worker thread dan membuat UI kehilangan event progres.
    let report = tokio::task::spawn_blocking(move || {
        crate::archive::extract_verified(&archive_path_owned, &extract_dir_owned, &options)
    })
    .await
    .map_err(|e| HubError::internal(format!("task ekstraksi panik: {e}")))??;

    on_event(JobEvent::Extracting {
        entries_done: report.entries_written,
        entries_total: report.entries_written,
    });

    let staged_bundle = extract_dir.join(&plan.archive_root);
    journal.staged_bundle = Some(staged_bundle.clone());
    check_cancelled(&ctx.cancel)?;

    // ── PreflightRun ─────────────────────────────────────────────────────
    //
    // Pemeriksaan diulang tepat sebelum penulisan (PRD §13.2). Antara pengguna
    // menekan Install dan sekarang, mereka bisa membuka DAW, mengisi disk, atau
    // mencabut drive eksternal — dan unduhan bisa berlangsung beberapa menit.
    stage!(Stage::PreflightRun);
    let lock = crate::daw::check_lock(&plan.target.install_dir, ctx.known_daws);
    if lock.locked {
        let blocker = Blocker::FileLocked {
            path: plan.target.install_dir.to_string_lossy().to_string(),
            holders: lock.holders.clone(),
            reboot_option_available: plan.target.requires_elevation,
        };
        on_event(JobEvent::Blocked { blocker });
        return Err(HubError::FileLocked {
            path: plan.target.install_dir.to_string_lossy().to_string(),
            holders: lock.holders,
        });
    }

    // Elevasi diminta **per-operasi**, tepat sebelum penulisan, bukan saat
    // startup (ADR-2, G5). Sampai titik ini seluruh pemrosesan input tidak
    // tepercaya — unduhan, JSON, ZIP — sudah selesai di integrity level Medium.
    let elevator = if plan.target.requires_elevation {
        stage!(Stage::Elevating);
        on_event(JobEvent::Elevating);
        Some(ctx.elevator.ok_or(HubError::ElevationDenied)?)
    } else {
        None
    };

    // ── BackingUp + Committing ───────────────────────────────────────────
    stage!(Stage::BackingUp);
    on_event(JobEvent::BackingUp);

    stage!(Stage::Committing);
    on_event(JobEvent::Installing);

    let commit = match elevator {
        Some(elevator) => commit_elevated(elevator, &staged_bundle, &plan.target.install_dir),
        None => vst3::commit_bundle(&staged_bundle, &plan.target.install_dir),
    };

    let outcome = match commit {
        Ok(o) => o,
        Err(e) => {
            stage!(Stage::RollingBack);
            on_event(JobEvent::RollingBack {
                reason: e.to_string(),
            });
            stage!(Stage::Failed);
            return Err(e);
        }
    };
    journal.displaced = outcome.displaced.clone();

    // ── Finalizing ───────────────────────────────────────────────────────
    stage!(Stage::Finalizing);

    let mut backup = None;
    if let Some(displaced) = &outcome.displaced {
        let previous_version = db
            .get(&plan.plugin_id)
            .map(|e| e.version.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let backup_dir = ctx.paths.backup_for(&plan.plugin_id, &previous_version);

        match vst3::archive_displaced(displaced, &backup_dir) {
            Ok(path) => {
                backup = Some(BackupRef {
                    version: previous_version,
                    path,
                    created_at: now_rfc3339(),
                })
            }
            Err(e) => {
                // Instalasi sudah berhasil; yang hilang hanya kemampuan
                // rollback. Menggagalkan job di sini akan lebih buruk.
                tracing::warn!(error = %e, "gagal mengarsipkan versi lama; rollback tidak tersedia");
            }
        }
    }

    let entry = InstalledEntry {
        plugin_id: plan.plugin_id.clone(),
        version: plan.to_version.clone(),
        installed_at: now_rfc3339(),
        scope: plan.target.scope.clone(),
        install_dir: plan.target.install_dir.clone(),
        artifact_sha256: Some(plan.download.sha256.clone()),
        installed_files: report.files.clone(),
        backup,
        skipped_versions: Vec::new(),
        adopted: false,
        health: Health::Ok,
        highest_version_seen: None,
    };
    db.upsert(entry.clone());
    db.save(&ctx.paths.installed_db())?;

    stage!(Stage::Succeeded);
    Ok(entry)
}

/// Versi [`vst3::commit_bundle`] yang setiap langkah renamenya dijalankan oleh
/// helper elevated. Urutannya identik dengan §13.3 — yang berbeda hanya siapa
/// yang memegang handle.
fn commit_elevated(
    elevator: &dyn Elevator,
    staged_bundle: &Path,
    install_dir: &Path,
) -> Result<vst3::CommitOutcome> {
    let mut displaced = None;

    if install_dir.exists() {
        let old = staged_bundle.with_extension("old");
        let _ = elevator.remove_dir(&old);
        elevator.move_dir(install_dir, &old)?;
        displaced = Some(old);
    }

    if let Err(e) = elevator.move_dir(staged_bundle, install_dir) {
        if let Some(old) = &displaced {
            if let Err(restore) = elevator.move_dir(old, install_dir) {
                tracing::error!(error = %restore, "gagal memulihkan bundle lama (elevated)");
            }
        }
        return Err(e);
    }

    Ok(vst3::CommitOutcome {
        displaced,
        install_dir: install_dir.to_path_buf(),
    })
}

fn check_cancelled(cancel: &CancellationToken) -> Result<()> {
    if cancel.is_cancelled() {
        Err(HubError::Cancelled)
    } else {
        Ok(())
    }
}

fn blocker_to_error(blocker: Blocker) -> HubError {
    match blocker {
        Blocker::InsufficientDisk {
            required,
            available,
            volume,
        } => HubError::InsufficientDisk {
            required,
            available,
            volume,
        },
        Blocker::CpuFeatureMissing { feature } => HubError::PrereqMissing {
            name: format!("CPU: {feature}"),
            help_url: None,
        },
        Blocker::OsTooOld { required, .. } => HubError::PrereqMissing {
            name: format!("Windows build {required}"),
            help_url: None,
        },
        Blocker::LauncherTooOld { required, current } => {
            HubError::LauncherTooOld { required, current }
        }
        Blocker::NoDownloadUrl { plugin_id } => HubError::NoCompatibleBuild {
            plugin_id,
            version: String::new(),
        },
        Blocker::NoCompatibleBuild { target } => HubError::NoCompatibleBuild {
            plugin_id: String::new(),
            version: target,
        },
        Blocker::FileLocked { path, holders, .. } => HubError::FileLocked { path, holders },
    }
}

/// Uninstall (FR-5.1 s/d FR-5.3).
pub fn uninstall(
    db: &mut InstalledDb,
    paths: &AppPaths,
    plugin_id: &str,
    remove_user_data: bool,
    user_data_paths: &[String],
) -> Result<Vec<String>> {
    let entry = db
        .get(plugin_id)
        .cloned()
        .ok_or_else(|| HubError::NotInstalled {
            plugin_id: plugin_id.to_string(),
        })?;

    let mut failures =
        vst3::remove_installed(&entry.install_dir, &entry.installed_files, entry.adopted)?;

    // FR-5.3: arsip backup ikut dihapus.
    let backup_root = paths.backup_dir.join(plugin_id);
    if backup_root.exists() {
        if let Err(e) = std::fs::remove_dir_all(&backup_root) {
            failures.push(format!("backup: {e}"));
        }
    }

    // FR-5.2: data pengguna hanya dihapus jika diminta eksplisit. Default
    // `false` diputuskan di UI, bukan di sini.
    if remove_user_data {
        for raw in user_data_paths {
            let path = PathBuf::from(crate::paths::expand_env_vars(raw));
            if path.is_dir() {
                if let Err(e) = std::fs::remove_dir_all(&path) {
                    failures.push(format!("{}: {e}", path.display()));
                }
            }
        }
    }

    db.remove(plugin_id);
    db.save(&paths.installed_db())?;
    Ok(failures)
}

/// Rollback ke versi yang diarsipkan (FR-4.9, PRD §13.5).
pub fn rollback(db: &mut InstalledDb, paths: &AppPaths, plugin_id: &str) -> Result<InstalledEntry> {
    let entry = db
        .get(plugin_id)
        .cloned()
        .ok_or_else(|| HubError::NotInstalled {
            plugin_id: plugin_id.to_string(),
        })?;
    let backup = entry
        .backup
        .clone()
        .ok_or_else(|| HubError::internal("tidak ada backup untuk plugin ini"))?;

    let staging = paths.staging_for(
        entry.install_dir.parent().unwrap_or(&entry.install_dir),
        &format!("rollback-{}", uuid::Uuid::new_v4().simple()),
    );
    std::fs::create_dir_all(&staging)?;

    let result = vst3::restore_from_backup(&backup.path, &staging, &entry.install_dir);
    let _ = std::fs::remove_dir_all(&staging);
    result?;

    let rolled_back_from = entry.version.clone();
    let mut restored = entry;
    restored.version = backup.version.clone();
    restored.installed_at = now_rfc3339();
    // Tidak ada rollback berlapis: satu versi backup, satu langkah mundur.
    restored.backup = None;
    restored.health = Health::Ok;
    // §13.5 langkah 4: versi yang baru saja dibatalkan masuk daftar skip.
    // Launcher yang langsung menawarkan kembali update yang baru di-rollback
    // terasa tidak memperhatikan.
    if !restored.skipped_versions.contains(&rolled_back_from) {
        restored.skipped_versions.push(rolled_back_from);
    }

    db.upsert(restored.clone());
    db.save(&paths.installed_db())?;
    Ok(restored)
}

/// Pembersihan setelah crash, dijalankan saat startup (PRD §13.8).
pub fn cleanup_after_crash(paths: &AppPaths, db: &mut InstalledDb) -> Result<usize> {
    let mut handled = 0;

    let Ok(entries) = std::fs::read_dir(&paths.staging_dir) else {
        return Ok(0);
    };

    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }

        let journal: Option<Journal> = std::fs::read(dir.join("journal.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());

        match journal {
            // Job berhenti sebelum Committing: tidak ada efek samping.
            None
            | Some(Journal {
                stage:
                    Stage::Planned
                    | Stage::Downloading
                    | Stage::Verifying
                    | Stage::Extracting
                    | Stage::PreflightRun
                    | Stage::Elevating
                    | Stage::BackingUp,
                ..
            }) => {
                let _ = std::fs::remove_dir_all(&dir);
            }
            // Berhenti antara Committing dan Finalizing: bundle di tujuan bisa
            // sudah benar. Periksa dulu, baru putuskan.
            Some(journal) if journal.stage == Stage::Committing => {
                if crate::registry::reconcile::bundle_looks_valid(&journal.install_dir) {
                    tracing::info!(job = %journal.job_id, "menyelesaikan job yang terputus");
                    finish_interrupted(db, paths, &journal)?;
                } else if let Some(displaced) = &journal.displaced {
                    tracing::warn!(job = %journal.job_id, "memulihkan bundle lama");
                    let _ = std::fs::rename(displaced, &journal.install_dir);
                }
                let _ = std::fs::remove_dir_all(&dir);
                handled += 1;
            }
            Some(_) => {
                let _ = std::fs::remove_dir_all(&dir);
                handled += 1;
            }
        }
    }

    crate::download::sweep_stale_parts(
        &paths.downloads_dir(),
        std::time::Duration::from_secs(7 * 24 * 3600),
    );

    Ok(handled)
}

fn finish_interrupted(db: &mut InstalledDb, paths: &AppPaths, journal: &Journal) -> Result<()> {
    let mut entry = db
        .get(&journal.plugin_id)
        .cloned()
        .unwrap_or(InstalledEntry {
            plugin_id: journal.plugin_id.clone(),
            version: journal.version.clone(),
            installed_at: now_rfc3339(),
            scope: crate::paths::InstallScope::CurrentUser,
            install_dir: journal.install_dir.clone(),
            artifact_sha256: None,
            installed_files: Vec::new(),
            backup: None,
            skipped_versions: Vec::new(),
            adopted: false,
            health: Health::Ok,
            highest_version_seen: None,
        });
    entry.version = journal.version.clone();
    entry.install_dir = journal.install_dir.clone();
    entry.health = Health::Ok;
    db.upsert(entry);
    db.save(&paths.installed_db())
}
