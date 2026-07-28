//! Ekstraksi ZIP yang aman (PRD §11.8).
//!
//! `zip::ZipArchive::extract()` bawaan **tidak dipakai**. Setiap entri divalidasi
//! sebelum satu byte pun ditulis; validasi yang berjalan sambil menulis berarti
//! file berbahaya pertama sudah mendarat di disk sebelum entri kedua ditolak.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use crate::{HubError, Result};

/// Batas pertahanan zip bomb (PRD §11.8 aturan 6–8).
const MAX_ENTRIES: usize = 10_000;
const MAX_COMPRESSION_RATIO: u64 = 1_000;
/// Dipakai jika katalog tidak menyebut `requirements.disk_bytes`.
const DEFAULT_MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

/// Nama device reserved Windows. Sebuah entri bernama `CON.dll` dapat membuat
/// operasi file berperilaku aneh alih-alih membuat file biasa.
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

pub struct ExtractOptions {
    /// Nama entri akar yang wajib ada, dari `build.archive_root` (§10.3).
    pub required_root: String,
    /// Batas total byte terdekompresi. Umumnya `requirements.disk_bytes * 1.5`.
    pub max_total_bytes: u64,
}

impl ExtractOptions {
    pub fn new(required_root: impl Into<String>, disk_bytes: Option<u64>) -> Self {
        let max_total_bytes = disk_bytes
            .map(|b| b.saturating_mul(3) / 2)
            .unwrap_or(DEFAULT_MAX_TOTAL_BYTES);
        ExtractOptions {
            required_root: required_root.into(),
            max_total_bytes,
        }
    }
}

#[derive(Debug, Default)]
pub struct ExtractReport {
    pub entries_written: usize,
    pub bytes_written: u64,
    /// Path relatif setiap file yang ditulis, untuk dicatat di `installed.json`
    /// (PRD §11.4) sehingga uninstall dapat menghapus tepat apa yang dipasang.
    pub files: Vec<String>,
}

fn reject(reason: impl Into<String>) -> HubError {
    HubError::ArchiveRejected {
        reason: reason.into(),
    }
}

