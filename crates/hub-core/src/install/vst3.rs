//! Strategi commit untuk `install_kind = vst3_bundle` (PRD §13.3).
//!
//! Seluruh urutan di bawah terdiri dari `rename` dalam satu volume, yang atomik
//! di level filesystem. Tidak ada titik di mana bundle berada dalam keadaan
//! setengah tertulis (NFR-2.1).

use std::path::{Path, PathBuf};

use crate::{HubError, Result};

/// Hasil commit, cukup untuk melakukan rollback jika langkah berikutnya gagal.
pub struct CommitOutcome {
    /// Lokasi bundle lama yang di-rename ke staging, jika ada sebelumnya.
    pub displaced: Option<PathBuf>,
    pub install_dir: PathBuf,
}

/// Pindahkan bundle dari staging ke tujuan.
///
/// Urutan (PRD §13.3):
/// 1. Bundle terekstrak ada di `staged_bundle`.
/// 2. Jika tujuan sudah ada, **rename** ke `<staging>/<nama>.old` — bukan
///    delete. Rename juga gagal jika file terkunci, artinya kita gagal
///    *sebelum* merusak apa pun.
/// 3. Rename staged → tujuan.
/// 4. Jika (3) gagal, kembalikan `.old` ke posisi semula.
pub fn commit_bundle(staged_bundle: &Path, install_dir: &Path) -> Result<CommitOutcome> {
    if !staged_bundle.is_dir() {
        return Err(HubError::internal(format!(
            "bundle staging tidak ada: {}",
            staged_bundle.display()
        )));
    }
    if let Some(parent) = install_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut displaced = None;

    if install_dir.exists() {
        let old = staging_sibling(staged_bundle, "old");
        // Bersihkan sisa percobaan sebelumnya agar rename tidak gagal.
        let _ = std::fs::remove_dir_all(&old);

        std::fs::rename(install_dir, &old).map_err(|e| map_lock_error(e, install_dir))?;
        displaced = Some(old);
    }

    if let Err(e) = std::fs::rename(staged_bundle, install_dir) {
        // Langkah 4: pulihkan sebelum melapor.
        if let Some(old) = &displaced {
            if let Err(restore_err) = std::fs::rename(old, install_dir) {
                tracing::error!(
                    error = %restore_err,
                    path = ?install_dir,
                    "gagal memulihkan bundle lama setelah commit gagal"
                );
            }
        }
        return Err(map_lock_error(e, install_dir));
    }

    Ok(CommitOutcome {
        displaced,
        install_dir: install_dir.to_path_buf(),
    })
}

/// Langkah 5: pindahkan bundle lama ke direktori backup, menimpa backup lama.
///
/// Backup menyimpan satu versi sebelumnya per plugin (FR-4.8). Kegagalan di
/// sini **tidak** menggagalkan instalasi — instalasinya sudah berhasil; yang
/// hilang hanya kemampuan rollback, dan itu dilaporkan sebagai peringatan.
pub fn archive_displaced(displaced: &Path, backup_dir: &Path) -> Result<PathBuf> {
    if let Some(parent) = backup_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_dir_all(backup_dir);

    match std::fs::rename(displaced, backup_dir) {
        Ok(()) => Ok(backup_dir.to_path_buf()),
        Err(_) => {
            // Backup dan staging bisa berada di volume berbeda ketika tujuan
            // ada di drive lain (PRD §11.3). Di situ rename tidak mungkin dan
            // copy adalah satu-satunya jalan.
            copy_dir_all(displaced, backup_dir)?;
            let _ = std::fs::remove_dir_all(displaced);
            Ok(backup_dir.to_path_buf())
        }
    }
}

/// Rollback: pulihkan bundle dari backup (PRD §13.5).
///
/// Backup disalin, bukan dipindahkan, sehingga kegagalan di tengah jalan tidak
/// menghancurkan satu-satunya salinan versi lama.
pub fn restore_from_backup(backup: &Path, staging_dir: &Path, install_dir: &Path) -> Result<()> {
    if !backup.is_dir() {
        return Err(HubError::internal("backup tidak ditemukan"));
    }
    let staged = staging_dir.join(
        install_dir
            .file_name()
            .ok_or_else(|| HubError::internal("install_dir tanpa nama"))?,
    );
    let _ = std::fs::remove_dir_all(&staged);
    copy_dir_all(backup, &staged)?;

    let outcome = commit_bundle(&staged, install_dir)?;
    if let Some(displaced) = outcome.displaced {
        let _ = std::fs::remove_dir_all(displaced);
    }
    Ok(())
}

