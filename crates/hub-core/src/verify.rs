//! Verifikasi SHA-256 (PRD §11.7).
//!
//! Fungsi di modul ini **tidak punya parameter untuk melewati verifikasi**, dan
//! itu disengaja. Kombinasinya dengan [`crate::install::plan::InstallPlan`],
//! yang tidak dapat dikonstruksi tanpa hash yang diharapkan, membuat "lupa
//! memverifikasi" menjadi kesalahan compile-time — bukan bug runtime yang bisa
//! lolos ke produksi (G4).

use std::path::Path;

use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::{HubError, Result};

pub type Sha256Digest = [u8; 32];

/// Parse hash heksadesimal dari katalog.
pub fn parse_hex(hex: &str) -> Result<Sha256Digest> {
    if hex.len() != 64 {
        return Err(HubError::CatalogInvalid {
            detail: format!("sha256 harus 64 karakter, dapat {}", hex.len()),
        });
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).map_err(|_| HubError::CatalogInvalid {
            detail: "sha256 bukan ASCII".into(),
        })?;
        out[i] = u8::from_str_radix(s, 16).map_err(|_| HubError::CatalogInvalid {
            detail: format!("sha256 bukan heksadesimal: {s}"),
        })?;
    }
    Ok(out)
}

pub fn to_hex(digest: &Sha256Digest) -> String {
    let mut s = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(s, "{byte:02x}");
    }
    s
}

/// Perbandingan constant-time.
///
/// Konteks ini bukan yang benar-benar rentan timing attack, tapi biayanya nol
/// dan menghilangkan pertanyaan (PRD §11.7).
pub fn digests_equal(a: &Sha256Digest, b: &Sha256Digest) -> bool {
    let mut diff = 0u8;
    for i in 0..32 {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Hitung SHA-256 sebuah file secara streaming (memori konstan).
pub async fn hash_file(path: &Path) -> Result<Sha256Digest> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 128 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}

/// Verifikasi file terhadap hash yang diharapkan.
///
/// Tidak ada parameter `skip_verify` di API ini, secara sengaja (NFR-3.2).
pub async fn verify_sha256(path: &Path, expected: &Sha256Digest) -> Result<()> {
    let actual = hash_file(path).await?;
    if digests_equal(&actual, expected) {
        Ok(())
    } else {
        Err(HubError::IntegrityMismatch {
            expected: to_hex(expected),
            actual: to_hex(&actual),
        })
    }
}

/// Hasher inkremental untuk unduhan: hash dihitung sambil byte mengalir, bukan
/// dengan membaca ulang file setelahnya. Menghemat satu pass I/O dan menutup
/// race di mana file berubah antara unduh dan verifikasi (PRD §11.6 aturan 5).
pub struct StreamingHasher(Sha256);

impl StreamingHasher {
    pub fn new() -> Self {
        StreamingHasher(Sha256::new())
    }

    pub fn update(&mut self, chunk: &[u8]) {
        self.0.update(chunk);
    }

    pub fn finish(self) -> Sha256Digest {
        self.0.finalize().into()
    }
}

impl Default for StreamingHasher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn hex_roundtrip() {
        let digest = parse_hex(EMPTY_SHA256).unwrap();
        assert_eq!(to_hex(&digest), EMPTY_SHA256);
    }

    #[test]
    fn malformed_hex_is_rejected() {
        assert!(parse_hex("abc").is_err());
        assert!(parse_hex(&"z".repeat(64)).is_err());
    }

    #[test]
    fn streaming_matches_oneshot() {
        let mut h = StreamingHasher::new();
        h.update(b"hello ");
        h.update(b"world");
        let streamed = h.finish();

        let oneshot: Sha256Digest = Sha256::digest(b"hello world").into();
        assert!(digests_equal(&streamed, &oneshot));
    }

    #[tokio::test]
    async fn mismatch_is_reported_with_both_values() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.bin");
        std::fs::write(&file, b"tampered").unwrap();

        let err = verify_sha256(&file, &parse_hex(EMPTY_SHA256).unwrap())
            .await
            .unwrap_err();
        match err {
            HubError::IntegrityMismatch { expected, actual } => {
                assert_eq!(expected, EMPTY_SHA256);
                assert_ne!(actual, EMPTY_SHA256);
            }
            other => panic!("varian salah: {other:?}"),
        }
    }
}
