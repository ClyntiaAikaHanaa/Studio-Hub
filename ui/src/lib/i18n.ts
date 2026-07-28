// Struktur i18n disiapkan sejak awal; tidak ada string yang di-hardcode di
// komponen (NFR-4.3). v1 mengirim Bahasa Indonesia dan Bahasa Inggris.

import { writable, derived, get } from "svelte/store";

import id from "../locales/id.json";
import en from "../locales/en.json";

type Dict = Record<string, string>;

const DICTS: Record<string, Dict> = { id, en };

export const locale = writable<string>("id");

/** `t("key", { name: "MyComp" })` → string dengan placeholder tergantikan. */
export const t = derived(locale, ($locale) => {
  const dict = DICTS[$locale] ?? DICTS.id;
  const fallback = DICTS.en;

  return (key: string, vars?: Record<string, string | number>): string => {
    // Kunci yang belum diterjemahkan jatuh ke bahasa Inggris, lalu ke kuncinya
    // sendiri — UI yang menampilkan `library.title` jelek tapi masih dapat
    // dipakai; UI yang menampilkan `undefined` tidak.
    let text = dict[key] ?? fallback[key] ?? key;
    if (vars) {
      for (const [name, value] of Object.entries(vars)) {
        text = text.replaceAll(`{${name}}`, String(value));
      }
    }
    return text;
  };
});

export function setLocale(next: string) {
  locale.set(DICTS[next] ? next : "id");
}

export function translate(key: string, vars?: Record<string, string | number>): string {
  return get(t)(key, vars);
}
