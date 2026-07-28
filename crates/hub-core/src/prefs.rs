//! Preferensi pengguna, `prefs.json` (PRD §25.3).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::paths::InstallScope;
use crate::Result;

/// URL katalog, ditanam di binary.
///
/// Sengaja **tidak** dapat diubah pengguna. Konsekuensinya harus disadari:
/// FR-8.5 dan mitigasi R4 mengandaikan URL ini dapat diganti tanpa merilis
/// launcher, jadi kalau katalog suatu saat pindah dari GitHub Pages, pemindahan
/// itu menuntut rilis launcher baru. Ditukar dengan satu layar pengaturan yang
/// lebih sederhana dan satu jalur yang tidak dapat disalahsetel pengguna.
pub const CATALOG_URL: &str = "https://ClyntiaAikaHanaa.github.io/plugin-catalog/catalog.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    #[serde(alias = "schema_version")]
    pub schema_version: u32,
    pub locale: String,
    pub theme: String,
    #[serde(alias = "default_install_scope")]
    pub default_install_scope: InstallScope,
    #[serde(alias = "check_updates_on_launch")]
    pub check_updates_on_launch: bool,
    /// Default **false** (FR-8.3). Data yang didapat dengan mengorbankan
    /// kepercayaan tidak sebanding (PRD §17.1).
    #[serde(default, alias = "telemetry_enabled")]
    pub telemetry_enabled: bool,
    #[serde(default, alias = "telemetry_prompt_shown")]
    pub telemetry_prompt_shown: bool,
    /// UUID v4 acak, dibuat di mesin, tidak diturunkan dari hardware ID dan
    /// dapat di-reset dari Settings (PRD §17.2).
    #[serde(alias = "install_id")]
    pub install_id: String,
    #[serde(default, alias = "verbose_logging")]
    pub verbose_logging: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            schema_version: 1,
            locale: "id".to_string(),
            theme: "system".to_string(),
            default_install_scope: InstallScope::CurrentUser,
            check_updates_on_launch: true,
            telemetry_enabled: false,
            telemetry_prompt_shown: false,
            install_id: uuid::Uuid::new_v4().to_string(),
            verbose_logging: false,
        }
    }
}

impl Prefs {
    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<Prefs>(&bytes) {
                Ok(prefs) => prefs,
                Err(e) => {
                    tracing::warn!(error = %e, "prefs.json rusak, memakai default");
                    Prefs::default()
                }
            },
            Err(_) => Prefs::default(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| crate::HubError::internal(format!("serialisasi prefs: {e}")))?;
        crate::registry::write_atomic(path, &bytes)
    }

    /// URL katalog yang berlaku. Selalu konstanta di binary.
    pub fn catalog_url(&self) -> &'static str {
        CATALOG_URL
    }

    /// Scope yang dipakai sebagai default berikutnya (FR-3.9).
    ///
    /// `Custom` tidak lagi dapat dipilih dari UI; nilai yang tersisa di
    /// `prefs.json` lama dikembalikan ke per-user, bukan dihormati diam-diam.
    pub fn resolved_scope(&self) -> InstallScope {
        match &self.default_install_scope {
            InstallScope::Custom { .. } => InstallScope::CurrentUser,
            scope => scope.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_defaults_to_off() {
        assert!(!Prefs::default().telemetry_enabled);
    }

    #[test]
    fn default_scope_is_per_user() {
        // ADR-3: jalur paling umum tidak boleh memicu UAC.
        assert_eq!(Prefs::default().default_install_scope, InstallScope::CurrentUser);
    }

    #[test]
    fn catalog_url_is_not_user_configurable() {
        // `prefs.json` lama memuat `catalogUrl`; ia diabaikan, bukan dipakai.
        // Tanpa ini, mesin yang pernah menyimpan URL placeholder akan terkunci
        // ke sana selamanya.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prefs.json");
        std::fs::write(
            &path,
            br#"{"schemaVersion":1,"locale":"id","theme":"system",
                 "defaultInstallScope":{"kind":"current_user"},
                 "checkUpdatesOnLaunch":true,
                 "catalogUrl":"https://akun-lama.github.io/x/catalog.json",
                 "installId":"abc"}"#,
        )
        .unwrap();

        assert_eq!(Prefs::load(&path).catalog_url(), CATALOG_URL);
    }

    #[test]
    fn legacy_custom_scope_falls_back_to_per_user() {
        let prefs = Prefs {
            default_install_scope: InstallScope::Custom {
                path: std::path::PathBuf::from("D:\\VST3"),
            },
            ..Default::default()
        };
        assert_eq!(prefs.resolved_scope(), InstallScope::CurrentUser);
    }

    #[test]
    fn corrupt_prefs_fall_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prefs.json");
        std::fs::write(&path, b"nope").unwrap();
        assert_eq!(Prefs::load(&path).locale, "id");
    }

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prefs.json");
        let prefs = Prefs {
            locale: "en".into(),
            ..Default::default()
        };
        prefs.save(&path).unwrap();
        assert_eq!(Prefs::load(&path).locale, "en");
    }
}
