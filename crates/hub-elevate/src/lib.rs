//! IPC ke helper elevated (PRD §11.1, §13.7).
//!
//! Crate ini dipakai dua sisi:
//! * proses utama memakai [`client::ElevatedSession`] untuk meminta operasi;
//! * `hub-helper.exe` memakai [`guard`] dan [`protocol`] untuk memvalidasi dan
//!   menjalankannya.
//!
//! Yang **tidak** ada di sini, dan tidak boleh ditambahkan: cara mengirim
//! perintah shell, cara membaca/menulis isi file, dan cara menjalankan proses
//! lain. Setiap kemampuan baru di protokol adalah kemampuan baru bagi penyerang
//! yang berhasil bicara ke pipe.

pub mod client;
pub mod guard;
pub mod protocol;

pub use client::{default_helper_path, ElevatedSession};
pub use guard::AllowedRoots;
pub use protocol::{HelperCommand, HelperResponse};
