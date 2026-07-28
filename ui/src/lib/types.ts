// Tipe yang dibagi frontend & backend (PRD §12.2).
//
// Di M1 tipe ini ditulis tangan agar mengikuti `views.rs`. Sebelum M3, gantikan
// file ini dengan hasil generate `ts-rs` dari Rust — dua definisi yang ditulis
// tangan akan menyimpang, dan penyimpangannya muncul sebagai `undefined` di UI,
// bukan sebagai error kompilasi.

export type UpdateState =
  | { state: "upToDate" }
  | { state: "updateAvailable"; from: string; to: string; breaking: boolean }
  | { state: "aheadOfCatalog" }
  | { state: "unknown" }
  | { state: "skipped" };

export type Health = "ok" | "missing" | "unknown_version";

export type InstallScope =
  | { kind: "current_user" }
  | { kind: "all_users" }
  | { kind: "custom"; path: string };

export interface InstalledView {
  version: string;
  installedAt: string;
  scope: string;
  installDir: string;
  health: Health;
  adopted: boolean;
  hasBackup: boolean;
  backupVersion: string | null;
  skippedVersions: string[];
}

/** Satu versi yang dapat dipilih pengguna di halaman detail. */
export interface VersionOption {
  version: string;
  releasedAt: string | null;
  breaking: boolean;
  security: boolean;
  changelog: string;
  downloadSizeBytes: number;
  isLatest: boolean;
  isInstalled: boolean;
}

export interface PluginView {
  id: string;
  name: string;
  vendor: string;
  category: string;
  categoryLabel: string | null;
  tagline: string;
  description: string;
  /** README repo, sudah dibakukan ke katalog saat ingest. */
  readme: string;
  iconUrl: string | null;
  screenshots: string[];
  homepageUrl: string | null;
  sourceUrl: string | null;
  license: string | null;
  /** Teks lisensi lengkap; dialog instalasi menampilkannya sebelum Install. */
  licenseText: string;
  deprecated: boolean;
  deprecationNotice: string | null;
  latestVersion: string;
  releasedAt: string | null;
  changelog: string;
  breaking: boolean;
  security: boolean;
  downloadSizeBytes: number;
  availableForPlatform: boolean;
  availableVersions: VersionOption[];
  installed: InstalledView | null;
  update: UpdateState;
  commercialModel: string;
}

export interface CatalogView {
  generatedAt: string;
  categories: { id: string; label: string }[];
  plugins: PluginView[];
  stale: boolean;
  lastSuccessAt: string | null;
  skippedEntries: number;
}

export interface LibraryEntry {
  pluginId: string;
  name: string;
  installed: InstalledView;
  update: UpdateState;
  inCatalog: boolean;
  iconUrl: string | null;
}

export interface UpdateItem {
  pluginId: string;
  name: string;
  iconUrl: string | null;
  fromVersion: string;
  toVersion: string;
  releasedAt: string | null;
  breaking: boolean;
  security: boolean;
  changelog: string;
  downloadSizeBytes: number;
}

export interface UpdateSummary {
  items: UpdateItem[];
  nonBreakingCount: number;
  breakingCount: number;
}

export interface ProcessHolder {
  name: string | null;
  executable: string;
  pid: number;
}

export type Blocker =
  | { kind: "insufficientDisk"; required: number; available: number; volume: string }
  | { kind: "cpuFeatureMissing"; feature: string }
  | { kind: "osTooOld"; required: number; current: number | null }
  | { kind: "launcherTooOld"; required: string; current: string }
  | { kind: "noDownloadUrl"; pluginId: string }
  | { kind: "noCompatibleBuild"; target: string }
  | {
      kind: "fileLocked";
      path: string;
      holders: ProcessHolder[];
      rebootOptionAvailable: boolean;
    };

export type Warning =
  | { kind: "breakingChange"; summary: string }
  | { kind: "prereqMissing"; name: string; detail: string; helpUrl: string | null }
  | { kind: "perUserLocationMayNeedDawConfig"; path: string }
  | { kind: "elevationWillBeRequested" }
  | { kind: "replacingAdoptedInstall" }
  | { kind: "rollbackMayBreakPresets" };

export interface CheckResult {
  name: string;
  satisfied: boolean;
  detail: string;
  helpUrl: string | null;
}

export interface PrereqReport {
  vcRedist: CheckResult | null;
  cpuFeatures: CheckResult[];
  osBuild: CheckResult | null;
  disk: {
    volume: string;
    requiredBytes: number;
    availableBytes: number;
    sufficient: boolean;
  } | null;
  blocking: boolean;
}

export interface InstallPlan {
  planId: string;
  pluginId: string;
  pluginName: string;
  fromVersion: string | null;
  toVersion: string;
  breaking: boolean;
  changelog: string;
  download: { url: string; sizeBytes: number; sha256: string; cached: boolean };
  target: { scope: InstallScope; installDir: string; requiresElevation: boolean };
  disk: { requiredBytes: number; availableBytes: number; sufficient: boolean };
  prereqs: PrereqReport;
  blockers: Blocker[];
  warnings: Warning[];
  backupWillBeCreated: boolean;
  userDataPreserved: string[];
}

export type JobEvent =
  | { jobId: string; kind: "queued" }
  | { jobId: string; kind: "downloading"; received: number; total: number; bytesPerSec: number }
  | { jobId: string; kind: "verifying" }
  | { jobId: string; kind: "extracting"; entriesDone: number; entriesTotal: number }
  | { jobId: string; kind: "elevating" }
  | { jobId: string; kind: "installing" }
  | { jobId: string; kind: "backingUp" }
  | { jobId: string; kind: "blocked"; blocker: Blocker }
  | { jobId: string; kind: "rollingBack"; reason: string }
  | { jobId: string; kind: "succeeded"; version: string; needsRescan: boolean }
  | { jobId: string; kind: "failed"; error: HubError }
  | { jobId: string; kind: "cancelled" };

export type AppEvent =
  | { kind: "catalogUpdated"; pluginCount: number; updateCount: number }
  | { kind: "catalogStale"; lastSuccessAt: string | null }
  | { kind: "libraryChanged" };

export type HubError =
  | { code: "network_unreachable"; retryable: boolean; detail: string }
  | { code: "integrity_mismatch"; expected: string; actual: string }
  | { code: "archive_rejected"; reason: string }
  | { code: "file_locked"; path: string; holders: ProcessHolder[] }
  | { code: "elevation_denied" }
  | { code: "insufficient_disk"; required: number; available: number; volume: string }
  | { code: "prereq_missing"; name: string; helpUrl: string | null }
  | { code: "launcher_too_old"; required: string; current: string }
  | { code: "catalog_invalid"; detail: string }
  | { code: "plugin_not_found"; pluginId: string }
  | { code: "no_compatible_build"; pluginId: string; version: string }
  | { code: "not_installed"; pluginId: string }
  | { code: "cancelled" }
  | { code: "internal"; correlationId: string };

export interface Prefs {
  schemaVersion: number;
  locale: string;
  theme: string;
  defaultInstallScope: InstallScope;
  checkUpdatesOnLaunch: boolean;
  telemetryEnabled: boolean;
  telemetryPromptShown: boolean;
  installId: string;
  verboseLogging: boolean;
}

export interface LauncherUpdate {
  currentVersion: string;
  availableVersion: string;
  notes: string;
  security: boolean;
  required: boolean;
}

export interface DiagnosticsSummary {
  launcherVersion: string;
  osBuild: number | null;
  arch: string;
  vcRedist: boolean;
  installedCount: number;
  catalogGeneratedAt: string | null;
  detectedDaws: string[];
  logsDir: string;
}
