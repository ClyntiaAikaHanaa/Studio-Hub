//! Pengambilan katalog: HTTP + ETag + cache lokal (FR-1.1 s/d FR-1.4).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use super::Catalog;
use crate::{HubError, Result};

/// Batas ukuran katalog. Katalog nyata berukuran puluhan KB; batas ini menahan
/// respons yang dimanipulasi agar tidak menghabiskan memori.
const MAX_CATALOG_BYTES: u64 = 8 * 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(45);

/// Metadata kesegaran cache, ditampilkan sebagai indikator di UI (FR-1.2).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CacheStatus {
    /// RFC 3339. `None` jika belum pernah berhasil fetch.
    pub last_success_at: Option<String>,
    pub etag: Option<String>,
    pub stale: bool,
    /// Diisi jika fetch terakhir gagal dan kita menampilkan cache.
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum FetchOutcome {
    /// Cache masih dalam TTL; tidak ada request jaringan sama sekali (FR-1.4).
    CacheFresh(Catalog),
    /// Server menjawab 304; cache diperpanjang tanpa unduhan (§10.6).
    NotModified(Catalog),
    Fetched(Catalog),
    /// Fetch gagal, kita menyajikan cache terakhir + menandainya usang.
    StaleFallback { catalog: Catalog, error: HubError },
}

impl FetchOutcome {
    pub fn catalog(&self) -> &Catalog {
        match self {
            FetchOutcome::CacheFresh(c)
            | FetchOutcome::NotModified(c)
            | FetchOutcome::Fetched(c)
            | FetchOutcome::StaleFallback { catalog: c, .. } => c,
        }
    }

    pub fn is_stale(&self) -> bool {
        matches!(self, FetchOutcome::StaleFallback { .. })
    }
}

pub struct CatalogFetcher {
    client: reqwest::Client,
    cache_dir: PathBuf,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct CacheMeta {
    etag: Option<String>,
    fetched_at_unix: Option<u64>,
    ttl_seconds: Option<u64>,
    url: Option<String>,
}

impl CatalogFetcher {
    pub fn new(cache_dir: impl Into<PathBuf>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("StudioHub/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(TOTAL_TIMEOUT)
            .https_only(true) // NFR-3.1
            .build()
            .map_err(|e| HubError::internal(format!("gagal membangun HTTP client: {e}")))?;
        Ok(Self {
            client,
            cache_dir: cache_dir.into(),
        })
    }

    fn catalog_path(&self) -> PathBuf {
        self.cache_dir.join("catalog.json")
    }

    fn meta_path(&self) -> PathBuf {
        self.cache_dir.join("catalog.meta.json")
    }

    /// Ambil katalog.
    ///
    /// Urutan sengaja: cache dulu, jaringan kemudian. UI merender apa yang ada
    /// sebelum jaringan menjawab (NFR-1.5) — koneksi lambat tidak boleh membuat
    /// aplikasi terasa rusak.
    pub async fn fetch(&self, url: &str, force: bool) -> Result<FetchOutcome> {
        validate_catalog_url(url)?;
        let meta = self.read_meta();
        let cached = self.read_cached_catalog();

        // URL katalog yang berubah (mis. pengguna beralih ke beta channel)
        // membuat cache lama tidak relevan.
        let same_url = meta.url.as_deref() == Some(url);

        if !force && same_url {
            if let (Some(catalog), true) = (cached.as_ref(), self.cache_is_fresh(&meta)) {
                tracing::debug!("katalog: cache masih segar, tidak ada request jaringan");
                return Ok(FetchOutcome::CacheFresh(catalog.clone()));
            }
        }

        let etag = if same_url { meta.etag.clone() } else { None };
        match self.fetch_remote(url, etag.as_deref()).await {
            Ok(RemoteResult::NotModified) => {
                if let Some(catalog) = cached {
                    self.write_meta(&CacheMeta {
                        etag,
                        fetched_at_unix: Some(now_unix()),
                        ttl_seconds: Some(catalog.catalog_ttl_seconds),
                        url: Some(url.to_string()),
                    });
                    Ok(FetchOutcome::NotModified(catalog))
                } else {
                    // 304 tanpa cache berarti state kita tidak konsisten.
                    // Ambil ulang tanpa ETag.
                    match self.fetch_remote(url, None).await? {
                        RemoteResult::Body { bytes, etag } => {
                            self.store(url, &bytes, etag).map(FetchOutcome::Fetched)
                        }
                        RemoteResult::NotModified => Err(HubError::CatalogInvalid {
                            detail: "server menjawab 304 tanpa cache lokal".into(),
                        }),
                    }
                }
            }
            Ok(RemoteResult::Body { bytes, etag }) => {
                self.store(url, &bytes, etag).map(FetchOutcome::Fetched)
            }
            Err(error) => match cached {
                Some(catalog) => {
                    tracing::warn!(%error, "fetch katalog gagal, memakai cache");
                    Ok(FetchOutcome::StaleFallback { catalog, error })
                }
                None => Err(error),
            },
        }
    }

    /// Status cache untuk indikator UI.
    pub fn status(&self) -> CacheStatus {
        let meta = self.read_meta();
        CacheStatus {
            last_success_at: meta.fetched_at_unix.map(format_unix),
            etag: meta.etag.clone(),
            stale: !self.cache_is_fresh(&meta),
            last_error: None,
        }
    }

    fn cache_is_fresh(&self, meta: &CacheMeta) -> bool {
        let (Some(fetched), Some(ttl)) = (meta.fetched_at_unix, meta.ttl_seconds) else {
            return false;
        };
        now_unix().saturating_sub(fetched) < ttl
    }

    fn read_cached_catalog(&self) -> Option<Catalog> {
        let bytes = std::fs::read(self.catalog_path()).ok()?;
        match Catalog::parse(&bytes) {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!(error = %e, "cache katalog rusak, diabaikan");
                None
            }
        }
    }

