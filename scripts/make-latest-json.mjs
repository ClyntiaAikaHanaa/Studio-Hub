#!/usr/bin/env node
//
// Susun `latest.json` untuk `tauri-plugin-updater` (PRD Â§15.4).
//
// `tauri build` menghasilkan `.sig` untuk setiap bundle; manifest ini merujuk
// URL bundle di GitHub Releases plus isi `.sig`-nya. Signature Ed25519 itulah
// yang diverifikasi launcher sebelum memasang update dirinya sendiri â€” tanpa
// itu, siapa pun yang dapat mengubah manifest dapat mengirim launcher palsu ke
// semua pengguna terpasang.

import { readFile, readdir, writeFile } from "node:fs/promises";
import { join } from "node:path";

const REPO = process.env.GITHUB_REPOSITORY ?? "ClyntiaAikaHanaa/studio-hub";
const TAG = (process.env.GITHUB_REF_NAME ?? "launcher-v1.0.0").trim();
const VERSION = TAG.replace(/^launcher-v/, "");

const BUNDLE_ROOT = join("target", "x86_64-pc-windows-msvc", "release", "bundle");

// NSIS lebih kecil dan lebih fleksibel; MSI lebih baik untuk deployment
// terkelola (Open Question Q4). Updater memakai NSIS kalau ada.
const bundle =
  (await findBundle(join(BUNDLE_ROOT, "nsis"), ".exe")) ??
  (await findBundle(join(BUNDLE_ROOT, "msi"), ".msi"));

if (!bundle) {
  console.error("tidak menemukan bundle NSIS maupun MSI â€” apakah `tauri build` berhasil?");
  process.exit(1);
}

const signature = (await readFile(`${bundle.path}.sig`, "utf8")).trim();
if (!signature) {
  console.error(`${bundle.path}.sig kosong â€” updater akan menolak update ini`);
  process.exit(1);
}

const manifest = {
  version: VERSION,
  notes: await readNotes(),
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      signature,
      url: `https://github.com/${REPO}/releases/download/${TAG}/${bundle.name}`,
    },
  },
};

await writeFile("latest.json", `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`âœ“ latest.json â€” ${VERSION} (${bundle.name})`);

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
