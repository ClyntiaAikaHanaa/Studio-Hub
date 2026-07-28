// Transisi bersama untuk perpindahan halaman.
//
// Tiga aturan yang dipegang di sini, dan alasannya:
//
// 1. **Hanya transisi masuk.** Elemen lama dan baru tidak pernah hidup
//    bersamaan, jadi tidak ada lompatan tata letak saat keduanya menempati
//    ruang yang sama. Crossfade terlihat mewah di demo dan berantakan di
//    aplikasi yang isinya setinggi layar.
//
// 2. **Cepat.** 200–280 ms. Di bawah itu terasa seperti kedipan; di atas itu
//    aplikasi terasa lambat, dan launcher yang baik adalah launcher yang cepat
//    ditutup.
//
// 3. **Menghormati `prefers-reduced-motion`.** Aturan CSS global tidak
//    berlaku untuk transisi Svelte — keduanya berjalan di JavaScript — jadi
//    pemeriksaannya harus eksplisit di sini. Gerakan yang tidak bisa dimatikan
//    bukan sekadar mengganggu; bagi sebagian orang ia memicu mual.

import { cubicOut } from "svelte/easing";
import { fly, type TransitionConfig } from "svelte/transition";

function reduced(): boolean {
  return (
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true
  );
}

/** Perpindahan antar tab: naik sedikit sambil memudar masuk. */
export function pageIn(node: Element): TransitionConfig {
  if (reduced()) return { duration: 0 };
  return fly(node, { y: 12, duration: 260, opacity: 0, easing: cubicOut });
}

/**
 * Kartu di grid, muncul berurutan.
 *
 * Jedanya dibatasi delapan kartu: tanpa batas, katalog dengan tiga puluh
 * plugin butuh lebih dari satu detik sebelum kartu terakhir terlihat — dan
 * yang tadinya terasa halus berubah jadi terasa lambat.
 */
export function cardIn(node: Element, { index = 0 }: { index?: number } = {}): TransitionConfig {
  if (reduced()) return { duration: 0 };
  return fly(node, {
    y: 14,
    duration: 300,
    delay: Math.min(index, 8) * 40,
    opacity: 0,
    easing: cubicOut,
  });
}

/**
 * Kartu yang dihapus dari daftar: memudar dan bergeser keluar sambil
 * **mengatupkan ruangnya sendiri**.
 *
 * Tinggi, padding, dan margin ikut dianimasikan ke nol. Tanpa itu, kartu
 * memudar di tempat lalu ruangnya hilang seketika, dan seluruh daftar di
 * bawahnya menyentak naik — persis kesan berantakan yang hendak dihindari
 * animasi ini.
 *
 * Nilai awalnya dibaca dari elemen yang sudah dirender, bukan ditulis tetap:
 * tinggi kartu bergantung pada panjang nama plugin dan jumlah tombolnya.
 */
export function cardOut(node: Element): TransitionConfig {
  if (reduced()) return { duration: 0 };

  const style = getComputedStyle(node);
  const height = parseFloat(style.height);
  const marginBottom = parseFloat(style.marginBottom);
  const paddingTop = parseFloat(style.paddingTop);
  const paddingBottom = parseFloat(style.paddingBottom);
  const borderTop = parseFloat(style.borderTopWidth);
  const borderBottom = parseFloat(style.borderBottomWidth);

  return {
    duration: 320,
    easing: cubicOut,
    css: (t, u) => `
      overflow: hidden;
      opacity: ${t};
      transform: translateX(${u * 28}px);
      height: ${t * height}px;
      padding-top: ${t * paddingTop}px;
      padding-bottom: ${t * paddingBottom}px;
      margin-bottom: ${t * marginBottom}px;
      border-top-width: ${t * borderTop}px;
      border-bottom-width: ${t * borderBottom}px;
    `,
  };
}

/** Durasi geser kartu tersisa ke posisi barunya. Sepadan dengan `cardOut`. */
export const REFLOW_MS = 300;

/**
 * Masuk ke halaman detail: geser dari kanan.
 *
 * Arahnya berbeda dari perpindahan tab dengan sengaja — bergerak ke samping
 * membaca sebagai "masuk lebih dalam", sedangkan bergerak ke atas membaca
 * sebagai "berganti tempat". Perbedaan itu yang membuat tombol kembali terasa
 * wajar.
 */
export function detailIn(node: Element): TransitionConfig {
  if (reduced()) return { duration: 0 };
  return fly(node, { x: 24, duration: 260, opacity: 0, easing: cubicOut });
}
