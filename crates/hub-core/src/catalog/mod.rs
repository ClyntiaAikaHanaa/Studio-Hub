//! Model katalog dan deserialisasinya (PRD §10.2).

pub mod fetch;
pub mod validate;

use serde::{Deserialize, Serialize};

/// Versi format katalog yang dimengerti launcher ini.
///
/// Katalog dengan `schema_version` lain ditolak dengan pesan "update launcher",
/// bukan diparsing sebagian — salah parse lebih berbahaya daripada gagal parse.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Target build untuk platform tempat launcher ini dibangun.
pub const CURRENT_TARGET: &str = if cfg!(all(windows, target_arch = "x86_64")) {
    "windows-x86_64"
} else if cfg!(all(target_os = "macos")) {
    "macos-universal"
} else {
    "linux-x86_64"
};

fn default_ttl() -> u64 {
    21_600 // 6 jam (FR-1.4)
}

/// Bentuk mentah katalog. `plugins` sengaja `Value` agar satu entri rusak tidak
/// menggagalkan seluruh katalog (FR-1.6).
#[derive(Debug, Deserialize)]
struct RawCatalog {
    schema_version: u32,
    #[serde(default)]
    generated_at: String,
    #[serde(default = "default_ttl")]
    catalog_ttl_seconds: u64,
    #[serde(default)]
    launcher: LauncherInfo,
    #[serde(default)]
    daw_processes: Vec<DawProcess>,
    #[serde(default)]
    categories: Vec<Category>,
    #[serde(default)]
    licenses: std::collections::HashMap<String, String>,
    #[serde(default)]
    plugins: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub schema_version: u32,
    pub generated_at: String,
    pub catalog_ttl_seconds: u64,
    pub launcher: LauncherInfo,
    pub daw_processes: Vec<DawProcess>,
    pub categories: Vec<Category>,
    /// Peta SPDX id → teks lisensi.
    ///
    /// Disimpan sekali di tingkat katalog, bukan disalin ke setiap plugin:
    /// GPL-3.0 saja 35 KB, dan katalog ini diunduh ulang setiap TTL habis.
    #[serde(default)]
    pub licenses: std::collections::HashMap<String, String>,
    pub plugins: Vec<Plugin>,
    /// Entri yang dilewati karena tidak valid. Ditampilkan di diagnostik, tidak
    /// di UI utama — pengguna tidak dapat berbuat apa-apa tentangnya.
    #[serde(default)]
    pub skipped: Vec<SkippedEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedEntry {
    pub index: usize,
    pub plugin_id: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherInfo {
    #[serde(default, alias = "latest_version")]
    pub latest_version: Option<String>,
    #[serde(default, alias = "manifest_url")]
    pub manifest_url: Option<String>,
    #[serde(default, alias = "minimum_supported_version")]
    pub minimum_supported_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DawProcess {
    pub name: String,
    /// Nama executable, dicocokkan case-insensitive.
    pub executables: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub category: String,
    /// Teks bawaan, dalam bahasa Inggris.
    ///
    /// Terjemahan hidup di [`Self::tagline_i18n`]. Bahasa Inggris dipakai
    /// sebagai basis karena ia yang berasal dari repo plugin — dan karena
    /// bahasa yang tidak punya terjemahan harus jatuh ke sesuatu, bukan kosong.
    pub tagline: String,
    /// Terjemahan tagline per kode bahasa, mis. `{"id": "..."}`.
    #[serde(default, alias = "tagline_i18n")]
    pub tagline_i18n: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub description: String,
    #[serde(default, alias = "description_i18n")]
    pub description_i18n: std::collections::HashMap<String, String>,
    /// README repo, dibakukan ke katalog saat ingest.
    ///
    /// Disimpan di katalog, bukan diambil saat aplikasi jalan: halaman detail
    /// tetap terbaca offline, dan tidak ada request jaringan baru yang dipicu
    /// hanya karena pengguna membuka sebuah plugin.
    #[serde(default)]
    pub readme: String,
    #[serde(default, alias = "icon_url")]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub screenshots: Vec<String>,
    #[serde(default, alias = "homepage_url")]
    pub homepage_url: Option<String>,
    #[serde(default, alias = "source_url")]
    pub source_url: Option<String>,
    /// SPDX id, mis. `GPL-3.0`. Teksnya ada di [`Catalog::licenses`].
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(default, alias = "deprecation_notice")]
    pub deprecation_notice: Option<String>,
    #[serde(default)]
    pub commercial: Commercial,
    pub latest: Release,
    #[serde(default)]
    pub history: Vec<Release>,
    #[serde(default, alias = "user_data")]
    pub user_data: UserData,
    #[serde(default)]
    pub requirements: Requirements,
}

impl Plugin {
    /// Tagline dalam bahasa yang diminta, jatuh ke bahasa Inggris kalau
    /// terjemahannya belum ada.
    pub fn tagline_for(&self, locale: &str) -> &str {
        pick(&self.tagline, &self.tagline_i18n, locale)
    }

    /// Deskripsi dalam bahasa yang diminta.
    pub fn description_for(&self, locale: &str) -> &str {
        pick(&self.description, &self.description_i18n, locale)
    }

    /// Cari rilis dengan versi tertentu, atau `latest` jika `version` `None`.
    pub fn release(&self, version: Option<&str>) -> Option<&Release> {
        match version {
            None => Some(&self.latest),
            Some(v) => {
                let want = crate::version::parse(v)?;
                std::iter::once(&self.latest)
                    .chain(self.history.iter())
                    .find(|r| crate::version::parse(&r.version) == Some(want.clone()))
            }
        }
    }
}

/// Placeholder model bisnis (PRD §20.1). Semua `free` di v1; field ini ada
/// sekarang agar menambah plugin berbayar nanti tidak menaikkan `schema_version`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Commercial {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default, alias = "requires_license")]
    pub requires_license: bool,
    #[serde(default, alias = "purchase_url")]
    pub purchase_url: Option<String>,
}

/// Pilih varian bahasa, jatuh ke teks bawaan kalau tidak ada.
///
/// Terjemahan kosong diperlakukan sebagai tidak ada: entri katalog yang
/// terlanjur berisi string kosong tidak boleh menghasilkan kartu tanpa
/// keterangan sama sekali.
fn pick<'a>(
    fallback: &'a str,
    translations: &'a std::collections::HashMap<String, String>,
    locale: &str,
) -> &'a str {
    translations
        .get(locale)
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(fallback)
}

