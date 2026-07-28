//! Unduhan streaming dengan progres, pembatalan, dan hash inline (PRD §11.6).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::Serialize;
use tokio::io::AsyncWriteExt;

use crate::verify::{self, Sha256Digest};
use crate::{HubError, Result};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Tidak ada timeout total — unduhan besar di koneksi lambat itu sah. Yang
/// dibatasi adalah *diam*: 60 detik tanpa byte baru berarti koneksi mati
/// (PRD §11.6 aturan 7).
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_REDIRECTS: usize = 5;
/// Toleransi selisih ukuran terhadap `size_bytes` di katalog.
const SIZE_TOLERANCE_BYTES: u64 = 4096;

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub url: url::Url,
    pub expected_size: u64,
    pub expected_sha256: Sha256Digest,
    /// Direktori cache unduhan (`cache/downloads`).
    pub dest_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DownloadProgress {
    #[serde(rename_all = "camelCase")]
    Started {
        total_bytes: u64,
    },
    #[serde(rename_all = "camelCase")]
    Progress {
        received: u64,
        total: u64,
        bytes_per_sec: u64,
    },
    Verifying,
    #[serde(rename_all = "camelCase")]
    Done {
        path: PathBuf,
    },
}

/// Token pembatalan sederhana; job_cancel men-set flag ini.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

pub struct Downloader {
    client: reqwest::Client,
}

impl Downloader {
    pub fn new() -> Result<Self> {
        // Redirect divalidasi per-hop. Redirect adalah cara paling umum untuk
        // melewati pemeriksaan host yang hanya dilakukan sekali di awal
        // (PRD §11.6 aturan 3) — dan GitHub Releases *selalu* redirect ke
        // objects.githubusercontent.com, jadi jalur ini bukan kasus tepi.
        let policy = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("terlalu banyak redirect");
            }
            let url = attempt.url();
            if url.scheme() != "https" {
                return attempt.error("redirect ke skema non-https ditolak");
            }
            match url.host_str() {
                Some(host) if crate::host_is_allowed(host) => attempt.follow(),
                Some(_) => attempt.error("redirect ke host di luar allowlist ditolak"),
                None => attempt.error("redirect tanpa host"),
            }
        });

        let client = reqwest::Client::builder()
            .user_agent(concat!("StudioHub/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .redirect(policy)
            .https_only(true)
            .build()
            .map_err(|e| HubError::internal(format!("gagal membangun HTTP client: {e}")))?;

        Ok(Downloader { client })
    }

    /// Unduh, verifikasi, dan pindahkan ke nama final.
    ///
    /// File final dinamai dari hash yang diharapkan. Konsekuensinya, cache hit
    /// adalah "file dengan nama itu ada dan hash-nya benar" — tidak perlu
    /// metadata terpisah, dan file yang isinya tidak sesuai namanya tidak
    /// mungkin lolos.
    pub async fn download<F>(
        &self,
        request: &DownloadRequest,
        cancel: &CancellationToken,
        mut on_progress: F,
    ) -> Result<PathBuf>
    where
        F: FnMut(DownloadProgress),
    {
        validate_download_url(&request.url)?;
        tokio::fs::create_dir_all(&request.dest_dir).await?;

        let expected_hex = verify::to_hex(&request.expected_sha256);
        let final_path = request.dest_dir.join(format!("{expected_hex}.zip"));
        let part_path = request.dest_dir.join(format!("{expected_hex}.part"));

        // Cache hit: file sudah ada. Tetap diverifikasi ulang — file di cache
        // bisa saja rusak sejak terakhir disentuh.
        if final_path.exists() {
            on_progress(DownloadProgress::Verifying);
            match verify::verify_sha256(&final_path, &request.expected_sha256).await {
                Ok(()) => {
                    on_progress(DownloadProgress::Done {
                        path: final_path.clone(),
                    });
                    return Ok(final_path);
                }
                Err(_) => {
                    tracing::warn!("artefak cache rusak, mengunduh ulang");
                    let _ = tokio::fs::remove_file(&final_path).await;
                }
            }
        }

        // Resume (FR-3.4): lanjutkan dari `.part` yang tertinggal.
        let resume_from = tokio::fs::metadata(&part_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let resume_from = if resume_from >= request.expected_size {
            // `.part` yang lebih besar dari yang diharapkan berarti sesuatu
            // yang salah; mulai dari nol lebih murah daripada menebak.
            let _ = tokio::fs::remove_file(&part_path).await;
            0
        } else {
            resume_from
        };

        let mut req = self.client.get(request.url.clone());
        if resume_from > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
        }
        let response = req.send().await?;

        if !response.status().is_success() {
            return Err(HubError::NetworkUnreachable {
                retryable: response.status().is_server_error(),
                detail: format!("HTTP {}", response.status()),
            });
        }

        // Server yang mengabaikan Range mengembalikan 200 dan seluruh file.
        let resuming = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        let start_at = if resuming { resume_from } else { 0 };

        let total = request.expected_size;
        on_progress(DownloadProgress::Started { total_bytes: total });

        let mut file = if resuming {
            tokio::fs::OpenOptions::new()
                .append(true)
                .open(&part_path)
                .await?
        } else {
            tokio::fs::File::create(&part_path).await?
        };

        // Hash dari byte yang sudah ada di `.part` harus ikut dihitung, kalau
        // tidak resume menghasilkan hash yang salah untuk file yang benar.
        let mut hasher = verify::StreamingHasher::new();
        if start_at > 0 {
            hash_existing_prefix(&mut hasher, &part_path, start_at).await?;
        }

        let mut received = start_at;
        let mut stream = response.bytes_stream();
        let started = Instant::now();
        let mut last_report = Instant::now();

        loop {
            let next = tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await;
            let chunk = match next {
                Err(_) => {
                    return Err(HubError::NetworkUnreachable {
                        retryable: true,
                        detail: "koneksi diam melebihi batas".into(),
                    })
                }
                Ok(None) => break,
                Ok(Some(Err(e))) => return Err(e.into()),
                Ok(Some(Ok(chunk))) => chunk,
            };

            if cancel.is_cancelled() {
                // `.part` sengaja dipertahankan agar dapat di-resume
                // (PRD §11.6 aturan 8).
                file.flush().await?;
                return Err(HubError::Cancelled);
            }

            received = received.saturating_add(chunk.len() as u64);
            // Aturan 4: hentikan sebelum disk terisi oleh respons yang berbohong.
            if received > total.saturating_add(SIZE_TOLERANCE_BYTES) {
                let _ = tokio::fs::remove_file(&part_path).await;
                return Err(HubError::ArchiveRejected {
                    reason: format!("unduhan melebihi ukuran yang dijanjikan ({total} byte)"),
                });
            }

            hasher.update(&chunk);
            file.write_all(&chunk).await?;

            if last_report.elapsed() >= Duration::from_millis(120) {
                let elapsed = started.elapsed().as_secs_f64().max(0.001);
                on_progress(DownloadProgress::Progress {
                    received,
                    total,
                    bytes_per_sec: (((received - start_at) as f64) / elapsed) as u64,
                });
                last_report = Instant::now();
            }
        }

        file.flush().await?;
        file.sync_all().await?;
        drop(file);

        on_progress(DownloadProgress::Verifying);

        let actual = hasher.finish();
        if !verify::digests_equal(&actual, &request.expected_sha256) {
            // FR-3.5: artefak yang gagal verifikasi dihapus, tidak disimpan
            // "untuk berjaga-jaga". Menyimpannya berarti percobaan berikutnya
            // bisa memakainya sebagai resume base.
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err(HubError::IntegrityMismatch {
                expected: expected_hex,
                actual: verify::to_hex(&actual),
            });
        }

        tokio::fs::rename(&part_path, &final_path).await?;
        on_progress(DownloadProgress::Done {
            path: final_path.clone(),
        });
        Ok(final_path)
    }
}

