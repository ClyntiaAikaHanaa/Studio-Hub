//! Resolusi lokasi VST3, staging, backup (PRD §11.3).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{HubError, Result};

/// Nama direktori staging yang dibuat di dalam direktori tujuan saat tujuan
/// berada di volume berbeda dari `%LOCALAPPDATA%`. Rename lintas volume bukan
/// operasi atomik, jadi staging harus se-volume dengan tujuan (PRD §11.3).
pub const IN_PLACE_STAGING_DIR: &str = ".studiohub-staging";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallScope {
    /// `%LOCALAPPDATA%\Programs\Common\VST3` — tanpa elevasi. Default (ADR-3).
    #[default]
    CurrentUser,
    /// `%CommonProgramFiles%\VST3` — butuh elevasi.
    AllUsers,
    /// Path pilihan pengguna.
    Custom { path: PathBuf },
}

impl InstallScope {
    pub fn vst3_dir(&self) -> Result<PathBuf> {
        match self {
            InstallScope::CurrentUser => Ok(local_app_data()?
                .join("Programs")
                .join("Common")
                .join("VST3")),
            InstallScope::AllUsers => Ok(common_program_files()?.join("VST3")),
            InstallScope::Custom { path } => Ok(path.clone()),
        }
    }

    /// True jika penulisan ke lokasi ini butuh elevasi.
    ///
    /// Ditentukan dengan **mencoba menulis file probe**, bukan dengan
    /// membandingkan string path (PRD §11.3). Pengguna dapat punya izin yang
    /// tidak terduga: admin yang sudah mengubah ACL `Program Files`, atau
    /// sebaliknya, direktori per-user yang dikunci kebijakan grup.
    pub fn requires_elevation(&self) -> Result<bool> {
        let dir = self.vst3_dir()?;
        Ok(!probe_writable(&dir))
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            InstallScope::CurrentUser => "current_user",
            InstallScope::AllUsers => "all_users",
            InstallScope::Custom { .. } => "custom",
        }
    }
}

/// Uji tulis: buat file bernama acak, tulis, hapus.
///
/// Direktori yang belum ada dibuat dulu — kegagalan membuatnya adalah jawaban
/// yang sama informatifnya (tidak dapat ditulis).
pub fn probe_writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(
        ".studiohub-probe-{}",
        uuid::Uuid::new_v4().simple()
    ));
    match std::fs::write(&probe, b"probe") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Semua direktori milik launcher.
#[derive(Debug, Clone)]
pub struct AppPaths {
    /// `%APPDATA%\StudioHub` — roaming: preferensi, DB terpasang.
    pub data_dir: PathBuf,
    /// `%LOCALAPPDATA%\StudioHub\cache`
    pub cache_dir: PathBuf,
    /// `%LOCALAPPDATA%\StudioHub\staging`
    pub staging_dir: PathBuf,
    /// `%LOCALAPPDATA%\StudioHub\backup`
    pub backup_dir: PathBuf,
    /// `%LOCALAPPDATA%\StudioHub\logs`
    pub logs_dir: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Result<Self> {
        let roaming = app_data()?.join("StudioHub");
        let local = local_app_data()?.join("StudioHub");
        Ok(AppPaths {
            data_dir: roaming,
            cache_dir: local.join("cache"),
            staging_dir: local.join("staging"),
            backup_dir: local.join("backup"),
            logs_dir: local.join("logs"),
        })
    }

    /// Untuk test: seluruh tree di bawah satu root.
    pub fn under(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        AppPaths {
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
            staging_dir: root.join("staging"),
            backup_dir: root.join("backup"),
            logs_dir: root.join("logs"),
        }
    }

