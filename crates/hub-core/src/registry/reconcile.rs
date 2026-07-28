//! Sinkronisasi database ↔ filesystem (FR-2.2, FR-2.3, FR-2.4).
//!
//! Database yang tidak diverifikasi terhadap disk akan berbohong begitu pengguna
//! menghapus folder plugin secara manual — dan mereka akan melakukannya.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{Health, InstalledDb, InstalledEntry};
use crate::paths::InstallScope;

/// Peta nama folder bundle (huruf kecil) → `plugin_id` di katalog.
///
/// Ini yang membatasi adopsi hanya pada plugin yang benar-benar kita
/// distribusikan. Tanpanya, launcher mengadopsi setiap bundle VST3 di sistem —
/// termasuk milik vendor lain — lalu menawarkan tombol Uninstall untuk
/// software yang sama sekali bukan urusannya. Studio Hub adalah launcher
/// first-party (PRD NG1), bukan pengelola seluruh folder VST3.
pub type KnownBundles = HashMap<String, String>;

/// Susun peta bundle yang dikenali dari katalog.
///
/// Nama bundle diambil dari `archive_root` setiap build — itu satu-satunya
/// sumber yang benar, karena nama folder bundle sering berbeda dari nama
/// plugin (mis. `PRODUCT_NAME` di JUCE). Nama plugin dipakai sebagai cadangan
/// untuk entri yang belum punya rilis.
pub fn known_bundles(catalog: &crate::catalog::Catalog) -> KnownBundles {
    let mut map = HashMap::new();
    for plugin in &catalog.plugins {
        for release in std::iter::once(&plugin.latest).chain(plugin.history.iter()) {
            for build in &release.builds {
                map.insert(build.archive_root.to_lowercase(), plugin.id.clone());
            }
        }
        map.insert(format!("{}.vst3", plugin.name.to_lowercase()), plugin.id.clone());
    }
    map
}

#[derive(Debug, Default)]
pub struct ReconcileReport {
    pub marked_missing: Vec<String>,
    pub adopted: Vec<String>,
    pub restored: Vec<String>,
    /// Entri adopsi yang ternyata bukan plugin kita, dilepas dari database.
    pub disowned: Vec<String>,
}

/// Rekonsiliasi penuh: periksa entri yang ada, lalu adopsi bundle yang belum
/// tercatat.
///
/// `known` adalah `None` kalau katalog belum tersedia (mis. startup offline
/// pertama). Dalam keadaan itu adopsi **tidak dijalankan sama sekali** — lebih
/// baik Library kosong sesaat daripada terisi plugin milik vendor lain yang
/// lalu ditawari tombol Uninstall. Begitu katalog tiba, `library_list`
/// menjalankan rekonsiliasi lagi dan daftarnya terisi.
pub fn reconcile(
    db: &mut InstalledDb,
    scan_dirs: &[PathBuf],
    known: Option<&KnownBundles>,
) -> ReconcileReport {
    let mut report = ReconcileReport::default();

    // ── Entri yang sudah tercatat ────────────────────────────────────────
    for entry in &mut db.entries {
        let present = bundle_looks_valid(&entry.install_dir);
        match (present, entry.health) {
            (false, Health::Missing) => {}
            (false, _) => {
                tracing::info!(plugin = %entry.plugin_id, "bundle hilang dari disk");
                entry.health = Health::Missing;
                report.marked_missing.push(entry.plugin_id.clone());
            }
            (true, Health::Missing) => {
                // Pengguna memasangnya kembali secara manual, atau drive
                // eksternal tersambung lagi.
                entry.health = Health::Ok;
                report.restored.push(entry.plugin_id.clone());
            }
            (true, _) => {}
        }
    }

    // Tanpa katalog kita tidak tahu mana yang plugin kita, jadi jangan menebak.
    let Some(known) = known else {
        return report;
    };

    // ── Lepas entri adopsi yang bukan plugin kita ────────────────────────
    //
    // Ini membersihkan database yang terlanjur terisi vendor lain. Entri yang
    // kita pasang sendiri (`adopted == false`) tidak pernah disentuh — itu
    // catatan instalasi kita, dan menghapusnya akan menghilangkan jejak file
    // yang dibutuhkan uninstall.
    db.entries.retain(|entry| {
        if !entry.adopted {
            return true;
        }
        let ours = bundle_is_known(&entry.install_dir, known).is_some();
        if !ours {
            tracing::info!(plugin = %entry.plugin_id, "bukan plugin Studio Hub, dilepas dari database");
            report.disowned.push(entry.plugin_id.clone());
        }
        ours
    });

    // ── Bundle yang belum tercatat ───────────────────────────────────────
    let recorded: Vec<PathBuf> = db.entries.iter().map(|e| e.install_dir.clone()).collect();
    for dir in scan_dirs {
        for bundle in list_vst3_bundles(dir) {
            if recorded.iter().any(|k| paths_equal(k, &bundle)) {
                continue;
            }
            let Some(plugin_id) = bundle_is_known(&bundle, known) else {
                tracing::debug!(path = ?bundle, "bundle bukan milik katalog, dilewati");
                continue;
            };
            let Some(entry) = adopt(&bundle, &plugin_id) else {
                continue;
            };
            tracing::info!(plugin = %entry.plugin_id, path = ?bundle, "mengadopsi plugin yang dipasang manual");
            report.adopted.push(entry.plugin_id.clone());
            db.upsert(entry);
        }
    }

    report
}

