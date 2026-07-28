//! Validasi path yang dijalankan **helper**, bukan client (PRD §13.7 langkah 5).
//!
//! Ini bagian paling penting dari mitigasi T5. Client berjalan Medium
//! integrity; jika ia di-compromise, ia dapat mengirim command apa pun ke pipe.
//! Satu-satunya hal yang berdiri di antara itu dan penulisan arbitrer sebagai
//! Administrator adalah fungsi-fungsi di file ini.
//!
//! Karena itu: validasi di sini tidak boleh mempercayai apa pun dari client,
//! termasuk klaim bahwa sebuah path "sudah divalidasi".

use std::path::{Component, Path, PathBuf};

/// Direktori yang boleh disentuh helper.
#[derive(Debug, Clone)]
pub struct AllowedRoots {
    roots: Vec<PathBuf>,
}

impl AllowedRoots {
    /// Roots default: direktori VST3 sistem, direktori VST3 per-user, dan
    /// direktori staging milik launcher. Tidak ada yang lain.
    pub fn system_default() -> Self {
        let mut roots = Vec::new();
        for scope in [
            hub_core::paths::InstallScope::AllUsers,
            hub_core::paths::InstallScope::CurrentUser,
        ] {
            if let Ok(dir) = scope.vst3_dir() {
                roots.push(dir);
            }
        }
        if let Ok(paths) = hub_core::paths::AppPaths::resolve() {
            roots.push(paths.staging_dir);
            roots.push(paths.backup_dir);
        }
        AllowedRoots { roots }
    }

    pub fn with_roots(roots: Vec<PathBuf>) -> Self {
        AllowedRoots { roots }
    }

    /// True jika `path` berada di bawah salah satu root yang diizinkan.
    ///
    /// Path tujuan sering belum ada, jadi `canonicalize` diterapkan pada
    /// **parent terdekat yang ada** — meng-canonicalize path yang belum ada
    /// selalu gagal, dan gagal berarti kita akan menolak operasi yang sah.
    pub fn permits(&self, path: &Path) -> bool {
        if has_traversal(path) || !path.is_absolute() {
            return false;
        }
        let Some(resolved) = resolve_existing_ancestor(path) else {
            return false;
        };
        self.roots.iter().any(|root| {
            let canonical_root = root.canonicalize().unwrap_or_else(|_| root.clone());
            resolved.starts_with(&canonical_root) || resolved.starts_with(root)
        })
    }
}

/// Tolak `..` dan komponen aneh sebelum menyentuh filesystem sama sekali.
pub fn has_traversal(path: &Path) -> bool {
    path.components().any(|c| matches!(c, Component::ParentDir))
        || path.to_string_lossy().contains("..")
}

/// Canonicalize parent terdekat yang ada, lalu tempelkan kembali sisa
/// komponennya. Hasilnya path absolut tanpa symlink di bagian yang nyata.
fn resolve_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut existing = path.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();

    loop {
        if existing.exists() {
            let canonical = existing.canonicalize().ok()?;
            let mut out = canonical;
            for part in tail.iter().rev() {
                out.push(part);
            }
            return Some(out);
        }
        let name = existing.file_name()?.to_os_string();
        tail.push(name);
        existing = existing.parent()?.to_path_buf();
    }
}

/// Kedua path harus berada di volume yang sama agar rename bersifat atomik.
pub fn same_volume(a: &Path, b: &Path) -> bool {
    hub_core::paths::same_volume(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(dir: &Path) -> AllowedRoots {
        AllowedRoots::with_roots(vec![dir.to_path_buf()])
    }

    #[test]
    fn paths_inside_allowed_root_pass() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        std::fs::create_dir_all(dir.path().join("VST3")).unwrap();
        assert!(roots.permits(&dir.path().join("VST3").join("MyComp.vst3")));
    }

    #[test]
    fn traversal_is_rejected_before_touching_the_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        assert!(!roots.permits(&dir.path().join("..").join("elsewhere")));
        assert!(has_traversal(Path::new("C:\\VST3\\..\\Windows\\System32")));
    }

    #[test]
    fn paths_outside_every_root_are_rejected() {
        let allowed = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let roots = roots(allowed.path());
        assert!(!roots.permits(&other.path().join("evil.dll")));
    }

    #[test]
    fn relative_paths_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!roots(dir.path()).permits(Path::new("MyComp.vst3")));
    }

    #[test]
    fn nonexistent_target_under_allowed_root_is_permitted() {
        // Tujuan instalasi belum ada saat divalidasi — ini kasus normal, bukan
        // kasus tepi.
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        assert!(roots.permits(&dir.path().join("belum").join("ada").join("MyComp.vst3")));
    }
}
