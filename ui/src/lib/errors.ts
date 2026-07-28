// Pemetaan error → pesan (PRD §18.3).
//
// Setiap error punya tiga bagian: **apa yang terjadi**, **kenapa**, dan **apa
// yang bisa dilakukan pengguna**. Kode teknis tetap tersedia di balik "Detail",
// tapi tidak menjadi hal pertama yang dilihat.
//
// Pola yang dihindari: menampilkan `error.to_string()` mentah.

import type { HubError } from "./types";
import { translate } from "./i18n";
import { formatBytes } from "./format";

export interface ErrorAction {
  labelKey: string;
  /** Ditangani pemanggil; nilainya menentukan tombol mana yang muncul. */
  action: "retry" | "installPerUser" | "openHelp" | "updateLauncher" | "report" | "viewLog" | "close";
  href?: string;
}

export interface ErrorPresentation {
  title: string;
  body: string;
  actions: ErrorAction[];
  /** Baris teknis yang muncul di balik "Detail". */
  detail: string;
}

export function presentError(error: HubError): ErrorPresentation {
  switch (error.code) {
    case "network_unreachable":
      return {
        title: translate("error.network.title"),
        body: translate("error.network.noCache"),
        actions: error.retryable
          ? [{ labelKey: "common.retry", action: "retry" }]
          : [{ labelKey: "common.close", action: "close" }],
        detail: error.detail,
      };

    case "integrity_mismatch":
      return {
        title: translate("error.integrity.title"),
        body: translate("error.integrity.body"),
        actions: [
          { labelKey: "common.retry", action: "retry" },
          { labelKey: "error.integrity.report", action: "report" },
        ],
        detail: `expected ${error.expected.slice(0, 16)}… · actual ${error.actual.slice(0, 16)}…`,
      };

    case "archive_rejected":
      return {
        title: translate("error.archive.title"),
        body: translate("error.archive.body"),
        actions: [{ labelKey: "error.integrity.report", action: "report" }],
        detail: error.reason,
      };

    case "file_locked": {
      const named = error.holders.map((h) => h.name).filter(Boolean) as string[];
      return {
        title: named.length
          ? translate("blocked.dawRunning.title", { daw: named[0] })
          : translate("blocked.dawUnknown.title"),
        body: named.length
          ? translate("blocked.dawRunning.body")
          : translate("blocked.dawUnknown.body"),
        actions: [{ labelKey: "blocked.closedDaw", action: "retry" }],
        detail: `ERROR_SHARING_VIOLATION · ${error.path}`,
      };
    }

    case "elevation_denied":
      return {
        title: translate("error.elevation.title"),
        body: translate("error.elevation.body"),
        actions: [
          { labelKey: "error.elevation.installPerUser", action: "installPerUser" },
          { labelKey: "common.cancel", action: "close" },
        ],
        detail: "ERROR_CANCELLED (1223)",
      };

    case "insufficient_disk":
      return {
        title: translate("error.disk.title"),
        body: translate("error.disk.body", {
          required: formatBytes(error.required),
          available: formatBytes(error.available),
          volume: error.volume,
        }),
        actions: [{ labelKey: "common.cancel", action: "close" }],
        detail: `${error.required} / ${error.available} bytes`,
      };

    case "prereq_missing":
      return {
        title: translate("error.prereq.title"),
        body: translate("error.prereq.body", { name: error.name }),
        actions: error.helpUrl
          ? [
              { labelKey: "error.prereq.download", action: "openHelp", href: error.helpUrl },
              { labelKey: "common.cancel", action: "close" },
            ]
          : [{ labelKey: "common.cancel", action: "close" }],
        detail: error.name,
      };

    case "launcher_too_old":
      return {
        title: translate("error.launcherTooOld.title"),
        body: translate("error.launcherTooOld.body", { required: error.required }),
        actions: [{ labelKey: "error.launcherTooOld.update", action: "updateLauncher" }],
        detail: `required ${error.required}, current ${error.current}`,
      };

    case "catalog_invalid":
      return {
        title: translate("error.catalog.title"),
        body: translate("error.catalog.body"),
        actions: [
          { labelKey: "common.retry", action: "retry" },
          { labelKey: "common.viewLog", action: "viewLog" },
        ],
        detail: error.detail,
      };

    case "plugin_not_found":
    case "no_compatible_build":
    case "not_installed":
      return {
        title: translate("error.title"),
        body: translate("explore.unavailable"),
        actions: [{ labelKey: "common.close", action: "close" }],
        detail: JSON.stringify(error),
      };

    case "cancelled":
      return {
        title: translate("error.cancelled"),
        body: "",
        actions: [],
        detail: "",
      };

    case "internal":
      return {
        title: translate("error.internal.title"),
        body: translate("error.internal.body", { id: error.correlationId }),
        actions: [
          { labelKey: "common.viewLog", action: "viewLog" },
          { labelKey: "error.integrity.report", action: "report" },
        ],
        detail: error.correlationId,
      };
  }
}
