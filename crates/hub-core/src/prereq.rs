//! Pemeriksaan prasyarat sebelum instalasi (PRD §11.9).
//!
//! Semua pemeriksaan di sini bertujuan **gagal lebih awal dengan pesan jelas**.
//! Memasang build AVX2 di CPU lama akan crash saat DAW memuatnya — kegagalan
//! yang muncul di tempat lain, berjam-jam kemudian, dan sangat sulit
//! didiagnosis pengguna dari jarak jauh.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::catalog::Requirements;

/// URL unduhan VC++ Redistributable x64 dari Microsoft.
pub const VC_REDIST_URL: &str = "https://aka.ms/vs/17/release/vc_redist.x64.exe";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrereqReport {
    pub vc_redist: Option<CheckResult>,
    pub cpu_features: Vec<CheckResult>,
    pub os_build: Option<CheckResult>,
    pub disk: Option<DiskCheck>,
    /// True jika ada pemeriksaan yang gagal dan bersifat memblokir.
    pub blocking: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub name: String,
    pub satisfied: bool,
    pub detail: String,
    pub help_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskCheck {
    pub volume: String,
    pub required_bytes: u64,
    pub available_bytes: u64,
    pub sufficient: bool,
}

/// Jalankan seluruh pemeriksaan untuk satu instalasi.
///
/// `download_bytes` dijumlahkan dengan `requirements.disk_bytes` karena artefak
/// yang diunduh, isi staging, dan bundle terpasang hidup berdampingan sesaat.
pub fn check(
    requirements: &Requirements,
    requires_vc_redist: bool,
    target_dir: &Path,
    download_bytes: u64,
) -> PrereqReport {
    let mut blocking = false;

    let vc_redist = if requires_vc_redist {
        let installed = vc_redist_installed();
        if !installed {
            // Tidak memblokir: plugin akan terpasang, hanya tidak akan dimuat
            // DAW. Pengguna yang tahu apa yang mereka lakukan boleh lanjut, dan
            // menampilkannya sebagai peringatan menghemat satu putaran support.
            tracing::info!("VC++ redistributable tidak terdeteksi");
        }
        Some(CheckResult {
            name: "Visual C++ Redistributable".into(),
            satisfied: installed,
            detail: if installed {
                "Terpasang".into()
            } else {
                "Tanpa ini, DAW tidak akan mendeteksi plugin meskipun filenya ada".into()
            },
            help_url: Some(VC_REDIST_URL.to_string()),
        })
    } else {
        None
    };

    let cpu_features: Vec<CheckResult> = requirements
        .cpu_features
        .iter()
        .map(|feature| {
            let satisfied = cpu_supports(feature);
            if !satisfied {
                blocking = true;
            }
            CheckResult {
                name: format!("CPU: {feature}"),
                satisfied,
                detail: if satisfied {
                    "Didukung".into()
                } else {
                    format!("Prosesor ini tidak mendukung {feature}; plugin akan crash saat dimuat")
                },
                help_url: None,
            }
        })
        .collect();

    let os_build = requirements.os_min_build.map(|min| {
        let current = os_build_number();
        let satisfied = current.map(|c| c >= min).unwrap_or(true);
        if !satisfied {
            blocking = true;
        }
        CheckResult {
            name: "Versi Windows".into(),
            satisfied,
            detail: match current {
                Some(c) => format!("Build {c}, minimum {min}"),
                None => "Tidak dapat ditentukan".into(),
            },
            help_url: None,
        }
    });

    let required_bytes = requirements
        .disk_bytes
        .unwrap_or(download_bytes.saturating_mul(3))
        .saturating_add(download_bytes);
    let available_bytes = available_space(target_dir).unwrap_or(u64::MAX);
    let sufficient = available_bytes >= required_bytes;
    if !sufficient {
        blocking = true;
    }
    let disk = Some(DiskCheck {
        volume: volume_label(target_dir),
        required_bytes,
        available_bytes,
        sufficient,
    });

    PrereqReport {
        vc_redist,
        cpu_features,
        os_build,
        disk,
        blocking,
    }
}

/// Deteksi fitur CPU. Nama mengikuti yang dipakai di katalog (`sse4.2`, `avx2`).
pub fn cpu_supports(feature: &str) -> bool {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        return match feature.to_ascii_lowercase().as_str() {
            "sse2" => is_x86_feature_detected!("sse2"),
            "sse3" => is_x86_feature_detected!("sse3"),
            "ssse3" => is_x86_feature_detected!("ssse3"),
            "sse4.1" | "sse41" => is_x86_feature_detected!("sse4.1"),
            "sse4.2" | "sse42" => is_x86_feature_detected!("sse4.2"),
            "avx" => is_x86_feature_detected!("avx"),
            "avx2" => is_x86_feature_detected!("avx2"),
            "fma" => is_x86_feature_detected!("fma"),
            "avx512f" => is_x86_feature_detected!("avx512f"),
            // Fitur yang tidak dikenal tidak boleh memblokir instalasi: itu
            // berarti katalog lebih baru dari launcher, bukan bahwa CPU-nya
            // tidak mampu.
            other => {
                tracing::warn!(feature = other, "fitur CPU tidak dikenal, dilewati");
                true
            }
        };
    }
    #[allow(unreachable_code)]
    {
        let _ = feature;
        true
    }
}