/// `Some(plugin_id)` kalau nama folder bundle cocok dengan entri katalog.
fn bundle_is_known(bundle: &Path, known: &KnownBundles) -> Option<String> {
    let name = bundle.file_name()?.to_string_lossy().to_lowercase();
    known.get(&name).cloned()
}

/// Direktori yang dipindai untuk adopsi.
pub fn default_scan_dirs(custom: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(p) = InstallScope::CurrentUser.vst3_dir() {
        dirs.push(p);
    }
    if let Ok(p) = InstallScope::AllUsers.vst3_dir() {
        dirs.push(p);
    }
    if let Some(p) = custom {
        dirs.push(p.to_path_buf());
    }
    dirs.retain(|d| d.is_dir());
    dirs.dedup();
    dirs
}

/// Bundle VST3 dianggap valid jika direktorinya ada dan memuat setidaknya satu
/// DLL di bawah `Contents`. Direktori kosong yang tertinggal dari uninstall
/// setengah jadi tidak boleh dihitung sebagai terpasang.
pub fn bundle_looks_valid(bundle: &Path) -> bool {
    if !bundle.is_dir() {
        return false;
    }
    let contents = bundle.join("Contents");
    if !contents.is_dir() {
        return false;
    }
    find_module_binary(&contents).is_some()
}

fn find_module_binary(contents: &Path) -> Option<PathBuf> {
    for arch_dir in ["x86_64-win", "x86-win", "MacOS", "x86_64-linux"] {
        let dir = contents.join(arch_dir);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.path().is_file() {
                return Some(entry.path());
            }
        }
    }
    None
}

fn list_vst3_bundles(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("vst3"))
                    .unwrap_or(false)
        })
        .filter(|p| bundle_looks_valid(p))
        .collect()
}

/// Metadata `moduleinfo.json` yang kita pedulikan (FR-2.4).
#[derive(Debug, Deserialize)]
struct ModuleInfo {
    #[serde(default, rename = "Version")]
    version: Option<String>,
    #[serde(default, rename = "Name")]
    name: Option<String>,
}

/// Buat entri untuk bundle yang dipasang di luar launcher.
///
/// `plugin_id` datang dari katalog, bukan diturunkan dari nama folder — hanya
/// bundle yang sudah dipastikan milik kita yang sampai ke sini.
fn adopt(bundle: &Path, plugin_id: &str) -> Option<InstalledEntry> {
    let plugin_id = plugin_id.to_string();
    let version = read_bundle_version(bundle);
    let health = if version.is_some() {
        Health::Ok
    } else {
        // FR-2.5: versi tidak dapat ditentukan → update ditawarkan sebagai
        // reinstall, bukan ditebak.
        Health::UnknownVersion
    };

    Some(InstalledEntry {
        plugin_id,
        version: version.unwrap_or_else(|| "0.0.0".to_string()),
        installed_at: super::now_rfc3339(),
        scope: scope_of(bundle),
        install_dir: bundle.to_path_buf(),
        artifact_sha256: None,
        installed_files: Vec::new(), // sengaja kosong: kita tidak tahu
        backup: None,
        skipped_versions: Vec::new(),
        adopted: true,
        health,
        highest_version_seen: None,
    })
}

fn scope_of(bundle: &Path) -> InstallScope {
    for scope in [InstallScope::CurrentUser, InstallScope::AllUsers] {
        if let Ok(dir) = scope.vst3_dir() {
            if bundle.starts_with(&dir) {
                return scope;
            }
        }
    }
    InstallScope::Custom {
        path: bundle.parent().unwrap_or(bundle).to_path_buf(),
    }
}