    pub fn ensure_all(&self) -> Result<()> {
        for dir in [
            &self.data_dir,
            &self.cache_dir,
            &self.staging_dir,
            &self.backup_dir,
            &self.logs_dir,
        ] {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::create_dir_all(self.downloads_dir())?;
        std::fs::create_dir_all(self.icons_dir())?;
        Ok(())
    }

    pub fn installed_db(&self) -> PathBuf {
        self.data_dir.join("installed.json")
    }

    pub fn prefs(&self) -> PathBuf {
        self.data_dir.join("prefs.json")
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.cache_dir.join("downloads")
    }

    pub fn icons_dir(&self) -> PathBuf {
        self.cache_dir.join("icons")
    }

    /// Buang katalog dan gambar yang di-cache, biarkan unduhan artefak.
    ///
    /// Dipakai tombol Refresh: menghapus cache lebih tegas daripada sekadar
    /// mengabaikan TTL, karena ia juga membuang ikon dan screenshot yang
    /// URL-nya sudah berubah di katalog baru. Unduhan artefak sengaja
    /// dipertahankan — ia diverifikasi hash dan mengunduhnya ulang mahal.
    pub fn clear_catalog_cache(&self) -> Result<()> {
        for file in ["catalog.json", "catalog.meta.json"] {
            let path = self.cache_dir.join(file);
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
        }
        let icons = self.icons_dir();
        if icons.exists() {
            std::fs::remove_dir_all(&icons)?;
        }
        Ok(())
    }

    /// Buang seluruh isi direktori cache, termasuk unduhan artefak.
    ///
    /// Tidak ada yang hilang secara permanen: semuanya dapat diambil ulang, dan
    /// artefak tetap diverifikasi SHA-256 saat diunduh lagi.
    pub fn clear_all_cache(&self) -> Result<()> {
        if self.cache_dir.exists() {
            std::fs::remove_dir_all(&self.cache_dir)?;
        }
        std::fs::create_dir_all(self.downloads_dir())?;
        std::fs::create_dir_all(self.icons_dir())?;
        Ok(())
    }

    pub fn backup_for(&self, plugin_id: &str, version: &str) -> PathBuf {
        // `plugin_id` sudah divalidasi `[a-z0-9_-]` (catalog::validate), jadi
        // tidak dapat keluar dari direktori backup. Versi disanitasi di sini
        // karena ia berasal dari sumber yang sama tapi punya bentuk lebih bebas.
        self.backup_dir
            .join(plugin_id)
            .join(sanitize_component(version))
    }

    /// Direktori staging untuk satu job, se-volume dengan `target_dir` jika
    /// mungkin (PRD §11.3).
    pub fn staging_for(&self, target_dir: &Path, job_id: &str) -> PathBuf {
        if same_volume(&self.staging_dir, target_dir) {
            self.staging_dir.join(job_id)
        } else {
            target_dir.join(IN_PLACE_STAGING_DIR).join(job_id)
        }
    }
}

/// Ganti karakter yang tidak aman di komponen path dengan `_`.
pub fn sanitize_component(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Perbandingan volume yang cukup untuk keputusan "rename ini atomik?".
///
/// Di Windows kita membandingkan prefix path (drive letter atau share UNC).
/// Ini heuristik — mount point direktori bisa menipu — tapi konsekuensi salah
/// tebak hanyalah staging di lokasi yang kurang optimal, dan langkah commit
/// tetap memvalidasi hasilnya.
pub fn same_volume(a: &Path, b: &Path) -> bool {
    fn prefix(p: &Path) -> Option<String> {
        use std::path::Component;
        match p.components().next() {
            Some(Component::Prefix(pre)) => {
                Some(pre.as_os_str().to_string_lossy().to_ascii_lowercase())
            }
            Some(Component::RootDir) => Some("/".to_string()),
            _ => None,
        }
    }
    match (prefix(a), prefix(b)) {
        (Some(x), Some(y)) => x == y,
        // Path relatif: anggap sama, keduanya di bawah cwd.
        _ => true,
    }
}

fn env_dir(var: &str) -> Result<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| HubError::internal(format!("variabel lingkungan {var} tidak tersedia")))
}

#[cfg(windows)]
pub fn local_app_data() -> Result<PathBuf> {
    env_dir("LOCALAPPDATA")
}

#[cfg(windows)]
pub fn app_data() -> Result<PathBuf> {
    env_dir("APPDATA")
}

#[cfg(windows)]
pub fn common_program_files() -> Result<PathBuf> {
    // `%CommonProgramFiles%` di proses 64-bit menunjuk ke
    // `C:\Program Files\Common Files`, yang benar untuk VST3 x64.
    env_dir("CommonProgramFiles")
}