/// Uninstall: hapus tepat file yang tercatat, bukan `remove_dir_all` membabi
/// buta pada folder yang mungkin berisi file pengguna (FR-5.1).
///
/// Untuk entri `adopted` (daftar file tidak diketahui) pemanggil harus
/// meneruskan `files` kosong; fungsi ini kemudian hanya menghapus bundle jika
/// isinya persis seperti bundle VST3 dan tidak ada file asing di dalamnya.
pub fn remove_installed(
    install_dir: &Path,
    files: &[String],
    adopted: bool,
) -> Result<Vec<String>> {
    let mut failures = Vec::new();

    if !files.is_empty() {
        let root = install_dir
            .parent()
            .ok_or_else(|| HubError::internal("install_dir tanpa parent"))?;
        for relative in files {
            let path = root.join(relative.replace('\\', "/"));
            // Pertahanan terhadap `installed.json` yang dimanipulasi: jangan
            // pernah menghapus di luar direktori bundle.
            if !path.starts_with(install_dir) {
                tracing::warn!(?path, "entri installed_files di luar bundle, dilewati");
                continue;
            }
            if path.is_file() {
                if let Err(e) = std::fs::remove_file(&path) {
                    failures.push(format!("{}: {e}", path.display()));
                }
            }
        }
        remove_empty_dirs(install_dir);
    } else if adopted {
        // Konservatif: hanya hapus jika seluruh isinya adalah struktur bundle
        // yang kita kenali.
        if bundle_contains_only_known_layout(install_dir) {
            if let Err(e) = std::fs::remove_dir_all(install_dir) {
                failures.push(format!("{}: {e}", install_dir.display()));
            }
        } else {
            failures.push(format!(
                "{} berisi file yang tidak dikenali; tidak dihapus otomatis",
                install_dir.display()
            ));
        }
    }

    Ok(failures)
}

fn bundle_contains_only_known_layout(bundle: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(bundle) else {
        return false;
    };
    entries.flatten().all(|entry| {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        name == "contents" || name == "desktop.ini"
    })
}

fn remove_empty_dirs(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            remove_empty_dirs(&entry.path());
        }
    }
    let empty = std::fs::read_dir(root)
        .map(|mut d| d.next().is_none())
        .unwrap_or(false);
    if empty {
        let _ = std::fs::remove_dir(root);
    }
}

pub fn copy_dir_all(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn staging_sibling(staged_bundle: &Path, suffix: &str) -> PathBuf {
    let name = staged_bundle
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "bundle".to_string());
    staged_bundle
        .parent()
        .unwrap_or(staged_bundle)
        .join(format!("{name}.{suffix}"))
}