/// Baca versi dari bundle: `moduleinfo.json` lebih dulu, lalu version resource
/// dari DLL (PRD §25.1).
pub fn read_bundle_version(bundle: &Path) -> Option<String> {
    let contents = bundle.join("Contents");

    // VST3 SDK 3.7.5+ menaruhnya di `Contents/Resources/`; sebagian generator
    // lama menaruhnya langsung di `Contents/`. Coba keduanya — menebak satu
    // lokasi berarti deteksi versi diam-diam selalu gagal untuk separuh bundle.
    for candidate in [
        contents.join("Resources").join("moduleinfo.json"),
        contents.join("moduleinfo.json"),
    ] {
        let Ok(bytes) = std::fs::read(&candidate) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        match serde_json::from_str::<ModuleInfo>(&relax_json(&text)) {
            Ok(info) => {
                if let Some(v) = info.version.as_deref().and_then(crate::version::parse) {
                    return Some(v.to_string());
                }
                tracing::debug!(name = ?info.name, "moduleinfo.json tanpa versi yang dapat diparsing");
            }
            Err(e) => tracing::debug!(path = ?candidate, error = %e, "moduleinfo.json tidak terbaca"),
        }
    }

    let binary = find_module_binary(&contents)?;
    read_file_version(&binary)
}

/// Longgarkan JSON5 yang ditulis VST3 SDK menjadi JSON yang dapat diparsing
/// `serde_json`: buang komentar `//` dan `/* */`, lalu koma sebelum `]`/`}`.
///
/// `moduleinfo.json` yang dihasilkan JUCE benar-benar memuat trailing comma
/// (`"Sub Categories": ["Fx",]`). Parser ketat menolaknya, dan akibatnya setiap
/// plugin yang dipasang manual tampil sebagai "versi tidak diketahui".
fn relax_json(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }

        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' => match chars.peek() {
                Some('/') => {
                    for c in chars.by_ref() {
                        if c == '\n' {
                            out.push('\n');
                            break;
                        }
                    }
                }
                Some('*') => {
                    chars.next();
                    let mut prev = '\0';
                    for c in chars.by_ref() {
                        if prev == '*' && c == '/' {
                            break;
                        }
                        prev = c;
                    }
                }
                _ => out.push(c),
            },
            ',' => {
                // Tahan komanya sampai tahu apa yang menyusul.
                let mut lookahead = String::new();
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace() {
                        lookahead.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                match chars.peek() {
                    Some(']') | Some('}') => out.push_str(&lookahead),
                    _ => {
                        out.push(',');
                        out.push_str(&lookahead);
                    }
                }
            }
            _ => out.push(c),
        }
    }

    out
}

#[cfg(windows)]
fn read_file_version(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
    };

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut buffer = vec![0u8; size as usize];
        GetFileVersionInfoW(
            PCWSTR(wide.as_ptr()),
            0,
            size,
            buffer.as_mut_ptr() as *mut _,
        )
        .ok()?;

        let mut value_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut value_len: u32 = 0;
        let sub_block: Vec<u16> = "\\\0".encode_utf16().collect();
        let ok = VerQueryValueW(
            buffer.as_ptr() as *const _,
            PCWSTR(sub_block.as_ptr()),
            &mut value_ptr,
            &mut value_len,
        );
        if !ok.as_bool() || value_ptr.is_null() || (value_len as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>() {
            return None;
        }
        let info = &*(value_ptr as *const VS_FIXEDFILEINFO);
        let major = (info.dwFileVersionMS >> 16) & 0xffff;
        let minor = info.dwFileVersionMS & 0xffff;
        let patch = (info.dwFileVersionLS >> 16) & 0xffff;
        Some(format!("{major}.{minor}.{patch}"))
    }
}

