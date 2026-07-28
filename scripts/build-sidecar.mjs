#!/usr/bin/env node
//
// Bangun `hub-helper` dan letakkan sebagai sidecar Tauri.
//
// Tauri menuntut sidecar dinamai `<nama>-<target-triple><ext>`, lalu
// membuang sufiks triple-nya saat membundel — sehingga di mesin pengguna ia
// mendarat sebagai `hub-helper.exe` di samping `StudioHub.exe`. Itu persis
// yang dicari `default_helper_path()`.
//
// Tanpa langkah ini, installer hanya berisi `StudioHub.exe`. Aplikasinya jalan
// dan terlihat sehat sampai seseorang memilih instalasi "All users", dan baru
// di situ ia gagal karena helper elevated-nya tidak pernah ikut terkirim.
// `npm run dev` tidak pernah menunjukkan masalah ini: di sana kedua biner
// kebetulan berada di direktori `target` yang sama.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const profile = process.argv.includes("--debug") ? "debug" : "release";

// Tanpa `shell: true`: `cargo` dan `rustc` adalah .exe sungguhan, jadi libuv
// menemukannya sendiri. Melewatkan argumen melalui shell hanya menambah
// permukaan injeksi tanpa memberi apa pun di sini.
function run(cmd, args) {
  execFileSync(cmd, args, { cwd: root, stdio: "inherit" });
}

// Triple host dibaca dari rustc, bukan ditebak dari process.platform: mesin
// yang sama bisa punya beberapa toolchain, dan nama berkas harus cocok persis
// dengan yang dicari Tauri.
const hostTriple = execFileSync("rustc", ["-vV"], { encoding: "utf8" })
  .split("\n")
  .find((l) => l.startsWith("host:"))
  ?.slice(5)
  .trim();

if (!hostTriple) {
  console.error("tidak dapat menentukan target triple dari `rustc -vV`");
  process.exit(1);
}

const args = ["build", "-p", "hub-helper"];
if (profile === "release") args.push("--release");
run("cargo", args);

const ext = process.platform === "win32" ? ".exe" : "";
const built = join(root, "target", profile, `hub-helper${ext}`);
const outDir = join(root, "src-tauri", "binaries");
const dest = join(outDir, `hub-helper-${hostTriple}${ext}`);

mkdirSync(outDir, { recursive: true });
copyFileSync(built, dest);

console.log(`sidecar siap: src-tauri/binaries/hub-helper-${hostTriple}${ext} (${profile})`);