/// `ERROR_SHARING_VIOLATION` (32) dan `ERROR_ACCESS_DENIED` (5) adalah dua cara
/// Windows mengatakan "ada yang memakai file ini". Memetakannya ke
/// [`HubError::FileLocked`] adalah yang membuat UI dapat menawarkan §8.7 alih-
/// alih menampilkan kode error.
fn map_lock_error(e: std::io::Error, path: &Path) -> HubError {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_LOCK_VIOLATION: i32 = 33;

    let code = e.raw_os_error().unwrap_or(0);
    if matches!(
        code,
        ERROR_SHARING_VIOLATION | ERROR_ACCESS_DENIED | ERROR_LOCK_VIOLATION
    ) || e.kind() == std::io::ErrorKind::PermissionDenied
    {
        HubError::FileLocked {
            path: path.to_string_lossy().to_string(),
            holders: Vec::new(),
        }
    } else {
        HubError::internal(format!("rename gagal: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bundle(root: &Path, name: &str, marker: &str) -> PathBuf {
        let bundle = root.join(name);
        let arch = bundle.join("Contents").join("x86_64-win");
        std::fs::create_dir_all(&arch).unwrap();
        std::fs::write(arch.join("MyComp.vst3"), marker.as_bytes()).unwrap();
        std::fs::write(bundle.join("desktop.ini"), b"[.ShellClassInfo]").unwrap();
        bundle
    }

    #[test]
    fn fresh_install_moves_bundle_into_place() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let staged = make_bundle(&staging, "MyComp.vst3", "v1");
        let target = dir.path().join("VST3").join("MyComp.vst3");

        let outcome = commit_bundle(&staged, &target).unwrap();
        assert!(outcome.displaced.is_none());
        assert_eq!(
            std::fs::read_to_string(target.join("Contents/x86_64-win/MyComp.vst3")).unwrap(),
            "v1"
        );
        assert!(!staged.exists());
    }

    #[test]
    fn update_displaces_old_bundle_instead_of_deleting_it() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let vst3 = dir.path().join("VST3");
        std::fs::create_dir_all(&vst3).unwrap();

        make_bundle(&vst3, "MyComp.vst3", "v1");
        let staged = make_bundle(&staging, "MyComp.vst3", "v2");
        let target = vst3.join("MyComp.vst3");

        let outcome = commit_bundle(&staged, &target).unwrap();
        let displaced = outcome.displaced.expect("versi lama harus dipertahankan");

        assert_eq!(
            std::fs::read_to_string(target.join("Contents/x86_64-win/MyComp.vst3")).unwrap(),
            "v2"
        );
        // Versi lama masih utuh dan dapat dijadikan backup.
        assert_eq!(
            std::fs::read_to_string(displaced.join("Contents/x86_64-win/MyComp.vst3")).unwrap(),
            "v1"
        );
    }

    #[test]
    fn archive_then_restore_returns_previous_version() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let vst3 = dir.path().join("VST3");
        std::fs::create_dir_all(&vst3).unwrap();

        make_bundle(&vst3, "MyComp.vst3", "v1");
        let staged = make_bundle(&staging, "MyComp.vst3", "v2");
        let target = vst3.join("MyComp.vst3");

        let outcome = commit_bundle(&staged, &target).unwrap();
        let backup = dir.path().join("backup").join("mycomp").join("1.2.1");
        archive_displaced(&outcome.displaced.unwrap(), &backup).unwrap();

        restore_from_backup(&backup, &staging, &target).unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join("Contents/x86_64-win/MyComp.vst3")).unwrap(),
            "v1"
        );
    }

    #[test]
    fn uninstall_removes_only_recorded_files() {
        let dir = tempfile::tempdir().unwrap();
        let vst3 = dir.path().join("VST3");
        std::fs::create_dir_all(&vst3).unwrap();
        let bundle = make_bundle(&vst3, "MyComp.vst3", "v1");

        // File yang tidak kita pasang — mis. preset yang pengguna taruh sendiri
        // di dalam bundle.
        std::fs::write(bundle.join("Contents").join("user-note.txt"), b"punya saya").unwrap();

        let files = vec![
            "MyComp.vst3\\Contents\\x86_64-win\\MyComp.vst3".to_string(),
            "MyComp.vst3\\desktop.ini".to_string(),
        ];
        let failures = remove_installed(&bundle, &files, false).unwrap();
        assert!(failures.is_empty());
        assert!(bundle.join("Contents").join("user-note.txt").exists());
    }

    #[test]
    fn uninstall_of_adopted_plugin_refuses_when_unknown_files_present() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = make_bundle(dir.path(), "MyVerb.vst3", "v1");
        std::fs::write(bundle.join("something-odd.dat"), b"?").unwrap();

        let failures = remove_installed(&bundle, &[], true).unwrap();
        assert_eq!(failures.len(), 1);
        assert!(bundle.exists());
    }

    #[test]
    fn installed_files_cannot_escape_the_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let vst3 = dir.path().join("VST3");
        std::fs::create_dir_all(&vst3).unwrap();
        let bundle = make_bundle(&vst3, "MyComp.vst3", "v1");
        let victim = vst3.join("OtherPlugin.vst3");
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::write(victim.join("important.dll"), b"jangan hapus").unwrap();

        let files = vec!["OtherPlugin.vst3\\important.dll".to_string()];
        remove_installed(&bundle, &files, false).unwrap();
        assert!(victim.join("important.dll").exists());
    }
}
