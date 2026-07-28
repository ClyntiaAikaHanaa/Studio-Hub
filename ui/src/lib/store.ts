// State aplikasi di sisi frontend.
//
// Prinsip yang dipegang di sini: **tampilkan state terpasang lebih dulu**
// (PRD §8.1). `library` dimuat tanpa menunggu jaringan; `catalog` menyusul dan
// memperkaya tampilan saat tiba.

import { writable, derived, get } from "svelte/store";

import * as api from "./api";
import { BackendError } from "./api";
import { setLocale } from "./i18n";
import type {
  CatalogView,
  HubError,
  JobEvent,
  LibraryEntry,
  Prefs,
  UpdateSummary,
} from "./types";

export type View = "library" | "explore" | "updates" | "settings";

export const view = writable<View>("library");
export const catalog = writable<CatalogView | null>(null);
export const library = writable<LibraryEntry[]>([]);
export const updates = writable<UpdateSummary>({
  items: [],
  nonBreakingCount: 0,
  breakingCount: 0,
});
export const prefs = writable<Prefs | null>(null);
export const catalogError = writable<HubError | null>(null);
export const catalogLoading = writable(false);

/** Job yang sedang berjalan, dipetakan dari `jobId`. */
export const jobs = writable<Record<string, JobEvent>>({});

/** Job terakhir per plugin, agar kartu tahu progres miliknya sendiri. */
export const jobByPlugin = writable<Record<string, string>>({});

export const updateCount = derived(updates, ($u) => $u.items.length);

export async function loadPrefs(): Promise<Prefs> {
  const loaded = await api.prefsGet();
  prefs.set(loaded);
  setLocale(loaded.locale);
  applyTheme(loaded.theme);
  return loaded;
}

export async function savePrefs(patch: Partial<Prefs>): Promise<void> {
  const updated = await api.prefsSet(patch);
  prefs.set(updated);
  if (patch.theme) applyTheme(updated.theme);

  if (patch.locale) {
    setLocale(updated.locale);
    // Tagline dan deskripsi plugin diterjemahkan di backend, bukan di store
    // i18n frontend — keduanya datang dari katalog. Tanpa memuat ulang di
    // sini, mengganti bahasa hanya mengubah teks antarmuka dan menyisakan
    // deskripsi plugin dalam bahasa sebelumnya.
    await refreshCatalog(false);
  }
}

function applyTheme(theme: string) {
  // NFR-4.4: menghormati preferensi OS kecuali pengguna memaksa satu tema.
  const root = document.documentElement;
  if (theme === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", theme);
  }
}

export async function refreshLibrary(): Promise<void> {
  library.set(await api.libraryList());
}

export async function refreshUpdates(): Promise<void> {
  try {
    updates.set(await api.updatesList());
  } catch {
    // Tanpa katalog tidak ada yang bisa dibandingkan; daftar kosong adalah
    // jawaban yang jujur, bukan error yang perlu ditampilkan.
    updates.set({ items: [], nonBreakingCount: 0, breakingCount: 0 });
  }
}

/// Muat ulang katalog **dan** state terpasang.
///
/// Dipakai setelah setiap aksi yang mengubah apa yang terpasang. Tanpa memuat
/// ulang katalog, `PluginView.installed` tetap memakai nilai lama dan Explore
/// masih menampilkan "Get plugin" untuk plugin yang barusan dipasang.
export async function refreshAll(): Promise<void> {
  await refreshLibrary();
  try {
    catalog.set(await api.catalogGet(false));
    await refreshUpdates();
  } catch {
    // Tanpa jaringan, state terpasang tetap yang terbaru — itu yang penting.
  }
}

/// Tombol Refresh: buang cache katalog dan gambar, lalu ambil ulang.
///
/// Menghapus berkasnya lebih tegas daripada sekadar mengabaikan TTL: ikon dan
/// screenshot yang URL-nya berubah di katalog baru ikut terbuang, sehingga
/// tidak ada gambar lama yang bertahan tanpa cara membedakannya dari yang benar.
export async function hardRefresh(): Promise<void> {
  catalogLoading.set(true);
  try {
    await api.cacheClear(false);
  } catch {
    // Cache yang gagal dihapus bukan alasan membatalkan pengambilan ulang.
  } finally {
    catalogLoading.set(false);
  }
  await refreshCatalog(true);
}

/// Kosongkan seluruh cache termasuk artefak yang sudah diunduh, lalu ambil ulang.
export async function clearAllCache(): Promise<void> {
  catalogLoading.set(true);
  try {
    await api.cacheClear(true);
  } finally {
    catalogLoading.set(false);
  }
  await refreshCatalog(true);
}

export async function refreshCatalog(force = false): Promise<void> {
  catalogLoading.set(true);
  try {
    catalog.set(await api.catalogGet(force));
    catalogError.set(null);
    // Library WAJIB dimuat ulang setelah katalog tiba, bukan hanya sebelumnya.
    // Backend baru bisa menentukan bundle mana yang milik kita setelah tahu isi
    // katalog; panggilan pertama (sebelum katalog ada) sengaja tidak mengadopsi
    // apa pun. Tanpa pemanggilan kedua ini, Library berhenti di hasil sementara
    // itu — plugin vendor lain tetap terdaftar dan milik kita tidak pernah
    // dikenali.
    await refreshLibrary();
    await refreshUpdates();
  } catch (e) {
    if (e instanceof BackendError) catalogError.set(e.hub);
  } finally {
    catalogLoading.set(false);
  }
}

/// Urutan startup: cache lokal dulu, jaringan kemudian (NFR-1.5).
export async function bootstrap(): Promise<void> {
  await loadPrefs();
  await refreshLibrary();

  await api.onJobEvent(handleJobEvent);
  await api.onAppEvent(async (event) => {
    // Setiap aksi yang mengubah state terpasang memicu pemuatan ulang penuh:
    // Library, Updates, DAN tampilan katalog. Kalau katalog dilewati, Explore
    // masih menawarkan "Get plugin" untuk plugin yang barusan dipasang.
    if (event.kind === "libraryChanged") {
      await refreshAll();
    }
  });

  const current = get(prefs);
  if (current?.checkUpdatesOnLaunch !== false) {
    void refreshCatalog(false);
  }
}

function handleJobEvent(event: JobEvent) {
  jobs.update((all) => ({ ...all, [event.jobId]: event }));

  // Job yang selesai mengubah apa yang terpasang. `libraryChanged` juga
  // dikirim backend, tapi hanya untuk instalasi yang berhasil — job yang gagal
  // di tengah tetap dapat meninggalkan state yang berbeda dari yang tampil.
  if (event.kind === "succeeded" || event.kind === "failed") {
    void refreshAll();
  }

  if (event.kind === "succeeded" || event.kind === "cancelled") {
    // Beri UI waktu menampilkan state akhir sebelum kartu kembali normal.
    setTimeout(() => {
      jobs.update((all) => {
        const next = { ...all };
        delete next[event.jobId];
        return next;
      });
      jobByPlugin.update((map) => {
        const next = { ...map };
        for (const [pluginId, jobId] of Object.entries(next)) {
          if (jobId === event.jobId) delete next[pluginId];
        }
        return next;
      });
    }, 2500);
  }
}

export function trackJob(pluginId: string, jobId: string) {
  jobByPlugin.update((map) => ({ ...map, [pluginId]: jobId }));
}