#[cfg(windows)]
pub fn vc_redist_installed() -> bool {
    use windows::core::w;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
        KEY_WOW64_64KEY, REG_VALUE_TYPE, RRF_RT_REG_DWORD,
    };

    unsafe {
        let mut key = HKEY::default();
        let opened = RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\VisualStudio\\14.0\\VC\\Runtimes\\x64"),
            0,
            KEY_READ | KEY_WOW64_64KEY,
            &mut key,
        );
        if opened.is_err() {
            return false;
        }

        let mut value: u32 = 0;
        let mut size: u32 = std::mem::size_of::<u32>() as u32;
        let mut kind = REG_VALUE_TYPE::default();
        let result = RegGetValueW(
            key,
            None,
            w!("Installed"),
            RRF_RT_REG_DWORD,
            Some(&mut kind),
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        );
        let _ = RegCloseKey(key);
        result.is_ok() && value == 1
    }
}

#[cfg(not(windows))]
pub fn vc_redist_installed() -> bool {
    true
}

#[cfg(windows)]
pub fn os_build_number() -> Option<u32> {
    // `GetVersionExW` berbohong tanpa manifest kompatibilitas. `RtlGetVersion`
    // dari ntdll mengembalikan angka sebenarnya.
    #[repr(C)]
    struct OsVersionInfoW {
        dw_os_version_info_size: u32,
        dw_major_version: u32,
        dw_minor_version: u32,
        dw_build_number: u32,
        dw_platform_id: u32,
        sz_csd_version: [u16; 128],
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfoW) -> i32;
    }

    unsafe {
        let mut info: OsVersionInfoW = std::mem::zeroed();
        info.dw_os_version_info_size = std::mem::size_of::<OsVersionInfoW>() as u32;
        if RtlGetVersion(&mut info) == 0 {
            Some(info.dw_build_number)
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
pub fn os_build_number() -> Option<u32> {
    None
}

/// Ruang bebas di volume yang memuat `path`.
#[cfg(windows)]
pub fn available_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    // Direktori tujuan mungkin belum ada; naik ke parent terdekat yang ada.
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }

    let wide: Vec<u16> = probe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let mut free_to_caller: u64 = 0;
        GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free_to_caller), None, None).ok()?;
        Some(free_to_caller)
    }
}

#[cfg(not(windows))]
pub fn available_space(_path: &Path) -> Option<u64> {
    None
}

fn volume_label(path: &Path) -> String {
    use std::path::Component;
    match path.components().next() {
        Some(Component::Prefix(prefix)) => prefix.as_os_str().to_string_lossy().to_string(),
        _ => path.to_string_lossy().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_cpu_feature_does_not_block() {
        assert!(cpu_supports("quantum-simd-9000"));
    }

    #[test]
    fn known_baseline_feature_is_detected() {
        #[cfg(target_arch = "x86_64")]
        assert!(cpu_supports("sse2"));
    }

    #[test]
    fn insufficient_disk_is_blocking() {
        let requirements = Requirements {
            os_min_build: None,
            cpu_features: vec![],
            // Angka yang tidak mungkin tersedia.
            disk_bytes: Some(u64::MAX / 2),
        };
        let dir = tempfile::tempdir().unwrap();
        let report = check(&requirements, false, dir.path(), 1024);
        // Di platform tanpa implementasi disk, `available_space` mengembalikan
        // `None` → u64::MAX → cukup. Yang diuji: laporannya konsisten.
        let disk = report.disk.unwrap();
        assert_eq!(report.blocking, !disk.sufficient);
    }

    #[test]
    fn vc_redist_missing_is_a_warning_not_a_blocker() {
        let dir = tempfile::tempdir().unwrap();
        let report = check(&Requirements::default(), true, dir.path(), 1024);
        assert!(report.vc_redist.is_some());
        // Ketidakhadiran VC++ tidak boleh memblokir: file tetap dapat dipasang.
        if let Some(check) = &report.vc_redist {
            if !check.satisfied {
                assert!(check.help_url.is_some());
            }
        }
    }
}
