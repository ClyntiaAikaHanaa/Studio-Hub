//! Kebijakan perbandingan versi (PRD §11.5).
//!
//! Seluruh perbandingan versi di aplikasi ini harus lewat modul ini. Tidak ada
//! perbandingan string di mana pun — itu bug klasik `1.10.0 < 1.9.0`.

use semver::Version;
use serde::{Deserialize, Serialize};

/// Parse versi, menoleransi prefix tag Git `v`.
///
/// Mengembalikan `None` (bukan fallback ke perbandingan string) jika gagal:
/// PRD §11.5 aturan 3 menyatakan versi yang tidak dapat diparsing ditandai
/// `Unknown`, dan CI menolak katalog yang memuatnya.
pub fn parse(raw: &str) -> Option<Version> {
    let trimmed = raw.trim();
    let normalized = trimmed.strip_prefix('v').unwrap_or(trimmed);
    Version::parse(normalized).ok()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum UpdateState {
    UpToDate,
    #[serde(rename_all = "camelCase")]
    UpdateAvailable {
        from: String,
        to: String,
        breaking: bool,
    },
    /// Terpasang lebih baru dari katalog — pengguna memasang build dev, atau
    /// katalog di-rollback. Jangan pernah menawarkan "update" ke versi lebih
    /// rendah; itu adalah downgrade attack (PRD T8).
    AheadOfCatalog,
    /// Versi terpasang tidak dapat ditentukan (mis. plugin diadopsi).
    Unknown,
    /// Pengguna menekan "skip" untuk versi katalog saat ini.
    Skipped,
}

/// Hitung state update untuk satu plugin.
///
/// `skipped` adalah daftar versi yang pengguna tolak (FR-4.7). Versi yang
/// di-skip berhenti memicu badge, tapi versi *berikutnya* tetap ditawarkan —
/// itulah sebabnya perbandingan di bawah memakai daftar, bukan satu flag.
pub fn compute_update_state(
    installed: Option<&str>,
    catalog_latest: &str,
    breaking: bool,
    skipped: &[String],
) -> UpdateState {
    let Some(installed_raw) = installed else {
        return UpdateState::Unknown;
    };
    let (Some(installed_v), Some(latest_v)) = (parse(installed_raw), parse(catalog_latest)) else {
        return UpdateState::Unknown;
    };

    match installed_v.cmp(&latest_v) {
        std::cmp::Ordering::Equal => UpdateState::UpToDate,
        std::cmp::Ordering::Greater => UpdateState::AheadOfCatalog,
        std::cmp::Ordering::Less => {
            let is_skipped = skipped
                .iter()
                .filter_map(|s| parse(s))
                .any(|s| s == latest_v);
            if is_skipped {
                UpdateState::Skipped
            } else {
                UpdateState::UpdateAvailable {
                    from: installed_v.to_string(),
                    to: latest_v.to_string(),
                    breaking,
                }
            }
        }
    }
}

/// True jika `version` adalah pre-release. Build pre-release hanya ditawarkan
/// jika pengguna mengaktifkan beta channel (PRD §11.5 aturan 4).
pub fn is_prerelease(version: &str) -> bool {
    parse(version).map(|v| !v.pre.is_empty()).unwrap_or(false)
}

/// True jika `current` memenuhi `min_required` (FR-1.7).
pub fn satisfies_minimum(current: &str, min_required: &str) -> bool {
    match (parse(current), parse(min_required)) {
        (Some(c), Some(m)) => c >= m,
        // Jika minimum tidak dapat diparsing, entri katalog itu rusak; sikap
        // aman adalah menolaknya, bukan menganggapnya terpenuhi.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_not_lexicographic() {
        // Regresi untuk bug perbandingan string (PRD §19.2).
        assert!(parse("1.10.0").unwrap() > parse("1.9.0").unwrap());
    }

    #[test]
    fn prerelease_orders_below_final() {
        assert!(parse("1.3.0-beta.1").unwrap() < parse("1.3.0").unwrap());
    }

    #[test]
    fn git_tag_prefix_is_stripped() {
        assert_eq!(parse("v1.2.3"), parse("1.2.3"));
    }

    #[test]
    fn unparseable_is_none_not_fallback() {
        assert!(parse("latest").is_none());
        assert!(parse("1.2.3.4").is_none());
    }

    #[test]
    fn installed_newer_than_catalog_offers_nothing() {
        assert_eq!(
            compute_update_state(Some("2.0.0"), "1.9.0", false, &[]),
            UpdateState::AheadOfCatalog
        );
    }

    #[test]
    fn skipped_version_stops_badging_but_next_one_returns() {
        let skipped = vec!["1.3.0".to_string()];
        assert_eq!(
            compute_update_state(Some("1.2.0"), "1.3.0", false, &skipped),
            UpdateState::Skipped
        );
        assert!(matches!(
            compute_update_state(Some("1.2.0"), "1.4.0", false, &skipped),
            UpdateState::UpdateAvailable { .. }
        ));
    }

    #[test]
    fn minimum_launcher_version_gate() {
        assert!(satisfies_minimum("1.0.0", "1.0.0"));
        assert!(satisfies_minimum("1.2.0", "1.0.0"));
        assert!(!satisfies_minimum("1.0.0", "1.2.0"));
        assert!(!satisfies_minimum("1.0.0", "bogus"));
    }
}
