//! `InstallPlan` — objek pusat (PRD §12.3).
//!
//! Pola dua langkah (`install_plan` lalu `install_start`) ada karena dialog
//! konfirmasi yang menampilkan angka konkret — ukuran, lokasi, apakah UAC akan
//! muncul — hanya mungkin jika backend sudah menghitungnya. Menghitungnya di
//! frontend berarti frontend butuh akses filesystem, yang justru ingin dihindari
//! (ADR-5).
//!
//! Konsekuensi kedua: `blockers` yang dihitung *sebelum* eksekusi berarti
//! kegagalan yang dapat diprediksi muncul sebagai dialog informatif, bukan
//! sebagai error di tengah progress bar.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::catalog::{Build, Catalog, InstallKind, Plugin};
use crate::paths::{AppPaths, InstallScope};
use crate::prereq::PrereqReport;
use crate::registry::InstalledDb;
use crate::verify::{self, Sha256Digest};
use crate::{HubError, Result};

/// Alasan sebuah rencana tidak dapat dieksekusi.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Blocker {
    #[serde(rename_all = "camelCase")]
    InsufficientDisk {
        required: u64,
        available: u64,
        volume: String,
    },
    #[serde(rename_all = "camelCase")]
    CpuFeatureMissing { feature: String },
    #[serde(rename_all = "camelCase")]
    OsTooOld { required: u32, current: Option<u32> },
    #[serde(rename_all = "camelCase")]
    LauncherTooOld { required: String, current: String },
    #[serde(rename_all = "camelCase")]
    NoDownloadUrl { plugin_id: String },
    #[serde(rename_all = "camelCase")]
    NoCompatibleBuild { target: String },
    /// DAW berjalan / file terkunci. Ini blocker yang *dapat diselesaikan*
    /// pengguna, jadi UI menampilkan tiga pilihan §8.7, bukan sekadar menolak.
    #[serde(rename_all = "camelCase")]
    FileLocked {
        path: String,
        holders: Vec<crate::error::ProcessHolder>,
        /// Opsi "pasang saat restart" hanya sah jika elevasi tersedia — ia
        /// menulis ke `PendingFileRenameOperations` di registry (PRD §8.7).
        reboot_option_available: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Warning {
    /// Rilis mengubah perilaku DSP atau format preset (FR-4.5).
    BreakingChange { summary: String },
    /// Prasyarat tidak terpenuhi tapi tidak memblokir.
    PrereqMissing {
        name: String,
        detail: String,
        help_url: Option<String>,
    },
    /// Lokasi per-user tidak dipindai semua DAW secara default (R6).
    PerUserLocationMayNeedDawConfig { path: String },
    /// Instalasi akan meminta UAC.
    ElevationWillBeRequested,
    /// Plugin ini diadopsi; daftar file lamanya tidak diketahui.
    ReplacingAdoptedInstall,
    /// Rollback dari versi `breaking` dapat membuat preset tidak terbaca
    /// (PRD §13.6).
    RollbackMayBreakPresets,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadSpec {
    pub url: String,
    pub size_bytes: u64,
    pub sha256: String,
    /// Sudah ada di cache dan terverifikasi.
    pub cached: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetSpec {
    pub scope: InstallScope,
    /// Direktori bundle yang akan ditulis, ditampilkan apa adanya ke pengguna.
    pub install_dir: PathBuf,
    pub requires_elevation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskSpec {
    pub required_bytes: u64,
    pub available_bytes: u64,
    pub sufficient: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPlan {
    pub plan_id: String,
    pub plugin_id: String,
    pub plugin_name: String,
    /// `None` = instalasi baru.
    pub from_version: Option<String>,
    pub to_version: String,
    pub breaking: bool,
    pub changelog: String,

    pub download: DownloadSpec,
    pub target: TargetSpec,
    pub disk: DiskSpec,
    pub prereqs: PrereqReport,

    /// Non-empty = tidak dapat dieksekusi.
    pub blockers: Vec<Blocker>,
    /// Dapat dieksekusi, tapi tampilkan ini.
    pub warnings: Vec<Warning>,
    pub backup_will_be_created: bool,
    /// Path yang secara eksplisit tidak disentuh (FR-4.10). Ditampilkan ke
    /// pengguna karena "apa yang TIDAK akan kamu sentuh" adalah pertanyaan yang
    /// benar-benar mereka miliki.
    pub user_data_preserved: Vec<String>,

    // ── Field internal, tidak dikirim ke frontend ────────────────────────
    #[serde(skip)]
    pub archive_root: String,
    #[serde(skip)]
    pub install_kind: Option<InstallKind>,
    #[serde(skip)]
    pub expected_digest: Option<Sha256Digest>,
    #[serde(skip)]
    pub max_extract_bytes: Option<u64>,
}

impl InstallPlan {
    pub fn executable(&self) -> bool {
        self.blockers.is_empty()
    }

    /// Hash yang diharapkan. `None` hanya mungkin jika plan dideserialisasi
    /// dari frontend — dan eksekutor menolak plan seperti itu (lihat
    /// [`crate::install::execute`]).
    pub fn digest(&self) -> Result<Sha256Digest> {
        self.expected_digest
            .ok_or_else(|| HubError::internal("InstallPlan tanpa hash tidak dapat dieksekusi"))
    }
}

pub struct PlanInput<'a> {
    pub catalog: &'a Catalog,
    pub db: &'a InstalledDb,
    pub paths: &'a AppPaths,
    pub plugin_id: &'a str,
    /// `None` = versi `latest`.
    pub version: Option<&'a str>,
    pub scope: InstallScope,
}

/// Hitung apa yang akan terjadi tanpa melakukan apa pun (FR-3.2).
pub fn build_plan(input: PlanInput<'_>) -> Result<InstallPlan> {
    let plugin: &Plugin =
        input
            .catalog
            .plugin(input.plugin_id)
            .ok_or_else(|| HubError::PluginNotFound {
                plugin_id: input.plugin_id.to_string(),
            })?;

    let release = plugin
        .release(input.version)
        .ok_or_else(|| HubError::NoCompatibleBuild {
            plugin_id: plugin.id.clone(),
            version: input.version.unwrap_or("latest").to_string(),
        })?;

    let mut blockers = Vec::new();
    let mut warnings = Vec::new();

    // FR-1.7 / FR-7.3: build yang butuh launcher lebih baru tidak dipasang.
    if let Some(min) = &release.min_launcher_version {
        if !crate::version::satisfies_minimum(crate::LAUNCHER_VERSION, min) {
            blockers.push(Blocker::LauncherTooOld {
                required: min.clone(),
                current: crate::LAUNCHER_VERSION.to_string(),
            });
        }
    }

    let build: Option<&Build> = release.build_for_current_target();
    let Some(build) = build else {
        blockers.push(Blocker::NoCompatibleBuild {
            target: crate::catalog::CURRENT_TARGET.to_string(),
        });
        return Ok(skeleton_plan(
            plugin,
            release,
            input.scope,
            blockers,
            warnings,
        ));
    };

    // PRD §20.3: `url` adalah nilai yang di-*resolve*, bukan konstanta. Untuk
    // plugin berbayar di v2 ia null sampai endpoint terautentikasi menjawab.
    let Some(url) = build.url.clone() else {
        blockers.push(Blocker::NoDownloadUrl {
            plugin_id: plugin.id.clone(),
        });
        return Ok(skeleton_plan(
            plugin,
            release,
            input.scope,
            blockers,
            warnings,
        ));
    };

    let expected_digest = verify::parse_hex(&build.sha256)?;
    let target_dir = input.scope.vst3_dir()?;
    let install_dir = target_dir.join(&build.archive_root);

    let requires_elevation = input.scope.requires_elevation().unwrap_or(true);
    if requires_elevation {
        warnings.push(Warning::ElevationWillBeRequested);
    }
    if matches!(input.scope, InstallScope::CurrentUser) {
        warnings.push(Warning::PerUserLocationMayNeedDawConfig {
            path: target_dir.to_string_lossy().to_string(),
        });
    }

    // ── Prasyarat ────────────────────────────────────────────────────────
    let prereqs = crate::prereq::check(
        &plugin.requirements,
        build.requires_vc_redist,
        &target_dir,
        build.size_bytes,
    );

    for check in &prereqs.cpu_features {
        if !check.satisfied {
            blockers.push(Blocker::CpuFeatureMissing {
                feature: check.name.replace("CPU: ", ""),
            });
        }
    }
    if let Some(os) = &prereqs.os_build {
        if !os.satisfied {
            blockers.push(Blocker::OsTooOld {
                required: plugin.requirements.os_min_build.unwrap_or(0),
                current: crate::prereq::os_build_number(),
            });
        }
    }
    if let Some(vc) = &prereqs.vc_redist {
        if !vc.satisfied {
            warnings.push(Warning::PrereqMissing {
                name: vc.name.clone(),
                detail: vc.detail.clone(),
                help_url: vc.help_url.clone(),
            });
        }
    }

    let disk = prereqs
        .disk
        .as_ref()
        .map(|d| DiskSpec {
            required_bytes: d.required_bytes,
            available_bytes: d.available_bytes,
            sufficient: d.sufficient,
        })
        .unwrap_or(DiskSpec {
            required_bytes: build.size_bytes,
            available_bytes: u64::MAX,
            sufficient: true,
        });
    if !disk.sufficient {
        blockers.push(Blocker::InsufficientDisk {
            required: disk.required_bytes,
            available: disk.available_bytes,
            volume: prereqs
                .disk
                .as_ref()
                .map(|d| d.volume.clone())
                .unwrap_or_default(),
        });
    }

    // ── File terkunci ────────────────────────────────────────────────────
    let lock = crate::daw::check_lock(&install_dir, &input.catalog.daw_processes);
    if lock.locked {
        blockers.push(Blocker::FileLocked {
            path: install_dir.to_string_lossy().to_string(),
            holders: lock.holders,
            reboot_option_available: matches!(
                input.scope,
                InstallScope::AllUsers | InstallScope::Custom { .. }
            ) || requires_elevation,
        });
    }

    // ── State terpasang ──────────────────────────────────────────────────
    let installed = input.db.get(&plugin.id);
    let from_version = installed.map(|e| e.version.clone());
    if installed.map(|e| e.adopted).unwrap_or(false) {
        warnings.push(Warning::ReplacingAdoptedInstall);
    }
    if release.breaking {
        warnings.push(Warning::BreakingChange {
            summary: first_line(&release.changelog),
        });
    }

    // ── Cache ────────────────────────────────────────────────────────────
    let cached_path = input
        .paths
        .downloads_dir()
        .join(format!("{}.zip", build.sha256));
    let cached = cached_path.exists();

    let user_data_preserved: Vec<String> = plugin
        .user_data
        .preset_paths
        .iter()
        .chain(plugin.user_data.config_paths.iter())
        .map(|p| crate::paths::expand_env_vars(p))
        .collect();

    Ok(InstallPlan {
        plan_id: uuid::Uuid::new_v4().to_string(),
        plugin_id: plugin.id.clone(),
        plugin_name: plugin.name.clone(),
        from_version,
        to_version: release.version.clone(),
        breaking: release.breaking,
        changelog: release.changelog.clone(),
        download: DownloadSpec {
            url,
            size_bytes: build.size_bytes,
            sha256: build.sha256.clone(),
            cached,
        },
        target: TargetSpec {
            scope: input.scope.clone(),
            install_dir,
            requires_elevation,
        },
        disk,
        prereqs,
        blockers,
        warnings,
        backup_will_be_created: installed.is_some(),
        user_data_preserved,
        archive_root: build.archive_root.clone(),
        install_kind: Some(build.install_kind),
        expected_digest: Some(expected_digest),
        max_extract_bytes: plugin.requirements.disk_bytes,
    })
}

/// Plan yang tidak dapat dieksekusi karena tidak ada build/URL, tapi tetap punya
/// cukup informasi untuk dijelaskan ke pengguna.
fn skeleton_plan(
    plugin: &Plugin,
    release: &crate::catalog::Release,
    scope: InstallScope,
    blockers: Vec<Blocker>,
    warnings: Vec<Warning>,
) -> InstallPlan {
    InstallPlan {
        plan_id: uuid::Uuid::new_v4().to_string(),
        plugin_id: plugin.id.clone(),
        plugin_name: plugin.name.clone(),
        from_version: None,
        to_version: release.version.clone(),
        breaking: release.breaking,
        changelog: release.changelog.clone(),
        download: DownloadSpec {
            url: String::new(),
            size_bytes: 0,
            sha256: String::new(),
            cached: false,
        },
        target: TargetSpec {
            install_dir: scope.vst3_dir().unwrap_or_default(),
            scope,
            requires_elevation: false,
        },
        disk: DiskSpec {
            required_bytes: 0,
            available_bytes: 0,
            sufficient: true,
        },
        prereqs: PrereqReport {
            vc_redist: None,
            cpu_features: vec![],
            os_build: None,
            disk: None,
            blocking: true,
        },
        blockers,
        warnings,
        backup_will_be_created: false,
        user_data_preserved: vec![],
        archive_root: String::new(),
        install_kind: None,
        expected_digest: None,
        max_extract_bytes: None,
    }
}

fn first_line(changelog: &str) -> String {
    changelog
        .lines()
        .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .unwrap_or("Rilis ini mengubah perilaku yang dapat memengaruhi project lama.")
        .trim()
        .trim_start_matches("- ")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_without_digest_cannot_be_executed() {
        // Ini yang membuat "lupa memverifikasi" tidak mungkin: plan yang
        // dideserialisasi dari frontend kehilangan `expected_digest`, dan
        // eksekutor menolaknya.
        let mut plan = skeleton_plan(
            &Plugin {
                id: "x".into(),
                name: "X".into(),
                vendor: "V".into(),
                category: "c".into(),
                tagline: "t".into(),
                tagline_i18n: Default::default(),
                description: String::new(),
                description_i18n: Default::default(),
                readme: String::new(),
                icon_url: None,
                screenshots: vec![],
                homepage_url: None,
                source_url: None,
                license: None,
                hidden: false,
                deprecated: false,
                deprecation_notice: None,
                commercial: Default::default(),
                latest: crate::catalog::Release {
                    version: "1.0.0".into(),
                    released_at: None,
                    breaking: false,
                    security: false,
                    min_launcher_version: None,
                    changelog: String::new(),
                    builds: vec![],
                },
                history: vec![],
                user_data: Default::default(),
                requirements: Default::default(),
            },
            &crate::catalog::Release {
                version: "1.0.0".into(),
                released_at: None,
                breaking: false,
                security: false,
                min_launcher_version: None,
                changelog: String::new(),
                builds: vec![],
            },
            InstallScope::CurrentUser,
            vec![],
            vec![],
        );
        plan.blockers.clear();
        assert!(plan.digest().is_err());
    }

    #[test]
    fn first_line_skips_markdown_heading() {
        assert_eq!(
            first_line("### 1.3.0\n- Mode sidechain eksternal\n- Perbaikan denormal"),
            "Mode sidechain eksternal"
        );
    }
}
