//! `hub-core` — seluruh logika Studio Hub yang tidak bergantung pada Tauri.
//!
//! Pembagian modul mengikuti PRD §11.1. Aturan yang dipegang di seluruh crate:
//!
//! * Tidak ada jalur kode yang dapat memasang artefak tanpa verifikasi SHA-256
//!   (PRD G4). [`install::plan::InstallPlan`] tidak dapat dikonstruksi tanpa hash.
//! * Tidak ada operasi tulis yang tidak atomik (PRD NFR-2.1).
//! * Katalog diperlakukan sebagai input tidak tepercaya meskipun kita yang
//!   menerbitkannya (PRD §14.5).

pub mod archive;
pub mod catalog;
pub mod daw;
pub mod download;
pub mod error;
pub mod icons;
pub mod install;
pub mod paths;
pub mod prefs;
pub mod prereq;
pub mod registry;
pub mod telemetry;
pub mod verify;
pub mod version;

pub use error::{HubError, Result};

/// Versi launcher, dipakai untuk memeriksa `min_launcher_version` di katalog.
pub const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Host yang boleh menjadi sumber unduhan.
///
/// Ini adalah **pertahanan lapis kedua** (PRD §11.6 aturan 2). CI repo katalog
/// sudah memvalidasi hal yang sama, tapi launcher tidak boleh mempercayai
/// katalog sepenuhnya — katalog datang lewat jaringan.
pub const DOWNLOAD_HOST_ALLOWLIST: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
    "raw.githubusercontent.com",
];

/// Suffix host yang diizinkan (cocok untuk subdomain milik sendiri).
pub const DOWNLOAD_HOST_SUFFIX_ALLOWLIST: &[&str] = &[".githubusercontent.com", ".github.io"];

/// True jika `host` lolos allowlist unduhan.
pub fn host_is_allowed(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if DOWNLOAD_HOST_ALLOWLIST.iter().any(|h| *h == host) {
        return true;
    }
    DOWNLOAD_HOST_SUFFIX_ALLOWLIST
        .iter()
        .any(|suffix| host.ends_with(suffix))
}

#[cfg(test)]
mod host_tests {
    use super::host_is_allowed;

    #[test]
    fn allows_known_hosts() {
        assert!(host_is_allowed("github.com"));
        assert!(host_is_allowed("GitHub.com"));
        assert!(host_is_allowed("objects.githubusercontent.com"));
        assert!(host_is_allowed("robi.github.io"));
    }

    #[test]
    fn rejects_lookalikes() {
        // Ini kelas bug yang membuat allowlist berbasis `contains` berbahaya.
        assert!(!host_is_allowed("github.com.evil.tld"));
        assert!(!host_is_allowed("evil-github.com"));
        assert!(!host_is_allowed("githubusercontent.com.attacker.net"));
        assert!(!host_is_allowed("localhost"));
    }
}
