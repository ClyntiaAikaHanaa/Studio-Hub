#!/usr/bin/env node
//
// Naikkan versi Studio Hub di semua tempat sekaligus.
//
//     node scripts/bump-version.mjs 1.0.1
//
// Angka versi tinggal di empat berkas plus tiga lockfile. Menaikkannya manual
// berarti tujuh kesempatan untuk lupa satu, dan yang paling berbahaya adalah
// lupa `tauri.conf.json`: installer akan membawa versi lama sementara manifest
// updater mengumumkan versi baru, dan setiap pengguna terjebak memasang update
// yang sama berulang kali tanpa pesan error apa pun.

import { execFileSync } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const version = process.argv[2];

if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) {
  console.error("pemakaian: node scripts/bump-version.mjs <major.minor.patch>");
  console.error("contoh   : node scripts/bump-version.mjs 1.0.1");
  process.exit(1);
}

// JSON diedit sebagai teks, bukan lewat parse lalu stringify. Menulis ulang
// hasil parse akan membuang komentar posisi kunci dan memformat ulang seluruh
// berkas, sehingga diff-nya jadi ratusan baris untuk satu angka yang berubah.
async function patchJson(relative) {
  const path = join(root, relative);
  const before = await readFile(path, "utf8");
  const after = before.replace(/("version"\s*:\s*)"\d+\.\d+\.\d+"/, `$1"${version}"`);
  if (before === after) {
    console.error(`tidak menemukan field version di ${relative}`);
    process.exit(1);
  }
  await writeFile(path, after);
  console.log(`  ${relative}`);
}

async function patchCargoWorkspace() {
  const path = join(root, "Cargo.toml");
  const before = await readFile(path, "utf8");
  // Hanya `version` di dalam [workspace.package]; versi dependensi di bagian
  // lain berkas ini tidak boleh ikut tersentuh.
  const after = before.replace(
    /(\[workspace\.package\][\s\S]*?\nversion\s*=\s*)"\d+\.\d+\.\d+"/,
    `$1"${version}"`,
  );
  if (before === after) {
    console.error("tidak menemukan version di [workspace.package] pada Cargo.toml");
    process.exit(1);
  }
  await writeFile(path, after);
  console.log("  Cargo.toml");
}

console.log(`Menaikkan versi ke ${version}:`);
await patchJson("package.json");
await patchJson(join("ui", "package.json"));
await patchJson(join("src-tauri", "tauri.conf.json"));
await patchCargoWorkspace();

// Lockfile ikut memuat versi paket. Kalau tertinggal, `npm ci` di CI menolak
// jalan karena package.json dan lockfile-nya tidak sinkron.
// Lockfile npm disunting langsung, bukan lewat `npm install
// --package-lock-only`. Di Windows `npm` adalah npm.cmd, dan Node modern
// menolak menjalankan berkas .cmd tanpa shell; memakai shell hanya untuk ini
// berarti menggabungkan argumen jadi satu baris perintah tanpa escaping.
// Yang perlu diubah cuma dua field, dan lockfile memang berkas mesin sehingga
// menulis ulang hasil parse-nya tidak menghilangkan apa pun.
async function patchLockfile(relative) {
  const path = join(root, relative);
  const lock = JSON.parse(await readFile(path, "utf8"));
  lock.version = version;
  if (lock.packages?.[""]) lock.packages[""].version = version;
  await writeFile(path, `${JSON.stringify(lock, null, 2)}\n`);
  console.log(`  ${relative}`);
}

console.log("Menyegarkan lockfile:");
await patchLockfile("package-lock.json");
await patchLockfile(join("ui", "package-lock.json"));

// Cargo justru sebaliknya: ia .exe sungguhan, dan `--workspace` membatasi
// pembaruan hanya pada crate milik kita, tidak menyentuh dependensi.
execFileSync("cargo", ["update", "--workspace", "--quiet"], { cwd: root, stdio: "inherit" });
console.log("  Cargo.lock");

console.log(`
Selesai. Langkah berikutnya:

  git add -A
  git commit -m "Studio Hub ${version}"
  git push origin main
  git tag launcher-v${version}
  git push origin launcher-v${version}
`);
