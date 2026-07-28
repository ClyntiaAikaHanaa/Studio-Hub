#!/usr/bin/env node
//
// Susun `latest.json` untuk `tauri-plugin-updater` (PRD §15.4).
//
// `tauri build` menghasilkan `.sig` untuk setiap bundle; manifest ini merujuk
// URL bundle di GitHub Releases plus isi `.sig`-nya. Signature Ed25519 itulah
// yang diverifikasi launcher sebelum memasang update dirinya sendiri — tanpa
// itu, siapa pun yang dapat mengubah manifest dapat mengirim launcher palsu ke
// semua pengguna terpasang.

import { readFile, readdir, writeFile } from "node:fs/promises";
import { join } from "node:path";

const REPO = process.env.GITHUB_REPOSITORY ?? "ClyntiaAikaHanaa/Studio-Hub";

const conf = JSON.parse(await readFile(join("src-tauri", "tauri.conf.json"), "utf8"));

// Di CI, `GITHUB_REF_NAME` adalah tag yang memicu workflow. Di luar CI kita
// jatuh ke versi di config, bukan ke angka yang ditulis tangan: nilai tetap
// seperti "launcher-v1.0.0" akan basi pada rilis pertama berikutnya dan
// membuat skrip ini menolak jalan di mesin sendiri tanpa alasan yang jelas.
const TAG = (process.env.GITHUB_REF_NAME ?? `launcher-v${conf.version}`).trim();
const VERSION = TAG.replace(/^launcher-v/, "");

// Versi di tag HARUS sama dengan versi di tauri.conf.json.
//
// Manifest ini mengumumkan versi dari tag, sedangkan installer yang diunduh
// pengguna membawa versi dari tauri.conf.json. Kalau keduanya berbeda,
// updater akan mengunduh dan memasang installer, melihat versinya masih lebih
// rendah dari yang diumumkan, lalu menawarkan update yang sama lagi. Setiap
// pengguna terjebak dalam lingkaran itu, dan tidak ada pesan error di mana pun
// yang menjelaskan kenapa.
//
// Gagal keras di sini jauh lebih murah daripada menemukannya setelah rilis.
if (conf.version !== VERSION) {
  console.error(
    `versi tidak cocok: tag "${TAG}" berarti ${VERSION}, ` +
      `tapi src-tauri/tauri.conf.json berisi ${conf.version}.\n` +
      `Jalankan \`node scripts/bump-version.mjs ${VERSION}\`, commit, lalu tag ulang.`,
  );
  process.exit(1);
}

// `tauri build --target <triple>` menaruh bundle di bawah triple-nya, sedangkan
// `tauri build` biasa menaruhnya di `target/release`. Keduanya dicari supaya
// skrip ini bekerja di CI maupun saat dijalankan manual di mesin sendiri.
const BUNDLE_ROOTS = [
  join("target", "x86_64-pc-windows-msvc", "release", "bundle"),
  join("target", "release", "bundle"),
];

// NSIS lebih kecil dan lebih fleksibel; MSI lebih baik untuk deployment
// terkelola (Open Question Q4). Updater memakai NSIS kalau ada.
let bundle = null;
for (const root of BUNDLE_ROOTS) {
  bundle =
    (await findBundle(join(root, "nsis"), ".exe")) ??
    (await findBundle(join(root, "msi"), ".msi"));
  if (bundle) break;
}

if (!bundle) {
  console.error("tidak menemukan bundle NSIS maupun MSI — apakah `tauri build` berhasil?");
  process.exit(1);
}

// Berkas .sig hilang adalah kegagalan yang paling sering terjadi di sini, dan
// stack trace Node tidak menjelaskan apa pun tentang penyebabnya. Ketiga sebab
// di bawah adalah semua yang pernah kami temui.
let signature;
try {
  signature = (await readFile(`${bundle.path}.sig`, "utf8")).trim();
} catch {
  console.error(
    `${bundle.path}.sig tidak ada.\n` +
      "Tauri hanya menulisnya kalau ketiga hal ini terpenuhi:\n" +
      "  1. bundle.createUpdaterArtifacts = true di tauri.conf.json\n" +
      "  2. plugins.updater.pubkey terisi\n" +
      "  3. TAURI_SIGNING_PRIVATE_KEY dan _PASSWORD tersedia saat build",
  );
  process.exit(1);
}

if (!signature) {
  console.error(`${bundle.path}.sig kosong — updater akan menolak update ini`);
  process.exit(1);
}

const manifest = {
  version: VERSION,
  notes: await readNotes(),
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      signature,
      url: `https://github.com/${REPO}/releases/download/${TAG}/${assetName(bundle.name)}`,
    },
  },
};

await writeFile("latest.json", `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`✓ latest.json — ${VERSION} (${bundle.name})`);

/// Nama aset seperti yang akan ada di GitHub, bukan seperti di disk.
///
/// `productName` di sini adalah "Studio Hub", jadi installer-nya bernama
/// `Studio Hub_1.0.0_x64-setup.exe`. GitHub tidak menerima nama aset dengan
/// spasi dan mengganti setiap karakter di luar [A-Za-z0-9._-] dengan titik saat
/// diunggah, sehingga di sana ia menjadi `Studio.Hub_1.0.0_x64-setup.exe`.
///
/// Menuliskan nama aslinya ke manifest membuat URL-nya menunjuk berkas yang
/// tidak pernah ada, dan updater gagal dengan 404 pada setiap pembaruan. Ini
/// tidak terlihat pada rilis pertama: versi di manifest sama dengan versi yang
/// terpasang, jadi tidak ada yang pernah mencoba mengunduhnya.
function assetName(name) {
  return name.replace(/[^A-Za-z0-9._-]+/g, ".");
}

async function findBundle(dir, extension) {
  try {
    const names = await readdir(dir);
    const name = names.find((n) => n.endsWith(extension));
    return name ? { name, path: join(dir, name) } : null;
  } catch {
    return null;
  }
}

async function readNotes() {
  try {
    return (await readFile("CHANGELOG-latest.md", "utf8")).trim();
  } catch {
    return `Studio Hub ${VERSION}`;
  }
}
