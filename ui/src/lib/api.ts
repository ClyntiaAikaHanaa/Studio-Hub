// Pembungkus bertipe untuk command Tauri (PRD §12.2).
//
// Ini satu-satunya file yang memanggil `invoke`. Semua akses ke sistem melewati
// sini, sehingga jelas terlihat bahwa frontend tidak punya jalur lain ke
// filesystem, jaringan, atau shell (ADR-5).

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  AppEvent,
  CatalogView,
  DiagnosticsSummary,
  HubError,
  InstallPlan,
  InstallScope,
  JobEvent,
  LauncherUpdate,
  LibraryEntry,
  Prefs,
  ProcessHolder,
  UpdateSummary,
} from "./types";

/// Error yang datang dari backend sudah terstruktur; bungkus agar `catch`
/// di pemanggil selalu mendapat bentuk yang sama.
export class BackendError extends Error {
  constructor(public readonly hub: HubError) {
    super(hub.code);
    this.name = "BackendError";
  }
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (raw) {
    if (raw && typeof raw === "object" && "code" in raw) {
      throw new BackendError(raw as HubError);
    }
    throw new BackendError({
      code: "internal",
      correlationId: String(raw).slice(0, 64),
    });
  }
}

// ── Katalog ──────────────────────────────────────────────────────────────
export const catalogGet = (force = false) => call<CatalogView>("catalog_get", { force });

// ── State terpasang ──────────────────────────────────────────────────────
export const libraryList = () => call<LibraryEntry[]>("library_list");
export const updatesList = () => call<UpdateSummary>("updates_list");

// ── Perencanaan & eksekusi ───────────────────────────────────────────────
export const installPlan = (pluginId: string, version?: string, scope?: InstallScope) =>
  call<InstallPlan>("install_plan", {
    args: { pluginId, version: version ?? null, scope: scope ?? null },
  });

export const installStart = (pluginId: string, version?: string, scope?: InstallScope) =>
  call<string>("install_start", {
    args: { pluginId, version: version ?? null, scope: scope ?? null },
  });

export const updateAllStart = (includeBreaking: boolean) =>
  call<string[]>("update_all_start", { includeBreaking });

export const rollbackStart = (pluginId: string) => call<string>("rollback_start", { pluginId });

export const uninstallStart = (pluginId: string, removeUserData: boolean) =>
  call<string[]>("uninstall_start", { args: { pluginId, removeUserData } });

export const jobCancel = (jobId: string) => call<boolean>("job_cancel", { jobId });

// ── Sistem ───────────────────────────────────────────────────────────────
export const dawRunning = () => call<ProcessHolder[]>("daw_running");
export const revealInExplorer = (pluginId: string) => call<void>("reveal_in_explorer", { pluginId });
/// Path lokal ikon yang sudah di-cache backend; `null` kalau tidak ada.
export const pluginIcon = (pluginId: string) => call<string | null>("plugin_icon", { pluginId });
/// Cache gambar README; mengembalikan peta `URL asal → path lokal`.
export const cacheImages = (urls: string[]) =>
  call<Record<string, string>>("cache_images", { urls });
/// `full = false` membuang katalog + gambar; `true` membuang seluruh cache.
export const cacheClear = (full: boolean) => call<void>("cache_clear", { full });
export const logsOpen = () => call<void>("logs_open");
/// Backend yang memvalidasi skema dan host sebelum membukanya (ADR-5).
export const openExternal = (url: string) => call<void>("open_external", { url });
export const diagnosticsSummary = () => call<DiagnosticsSummary>("diagnostics_summary");

// ── Preferensi ───────────────────────────────────────────────────────────
export const prefsGet = () => call<Prefs>("prefs_get");
export const prefsSet = (patch: Partial<Prefs>) => call<Prefs>("prefs_set", { patch });
export const telemetryResetId = () => call<string>("telemetry_reset_id");
export const versionSkip = (pluginId: string, version: string) =>
  call<void>("version_skip", { pluginId, version });

// ── Self-update ──────────────────────────────────────────────────────────
export const launcherUpdateRequired = () =>
  call<LauncherUpdate | null>("launcher_update_required");

// ── Event ────────────────────────────────────────────────────────────────
export const onJobEvent = (handler: (event: JobEvent) => void): Promise<UnlistenFn> =>
  listen<JobEvent>("job", (e) => handler(e.payload));

export const onAppEvent = (handler: (event: AppEvent) => void): Promise<UnlistenFn> =>
  listen<AppEvent>("app", (e) => handler(e.payload));