fn default_model() -> String {
    "free".to_string()
}

impl Default for Commercial {
    fn default() -> Self {
        Commercial {
            model: default_model(),
            requires_license: false,
            purchase_url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    #[serde(default, alias = "released_at")]
    pub released_at: Option<String>,
    #[serde(default)]
    pub breaking: bool,
    #[serde(default)]
    pub security: bool,
    #[serde(default, alias = "min_launcher_version")]
    pub min_launcher_version: Option<String>,
    #[serde(default)]
    pub changelog: String,
    #[serde(default)]
    pub builds: Vec<Build>,
}

impl Release {
    /// Build untuk platform saat ini, jika ada.
    pub fn build_for_current_target(&self) -> Option<&Build> {
        self.builds.iter().find(|b| b.target == CURRENT_TARGET)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Build {
    pub target: String,
    pub format: String,
    /// PRD §20.3: perlakukan sebagai nilai yang di-*resolve*, bukan konstanta.
    /// Untuk plugin berbayar nanti, ini `null` di katalog publik dan diminta
    /// dari endpoint terautentikasi saat install.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(alias = "size_bytes")]
    pub size_bytes: u64,
    pub sha256: String,
    #[serde(alias = "archive_root")]
    pub archive_root: String,
    #[serde(alias = "install_kind")]
    pub install_kind: InstallKind,
    #[serde(default, alias = "requires_vc_redist")]
    pub requires_vc_redist: bool,
}

/// Strategi instalasi. Dispatch eksplisit, bukan menebak dari ekstensi
/// (PRD §10.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallKind {
    Vst3Bundle,
    ClapFile,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserData {
    #[serde(default, alias = "preset_paths")]
    pub preset_paths: Vec<String>,
    #[serde(default, alias = "config_paths")]
    pub config_paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Requirements {
    #[serde(default, alias = "os_min_build")]
    pub os_min_build: Option<u32>,
    #[serde(default, alias = "cpu_features")]
    pub cpu_features: Vec<String>,
    #[serde(default, alias = "disk_bytes")]
    pub disk_bytes: Option<u64>,
}

impl Catalog {
    /// Parse katalog dari JSON mentah.
    ///
    /// Entri plugin yang tidak valid dilewati dan dicatat, bukan menggagalkan
    /// seluruh katalog (FR-1.6). Sembilan plugin valid tetap terpakai meskipun
    /// entri kesepuluh rusak.
    pub fn parse(bytes: &[u8]) -> crate::Result<Catalog> {
        let raw: RawCatalog =
            serde_json::from_slice(bytes).map_err(|e| crate::HubError::CatalogInvalid {
                detail: format!("JSON tidak dapat diparsing: {e}"),
            })?;

        if raw.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(crate::HubError::LauncherTooOld {
                required: format!("schema_version {}", raw.schema_version),
                current: crate::LAUNCHER_VERSION.to_string(),
            });
        }

        let mut plugins = Vec::with_capacity(raw.plugins.len());
        let mut skipped = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        for (index, value) in raw.plugins.into_iter().enumerate() {
            let probe_id = value
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::to_string);

            match serde_json::from_value::<Plugin>(value) {
                Ok(plugin) => match validate::check_plugin(&plugin) {
                    Ok(()) if !seen_ids.insert(plugin.id.clone()) => {
                        skipped.push(SkippedEntry {
                            index,
                            plugin_id: Some(plugin.id),
                            reason: "id duplikat".to_string(),
                        });
                    }
                    Ok(()) => plugins.push(plugin),
                    Err(reason) => skipped.push(SkippedEntry {
                        index,
                        plugin_id: Some(plugin.id),
                        reason,
                    }),
                },
                Err(e) => skipped.push(SkippedEntry {
                    index,
                    plugin_id: probe_id,
                    reason: e.to_string(),
                }),
            }
        }

        for entry in &skipped {
            tracing::warn!(
                index = entry.index,
                plugin_id = ?entry.plugin_id,
                reason = %entry.reason,
                "entri katalog dilewati"
            );
        }

        Ok(Catalog {
            schema_version: raw.schema_version,
            generated_at: raw.generated_at,
            catalog_ttl_seconds: raw.catalog_ttl_seconds.clamp(300, 604_800),
            launcher: raw.launcher,
            daw_processes: raw.daw_processes,
            categories: raw.categories,
            licenses: raw.licenses,
            plugins,
            skipped,
        })
    }

    /// Plugin yang tampil di UI: menyembunyikan entri `hidden` (FR-1.5).
    pub fn visible_plugins(&self) -> impl Iterator<Item = &Plugin> {
        self.plugins.iter().filter(|p| !p.hidden)
    }

    pub fn plugin(&self, id: &str) -> Option<&Plugin> {
        self.plugins.iter().find(|p| p.id == id)
    }

    /// Teks lisensi untuk sebuah plugin, kalau katalog memuatnya.
    pub fn license_text(&self, plugin: &Plugin) -> &str {
        plugin
            .license
            .as_deref()
            .and_then(|id| self.licenses.get(id))
            .map(String::as_str)
            .unwrap_or("")
    }

    pub fn category_label(&self, id: &str) -> Option<&str> {
        self.categories
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.label.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_json(id: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "name": "MyComp",
            "vendor": "Studio Robi",
            "category": "dynamics",
            "tagline": "Kompresor VCA effect",
            "latest": {
                "version": "1.3.0",
                "builds": [{
                    "target": "windows-x86_64",
                    "format": "vst3",
                    "url": "https://github.com/robi/MyComp/releases/download/v1.3.0/x.zip",
                    "size_bytes": 100,
                    "sha256": "9f2b1c0a7d3e5f8a4b6c9d0e1f2a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c",
                    "archive_root": "MyComp.vst3",
                    "install_kind": "vst3_bundle"
                }]
            }
        })
    }

    fn catalog_with(plugins: Vec<serde_json::Value>) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "generated_at": "2026-07-27T09:14:22Z",
            "plugins": plugins,
        }))
        .unwrap()
    }

    #[test]
    fn locale_variant_is_used_when_present() {
        let mut p = plugin_json("mycomp");
        p["tagline_i18n"] = serde_json::json!({ "id": "Kompresor VCA" });
        let catalog = Catalog::parse(&catalog_with(vec![p])).unwrap();
        let plugin = catalog.plugin("mycomp").unwrap();

        assert_eq!(plugin.tagline_for("id"), "Kompresor VCA");
        // Bahasa tanpa terjemahan jatuh ke teks bawaan, bukan kosong.
        assert_eq!(plugin.tagline_for("en"), "Kompresor VCA effect");
        assert_eq!(plugin.tagline_for("de"), "Kompresor VCA effect");
    }

    #[test]
    fn empty_translation_falls_back_instead_of_blanking_the_card() {
        let mut p = plugin_json("mycomp");
        p["tagline_i18n"] = serde_json::json!({ "id": "   " });
        let catalog = Catalog::parse(&catalog_with(vec![p])).unwrap();
        assert_eq!(
            catalog.plugin("mycomp").unwrap().tagline_for("id"),
            "Kompresor VCA effect"
        );
    }

    #[test]
    fn one_broken_entry_does_not_sink_the_catalog() {
        let bytes = catalog_with(vec![
            plugin_json("mycomp"),
            serde_json::json!({ "id": "broken" }), // tidak punya `latest`
            plugin_json("myverb"),
        ]);
        let catalog = Catalog::parse(&bytes).unwrap();
        assert_eq!(catalog.plugins.len(), 2);
        assert_eq!(catalog.skipped.len(), 1);
        assert_eq!(catalog.skipped[0].plugin_id.as_deref(), Some("broken"));
    }

    #[test]
    fn unknown_schema_version_is_rejected_not_guessed() {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schema_version": 2,
            "plugins": []
        }))
        .unwrap();
        assert!(matches!(
            Catalog::parse(&bytes),
            Err(crate::HubError::LauncherTooOld { .. })
        ));
    }

    #[test]
    fn duplicate_ids_are_dropped() {
        let bytes = catalog_with(vec![plugin_json("mycomp"), plugin_json("mycomp")]);
        let catalog = Catalog::parse(&bytes).unwrap();
        assert_eq!(catalog.plugins.len(), 1);
        assert_eq!(catalog.skipped.len(), 1);
    }

    #[test]
    fn download_url_outside_allowlist_is_rejected() {
        let mut p = plugin_json("evil");
        p["latest"]["builds"][0]["url"] = serde_json::json!("https://evil.tld/x.zip");
        let catalog = Catalog::parse(&catalog_with(vec![p])).unwrap();
        assert!(catalog.plugins.is_empty());
        assert_eq!(catalog.skipped.len(), 1);
    }

    #[test]
    fn plain_http_url_is_rejected() {
        let mut p = plugin_json("insecure");
        p["latest"]["builds"][0]["url"] = serde_json::json!("http://github.com/x.zip");
        let catalog = Catalog::parse(&catalog_with(vec![p])).unwrap();
        assert!(catalog.plugins.is_empty());
    }
}
