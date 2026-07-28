//! Validasi entri katalog di sisi client (PRD §10.4, §14.5).
//!
//! CI repo katalog sudah menjalankan validasi yang lebih ketat sebelum deploy.
//! Yang ada di sini adalah lapis kedua: launcher tidak boleh mempercayai
//! katalog, karena katalog datang lewat jaringan dan repo-nya bisa diambil alih.

use super::{InstallKind, Plugin, Release};

/// Panjang maksimum field teks yang dirender. Katalog yang mengirim string
/// 10 MB tidak boleh membuat UI membeku.
const MAX_SHORT_TEXT: usize = 200;
const MAX_LONG_TEXT: usize = 20_000;
/// README boleh lebih panjang daripada deskripsi, tapi tetap berbatas — UI
/// merendernya di satu halaman, dan katalog yang mengirim megabyte teks akan
/// membekukan WebView.
const MAX_README_TEXT: usize = 80_000;

/// Periksa satu entri plugin. `Err(reason)` berarti entri dilewati (FR-1.6).
pub fn check_plugin(plugin: &Plugin) -> Result<(), String> {
    check_id(&plugin.id)?;

    if plugin.name.trim().is_empty() || plugin.name.len() > MAX_SHORT_TEXT {
        return Err("name kosong atau terlalu panjang".into());
    }
    if plugin.vendor.len() > MAX_SHORT_TEXT || plugin.tagline.len() > MAX_SHORT_TEXT {
        return Err("vendor/tagline terlalu panjang".into());
    }
    if plugin.description.len() > MAX_LONG_TEXT {
        return Err("description terlalu panjang".into());
    }
    if plugin.readme.len() > MAX_README_TEXT {
        return Err("readme terlalu panjang".into());
    }

    for url in plugin
        .icon_url
        .iter()
        .chain(plugin.screenshots.iter())
        .chain(plugin.homepage_url.iter())
        .chain(plugin.source_url.iter())
    {
        check_https(url)?;
    }

    check_release(&plugin.latest)?;

    // Versi `latest` harus lebih tinggi daripada semua versi di `history`
    // (aturan CI §10.4 nomor 5, ditegakkan ulang di sini).
    let latest_v = crate::version::parse(&plugin.latest.version)
        .ok_or_else(|| format!("versi latest tidak valid: {}", plugin.latest.version))?;
    for old in &plugin.history {
        check_release(old)?;
        let old_v = crate::version::parse(&old.version)
            .ok_or_else(|| format!("versi history tidak valid: {}", old.version))?;
        if old_v >= latest_v {
            return Err(format!(
                "history {old_v} tidak lebih rendah dari latest {latest_v}"
            ));
        }
    }

    Ok(())
}

/// `id` adalah kunci primer di database pengguna dan tidak boleh pernah
/// berubah setelah dirilis (PRD §10.4 aturan 4). Ia juga dipakai sebagai
/// komponen nama direktori backup, jadi karakter path harus dilarang.
fn check_id(id: &str) -> Result<(), String> {
    if id.len() < 2 || id.len() > 32 {
        return Err("id harus 2–32 karakter".into());
    }
    let mut chars = id.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err("id harus diawali huruf kecil atau angka".into());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err("id hanya boleh [a-z0-9_-]".into());
    }
    Ok(())
}

fn check_release(release: &Release) -> Result<(), String> {
    if crate::version::parse(&release.version).is_none() {
        return Err(format!("versi tidak valid: {}", release.version));
    }
    if release.changelog.len() > MAX_LONG_TEXT {
        return Err("changelog terlalu panjang".into());
    }
    if let Some(min) = &release.min_launcher_version {
        if crate::version::parse(min).is_none() {
            return Err(format!("min_launcher_version tidak valid: {min}"));
        }
    }

    // Sebuah rilis boleh tidak punya build untuk platform ini (mis. rilis
    // khusus macOS). Yang tidak boleh: build yang ada tapi rusak.
    for build in &release.builds {
        check_build(build)?;
    }
    Ok(())
}

fn check_build(build: &super::Build) -> Result<(), String> {
    if !is_hex_sha256(&build.sha256) {
        return Err("sha256 bukan 64 hex lowercase".into());
    }
    if build.size_bytes == 0 {
        return Err("size_bytes nol".into());
    }
    if build.archive_root.trim().is_empty()
        || build.archive_root.contains(['/', '\\', ':'])
        || build.archive_root.contains("..")
    {
        return Err("archive_root tidak boleh berisi komponen path".into());
    }
    match build.install_kind {
        InstallKind::Vst3Bundle => {
            if !build.archive_root.to_ascii_lowercase().ends_with(".vst3") {
                return Err("archive_root vst3_bundle harus berakhiran .vst3".into());
            }
        }
        InstallKind::ClapFile => {
            if !build.archive_root.to_ascii_lowercase().ends_with(".clap") {
                return Err("archive_root clap_file harus berakhiran .clap".into());
            }
        }
    }

    // `url` boleh `None` — itu bentuk plugin berbayar di v2 (PRD §20.3).
    // Kalau ada, ia harus HTTPS dan host-nya ada di allowlist.
    if let Some(url) = &build.url {
        check_https(url)?;
    }
    Ok(())
}

fn check_https(raw: &str) -> Result<(), String> {
    let url = url::Url::parse(raw).map_err(|e| format!("URL tidak valid: {e}"))?;
    if url.scheme() != "https" {
        return Err(format!("URL bukan https: {raw}"));
    }
    let host = url.host_str().ok_or_else(|| "URL tanpa host".to_string())?;
    if !crate::host_is_allowed(host) {
        return Err(format!("host di luar allowlist: {host}"));
    }
    Ok(())
}

fn is_hex_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_ids_that_could_escape_a_directory() {
        assert!(check_id("../evil").is_err());
        assert!(check_id("MyComp").is_err()); // huruf besar
        assert!(check_id("a").is_err());
        assert!(check_id("mycomp").is_ok());
        assert!(check_id("my-comp_2").is_ok());
    }

    #[test]
    fn sha256_must_be_lowercase_hex() {
        assert!(is_hex_sha256(&"a".repeat(64)));
        assert!(!is_hex_sha256(&"A".repeat(64)));
        assert!(!is_hex_sha256(&"a".repeat(63)));
        assert!(!is_hex_sha256(&"g".repeat(64)));
    }

    #[test]
    fn archive_root_cannot_contain_path_separators() {
        let build = super::super::Build {
            target: "windows-x86_64".into(),
            format: "vst3".into(),
            url: None,
            size_bytes: 1,
            sha256: "a".repeat(64),
            archive_root: "../MyComp.vst3".into(),
            install_kind: InstallKind::Vst3Bundle,
            requires_vc_redist: false,
        };
        assert!(check_build(&build).is_err());
    }
}
