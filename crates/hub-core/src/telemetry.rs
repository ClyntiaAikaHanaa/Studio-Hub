//! Telemetry opt-in dan anonim (PRD §17).
//!
//! Tiga aturan yang tidak boleh dilanggar:
//!
//! 1. **Opt-out secara default.** Tidak ada satu pun event yang dikirim sebelum
//!    pengguna secara eksplisit menyalakannya (FR-8.3).
//! 2. **Tidak ada PII.** Nama pengguna, nama komputer, path filesystem, daftar
//!    software lain — tidak satu pun boleh masuk payload (§17.4). Tipe di bawah
//!    tidak punya field untuk menampungnya, jadi menambahkannya butuh perubahan
//!    yang terlihat di code review.
//! 3. **Fire-and-forget.** Kegagalan telemetry tidak boleh pernah memengaruhi
//!    fungsi launcher (§17.5).

use std::time::Duration;

use serde::Serialize;

const SEND_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    AppOpened,
    CatalogFetched {
        cache_hit: bool,
        duration_ms: u64,
    },
    InstallStarted {
        plugin_id: String,
        plugin_version: String,
        scope: String,
    },
    InstallCompleted {
        plugin_id: String,
        plugin_version: String,
        scope: String,
        duration_ms: u64,
        from_version: Option<String>,
    },
    /// Alasan utama telemetry ini ada (§17.3). Tanpa `error_code` dan `stage`,
    /// kegagalan instalasi hanya terlihat sebagai pengguna yang menghilang.
    InstallFailed {
        plugin_id: String,
        plugin_version: String,
        error_code: String,
        stage: String,
    },
    UpdateOffered {
        plugin_id: String,
        from: String,
        to: String,
        breaking: bool,
    },
    UpdateAccepted {
        plugin_id: String,
        from: String,
        to: String,
    },
    UpdateSkipped {
        plugin_id: String,
        version: String,
    },
    RollbackPerformed {
        plugin_id: String,
        from: String,
        to: String,
    },
    DawConflictDetected {
        daw_name: String,
    },
    PrereqMissing {
        name: String,
    },
    ElevationDenied {
        scope: String,
    },
}

impl Event {
    fn name(&self) -> &'static str {
        match self {
            Event::AppOpened => "app_opened",
            Event::CatalogFetched { .. } => "catalog_fetched",
            Event::InstallStarted { .. } => "install_started",
            Event::InstallCompleted { .. } => "install_completed",
            Event::InstallFailed { .. } => "install_failed",
            Event::UpdateOffered { .. } => "update_offered",
            Event::UpdateAccepted { .. } => "update_accepted",
            Event::UpdateSkipped { .. } => "update_skipped",
            Event::RollbackPerformed { .. } => "rollback_performed",
            Event::DawConflictDetected { .. } => "daw_conflict_detected",
            Event::PrereqMissing { .. } => "prereq_missing",
            Event::ElevationDenied { .. } => "elevation_denied",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct OsInfo {
    name: &'static str,
    build: Option<u32>,
    arch: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct Envelope<'a> {
    event: &'static str,
    install_id: &'a str,
    ts: String,
    launcher_version: &'static str,
    os: OsInfo,
    #[serde(flatten)]
    payload: &'a Event,
}

#[derive(Clone)]
pub struct Telemetry {
    enabled: bool,
    install_id: String,
    endpoint: Option<String>,
    client: reqwest::Client,
}

impl Telemetry {
    pub fn new(enabled: bool, install_id: String, endpoint: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(SEND_TIMEOUT)
            .https_only(true)
            .build()
            .unwrap_or_default();
        Telemetry {
            enabled,
            install_id,
            endpoint,
            client,
        }
    }

    pub fn disabled() -> Self {
        Telemetry::new(false, String::new(), None)
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled && self.endpoint.is_some()
    }

    /// Kirim satu event. Tidak pernah mengembalikan error ke pemanggil, dan
    /// tidak pernah menahan alur — event yang gagal dibuang, bukan ditumpuk.
    pub fn record(&self, event: Event) {
        if !self.is_enabled() {
            return;
        }
        let Some(endpoint) = self.endpoint.clone() else {
            return;
        };

        let envelope = Envelope {
            event: event.name(),
            install_id: &self.install_id,
            ts: crate::registry::now_rfc3339(),
            launcher_version: crate::LAUNCHER_VERSION,
            os: OsInfo {
                name: if cfg!(windows) { "windows" } else { "other" },
                build: crate::prereq::os_build_number(),
                arch: std::env::consts::ARCH,
            },
            payload: &event,
        };

        let Ok(body) = serde_json::to_vec(&envelope) else {
            return;
        };
        let client = self.client.clone();

        tokio::spawn(async move {
            let _ = client
                .post(&endpoint)
                .header("content-type", "application/json")
                .body(body)
                .send()
                .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_telemetry_reports_disabled() {
        assert!(!Telemetry::disabled().is_enabled());
    }

    #[test]
    fn enabled_without_endpoint_still_sends_nothing() {
        let t = Telemetry::new(true, "abc".into(), None);
        assert!(!t.is_enabled());
        // Tidak panik dan tidak butuh runtime karena tidak pernah spawn.
        t.record(Event::AppOpened);
    }

    #[test]
    fn payload_carries_no_paths_or_names() {
        // Pemeriksaan struktural: satu-satunya field bebas di seluruh enum
        // adalah id plugin, versi, dan nama DAW — semuanya berasal dari
        // katalog kami sendiri, bukan dari sistem pengguna.
        let json = serde_json::to_string(&Event::InstallCompleted {
            plugin_id: "mycomp".into(),
            plugin_version: "1.3.0".into(),
            scope: "current_user".into(),
            duration_ms: 8420,
            from_version: Some("1.2.1".into()),
        })
        .unwrap();
        assert!(!json.contains("C:\\"));
        assert!(!json.contains("Users"));
    }
}
