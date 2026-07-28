//! Deteksi DAW dan uji lock file (PRD §11.9, §13.4).
//!
//! Dua mekanisme dipakai bersama karena keduanya tidak lengkap:
//!
//! * Daftar nama proses (dari katalog, FR-6.2) memungkinkan pesan spesifik
//!   — "FL Studio sedang berjalan" jauh lebih berguna daripada kode error.
//! * Uji lock menangkap apa yang tidak ada di daftar: plugin host terpisah,
//!   scanner plugin, DAW yang belum kita kenal, bahkan antivirus yang sedang
//!   memindai file.
//!
//! **Uji lock adalah yang menentukan.** Daftar proses hanya untuk kualitas pesan.

use std::path::Path;

use crate::catalog::DawProcess;
use crate::error::ProcessHolder;

/// Enumerasi proses dan cocokkan dengan daftar dari katalog (case-insensitive).
pub fn detect_running_daws(known: &[DawProcess]) -> Vec<ProcessHolder> {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};

    let system =
        System::new_with_specifics(RefreshKind::new().with_processes(ProcessRefreshKind::new()));

    let mut found: Vec<ProcessHolder> = Vec::new();
    for (pid, process) in system.processes() {
        let exe_name = process.name().to_string_lossy().to_string();
        let Some(daw) = known.iter().find(|d| {
            d.executables
                .iter()
                .any(|e| e.eq_ignore_ascii_case(&exe_name))
        }) else {
            continue;
        };
        // Satu DAW dapat punya beberapa proses (mis. plugin host terpisah);
        // laporkan satu baris per nama agar dialog tidak mengulang-ulang.
        if found.iter().any(|f| f.name.as_deref() == Some(&daw.name)) {
            continue;
        }
        found.push(ProcessHolder {
            name: Some(daw.name.clone()),
            executable: exe_name,
            pid: pid.as_u32(),
        });
    }
    found
}

/// True jika `path` (file atau bundle) sedang dipegang proses lain.
///
/// Untuk direktori bundle, yang diuji adalah binary di dalamnya — itu yang
/// benar-benar dipetakan ke memori oleh DAW.
pub fn is_path_locked(path: &Path) -> bool {
    let target = if path.is_dir() {
        match locate_bundle_binary(path) {
            Some(p) => p,
            // Direktori tanpa binary: yang menentukan adalah apakah kita bisa
            // me-rename direktorinya, dan itu diuji langsung saat commit.
            None => return false,
        }
    } else {
        path.to_path_buf()
    };
    !can_open_exclusive(&target)
}

fn locate_bundle_binary(bundle: &Path) -> Option<std::path::PathBuf> {
    let contents = bundle.join("Contents");
    for arch in ["x86_64-win", "x86-win"] {
        let dir = contents.join(arch);
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry.path().is_file() {
                    return Some(entry.path());
                }
            }
        }
    }
    None
}

#[cfg(windows)]
fn can_open_exclusive(path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    // share_mode(0) = tidak berbagi. Ini persis yang gagal ketika DAW sedang
    // memetakan DLL, yang adalah kondisi yang ingin kita deteksi.
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(path)
        .is_ok()
}

#[cfg(not(windows))]
fn can_open_exclusive(path: &Path) -> bool {
    // Unix tidak mengunci file yang sedang dipetakan; uji ini tidak bermakna
    // di luar Windows, dan target v1 memang Windows saja.
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .is_ok()
}

/// Gabungan kedua mekanisme, dipanggil sebelum operasi tulis (FR-6.1).
///
/// Mengembalikan daftar pemegang lock yang *diketahui*. Daftar kosong dengan
/// `locked == true` berarti sesuatu memegang file tapi kita tidak tahu apa —
/// itu tetap informasi yang berguna dan pesannya menyebutkan kemungkinan AV.
pub struct LockCheck {
    pub locked: bool,
    pub holders: Vec<ProcessHolder>,
}

pub fn check_lock(target: &Path, known_daws: &[DawProcess]) -> LockCheck {
    let locked = target.exists() && is_path_locked(target);
    let holders = if locked {
        detect_running_daws(known_daws)
    } else {
        Vec::new()
    };
    LockCheck { locked, holders }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlocked_file_reports_unlocked() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("free.dll");
        std::fs::write(&file, b"x").unwrap();
        assert!(!is_path_locked(&file));
    }

    #[cfg(windows)]
    #[test]
    fn exclusively_held_file_reports_locked() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("held.dll");
        std::fs::write(&file, b"x").unwrap();

        let _guard = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&file)
            .unwrap();

        assert!(is_path_locked(&file));
    }

    #[test]
    fn nonexistent_path_is_not_reported_as_locked() {
        let check = check_lock(Path::new("Z:\\tidak\\ada\\MyComp.vst3"), &[]);
        assert!(!check.locked);
    }

    #[test]
    fn process_matching_is_case_insensitive() {
        let known = vec![DawProcess {
            name: "REAPER".into(),
            executables: vec!["reaper.exe".into()],
        }];
        // Tidak dapat memaksa proses ada di CI, jadi yang diuji hanya bahwa
        // pemanggilan aman dan tidak panik.
        let _ = detect_running_daws(&known);
    }
}
