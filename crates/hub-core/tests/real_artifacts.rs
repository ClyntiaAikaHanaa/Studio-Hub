//! Uji ekstraktor terhadap ZIP rilis yang sebenarnya.
//!
//! Test ini **menemukan sendiri** setiap `.zip` di folder `release-assets/`
//! dan menurunkan entri akarnya dari isi arsip. Tidak ada nama plugin yang
//! ditulis di sini, dan menambah plugin baru tidak menuntut berkas ini
//! disunting — tes yang harus diperbarui setiap rilis adalah tes yang cepat
//! atau lambat dibiarkan basi.
//!
//! Dilewati kalau `release-assets/` tidak ada, sehingga CI dan kontributor lain
//! tidak terganggu.
//!
//! Kelas bug yang ditangkapnya nyata: `Compress-Archive` di Windows PowerShell
//! menulis pemisah path sebagai backslash, yang melanggar spesifikasi ZIP dan
//! membuat seluruh path menjadi satu nama berkas. Arsip seperti itu terlihat
//! baik-baik saja di Explorer dan ditolak launcher setelah pengguna selesai
//! mengunduh.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use hub_core::archive::{extract_verified, ExtractOptions};

fn assets_dir() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)?
        .join("release-assets");
    dir.is_dir().then_some(dir)
}

fn zips_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("zip"))
        .collect();
    out.sort();
    out
}

/// Komponen pertama setiap entri arsip.
///
/// Arsip yang sah punya tepat satu: folder bundle itu sendiri. Dua atau lebih
/// berarti ZIP dibungkus salah — dan nol berarti nama entrinya tidak memakai
/// `/` sebagai pemisah, yaitu bug backslash tadi.
fn root_entries(zip_path: &Path) -> BTreeSet<String> {
    let file = std::fs::File::open(zip_path).expect("ZIP tidak dapat dibuka");
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file)).expect("ZIP rusak");

    let mut roots = BTreeSet::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).expect("entri tidak terbaca");
        if let Some(first) = entry.name().split('/').next() {
            if !first.is_empty() {
                roots.insert(first.to_string());
            }
        }
    }
    roots
}

#[test]
fn every_release_archive_is_accepted_by_the_installer() {
    let Some(dir) = assets_dir() else {
        eprintln!("release-assets/ tidak ada — dilewati");
        return;
    };

    let zips = zips_in(&dir);
    if zips.is_empty() {
        eprintln!("release-assets/ kosong — dilewati");
        return;
    }

    for zip in &zips {
        let name = zip.file_name().unwrap().to_string_lossy().to_string();

        let roots = root_entries(zip);
        assert_eq!(
            roots.len(),
            1,
            "{name}: entri akar harus tepat satu, ditemukan {roots:?}. \
             Nol biasanya berarti pemisah path memakai backslash; \
             lebih dari satu berarti folder induk ikut terbungkus."
        );
        let archive_root = roots.into_iter().next().unwrap();

        assert!(
            archive_root.to_lowercase().ends_with(".vst3"),
            "{name}: entri akar `{archive_root}` bukan bundle .vst3"
        );

        let out = tempfile::tempdir().unwrap();
        let report = extract_verified(
            zip,
            out.path(),
            &ExtractOptions::new(archive_root.clone(), Some(200 * 1024 * 1024)),
        )
        .unwrap_or_else(|e| panic!("{name}: ditolak validator arsip: {e}"));

        // Satu berkas sudah sah: bundle VST3 minimal hanya berisi DLL-nya.
        // `moduleinfo.json` opsional, dan JUCE tidak selalu menghasilkannya.
        assert!(report.entries_written >= 1, "{name}: arsip kosong");

        let bundle = out.path().join(&archive_root);
        assert!(
            hub_core::registry::reconcile::bundle_looks_valid(&bundle),
            "{name}: hasil ekstraksi bukan bundle VST3 yang valid — \
             DLL di Contents/x86_64-win/ tidak ditemukan"
        );

        // Versi harus terbaca dari bundle — inilah yang dipakai FR-2.4 untuk
        // mengenali plugin yang dipasang manual. Bundle tanpa `moduleinfo.json`
        // masih terbaca lewat version resource DLL-nya.
        let version = hub_core::registry::reconcile::read_bundle_version(&bundle);
        assert!(version.is_some(), "{name}: versi tidak terbaca dari bundle");

        println!(
            "{name}: akar `{archive_root}`, {} entri, versi {:?}",
            report.entries_written, version
        );
    }

    println!("{} arsip diperiksa, semuanya diterima.", zips.len());
}

/// Nama entri ZIP wajib memakai `/`, bukan `\`.
///
/// Diuji terpisah karena kegagalannya paling menyesatkan: Windows Explorer
/// membuka arsip seperti itu dengan normal, jadi pemeriksaan manual tidak akan
/// pernah menangkapnya.
#[test]
fn release_archives_use_forward_slashes() {
    let Some(dir) = assets_dir() else {
        return;
    };

    for zip in zips_in(&dir) {
        let name = zip.file_name().unwrap().to_string_lossy().to_string();
        let file = std::fs::File::open(&zip).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file)).unwrap();

        for i in 0..archive.len() {
            let entry = archive.by_index(i).unwrap();
            assert!(
                !entry.name().contains('\\'),
                "{name}: nama entri `{}` memakai backslash. \
                 Spesifikasi ZIP menetapkan `/`; `Compress-Archive` di Windows \
                 PowerShell melanggarnya.",
                entry.name()
            );
        }
    }
}