/// Ekstrak `zip_path` ke `dest_dir`, yang harus kosong dan sudah dibuat.
///
/// Dua fase: validasi seluruh direktori arsip lebih dulu, baru menulis. Arsip
/// yang ditolak tidak meninggalkan satu file pun.
pub fn extract_verified(
    zip_path: &Path,
    dest_dir: &Path,
    options: &ExtractOptions,
) -> Result<ExtractReport> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| reject(format!("ZIP tidak dapat dibaca: {e}")))?;

    if archive.len() > MAX_ENTRIES {
        return Err(reject(format!(
            "arsip punya {} entri, batas {MAX_ENTRIES}",
            archive.len()
        )));
    }

    // ── Fase 1: validasi ──────────────────────────────────────────────────
    let mut planned: Vec<PlannedEntry> = Vec::with_capacity(archive.len());
    let mut total_uncompressed: u64 = 0;
    let mut root_seen = false;

    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| reject(format!("entri {i} tidak dapat dibaca: {e}")))?;

        let raw_name = entry.name().to_string();

        // `enclosed_name()` mengembalikan `None` untuk path absolut, komponen
        // `..`, dan drive letter — yaitu tepat kelas zip-slip (aturan 1, 2, 5).
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| reject(format!("entri menulis di luar target: {raw_name}")))?;

        check_components(&relative, &raw_name)?;

        // Symlink dan hard link (aturan 4). Bit tipe file ada di 4 bit atas
        // mode Unix; `0xA000` adalah S_IFLNK.
        if let Some(mode) = entry.unix_mode() {
            if mode & 0xF000 == 0xA000 {
                return Err(reject(format!("entri adalah symlink: {raw_name}")));
            }
        }

        let is_dir = entry.is_dir();
        let uncompressed = entry.size();
        let compressed = entry.compressed_size();

        if !is_dir {
            if compressed > 0 && uncompressed / compressed.max(1) > MAX_COMPRESSION_RATIO {
                return Err(reject(format!(
                    "rasio kompresi entri {raw_name} melebihi {MAX_COMPRESSION_RATIO}:1"
                )));
            }
            total_uncompressed = total_uncompressed.saturating_add(uncompressed);
            if total_uncompressed > options.max_total_bytes {
                return Err(reject(format!(
                    "total ukuran terdekompresi melebihi {} byte",
                    options.max_total_bytes
                )));
            }
        }

        // Perbandingan case-insensitive: path Windows tidak membedakan huruf
        // besar-kecil, jadi `dihtortion.vst3` dan `Dihtortion.vst3` menunjuk
        // direktori yang sama. Membandingkan persis di sini berarti arsip yang
        // sebenarnya sah ditolak **setelah** pengguna selesai mengunduh — mode
        // kegagalan termahal yang ada di alur ini.
        if relative
            .components()
            .next()
            .map(|c| {
                c.as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&options.required_root)
            })
            .unwrap_or(false)
        {
            root_seen = true;
        }

        planned.push(PlannedEntry {
            index: i,
            relative,
            is_dir,
        });
    }

    // Aturan 9: arsip harus benar-benar berisi bundle yang dijanjikan katalog.
    if !root_seen {
        return Err(reject(format!(
            "arsip tidak memuat entri akar `{}`",
            options.required_root
        )));
    }

    // ── Fase 2: tulis ─────────────────────────────────────────────────────
    let mut report = ExtractReport::default();
    for planned in &planned {
        let out_path = dest_dir.join(&planned.relative);

        if planned.is_dir {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut entry = archive
            .by_index(planned.index)
            .map_err(|e| reject(format!("entri tidak dapat dibaca: {e}")))?;

        let mut out = std::fs::File::create(&out_path)?;
        // `entry.size()` sudah dibatasi di fase 1, tapi header ZIP bisa
        // berbohong tentang ukuran. `take` membuat batas itu mengikat pada
        // byte yang benar-benar mengalir.
        let mut limited = (&mut entry).take(options.max_total_bytes.saturating_sub(report.bytes_written) + 1);
        let written = std::io::copy(&mut limited, &mut out)?;
        report.bytes_written = report.bytes_written.saturating_add(written);
        if report.bytes_written > options.max_total_bytes {
            drop(out);
            let _ = std::fs::remove_file(&out_path);
            return Err(reject("arsip melebihi batas ukuran saat ekstraksi"));
        }

        report.entries_written += 1;
        report
            .files
            .push(planned.relative.to_string_lossy().replace('/', "\\"));
    }

    Ok(report)
}

struct PlannedEntry {
    index: usize,
    relative: PathBuf,
    is_dir: bool,
}

fn check_components(relative: &Path, raw_name: &str) -> Result<()> {
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_string_lossy();
                check_name(&part, raw_name)?;
            }
            // `enclosed_name()` seharusnya sudah menyaring ini; pemeriksaan
            // ulang di sini murah dan tidak bergantung pada perilaku dependensi.
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(reject(format!("komponen path terlarang di: {raw_name}")));
            }
            Component::CurDir => {}
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(reject("entri dengan nama kosong"));
    }
    Ok(())
}

