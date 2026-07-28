import { get } from "svelte/store";

import { locale } from "./i18n";

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 MB";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  // Byte dan KB tidak butuh desimal; MB ke atas butuh satu agar "12.4 MB"
  // tidak menjadi "12 MB" yang terasa dibulatkan seenaknya.
  const digits = unit <= 1 ? 0 : 1;
  return `${value.toFixed(digits)} ${units[unit]}`;
}

export function formatSpeed(bytesPerSec: number): string {
  return `${formatBytes(bytesPerSec)}/s`;
}

export function formatDate(iso: string | null): string {
  if (!iso) return "—";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "—";
  return date.toLocaleDateString(get(locale) === "en" ? "en-GB" : "id-ID", {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export function formatDateTime(iso: string | null): string {
  if (!iso) return "—";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "—";
  return date.toLocaleString(get(locale) === "en" ? "en-GB" : "id-ID", {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

export function percent(received: number, total: number): number {
  if (!total) return 0;
  return Math.min(100, Math.round((received / total) * 100));
}