// Fallback non-Windows: hub-core harus dapat dikompilasi dan diuji di CI Linux
// meskipun target rilis v1 hanya Windows.
#[cfg(not(windows))]
pub fn local_app_data() -> Result<PathBuf> {
    directories::BaseDirs::new()
        .map(|d| d.data_local_dir().to_path_buf())
        .ok_or_else(|| HubError::internal("tidak dapat menentukan data_local_dir"))
}

#[cfg(not(windows))]
pub fn app_data() -> Result<PathBuf> {
    directories::BaseDirs::new()
        .map(|d| d.data_dir().to_path_buf())
        .ok_or_else(|| HubError::internal("tidak dapat menentukan data_dir"))
}

#[cfg(not(windows))]
pub fn common_program_files() -> Result<PathBuf> {
    Ok(PathBuf::from("/usr/local/lib"))
}

/// Perluas `%VAR%` di path yang berasal dari katalog (`user_data.preset_paths`).
///
/// Variabel yang tidak dikenal dibiarkan apa adanya alih-alih diganti string
/// kosong — path `\VST3 Presets\...` yang tidak sengaja menjadi absolut jauh
/// lebih berbahaya daripada path yang jelas-jelas salah.
pub fn expand_env_vars(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) => out.push_str(&value),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                out.push_str(after);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_path_characters() {
        assert_eq!(sanitize_component("1.2.3"), "1.2.3");
        assert_eq!(sanitize_component("../../etc"), ".._.._etc");
        assert_eq!(sanitize_component("C:\\x"), "C__x");
    }

    #[test]
    fn clearing_catalog_cache_keeps_downloaded_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(dir.path());
        paths.ensure_all().unwrap();

        std::fs::write(paths.cache_dir.join("catalog.json"), b"{}").unwrap();
        std::fs::write(paths.cache_dir.join("catalog.meta.json"), b"{}").unwrap();
        std::fs::write(paths.icons_dir().join("a.png"), b"x").unwrap();
        let artifact = paths.downloads_dir().join("abc.zip");
        std::fs::write(&artifact, b"zip").unwrap();

        paths.clear_catalog_cache().unwrap();

        assert!(!paths.cache_dir.join("catalog.json").exists());
        assert!(!paths.icons_dir().exists());
        // Unduhan mahal dan sudah terverifikasi hash; membuangnya sia-sia.
        assert!(artifact.exists());
    }

    #[test]
    fn clearing_all_cache_leaves_usable_directories() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(dir.path());
        paths.ensure_all().unwrap();
        std::fs::write(paths.downloads_dir().join("abc.zip"), b"zip").unwrap();

        paths.clear_all_cache().unwrap();

        assert!(!paths.downloads_dir().join("abc.zip").exists());
        // Direktorinya harus tetap ada, kalau tidak operasi berikutnya gagal
        // dengan "path tidak ditemukan" alih-alih bekerja normal.
        assert!(paths.downloads_dir().is_dir());
        assert!(paths.icons_dir().is_dir());
    }

    #[test]
    fn backup_path_cannot_escape_backup_dir() {
        let paths = AppPaths::under("/tmp/hub");
        let p = paths.backup_for("mycomp", "../../../etc/passwd");
        assert!(p.starts_with(paths.backup_dir.join("mycomp")));
    }

    #[test]
    fn unknown_env_var_is_left_intact() {
        std::env::set_var("HUB_TEST_VAR", "VALUE");
        assert_eq!(expand_env_vars("%HUB_TEST_VAR%\\x"), "VALUE\\x");
        assert_eq!(
            expand_env_vars("%DEFINITELY_NOT_SET_12345%\\x"),
            "%DEFINITELY_NOT_SET_12345%\\x"
        );
        assert_eq!(expand_env_vars("plain"), "plain");
        assert_eq!(expand_env_vars("50% off"), "50% off");
    }

    #[test]
    fn same_volume_compares_drive_prefix() {
        assert!(same_volume(
            Path::new("C:\\Users\\x"),
            Path::new("C:\\Program Files")
        ));
    }
}