async fn hash_existing_prefix(
    hasher: &mut verify::StreamingHasher,
    path: &Path,
    len: u64,
) -> Result<()> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await?;
    let mut remaining = len;
    let mut buf = vec![0u8; 128 * 1024];
    while remaining > 0 {
        let want = buf.len().min(remaining as usize);
        let n = file.read(&mut buf[..want]).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        remaining -= n as u64;
    }
    Ok(())
}

/// Aturan 1 & 2: HTTPS wajib, host harus ada di allowlist yang dikompilasi ke
/// dalam binary.
pub fn validate_download_url(url: &url::Url) -> Result<()> {
    if url.scheme() != "https" {
        return Err(HubError::CatalogInvalid {
            detail: format!("URL unduhan bukan https: {url}"),
        });
    }
    let host = url.host_str().ok_or_else(|| HubError::CatalogInvalid {
        detail: "URL unduhan tanpa host".into(),
    })?;
    if !crate::host_is_allowed(host) {
        return Err(HubError::CatalogInvalid {
            detail: format!("host unduhan di luar allowlist: {host}"),
        });
    }
    Ok(())
}

/// Hapus file `.part` yang lebih tua dari `max_age` (PRD §13.8 langkah 4).
pub fn sweep_stale_parts(dir: &Path, max_age: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("part") {
            continue;
        }
        let too_old = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().map(|e| e > max_age).unwrap_or(false))
            .unwrap_or(false);
        if too_old && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_allowlisted_hosts_pass() {
        let ok =
            url::Url::parse("https://github.com/robi/MyComp/releases/download/v1/x.zip").unwrap();
        assert!(validate_download_url(&ok).is_ok());

        let plain = url::Url::parse("http://github.com/x.zip").unwrap();
        assert!(validate_download_url(&plain).is_err());

        let evil = url::Url::parse("https://evil.tld/x.zip").unwrap();
        assert!(validate_download_url(&evil).is_err());

        // Kelas bug yang membuat `host.contains("github.com")` berbahaya.
        let lookalike = url::Url::parse("https://github.com.evil.tld/x.zip").unwrap();
        assert!(validate_download_url(&lookalike).is_err());
    }

    #[test]
    fn cancellation_token_is_shared() {
        let a = CancellationToken::new();
        let b = a.clone();
        assert!(!b.is_cancelled());
        a.cancel();
        assert!(b.is_cancelled());
    }
}
