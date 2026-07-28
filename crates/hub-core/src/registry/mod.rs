//! Database lokal plugin terpasang (PRD §11.4, ADR-4).
//!
//! `installed.json` adalah sumber kebenaran launcher, disinkronkan dengan
//! filesystem saat startup. Metadata versi di bundle VST3 tidak selalu ada dan
//! tidak selalu dapat dipercaya; catatan sendiri membuat rollback, uninstall
//! bersih, dan deteksi drift jadi mungkin.

pub mod reconcile;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths::InstallScope;
use crate::Result;

pub const DB_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledDb {
    #[serde(alias = "schema_version")]
    pub schema_version: u32,
    #[serde(alias = "updated_at")]
    pub updated_at: String,
    pub entries: Vec<InstalledEntry>,
}

impl Default for InstalledDb {
    fn default() -> Self {
        InstalledDb {
            schema_version: DB_SCHEMA_VERSION,
            updated_at: now_rfc3339(),
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Ok,
    /// File yang tercatat tidak ada lagi (FR-2.2).
    Missing,
    /// Versi tidak dapat ditentukan (FR-2.5).
    UnknownVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRef {
    pub version: String,
    pub path: PathBuf,
    #[serde(alias = "created_at")]
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledEntry {
    #[serde(alias = "plugin_id")]
    pub plugin_id: String,
    pub version: String,
    #[serde(alias = "installed_at")]
    pub installed_at: String,
    pub scope: InstallScope,
    /// Direktori bundle, mis. `…\VST3\MyComp.vst3`.
    #[serde(alias = "install_dir")]
    pub install_dir: PathBuf,
    #[serde(default, alias = "artifact_sha256")]
    pub artifact_sha256: Option<String>,
    /// Setiap file yang dibuat launcher, relatif terhadap parent `install_dir`.
    ///
    /// Ini yang memungkinkan uninstall menghapus tepat apa yang dipasang alih-
    /// alih `remove_dir_all` pada folder yang mungkin berisi file pengguna
    /// (FR-5.1).
    #[serde(default, alias = "installed_files")]
    pub installed_files: Vec<String>,
    #[serde(default)]
    pub backup: Option<BackupRef>,
    #[serde(default, alias = "skipped_versions")]
    pub skipped_versions: Vec<String>,
    /// True jika entri ditemukan oleh pemindaian, bukan dipasang launcher
    /// (FR-2.3). Daftar filenya tidak lengkap, jadi uninstall harus konservatif.
    #[serde(default)]
    pub adopted: bool,
    #[serde(default = "default_health")]
    pub health: Health,
    /// Versi tertinggi yang pernah terpasang. Dipakai menolak downgrade yang
    /// disajikan sebagai "update" (PRD T8).
    #[serde(default, alias = "highest_version_seen")]
    pub highest_version_seen: Option<String>,
}

fn default_health() -> Health {
    Health::Ok
}

impl InstalledDb {
    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<InstalledDb>(&bytes) {
                Ok(db) if db.schema_version == DB_SCHEMA_VERSION => db,
                Ok(db) => {
                    tracing::warn!(
                        found = db.schema_version,
                        "schema installed.json tidak dikenal, memulai dari kosong"
                    );
                    InstalledDb::default()
                }
                Err(e) => {
                    // PRD §19.2: DB rusak tidak boleh mencegah aplikasi start.
                    // Rekonsiliasi filesystem (reconcile.rs) akan membangunnya
                    // kembali lewat adopsi.
                    tracing::error!(error = %e, "installed.json rusak, dibangun ulang dari pemindaian");
                    let _ = std::fs::rename(path, path.with_extension("json.corrupt"));
                    InstalledDb::default()
                }
            },
            Err(_) => InstalledDb::default(),
        }
    }

    pub fn save(&mut self, path: &Path) -> Result<()> {
        self.updated_at = now_rfc3339();
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| crate::HubError::internal(format!("serialisasi DB: {e}")))?;
        write_atomic(path, &bytes)
    }

    pub fn get(&self, plugin_id: &str) -> Option<&InstalledEntry> {
        self.entries.iter().find(|e| e.plugin_id == plugin_id)
    }

    pub fn get_mut(&mut self, plugin_id: &str) -> Option<&mut InstalledEntry> {
        self.entries.iter_mut().find(|e| e.plugin_id == plugin_id)
    }

    /// Sisipkan atau ganti entri, sambil mempertahankan riwayat yang tidak
    /// boleh hilang saat update: versi yang di-skip dan versi tertinggi.
    pub fn upsert(&mut self, mut entry: InstalledEntry) {
        if let Some(existing) = self.get(&entry.plugin_id) {
            if entry.skipped_versions.is_empty() {
                entry.skipped_versions = existing.skipped_versions.clone();
            }
            entry.highest_version_seen = highest_of(
                existing.highest_version_seen.as_deref(),
                Some(&entry.version),
            );
        } else {
            entry.highest_version_seen = Some(entry.version.clone());
        }
        self.entries.retain(|e| e.plugin_id != entry.plugin_id);
        self.entries.push(entry);
        self.entries.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
    }

    pub fn remove(&mut self, plugin_id: &str) -> Option<InstalledEntry> {
        let index = self.entries.iter().position(|e| e.plugin_id == plugin_id)?;
        Some(self.entries.remove(index))
    }

    /// FR-4.7: tandai versi sebagai di-skip.
    pub fn skip_version(&mut self, plugin_id: &str, version: &str) {
        if let Some(entry) = self.get_mut(plugin_id) {
            if !entry.skipped_versions.iter().any(|v| v == version) {
                entry.skipped_versions.push(version.to_string());
            }
        }
    }
}

fn highest_of(a: Option<&str>, b: Option<&str>) -> Option<String> {
    match (
        a.and_then(crate::version::parse),
        b.and_then(crate::version::parse),
    ) {
        (Some(x), Some(y)) => Some(if x >= y { x.to_string() } else { y.to_string() }),
        (Some(x), None) => Some(x.to_string()),
        (None, Some(y)) => Some(y.to_string()),
        (None, None) => None,
    }
}

/// Tulis file dengan pola write-temp → fsync → rename (NFR-2.3).
///
/// Rename dalam satu direktori bersifat atomik di NTFS, jadi crash di tengah
/// penulisan tidak pernah meninggalkan file rusak — paling buruk kehilangan
/// perubahan terakhir.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("dat")
    ));

    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }

    // Windows menolak rename ke path yang sudah ada, jadi `fs::rename` di sana
    // memakai MoveFileEx dengan REPLACE_EXISTING — perilaku yang kita mau.
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e.into())
        }
    }
}

pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, version: &str) -> InstalledEntry {
        InstalledEntry {
            plugin_id: id.into(),
            version: version.into(),
            installed_at: now_rfc3339(),
            scope: InstallScope::CurrentUser,
            install_dir: PathBuf::from("C:\\VST3\\MyComp.vst3"),
            artifact_sha256: None,
            installed_files: vec![],
            backup: None,
            skipped_versions: vec![],
            adopted: false,
            health: Health::Ok,
            highest_version_seen: None,
        }
    }

    #[test]
    fn atomic_write_leaves_no_temp_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.json");
        write_atomic(&path, b"{}").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn roundtrip_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.json");

        let mut db = InstalledDb::default();
        db.upsert(entry("mycomp", "1.3.0"));
        db.save(&path).unwrap();

        let loaded = InstalledDb::load(&path);
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.get("mycomp").unwrap().version, "1.3.0");
    }

    #[test]
    fn corrupt_db_does_not_prevent_startup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installed.json");
        std::fs::write(&path, b"{ ini bukan JSON").unwrap();

        let db = InstalledDb::load(&path);
        assert!(db.entries.is_empty());
        // File rusak disimpan untuk diagnosis, bukan dibuang diam-diam.
        assert!(path.with_extension("json.corrupt").exists());
    }

    #[test]
    fn upsert_preserves_skipped_versions_and_high_water_mark() {
        let mut db = InstalledDb::default();
        db.upsert(entry("mycomp", "1.3.0"));
        db.skip_version("mycomp", "1.4.0");

        // Rollback ke 1.2.1: skip list dan versi tertinggi tetap.
        db.upsert(entry("mycomp", "1.2.1"));

        let e = db.get("mycomp").unwrap();
        assert_eq!(e.version, "1.2.1");
        assert_eq!(e.skipped_versions, vec!["1.4.0"]);
        assert_eq!(e.highest_version_seen.as_deref(), Some("1.3.0"));
    }
}
