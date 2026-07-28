//! Integration test alur instalasi penuh terhadap filesystem sementara.
//!
//! Kasus di sini diambil langsung dari PRD §19.2 — masing-masing mewakili bug
//! yang mudah terjadi dan mahal jika lolos ke pengguna.

use std::io::Write;
use std::path::{Path, PathBuf};

use hub_core::catalog::Catalog;
use hub_core::download::CancellationToken;
use hub_core::install::{self, InstallContext, PlanInput};
use hub_core::paths::{AppPaths, InstallScope};
use hub_core::registry::InstalledDb;
use hub_core::HubError;
use sha2::Digest;

// ── Perkakas ─────────────────────────────────────────────────────────────

/// Bangun ZIP bundle VST3 yang valid, kembalikan (path, sha256, ukuran).
fn build_artifact(dir: &Path, marker: &str) -> (PathBuf, String, u64) {
    let path = dir.join(format!("MyComp-{marker}.zip"));
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();

    zip.start_file("MyComp.vst3/desktop.ini", options).unwrap();
    zip.write_all(b"[.ShellClassInfo]").unwrap();
    zip.start_file("MyComp.vst3/Contents/moduleinfo.json", options)
        .unwrap();
    zip.write_all(format!(r#"{{"Name":"MyComp","Version":"{marker}"}}"#).as_bytes())
        .unwrap();
    zip.start_file("MyComp.vst3/Contents/x86_64-win/MyComp.vst3", options)
        .unwrap();
    zip.write_all(marker.as_bytes()).unwrap();
    zip.finish().unwrap();

    let bytes = std::fs::read(&path).unwrap();
    let digest: [u8; 32] = sha2::Sha256::digest(&bytes).into();
    (path, hub_core::verify::to_hex(&digest), bytes.len() as u64)
}

fn catalog_json(version: &str, sha256: &str, size: u64, breaking: bool) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "generated_at": "2026-07-27T09:14:22Z",
        "daw_processes": [
            { "name": "REAPER", "executables": ["reaper.exe"] }
        ],
        "plugins": [{
            "id": "mycomp",
            "name": "MyComp",
            "vendor": "Studio Robi",
            "category": "dynamics",
            "tagline": "Kompresor VCA",
            "latest": {
                "version": version,
                "breaking": breaking,
                "changelog": format!("### {version}\n- Perubahan"),
                "builds": [{
                    "target": "windows-x86_64",
                    "format": "vst3",
                    "url": format!("https://github.com/robi/MyComp/releases/download/v{version}/MyComp.zip"),
                    "size_bytes": size,
                    "sha256": sha256,
                    "archive_root": "MyComp.vst3",
                    "install_kind": "vst3_bundle"
                }]
            },
            "user_data": {
                "preset_paths": ["%LOCALAPPDATA%\\VST3 Presets\\Studio Robi\\MyComp"]
            }
        }]
    }))
    .unwrap()
}

/// Simulasikan hasil unduhan dengan menaruh artefak langsung di cache dengan
/// nama hash-nya. Ini persis bentuk cache hit yang dipakai `Downloader`, jadi
/// alur instalasi berjalan tanpa jaringan tapi tetap melewati verifikasi.
fn seed_download_cache(paths: &AppPaths, artifact: &Path, sha256: &str) {
    let dest = paths.downloads_dir();
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::copy(artifact, dest.join(format!("{sha256}.zip"))).unwrap();
}

struct Fixture {
    _root: tempfile::TempDir,
    paths: AppPaths,
    vst3_dir: PathBuf,
    catalog: Catalog,
    db: InstalledDb,
}