#[cfg(not(windows))]
fn read_file_version(_path: &Path) -> Option<String> {
    None
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    // Path Windows tidak case-sensitive; membandingkan mentah akan mengadopsi
    // ulang bundle yang sudah tercatat hanya karena beda kapitalisasi.
    let norm = |p: &Path| p.to_string_lossy().to_ascii_lowercase().replace('/', "\\");
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bundle(root: &Path, name: &str, module_version: Option<&str>) -> PathBuf {
        let bundle = root.join(name);
        let arch = bundle.join("Contents").join("x86_64-win");
        std::fs::create_dir_all(&arch).unwrap();
        std::fs::write(arch.join(name), b"MZfake").unwrap();
        if let Some(v) = module_version {
            let resources = bundle.join("Contents").join("Resources");
            std::fs::create_dir_all(&resources).unwrap();
            // Bentuknya meniru keluaran JUCE/VST3 SDK yang sebenarnya,
            // trailing comma dan semuanya.
            std::fs::write(
                resources.join("moduleinfo.json"),
                format!(
                    "{{\n  \"Name\": \"X\",\n  \"Version\": \"{v}\",\n  \
                     \"Sub Categories\": [\n    \"Fx\",\n  ],\n}}\n"
                ),
            )
            .unwrap();
        }
        bundle
    }

    #[test]
    fn relax_json_handles_sdk_output() {
        let raw = "{\n  \"Version\": \"1.1.0\", // versi\n  \"List\": [\"Fx\",],\n}";
        let info: ModuleInfo = serde_json::from_str(&relax_json(raw)).unwrap();
        assert_eq!(info.version.as_deref(), Some("1.1.0"));
    }

    #[test]
    fn relax_json_leaves_string_contents_alone() {
        // Koma dan garis miring di dalam string tidak boleh disentuh.
        let raw = r#"{"Name": "a, b // c", "Version": "1.0.0"}"#;
        let info: ModuleInfo = serde_json::from_str(&relax_json(raw)).unwrap();
        assert_eq!(info.name.as_deref(), Some("a, b // c"));
    }

    #[test]
    fn version_is_read_from_resources_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = make_bundle(dir.path(), "Lazerator.vst3", Some("1.1.0"));
        assert_eq!(read_bundle_version(&bundle).as_deref(), Some("1.1.0"));
    }

    #[test]
    fn empty_directory_is_not_an_installation() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Ghost.vst3");
        std::fs::create_dir_all(bundle.join("Contents")).unwrap();
        assert!(!bundle_looks_valid(&bundle));
    }

    fn known(pairs: &[(&str, &str)]) -> KnownBundles {
        pairs
            .iter()
            .map(|(bundle, id)| (bundle.to_lowercase(), id.to_string()))
            .collect()
    }

    #[test]
    fn third_party_plugins_are_never_adopted() {
        // Inti NG1: Studio Hub adalah launcher first-party. Mengadopsi Serum
        // berarti menawarkan tombol Uninstall untuk software berbayar milik
        // vendor lain.
        let dir = tempfile::tempdir().unwrap();
        make_bundle(dir.path(), "Serum.vst3", Some("1.3.6"));
        make_bundle(dir.path(), "Lazerator.vst3", Some("1.1.0"));

        let mut db = InstalledDb::default();
        let report = reconcile(
            &mut db,
            &[dir.path().to_path_buf()],
            Some(&known(&[("Lazerator.vst3", "lazerator")])),
        );

        assert_eq!(report.adopted, vec!["lazerator"]);
        assert_eq!(db.entries.len(), 1);
        assert!(db.get("serum").is_none());
    }

    #[test]
    fn previously_adopted_third_party_entries_are_disowned() {
        let dir = tempfile::tempdir().unwrap();
        let serum = make_bundle(dir.path(), "Serum.vst3", Some("1.3.6"));

        let mut db = InstalledDb::default();
        db.upsert(InstalledEntry {
            plugin_id: "serum".into(),
            version: "1.3.6".into(),
            installed_at: super::super::now_rfc3339(),
            scope: InstallScope::CurrentUser,
            install_dir: serum,
            artifact_sha256: None,
            installed_files: vec![],
            backup: None,
            skipped_versions: vec![],
            adopted: true,
            health: Health::Ok,
            highest_version_seen: None,
        });

        let report = reconcile(
            &mut db,
            &[dir.path().to_path_buf()],
            Some(&known(&[("Lazerator.vst3", "lazerator")])),
        );

        assert_eq!(report.disowned, vec!["serum"]);
        assert!(db.entries.is_empty());
    }

    #[test]
    fn our_own_installs_are_never_disowned() {
        // Entri yang kita pasang sendiri membawa daftar file yang dibutuhkan
        // uninstall. Menghapusnya dari database berarti kehilangan jejak itu.
        let dir = tempfile::tempdir().unwrap();
        let bundle = make_bundle(dir.path(), "Dihtortion.vst3", Some("2.0.0"));

        let mut db = InstalledDb::default();
        db.upsert(InstalledEntry {
            plugin_id: "dihtortion".into(),
            version: "2.0.0".into(),
            installed_at: super::super::now_rfc3339(),
            scope: InstallScope::CurrentUser,
            install_dir: bundle,
            artifact_sha256: None,
            installed_files: vec!["Dihtortion.vst3\\Contents\\x86_64-win\\x".into()],
            backup: None,
            skipped_versions: vec![],
            adopted: false,
            health: Health::Ok,
            highest_version_seen: None,
        });

        // Katalog yang tidak menyebut plugin ini sekalipun tidak boleh
        // menghapus catatan instalasi kita sendiri.
        let report = reconcile(&mut db, &[dir.path().to_path_buf()], Some(&known(&[])));
        assert!(report.disowned.is_empty());
        assert!(db.get("dihtortion").is_some());
    }

    #[test]
    fn without_catalog_nothing_is_adopted_or_disowned() {
        let dir = tempfile::tempdir().unwrap();
        make_bundle(dir.path(), "Serum.vst3", Some("1.3.6"));

        let mut db = InstalledDb::default();
        let report = reconcile(&mut db, &[dir.path().to_path_buf()], None);

        assert!(report.adopted.is_empty());
        assert!(report.disowned.is_empty());
        assert!(db.entries.is_empty());
    }

    #[test]
    fn bundle_name_is_matched_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        make_bundle(dir.path(), "dihtortion.vst3", Some("2.0.0"));

        let mut db = InstalledDb::default();
        let report = reconcile(
            &mut db,
            &[dir.path().to_path_buf()],
            Some(&known(&[("Dihtortion.vst3", "dihtortion")])),
        );
        assert_eq!(report.adopted, vec!["dihtortion"]);
    }

    #[test]
    fn missing_files_flip_health_and_back() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = make_bundle(dir.path(), "MyComp.vst3", Some("1.3.0"));

        let mut db = InstalledDb::default();
        db.upsert(InstalledEntry {
            plugin_id: "mycomp".into(),
            version: "1.3.0".into(),
            installed_at: super::super::now_rfc3339(),
            scope: InstallScope::CurrentUser,
            install_dir: bundle.clone(),
            artifact_sha256: None,
            installed_files: vec![],
            backup: None,
            skipped_versions: vec![],
            adopted: false,
            health: Health::Ok,
            highest_version_seen: None,
        });

        std::fs::remove_dir_all(&bundle).unwrap();
        let report = reconcile(&mut db, &[], None);
        assert_eq!(report.marked_missing, vec!["mycomp"]);
        assert_eq!(db.get("mycomp").unwrap().health, Health::Missing);

        make_bundle(dir.path(), "MyComp.vst3", Some("1.3.0"));
        let report = reconcile(&mut db, &[], None);
        assert_eq!(report.restored, vec!["mycomp"]);
        assert_eq!(db.get("mycomp").unwrap().health, Health::Ok);
    }

    #[test]
    fn manually_installed_plugin_is_adopted_conservatively() {
        let dir = tempfile::tempdir().unwrap();
        make_bundle(dir.path(), "MyVerb.vst3", Some("2.0.1"));

        let mut db = InstalledDb::default();
        let report = reconcile(
            &mut db,
            &[dir.path().to_path_buf()],
            Some(&known(&[("MyVerb.vst3", "myverb")])),
        );

        assert_eq!(report.adopted, vec!["myverb"]);
        let entry = db.get("myverb").unwrap();
        assert!(entry.adopted);
        assert_eq!(entry.version, "2.0.1");
        // Daftar file kosong: uninstall harus bersikap konservatif untuk entri
        // yang tidak kita pasang sendiri.
        assert!(entry.installed_files.is_empty());
    }

    #[test]
    fn adopted_plugin_without_readable_version_is_flagged() {
        let dir = tempfile::tempdir().unwrap();
        make_bundle(dir.path(), "Mystery.vst3", None);

        let mut db = InstalledDb::default();
        reconcile(
            &mut db,
            &[dir.path().to_path_buf()],
            Some(&known(&[("Mystery.vst3", "mystery")])),
        );

        assert_eq!(db.get("mystery").unwrap().health, Health::UnknownVersion);
    }

    #[test]
    fn adoption_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        make_bundle(dir.path(), "MyVerb.vst3", Some("2.0.1"));

        let mut db = InstalledDb::default();
        let k = known(&[("MyVerb.vst3", "myverb")]);
        reconcile(&mut db, &[dir.path().to_path_buf()], Some(&k));
        reconcile(&mut db, &[dir.path().to_path_buf()], Some(&k));
        assert_eq!(db.entries.len(), 1);
    }
}