fn check_name(part: &str, raw_name: &str) -> Result<()> {
    // Aturan 3: karakter ilegal Windows.
    if part
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') || (c as u32) < 0x20)
    {
        return Err(reject(format!("karakter ilegal di nama entri: {raw_name}")));
    }
    // Trailing dot/spasi diperlakukan aneh oleh Win32 dan dapat dipakai untuk
    // membuat dua entri "berbeda" menunjuk ke file yang sama.
    if part.ends_with(' ') || part.ends_with('.') {
        return Err(reject(format!(
            "nama entri berakhir titik/spasi: {raw_name}"
        )));
    }
    // Aturan 3: nama device reserved, dengan atau tanpa ekstensi.
    let stem = part.split('.').next().unwrap_or(part).to_ascii_uppercase();
    if RESERVED_NAMES.contains(&stem.as_str()) {
        return Err(reject(format!("nama device reserved: {raw_name}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn build_zip(entries: &[(&str, &[u8])]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.zip");
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, data) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        (dir, path)
    }

    fn extract_to_temp(zip_path: &Path, root: &str) -> Result<(tempfile::TempDir, ExtractReport)> {
        let out = tempfile::tempdir().unwrap();
        let report = extract_verified(
            zip_path,
            out.path(),
            &ExtractOptions::new(root, Some(10 * 1024 * 1024)),
        )?;
        Ok((out, report))
    }

    #[test]
    fn happy_path_writes_expected_files() {
        let (_g, zip) = build_zip(&[
            ("MyComp.vst3/desktop.ini", b"[.ShellClassInfo]"),
            ("MyComp.vst3/Contents/x86_64-win/MyComp.vst3", b"MZfake"),
        ]);
        let (out, report) = extract_to_temp(&zip, "MyComp.vst3").unwrap();
        assert_eq!(report.entries_written, 2);
        assert!(out
            .path()
            .join("MyComp.vst3/Contents/x86_64-win/MyComp.vst3")
            .exists());
    }

    #[test]
    fn zip_slip_is_rejected_and_writes_nothing() {
        let (_g, zip) = build_zip(&[
            ("MyComp.vst3/ok.txt", b"ok"),
            ("../../evil.dll", b"pwned"),
        ]);
        let out = tempfile::tempdir().unwrap();
        let err = extract_verified(
            &zip,
            out.path(),
            &ExtractOptions::new("MyComp.vst3", Some(1024)),
        )
        .unwrap_err();
        assert!(matches!(err, HubError::ArchiveRejected { .. }));
        // Validasi terjadi sebelum penulisan: tidak ada file yang mendarat.
        assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
    }

    #[test]
    fn reserved_device_name_is_rejected() {
        let (_g, zip) = build_zip(&[("MyComp.vst3/CON.dll", b"x")]);
        assert!(extract_to_temp(&zip, "MyComp.vst3").is_err());
    }

    #[test]
    fn absolute_path_entry_is_rejected() {
        let (_g, zip) = build_zip(&[("/etc/passwd", b"x")]);
        assert!(extract_to_temp(&zip, "MyComp.vst3").is_err());
    }

    #[test]
    fn archive_root_matching_ignores_case() {
        // JUCE memakai `PRODUCT_NAME` apa adanya sebagai nama bundle, dan itu
        // sering berbeda kapitalisasi dari nama plugin di katalog.
        let (_g, zip) = build_zip(&[("dihtortion.vst3/Contents/x86_64-win/x.vst3", b"MZ")]);
        let (_out, report) = extract_to_temp(&zip, "Dihtortion.vst3").unwrap();
        assert_eq!(report.entries_written, 1);
    }

    #[test]
    fn archive_without_declared_root_is_rejected() {
        let (_g, zip) = build_zip(&[("SomethingElse.vst3/x.dll", b"x")]);
        let err = extract_to_temp(&zip, "MyComp.vst3").unwrap_err();
        match err {
            HubError::ArchiveRejected { reason } => assert!(reason.contains("entri akar")),
            other => panic!("varian salah: {other:?}"),
        }
    }

    #[test]
    fn oversized_expansion_is_rejected_before_filling_disk() {
        // 2 MB nol terkompresi sangat kecil; batas 64 KB harus menghentikannya.
        let (_g, zip) = build_zip(&[("MyComp.vst3/big.bin", &vec![0u8; 2 * 1024 * 1024])]);
        let out = tempfile::tempdir().unwrap();
        let err = extract_verified(
            &zip,
            out.path(),
            &ExtractOptions::new("MyComp.vst3", Some(64 * 1024)),
        )
        .unwrap_err();
        assert!(matches!(err, HubError::ArchiveRejected { .. }));
    }
}