fn fixture(version: &str, breaking: bool) -> (Fixture, String) {
    let root = tempfile::tempdir().unwrap();
    let paths = AppPaths::under(root.path());
    paths.ensure_all().unwrap();

    let vst3_dir = root.path().join("VST3");
    std::fs::create_dir_all(&vst3_dir).unwrap();

    let artifacts = root.path().join("artifacts");
    std::fs::create_dir_all(&artifacts).unwrap();
    let (artifact, sha256, size) = build_artifact(&artifacts, version);
    seed_download_cache(&paths, &artifact, &sha256);

    let catalog = Catalog::parse(&catalog_json(version, &sha256, size, breaking)).unwrap();

    (
        Fixture {
            _root: root,
            paths,
            vst3_dir,
            catalog,
            db: InstalledDb::default(),
        },
        sha256,
    )
}

async fn install(fx: &mut Fixture, version: Option<&str>) -> Result<(), HubError> {
    let scope = InstallScope::Custom {
        path: fx.vst3_dir.clone(),
    };
    let plan = install::plan::build_plan(PlanInput {
        catalog: &fx.catalog,
        db: &fx.db,
        paths: &fx.paths,
        plugin_id: "mycomp",
        version,
        scope,
    })?;

    let ctx = InstallContext {
        paths: &fx.paths,
        job_id: uuid::Uuid::new_v4().to_string(),
        cancel: CancellationToken::new(),
        known_daws: &fx.catalog.daw_processes,
        elevator: None,
    };

    install::execute(&plan, &ctx, &mut fx.db, |_| {}).await?;
    Ok(())
}

fn installed_marker(fx: &Fixture) -> String {
    std::fs::read_to_string(
        fx.vst3_dir
            .join("MyComp.vst3")
            .join("Contents")
            .join("x86_64-win")
            .join("MyComp.vst3"),
    )
    .unwrap()
}

// ── Kasus uji ────────────────────────────────────────────────────────────

#[tokio::test]
async fn fresh_install_writes_bundle_and_records_files() {
    let (mut fx, sha256) = fixture("1.3.0", false);
    install(&mut fx, None).await.unwrap();

    assert_eq!(installed_marker(&fx), "1.3.0");

    let entry = fx.db.get("mycomp").expect("entri DB harus ada");
    assert_eq!(entry.version, "1.3.0");
    assert_eq!(entry.artifact_sha256.as_deref(), Some(sha256.as_str()));
    assert!(!entry.adopted);
    // FR-5.1: daftar file harus lengkap agar uninstall dapat menghapus tepat
    // apa yang dipasang.
    assert_eq!(entry.installed_files.len(), 3);

    // Staging tidak boleh menyisakan apa pun.
    assert_eq!(
        std::fs::read_dir(&fx.paths.staging_dir).unwrap().count(),
        0
    );
}

#[tokio::test]
async fn plan_reports_concrete_numbers_before_anything_is_written() {
    // FR-3.2: dialog konfirmasi hanya bisa menampilkan angka nyata kalau
    // backend sudah menghitungnya.
    let (fx, sha256) = fixture("1.3.0", false);
    let plan = install::plan::build_plan(PlanInput {
        catalog: &fx.catalog,
        db: &fx.db,
        paths: &fx.paths,
        plugin_id: "mycomp",
        version: None,
        scope: InstallScope::Custom {
            path: fx.vst3_dir.clone(),
        },
    })
    .unwrap();

    assert!(plan.executable());
    assert_eq!(plan.to_version, "1.3.0");
    assert_eq!(plan.from_version, None);
    assert_eq!(plan.download.sha256, sha256);
    assert!(plan.download.size_bytes > 0);
    assert!(plan.download.cached);
    assert!(plan.target.install_dir.ends_with("MyComp.vst3"));
    assert!(!plan.backup_will_be_created);
    // FR-4.10: pengguna diberi tahu apa yang TIDAK akan disentuh.
    assert_eq!(plan.user_data_preserved.len(), 1);

    // Dry-run tidak menulis apa pun.
    assert!(!fx.vst3_dir.join("MyComp.vst3").exists());
}

