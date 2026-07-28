//! Cache ikon plugin (PRD §14.5).
//!
//! WebView **tidak pernah** memuat gambar langsung dari internet. Ikon diunduh
//! backend, divalidasi, disimpan di cache, lalu disajikan lewat protokol
//! `asset:` dengan scope terbatas.
//!
//! Alasannya bukan teoretis: `icon_url` datang dari katalog, dan katalog datang
//! lewat jaringan. Kalau WebView memuatnya langsung, katalog yang dimanipulasi
//! dapat memicu request ke host arbitrer dari dalam konteks aplikasi — persis
//! ancaman T7.

use std::path::{Path, PathBuf};

use crate::{HubError, Result};

/// Batas ukuran berkas ikon. Ikon nyata berukuran puluhan KB.
const MAX_ICON_BYTES: u64 = 2 * 1024 * 1024;

/// Format gambar yang diterima, dikenali dari magic bytes — bukan dari
/// ekstensi di URL, yang sepenuhnya dikendalikan penerbit katalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Webp,
    Gif,
}

impl ImageKind {
    pub fn extension(self) -> &'static str {
        match self {
            ImageKind::Png => "png",
            ImageKind::Jpeg => "jpg",
            ImageKind::Webp => "webp",
            ImageKind::Gif => "gif",
        }
    }

    /// Kenali format dari beberapa byte pertama.
    pub fn detect(bytes: &[u8]) -> Option<ImageKind> {
        if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
            return Some(ImageKind::Png);
        }
        if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            return Some(ImageKind::Jpeg);
        }
        if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
            return Some(ImageKind::Webp);
        }
        if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            return Some(ImageKind::Gif);
        }
        None
    }
}

/// Ambil ikon untuk sebuah plugin, dari cache kalau sudah ada.
///
/// Mengembalikan `None` — bukan error — kalau plugin tidak punya `icon_url`
/// atau unduhannya gagal. Ikon adalah hiasan; ketiadaannya tidak boleh membuat
/// daftar plugin gagal dirender.
pub async fn ensure_cached(
    icons_dir: &Path,
    plugin_id: &str,
    icon_url: Option<&str>,
) -> Option<PathBuf> {
    let url = icon_url?;
    let stem = cache_stem(plugin_id, url);

    if let Some(existing) = find_cached(icons_dir, &stem) {
        return Some(existing);
    }

    match download(icons_dir, plugin_id, &stem, url).await {
        Ok(path) => {
            // Berkas lain milik plugin ini adalah versi lama dari URL yang
            // sudah berganti. Dibiarkan, ia hanya menumpuk di cache.
            purge_other_revisions(icons_dir, plugin_id, &stem);
            Some(path)
        }
        Err(e) => {
            tracing::warn!(plugin = plugin_id, error = %e, "gagal mengambil ikon");
            None
        }
    }
}

/// Ambil gambar apa pun dari katalog (screenshot di README) ke cache.
///
/// Berbeda dari [`ensure_cached`], berkasnya dikunci **hanya** oleh URL: gambar
/// ini tidak dimiliki satu plugin tertentu, dan README yang menyebut gambar
/// yang sama dua kali tidak perlu mengunduhnya dua kali.
pub async fn ensure_url_cached(icons_dir: &Path, url: &str) -> Option<PathBuf> {
    let stem = url_stem(url);
    if let Some(existing) = find_cached(icons_dir, &stem) {
        return Some(existing);
    }
    match fetch_image(icons_dir, &stem, url).await {
        Ok(path) => Some(path),
        Err(e) => {
            tracing::warn!(url, error = %e, "gagal mengambil gambar");
            None
        }
    }
}

fn url_stem(url: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest: [u8; 32] = Sha256::digest(url.as_bytes()).into();
    let short: String = crate::verify::to_hex(&digest).chars().take(16).collect();
    format!("img-{short}")
}

/// Nama berkas cache: `<plugin_id>-<8 hex dari hash URL>`.
///
/// URL ikut menentukan nama karena logo **berubah** — vendor memperbarui
/// identitas visualnya. Cache yang hanya dikunci `plugin_id` akan menyajikan
/// logo lama selamanya, dan tidak ada cara membedakannya dari yang benar.
fn cache_stem(plugin_id: &str, url: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest: [u8; 32] = Sha256::digest(url.as_bytes()).into();
    let short: String = crate::verify::to_hex(&digest).chars().take(8).collect();
    format!("{plugin_id}-{short}")
}

