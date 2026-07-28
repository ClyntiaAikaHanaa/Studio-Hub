//! Tipe yang menyeberang IPC ke frontend (PRD §12).
//!
//! Semua field pakai `camelCase` agar frontend tidak perlu menerjemahkan nama.
//! Tipe di sini sengaja terpisah dari model `hub-core`: yang dikirim ke WebView
//! adalah apa yang dibutuhkan UI, bukan struktur internal — termasuk tidak
//! mengirim hash yang diharapkan, path staging, atau apa pun yang frontend
//! tidak punya urusan dengannya.

use serde::{Deserialize, Serialize};

use hub_core::catalog::{Catalog, Plugin};
use hub_core::registry::{Health, InstalledDb, InstalledEntry};
use hub_core::version::UpdateState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogView {
    pub generated_at: String,
    pub categories: Vec<CategoryView>,
    pub plugins: Vec<PluginView>,
    pub stale: bool,
    pub last_success_at: Option<String>,
    /// Jumlah entri yang dilewati karena tidak valid (FR-1.6). Ditampilkan di
    /// Diagnostics, tidak di UI utama — pengguna tidak dapat berbuat apa-apa.
    pub skipped_entries: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryView {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginView {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub category: String,
    pub category_label: Option<String>,
    pub tagline: String,
    /// Markdown terbatas. Frontend **wajib** merendernya lewat renderer
    /// allowlist, tidak pernah sebagai HTML mentah (PRD §14.5, T7).
    pub description: String,
    /// README repo, sudah dibakukan ke katalog. Aturan render sama seperti
    /// `description`.
    pub readme: String,
    pub icon_url: Option<String>,
    pub screenshots: Vec<String>,
    pub homepage_url: Option<String>,
    pub source_url: Option<String>,
    pub license: Option<String>,
    /// Teks lisensi lengkap. Dialog instalasi menampilkannya sebelum pengguna
    /// dapat menekan Install.
    pub license_text: String,
    pub deprecated: bool,
    pub deprecation_notice: Option<String>,
    pub latest_version: String,
    pub released_at: Option<String>,
    pub changelog: String,
    pub breaking: bool,
    pub security: bool,
    pub download_size_bytes: u64,
    pub available_for_platform: bool,
    /// Semua versi yang dapat dipasang, terbaru lebih dulu (FR-4.9, §10.3).
    ///
    /// Versi tanpa build untuk platform ini dikecualikan: menawarkan versi yang
    /// pasti gagal dipasang lebih buruk daripada tidak menawarkannya.
    pub available_versions: Vec<VersionOption>,
    pub installed: Option<InstalledView>,
    pub update: UpdateState,
    pub commercial_model: String,
}

/// Satu versi yang dapat dipilih pengguna di halaman detail.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionOption {
    pub version: String,
    pub released_at: Option<String>,
    pub breaking: bool,
    pub security: bool,
    pub changelog: String,
    pub download_size_bytes: u64,
    /// True untuk versi yang tercantum sebagai `latest` di katalog.
    pub is_latest: bool,
    /// True kalau versi inilah yang sedang terpasang.
    pub is_installed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledView {
    pub version: String,
    pub installed_at: String,
    pub scope: String,
    pub install_dir: String,
    pub health: Health,
    pub adopted: bool,
    pub has_backup: bool,
    pub backup_version: Option<String>,
    pub skipped_versions: Vec<String>,
}

/// Entri Library, termasuk plugin yang tidak ada di katalog (mis. diadopsi dari
/// vendor lain). Menghilangkannya akan membuat Library berbohong.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntry {
    pub plugin_id: String,
    pub name: String,
    pub installed: InstalledView,
    pub update: UpdateState,
    pub in_catalog: bool,
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSummary {
    pub items: Vec<UpdateItem>,
    /// Jumlah update non-breaking — angka yang dipakai tombol "Update all".
    pub non_breaking_count: usize,
    pub breaking_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateItem {
    pub plugin_id: String,
    pub name: String,
    pub icon_url: Option<String>,
    pub from_version: String,
    pub to_version: String,
    pub released_at: Option<String>,
    pub breaking: bool,
    pub security: bool,
    /// Changelog dikirim ter-expand: pengguna harus bisa membacanya tanpa usaha
    /// (PRD §8.5).
    pub changelog: String,
    pub download_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherUpdate {
    pub current_version: String,
    pub available_version: String,
    pub notes: String,
    pub security: bool,
    /// True jika katalog menuntut versi lebih baru (FR-7.3): instalasi plugin
    /// diblokir sampai launcher diperbarui.
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsSummary {
    pub launcher_version: String,
    pub os_build: Option<u32>,
    pub arch: String,
    pub vc_redist: bool,
    pub installed_count: usize,
    pub catalog_generated_at: Option<String>,
    pub detected_daws: Vec<String>,
    pub logs_dir: String,
}

// ── Konversi ─────────────────────────────────────────────────────────────

pub fn build_catalog_view(
    catalog: &Catalog,
    db: &InstalledDb,
    stale: bool,
    last_success_at: Option<String>,
    locale: &str,
) -> CatalogView {
    let plugins = catalog
        .visible_plugins()
        .map(|p| build_plugin_view(catalog, p, db.get(&p.id), locale))
        .collect();

    CatalogView {
        generated_at: catalog.generated_at.clone(),
        categories: catalog
            .categories
            .iter()
            .map(|c| CategoryView {
                id: c.id.clone(),
                label: c.label.clone(),
            })
            .collect(),
        plugins,
        stale,
        last_success_at,
        skipped_entries: catalog.skipped.len(),
    }
}

pub fn build_plugin_view(
    catalog: &Catalog,
    plugin: &Plugin,
    installed: Option<&InstalledEntry>,
    locale: &str,
) -> PluginView {
    let build = plugin.latest.build_for_current_target();

    let update = match installed {
        Some(entry) if entry.health == Health::UnknownVersion => UpdateState::Unknown,
        Some(entry) => hub_core::version::compute_update_state(
            Some(&entry.version),
            &plugin.latest.version,
            plugin.latest.breaking,
            &entry.skipped_versions,
        ),
        None => UpdateState::Unknown,
    };

    PluginView {
        id: plugin.id.clone(),
        name: plugin.name.clone(),
        vendor: plugin.vendor.clone(),
        category: plugin.category.clone(),
        category_label: catalog.category_label(&plugin.category).map(str::to_string),
        tagline: plugin.tagline_for(locale).to_string(),
        description: plugin.description_for(locale).to_string(),
        readme: plugin.readme.clone(),
        icon_url: plugin.icon_url.clone(),
        screenshots: plugin.screenshots.clone(),
        homepage_url: plugin.homepage_url.clone(),
        source_url: plugin.source_url.clone(),
        license: plugin.license.clone(),
        license_text: catalog.license_text(plugin).to_string(),
        deprecated: plugin.deprecated,
        deprecation_notice: plugin.deprecation_notice.clone(),
        latest_version: plugin.latest.version.clone(),
        released_at: plugin.latest.released_at.clone(),
        changelog: plugin.latest.changelog.clone(),
        breaking: plugin.latest.breaking,
        security: plugin.latest.security,
        download_size_bytes: build.map(|b| b.size_bytes).unwrap_or(0),
        available_for_platform: build.is_some(),
        available_versions: build_version_options(plugin, installed),
        installed: installed.map(build_installed_view),
        update,
        commercial_model: plugin.commercial.model.clone(),
    }
}

/// Daftar versi yang dapat dipasang, terbaru lebih dulu.
///
/// Rilis tanpa build untuk platform ini dilewati. Menampilkannya berarti
/// menawarkan tombol yang dijamin gagal — dan kegagalannya baru muncul setelah
/// pengguna menekan Install.
fn build_version_options(plugin: &Plugin, installed: Option<&InstalledEntry>) -> Vec<VersionOption> {
    let installed_version = installed.map(|e| e.version.as_str());

    let mut options: Vec<VersionOption> = std::iter::once((&plugin.latest, true))
        .chain(plugin.history.iter().map(|r| (r, false)))
        .filter_map(|(release, is_latest)| {
            let build = release.build_for_current_target()?;
            Some(VersionOption {
                version: release.version.clone(),
                released_at: release.released_at.clone(),
                breaking: release.breaking,
                security: release.security,
                changelog: release.changelog.clone(),
                download_size_bytes: build.size_bytes,
                is_latest,
                is_installed: installed_version
                    .and_then(hub_core::version::parse)
                    .zip(hub_core::version::parse(&release.version))
                    .map(|(a, b)| a == b)
                    .unwrap_or(false),
            })
        })
        .collect();

    // Urutkan menurun secara semver, bukan mengandalkan urutan di katalog.
    options.sort_by(|a, b| {
        match (
            hub_core::version::parse(&b.version),
            hub_core::version::parse(&a.version),
        ) {
            (Some(x), Some(y)) => x.cmp(&y),
            _ => std::cmp::Ordering::Equal,
        }
    });
    options
}

pub fn build_installed_view(entry: &InstalledEntry) -> InstalledView {
    InstalledView {
        version: entry.version.clone(),
        installed_at: entry.installed_at.clone(),
        scope: entry.scope.as_str().to_string(),
        install_dir: entry.install_dir.to_string_lossy().to_string(),
        health: entry.health,
        adopted: entry.adopted,
        has_backup: entry.backup.is_some(),
        backup_version: entry.backup.as_ref().map(|b| b.version.clone()),
        skipped_versions: entry.skipped_versions.clone(),
    }
}

pub fn build_library(catalog: Option<&Catalog>, db: &InstalledDb) -> Vec<LibraryEntry> {
    db.entries
        .iter()
        .map(|entry| {
            let plugin = catalog.and_then(|c| c.plugin(&entry.plugin_id));
            let update = match (plugin, entry.health) {
                (_, Health::UnknownVersion) => UpdateState::Unknown,
                (Some(p), _) => hub_core::version::compute_update_state(
                    Some(&entry.version),
                    &p.latest.version,
                    p.latest.breaking,
                    &entry.skipped_versions,
                ),
                // Tanpa katalog (mode offline) kita tidak tahu apa-apa tentang
                // update — dan menampilkan "UpToDate" akan berbohong.
                (None, _) => UpdateState::Unknown,
            };

            LibraryEntry {
                plugin_id: entry.plugin_id.clone(),
                name: plugin
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| entry.plugin_id.clone()),
                installed: build_installed_view(entry),
                update,
                in_catalog: plugin.is_some(),
                icon_url: plugin.and_then(|p| p.icon_url.clone()),
            }
        })
        .collect()
}

pub fn build_update_summary(catalog: &Catalog, db: &InstalledDb) -> UpdateSummary {
    let mut items = Vec::new();

    for entry in &db.entries {
        let Some(plugin) = catalog.plugin(&entry.plugin_id) else {
            continue;
        };
        if entry.health == Health::Missing {
            // Plugin yang filenya hilang butuh reinstall, bukan update; ia
            // muncul di Library dengan status sendiri.
            continue;
        }

        let state = hub_core::version::compute_update_state(
            Some(&entry.version),
            &plugin.latest.version,
            plugin.latest.breaking,
            &entry.skipped_versions,
        );

        if let UpdateState::UpdateAvailable { from, to, breaking } = state {
            items.push(UpdateItem {
                plugin_id: plugin.id.clone(),
                name: plugin.name.clone(),
                icon_url: plugin.icon_url.clone(),
                from_version: from,
                to_version: to,
                released_at: plugin.latest.released_at.clone(),
                breaking,
                security: plugin.latest.security,
                changelog: plugin.latest.changelog.clone(),
                download_size_bytes: plugin
                    .latest
                    .build_for_current_target()
                    .map(|b| b.size_bytes)
                    .unwrap_or(0),
            });
        }
    }

    // Security update lebih dulu, lalu breaking (agar terlihat dan dibaca),
    // lalu sisanya menurut nama.
    items.sort_by(|a, b| {
        b.security
            .cmp(&a.security)
            .then(b.breaking.cmp(&a.breaking))
            .then(a.name.cmp(&b.name))
    });

    let breaking_count = items.iter().filter(|i| i.breaking).count();
    UpdateSummary {
        non_breaking_count: items.len() - breaking_count,
        breaking_count,
        items,
    }
}
