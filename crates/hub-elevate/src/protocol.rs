//! Protokol antara proses utama dan `hub-helper.exe` (PRD §13.7).
//!
//! Command di bawah sengaja **sangat terbatas**. Helper tidak menerima perintah
//! shell, tidak menerima path arbitrer, dan tidak dapat diminta membaca atau
//! menulis isi file. Ia hanya memindahkan dan menghapus direktori, di lokasi
//! yang divalidasinya sendiri.
//!
//! Alasan pembatasan ini: helper berjalan pada High integrity level. Setiap
//! kemampuan yang diberikan padanya adalah kemampuan yang didapat penyerang
//! jika ia berhasil bicara ke pipe.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Versi protokol. Helper menolak client dengan versi berbeda alih-alih
/// menebak — helper lama + launcher baru adalah kombinasi yang mungkin terjadi
/// saat update parsial.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum HelperCommand {
    /// Handshake. Selalu command pertama.
    Hello { protocol_version: u32 },
    /// Pindahkan direktori dalam volume yang sama.
    MoveDir { from: PathBuf, to: PathBuf },
    /// Hapus direktori yang berada di bawah direktori VST3 sistem.
    RemoveDir { path: PathBuf },
    /// Jadwalkan penggantian saat reboot (`MOVEFILE_DELAY_UNTIL_REBOOT`).
    ScheduleReplaceOnReboot { from: PathBuf, to: PathBuf },
    /// Uji apakah direktori dapat ditulis.
    ProbeWritable { path: PathBuf },
    /// Selesai; helper keluar. Helper tidak persisten — satu instance per
    /// sesi elevasi, bukan service yang selalu berjalan.
    Goodbye,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum HelperResponse {
    Ok,
    Writable { writable: bool },
    Error { message: String },
}

/// Batas ukuran satu pesan. Pesan yang lebih besar dari ini tidak mungkin sah
/// dan hanya bisa berarti sesuatu sedang mencoba membanjiri helper.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// Nama pipe untuk satu sesi. Komponen acak berarti pipe tidak dapat ditebak
/// atau di-squat oleh proses lain sebelum helper terhubung.
pub fn session_pipe_name(session: &uuid::Uuid) -> String {
    format!("\\\\.\\pipe\\StudioHub-{}", session.simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_roundtrip_as_json_lines() {
        let cmd = HelperCommand::MoveDir {
            from: PathBuf::from("C:\\staging\\MyComp.vst3"),
            to: PathBuf::from("C:\\Program Files\\Common Files\\VST3\\MyComp.vst3"),
        };
        let line = serde_json::to_string(&cmd).unwrap();
        assert!(!line.contains('\n'), "pesan harus muat dalam satu baris");
        let back: HelperCommand = serde_json::from_str(&line).unwrap();
        assert!(matches!(back, HelperCommand::MoveDir { .. }));
    }

    #[test]
    fn pipe_name_is_unpredictable_per_session() {
        let a = session_pipe_name(&uuid::Uuid::new_v4());
        let b = session_pipe_name(&uuid::Uuid::new_v4());
        assert_ne!(a, b);
        assert!(a.starts_with("\\\\.\\pipe\\StudioHub-"));
    }
}
