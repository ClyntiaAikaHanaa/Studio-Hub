//! Emitter event ke frontend (PRD §12.4).

use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub const JOB_EVENT: &str = "job";
pub const APP_EVENT: &str = "app";

/// Setiap event job membawa `jobId` agar UI dapat mengarahkannya ke elemen yang
/// benar — beberapa job dapat berjalan bersamaan ("Update all").
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobEnvelope {
    pub job_id: String,
    #[serde(flatten)]
    pub event: hub_core::install::JobEvent,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AppEvent {
    #[serde(rename_all = "camelCase")]
    CatalogUpdated {
        plugin_count: usize,
        update_count: usize,
    },
    #[serde(rename_all = "camelCase")]
    CatalogStale {
        last_success_at: Option<String>,
    },
    LibraryChanged,
}

pub fn emit_job(app: &AppHandle, job_id: &str, event: hub_core::install::JobEvent) {
    let envelope = JobEnvelope {
        job_id: job_id.to_string(),
        event,
    };
    if let Err(e) = app.emit(JOB_EVENT, &envelope) {
        // Kegagalan emit tidak boleh menggagalkan instalasi yang sedang
        // berjalan — paling buruk UI kehilangan satu frame progres.
        tracing::warn!(error = %e, "gagal mengirim event job");
    }
}

pub fn emit_app(app: &AppHandle, event: AppEvent) {
    if let Err(e) = app.emit(APP_EVENT, &event) {
        tracing::warn!(error = %e, "gagal mengirim event aplikasi");
    }
}