    fn read_meta(&self) -> CacheMeta {
        std::fs::read(self.meta_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn write_meta(&self, meta: &CacheMeta) {
        if let Ok(bytes) = serde_json::to_vec_pretty(meta) {
            let _ = crate::registry::write_atomic(&self.meta_path(), &bytes);
        }
    }

    fn store(&self, url: &str, bytes: &[u8], etag: Option<String>) -> Result<Catalog> {
        // Parse dulu, tulis kemudian: katalog yang tidak dapat diparsing tidak
        // boleh menimpa cache yang masih baik.
        let catalog = Catalog::parse(bytes)?;
        let _ = std::fs::create_dir_all(&self.cache_dir);
        crate::registry::write_atomic(&self.catalog_path(), bytes)?;
        self.write_meta(&CacheMeta {
            etag,
            fetched_at_unix: Some(now_unix()),
            ttl_seconds: Some(catalog.catalog_ttl_seconds),
            url: Some(url.to_string()),
        });
        Ok(catalog)
    }

    async fn fetch_remote(&self, url: &str, etag: Option<&str>) -> Result<RemoteResult> {
        let mut request = self.client.get(url);
        if let Some(etag) = etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }

        let response = request.send().await?;

        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(RemoteResult::NotModified);
        }
        if !response.status().is_success() {
            return Err(HubError::NetworkUnreachable {
                retryable: response.status().is_server_error(),
                detail: format!("HTTP {}", response.status()),
            });
        }
        if let Some(len) = response.content_length() {
            if len > MAX_CATALOG_BYTES {
                return Err(HubError::CatalogInvalid {
                    detail: format!("katalog terlalu besar: {len} byte"),
                });
            }
        }

        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        let bytes = response.bytes().await?;
        if bytes.len() as u64 > MAX_CATALOG_BYTES {
            return Err(HubError::CatalogInvalid {
                detail: "katalog melebihi batas ukuran".into(),
            });
        }

        Ok(RemoteResult::Body {
            bytes: bytes.to_vec(),
            etag,
        })
    }
}

enum RemoteResult {
    NotModified,
    Body {
        bytes: Vec<u8>,
        etag: Option<String>,
    },
}

/// URL katalog dapat diubah pengguna (FR-8.5), jadi ia divalidasi seperti input
/// lain: HTTPS wajib. Host katalog sengaja *tidak* dibatasi ke allowlist —
/// FR-8.5 dan R4 mengharuskan katalog dapat dipindah ke hosting lain. Yang
/// dibatasi allowlist adalah URL *unduhan* di dalam katalog (§11.6).
pub fn validate_catalog_url(raw: &str) -> Result<()> {
    let url = url::Url::parse(raw).map_err(|e| HubError::CatalogInvalid {
        detail: format!("URL katalog tidak valid: {e}"),
    })?;
    if url.scheme() != "https" {
        return Err(HubError::CatalogInvalid {
            detail: "URL katalog harus https".into(),
        });
    }
    if url.host_str().is_none() {
        return Err(HubError::CatalogInvalid {
            detail: "URL katalog tanpa host".into(),
        });
    }
    Ok(())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn format_unix(secs: u64) -> String {
    time::OffsetDateTime::from_unix_timestamp(secs as i64)
        .ok()
        .and_then(|t| t.format(&time::format_description::well_known::Rfc3339).ok())
        .unwrap_or_default()
}

/// Baca katalog dari file lokal tanpa jaringan. Dipakai mode offline dan test.
pub fn read_local(path: &Path) -> Result<Catalog> {
    Catalog::parse(&std::fs::read(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_url_must_be_https() {
        assert!(validate_catalog_url("https://robi.github.io/plugin-catalog/catalog.json").is_ok());
        assert!(validate_catalog_url("http://robi.github.io/catalog.json").is_err());
        assert!(validate_catalog_url("file:///C:/catalog.json").is_err());
        assert!(validate_catalog_url("bukan-url").is_err());
    }
}
