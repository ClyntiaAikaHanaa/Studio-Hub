//! Tipe error terstruktur (PRD §12.5).
//!
//! Setiap varian dipetakan ke satu pesan UI di §8.8. Frontend tidak pernah
//! menampilkan `to_string()` mentah — ia melakukan `switch` pada field `code`.

use serde::Serialize;

pub type Result<T> = std::result::Result<T, HubError>;

/// Proses yang memegang lock atas sebuah file.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ProcessHolder {
    /// Nama ramah dari katalog, mis. "FL Studio". `None` jika tidak dikenali.
    pub name: Option<String>,
    pub executable: String,
    pub pid: u32,
}

#[derive(Debug, Clone, thiserror::Error, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum HubError {
    #[error("tidak dapat menghubungi server katalog")]
    NetworkUnreachable { retryable: bool, detail: String },

    #[error("verifikasi integritas gagal")]
    IntegrityMismatch { expected: String, actual: String },

    #[error("arsip tidak valid: {reason}")]
    ArchiveRejected { reason: String },

    #[error("file sedang dipakai proses lain")]
    FileLocked {
        path: String,
        holders: Vec<ProcessHolder>,
    },

    #[error("izin administrator ditolak")]
    ElevationDenied,

    #[error("ruang disk tidak cukup")]
    InsufficientDisk {
        required: u64,
        available: u64,
        volume: String,
    },

    #[error("prasyarat belum terpasang: {name}")]
    PrereqMissing {
        name: String,
        help_url: Option<String>,
    },

    #[error("butuh Studio Hub versi {required} atau lebih baru")]
    LauncherTooOld { required: String, current: String },

    #[error("katalog tidak valid")]
    CatalogInvalid { detail: String },

    #[error("plugin tidak ditemukan di katalog")]
    PluginNotFound { plugin_id: String },

    #[error("tidak ada build yang cocok untuk platform ini")]
    NoCompatibleBuild { plugin_id: String, version: String },

    #[error("plugin belum terpasang")]
    NotInstalled { plugin_id: String },

    #[error("operasi dibatalkan")]
    Cancelled,

    #[error("kesalahan tak terduga")]
    Internal { correlation_id: String },
}

impl HubError {
    /// Bungkus error yang tidak punya representasi UI sendiri.
    ///
    /// `correlation_id` juga ditulis ke log, sehingga pengguna dapat menyebut ID
    /// itu saat melapor tanpa perlu mengirim seluruh log (PRD §12.5).
    pub fn internal(context: impl std::fmt::Display) -> Self {
        let correlation_id = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
        tracing::error!(correlation_id, error = %context, "internal error");
        HubError::Internal { correlation_id }
    }

    /// True jika mencoba ulang operasi yang sama masuk akal.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            HubError::NetworkUnreachable {
                retryable: true,
                ..
            } | HubError::FileLocked { .. }
        )
    }
}

impl From<std::io::Error> for HubError {
    fn from(e: std::io::Error) -> Self {
        HubError::internal(format!("io: {e}"))
    }
}

impl From<serde_json::Error> for HubError {
    fn from(e: serde_json::Error) -> Self {
        HubError::CatalogInvalid {
            detail: e.to_string(),
        }
    }
}

impl From<reqwest::Error> for HubError {
    fn from(e: reqwest::Error) -> Self {
        HubError::NetworkUnreachable {
            // Timeout dan error koneksi layak dicoba ulang; error status tidak.
            retryable: e.is_timeout() || e.is_connect() || e.is_request(),
            detail: e.to_string(),
        }
    }
}