#[tokio::test]
async fn update_backs_up_previous_version_and_rollback_restores_it() {
    let (mut fx, _) = fixture("1.2.1", false);
    install(&mut fx, None).await.unwrap();
    assert_eq!(installed_marker(&fx), "1.2.1");

    // Katalog naik ke 1.3.0.
    let artifacts = fx.paths.cache_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts).unwrap();
    let (artifact, sha256, size) = build_artifact(&artifacts, "1.3.0");
    seed_download_cache(&fx.paths, &artifact, &sha256);
    fx.catalog = Catalog::parse(&catalog_json("1.3.0", &sha256, size, false)).unwrap();

    install(&mut fx, None).await.unwrap();
    assert_eq!(installed_marker(&fx), "1.3.0");

    // FR-4.8: versi lama diarsipkan, bukan dihapus.
    let entry = fx.db.get("mycomp").unwrap();
    let backup = entry.backup.clone().expect("backup harus dibuat");
    assert_eq!(backup.version, "1.2.1");
    assert!(backup.path.exists());

    // FR-4.9 + §13.5.
    let restored = install::rollback(&mut fx.db, &fx.paths, "mycomp").unwrap();
    assert_eq!(restored.version, "1.2.1");
    assert_eq!(installed_marker(&fx), "1.2.1");
    // Tidak ada rollback berlapis.
    assert!(restored.backup.is_none());
    // §13.5 langkah 4: versi yang baru dibatalkan tidak langsung ditawarkan lagi.
    assert!(restored.skipped_versions.contains(&"1.3.0".to_string()));
}

#[tokio::test]
async fn tampered_artifact_aborts_and_leaves_target_untouched() {
    // PRD §19.2: hash tidak cocok → instalasi dibatalkan, tidak ada file yang
    // tertulis ke tujuan.
    let (mut fx, sha256) = fixture("1.3.0", false);

    // Ganti isi cache dengan artefak lain, tapi biarkan namanya (hash lama).
    let artifacts = fx.paths.cache_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts).unwrap();
    let (evil, _, _) = build_artifact(&artifacts, "9.9.9");
    std::fs::copy(
        &evil,
        fx.paths.downloads_dir().join(format!("{sha256}.zip")),
    )
    .unwrap();

    // Cache yang tidak cocok dibuang lalu diunduh ulang; tanpa jaringan di
    // test, unduhan gagal. Yang penting: tujuan tidak tersentuh.
    let result = install(&mut fx, None).await;
    assert!(result.is_err(), "instalasi harus dibatalkan");
    assert!(!fx.vst3_dir.join("MyComp.vst3").exists());
    assert!(fx.db.get("mycomp").is_none());
}

#[tokio::test]
async fn breaking_release_surfaces_a_warning_not_a_silent_install() {
    // FR-4.5: rilis yang ditandai breaking harus terlihat sebelum konfirmasi.
    let (fx, _) = fixture("2.0.0", true);
    let plan = install::plan::build_plan(PlanInput {
        catalog: &fx.catalog,
        db: &fx.db,
        paths: &fx.paths,
        plugin_id: "mycomp",
        version: None,
        scope: InstallScope::Custom {
            path: fx.vst3_dir.clone(),
        },
    })
    .unwrap();

    assert!(plan.breaking);
    assert!(plan
        .warnings
        .iter()
        .any(|w| matches!(w, install::Warning::BreakingChange { .. })));
}

#[tokio::test]
async fn uninstall_removes_plugin_but_keeps_user_presets() {
    // FR-5.2: default tidak menghapus data pengguna.
    let (mut fx, _) = fixture("1.3.0", false);
    install(&mut fx, None).await.unwrap();

    let presets = fx.paths.data_dir.join("presets");
    std::fs::create_dir_all(&presets).unwrap();
    std::fs::write(presets.join("my.vstpreset"), b"preset").unwrap();

    let failures = install::uninstall(
        &mut fx.db,
        &fx.paths,
        "mycomp",
        false,
        &[presets.to_string_lossy().to_string()],
    )
    .unwrap();

    assert!(failures.is_empty(), "uninstall gagal: {failures:?}");
    assert!(fx.db.get("mycomp").is_none());
    assert!(!fx
        .vst3_dir
        .join("MyComp.vst3")
        .join("Contents")
        .join("x86_64-win")
        .join("MyComp.vst3")
        .exists());
    assert!(presets.join("my.vstpreset").exists(), "preset harus selamat");
}