fn find_cached(icons_dir: &Path, stem: &str) -> Option<PathBuf> {
    for kind in [
        ImageKind::Png,
        ImageKind::Jpeg,
        ImageKind::Webp,
        ImageKind::Gif,
    ] {
        let path = icons_dir.join(format!("{stem}.{}", kind.extension()));
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn purge_other_revisions(icons_dir: &Path, plugin_id: &str, keep_stem: &str) {
    let Ok(entries) = std::fs::read_dir(icons_dir) else {
        return;
    };
    let revision_prefix = format!("{plugin_id}-");
    // Penamaan lama `<plugin_id>.<ext>`, dari sebelum nama cache menyertakan
    // hash URL. Tanpa ini ia tertinggal selamanya di mesin yang pernah
    // menjalankan versi itu.
    let legacy_prefix = format!("{plugin_id}.");

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_old_revision = name.starts_with(&revision_prefix) && !name.starts_with(keep_stem);
        let is_legacy = name.starts_with(&legacy_prefix);
        if is_old_revision || is_legacy {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

async fn download(
    icons_dir: &Path,
    plugin_id: &str,
    stem: &str,
    url: &str,
) -> Result<PathBuf> {
    // `plugin_id` sudah divalidasi `[a-z0-9_-]` oleh catalog::validate, jadi ia
    // tidak dapat keluar dari direktori cache. Diperiksa ulang di sini karena
    // fungsi ini juga dapat dipanggil dari jalur lain.
    if !plugin_id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(HubError::internal("plugin_id tidak aman untuk nama berkas"));
    }
    fetch_image(icons_dir, stem, url).await
}

/// Unduh, validasi, dan simpan satu gambar.
///
/// `stem` sepenuhnya diturunkan dari hash, jadi nama berkasnya tidak pernah
/// berasal dari data katalog secara langsung.
async fn fetch_image(icons_dir: &Path, stem: &str, url: &str) -> Result<PathBuf> {
    let parsed = url::Url::parse(url).map_err(|e| HubError::CatalogInvalid {
        detail: format!("URL ikon tidak valid: {e}"),
    })?;
    crate::download::validate_download_url(&parsed)?;

    let client = reqwest::Client::builder()
        .user_agent(concat!("StudioHub/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(20))
        .https_only(true)
        .build()
        .map_err(|e| HubError::internal(format!("HTTP client: {e}")))?;

    let response = client.get(parsed).send().await?;
    if !response.status().is_success() {
        return Err(HubError::NetworkUnreachable {
            retryable: response.status().is_server_error(),
            detail: format!("HTTP {}", response.status()),
        });
    }
    if let Some(len) = response.content_length() {
        if len > MAX_ICON_BYTES {
            return Err(HubError::ArchiveRejected {
                reason: format!("ikon terlalu besar: {len} byte"),
            });
        }
    }

    let bytes = response.bytes().await?;
    if bytes.len() as u64 > MAX_ICON_BYTES {
        return Err(HubError::ArchiveRejected {
            reason: "ikon melebihi batas ukuran".into(),
        });
    }

    let kind = ImageKind::detect(&bytes).ok_or_else(|| HubError::ArchiveRejected {
        reason: "berkas ikon bukan gambar yang dikenali".into(),
    })?;

    std::fs::create_dir_all(icons_dir)?;
    let path = icons_dir.join(format!("{stem}.{}", kind.extension()));
    crate::registry::write_atomic(&path, &bytes)?;
    tracing::debug!(stem, ?kind, "gambar di-cache");
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_formats_from_magic_bytes_not_extension() {
        let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0];
        assert_eq!(ImageKind::detect(&png), Some(ImageKind::Png));
        assert_eq!(ImageKind::detect(&[0xff, 0xd8, 0xff, 0xe0]), Some(ImageKind::Jpeg));
        assert_eq!(ImageKind::detect(b"GIF89a...."), Some(ImageKind::Gif));

        let mut webp = b"RIFF____WEBPVP8 ".to_vec();
        webp.truncate(16);
        assert_eq!(ImageKind::detect(&webp), Some(ImageKind::Webp));
    }

    #[test]
    fn rejects_content_that_is_not_an_image() {
        // Berkas yang dinamai .png tapi isinya HTML — kasus umum ketika URL
        // sebenarnya mengembalikan halaman error.
        assert_eq!(ImageKind::detect(b"<!DOCTYPE html><html>"), None);
        assert_eq!(ImageKind::detect(b"MZ\x90\x00"), None); // executable
        assert_eq!(ImageKind::detect(b""), None);
    }

    #[tokio::test]
    async fn missing_icon_url_yields_none_not_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(ensure_cached(dir.path(), "mycomp", None).await.is_none());
    }

    #[tokio::test]
    async fn cached_file_is_reused_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let url = "https://raw.githubusercontent.com/x/y/HEAD/logo.png";
        let stem = cache_stem("mycomp", url);
        std::fs::write(dir.path().join(format!("{stem}.png")), b"x").unwrap();

        let path = ensure_cached(dir.path(), "mycomp", Some(url))
            .await
            .expect("cache harus dipakai");
        assert!(path.ends_with(format!("{stem}.png")));
    }

    #[tokio::test]
    async fn changed_url_does_not_serve_the_old_icon() {
        // Kasus nyata: logo diperbarui di repo, URL-nya berubah. Cache yang
        // hanya dikunci `plugin_id` akan menyajikan logo lama selamanya.
        let dir = tempfile::tempdir().unwrap();
        let old_url = "https://raw.githubusercontent.com/x/y/v1.0.0/logo.png";
        let new_url = "https://raw.githubusercontent.com/x/y/HEAD/logo.png";

        let old_stem = cache_stem("mycomp", old_url);
        std::fs::write(dir.path().join(format!("{old_stem}.png")), b"logo lama").unwrap();

        // URL baru tidak menemukan cache, jadi ia mencoba mengunduh. Host-nya
        // ada di allowlist tapi berkasnya tidak ada, jadi hasilnya `None` —
        // yang penting: ia TIDAK mengembalikan berkas lama.
        assert_ne!(cache_stem("mycomp", new_url), old_stem);
        assert!(find_cached(dir.path(), &cache_stem("mycomp", new_url)).is_none());
    }

    #[test]
    fn cache_stem_is_stable_and_url_dependent() {
        let a = cache_stem("mycomp", "https://example.com/a.png");
        assert_eq!(a, cache_stem("mycomp", "https://example.com/a.png"));
        assert_ne!(a, cache_stem("mycomp", "https://example.com/b.png"));
        assert_ne!(a, cache_stem("myverb", "https://example.com/a.png"));
        assert!(a.starts_with("mycomp-"));
    }
}
