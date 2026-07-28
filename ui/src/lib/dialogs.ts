// State dialog global.
//
// PRD §8.1 prinsip 4: tidak ada aksi destruktif tanpa konfirmasi, dan tidak ada
// konfirmasi untuk aksi yang tidak destruktif. Karena itu hanya dua dialog yang
// ada di sini — pasang/perbarui (menulis ke sistem) dan hapus (menghancurkan) —
// bukan satu dialog untuk setiap tombol.

import { writable } from "svelte/store";

import type { InstallScope } from "./types";

export interface InstallRequest {
  pluginId: string;
  pluginName: string;
  version?: string;
  scope?: InstallScope;
}

export interface UninstallRequest {
  pluginId: string;
  pluginName: string;
  adopted: boolean;
}

export const installRequest = writable<InstallRequest | null>(null);
export const uninstallRequest = writable<UninstallRequest | null>(null);

export function requestInstall(request: InstallRequest) {
  installRequest.set(request);
}

export function requestUninstall(request: UninstallRequest) {
  uninstallRequest.set(request);
}