#[tokio::test]
async fn uninstall_with_explicit_consent_removes_user_data() {
    let (mut fx, _) = fixture("1.3.0", false);
    install(&mut fx, None).await.unwrap();

    let presets = fx.paths.data_dir.join("presets");
    std::fs::create_dir_all(&presets).unwrap();
    std::fs::write(presets.join("my.vstpreset"), b"preset").unwrap();

    install::uninstall(
        &mut fx.db,
        &fx.paths,
        "mycomp",
        true,
        &[presets.to_string_lossy().to_string()],
    )
    .unwrap();

    assert!(!presets.exists());
}

#[tokio::test]
async fn cancelled_job_leaves_no_partial_bundle() {
    let (mut fx, _) = fixture("1.3.0", false);

    let plan = install::plan::build_plan(PlanInput {
        catalog: &fx.catalog,
        db: &fx.db,
        paths: &fx.paths,
        plugin_id: "mycomp",
        version: None,
        scope: InstallScope::Custom {
            path: fx.vst3_dir.clone(),
        },
    })
    .unwrap();

    let cancel = CancellationToken::new();
    cancel.cancel();

    let ctx = InstallContext {
        paths: &fx.paths,
        job_id: "cancel-test".into(),
        cancel,
        known_daws: &fx.catalog.daw_processes,
        elevator: None,
    };

    let result = install::execute(&plan, &ctx, &mut fx.db, |_| {}).await;
    assert!(matches!(result, Err(HubError::Cancelled)));
    assert!(!fx.vst3_dir.join("MyComp.vst3").exists());
    assert_eq!(
        std::fs::read_dir(&fx.paths.staging_dir).unwrap().count(),
        0
    );
}

#[tokio::test]
async fn crash_cleanup_removes_staging_from_incomplete_jobs() {
    // PRD §13.8 langkah 2: job yang berhenti sebelum Committing tidak punya
    // efek samping, jadi staging cukup dihapus.
    let (mut fx, _) = fixture("1.3.0", false);

    let orphan = fx.paths.staging_dir.join("job-yang-mati");
    std::fs::create_dir_all(orphan.join("extract")).unwrap();
    std::fs::write(
        orphan.join("journal.json"),
        serde_json::to_vec(&serde_json::json!({
            "jobId": "job-yang-mati",
            "pluginId": "mycomp",
            "version": "1.3.0",
            "stage": "extracting",
            "installDir": fx.vst3_dir.join("MyComp.vst3"),
            "stagedBundle": null,
            "displaced": null,
            "updatedAt": "2026-07-27T09:00:00Z"
        }))
        .unwrap(),
    )
    .unwrap();

    install::cleanup_after_crash(&fx.paths, &mut fx.db).unwrap();
    assert!(!orphan.exists());
}

#[tokio::test]
async fn version_state_transitions_across_install_and_catalog_changes() {
    use hub_core::version::{compute_update_state, UpdateState};

    let (mut fx, _) = fixture("1.2.1", false);
    install(&mut fx, None).await.unwrap();
    let installed = fx.db.get("mycomp").unwrap().version.clone();

    assert_eq!(
        compute_update_state(Some(&installed), "1.2.1", false, &[]),
        UpdateState::UpToDate
    );
    assert!(matches!(
        compute_update_state(Some(&installed), "1.10.0", false, &[]),
        UpdateState::UpdateAvailable { .. }
    ));
    // T8: katalog yang di-rollback tidak boleh memicu "update" ke bawah.
    assert_eq!(
        compute_update_state(Some(&installed), "1.0.0", false, &[]),
        UpdateState::AheadOfCatalog
    );
}
