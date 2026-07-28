<script lang="ts">
  // Halaman detail plugin: README dari repo + pemilih versi.
  //
  // README dibakukan ke katalog saat ingest, bukan diambil saat halaman dibuka.
  // Konsekuensinya halaman ini tetap terbaca offline, dan membuka sebuah plugin
  // tidak memicu satu pun request jaringan.

  import { untrack } from "svelte";

  import { requestInstall } from "../lib/dialogs";
  import { t } from "../lib/i18n";
  import { formatBytes, formatDate } from "../lib/format";
  import { convertFileSrc } from "@tauri-apps/api/core";

  import * as api from "../lib/api";
  import { extractImageUrls, renderMarkdown } from "../lib/markdown";
  import { jobByPlugin, jobs } from "../lib/store";
  import type { PluginView } from "../lib/types";
  import JobProgress from "../components/JobProgress.svelte";
  import PluginIcon from "../components/PluginIcon.svelte";

  interface Props {
    plugin: PluginView;
    onback: () => void;
  }

  let { plugin, onback }: Props = $props();

  let selected = $state<string | null>(null);

  // `<select>` harus punya nilai awal yang benar-benar cocok dengan salah satu
  // `<option>`. Dibiarkan `null`, browser menampilkannya kosong — terlihat
  // seperti dropdown rusak, padahal daftarnya terisi. Nilai awalnya versi
  // pertama, yaitu yang paling baru.
  $effect(() => {
    const first = plugin.availableVersions[0]?.version ?? null;
    untrack(() => {
      const stillValid =
        selected !== null && plugin.availableVersions.some((v) => v.version === selected);
      if (!stillValid) selected = first;
    });
  });

  let chosen = $derived(
    plugin.availableVersions.find((v) => v.version === selected) ??
      plugin.availableVersions[0]
  );

  let job = $derived.by(() => {
    const id = $jobByPlugin[plugin.id];
    return id ? ($jobs[id] ?? null) : null;
  });

  // Gambar README diunduh dan divalidasi backend lebih dulu; renderer hanya
  // menampilkan yang ada di peta ini. Selama peta masih kosong, gambarnya
  // tampil sebagai teks alt — bukan sebagai ikon rusak.
  let images = $state<Record<string, string>>({});

  // README hampir selalu dibuka dengan logo yang sama dengan yang sudah tampil
  // di header halaman ini. Membiarkannya berarti logo muncul dua kali beruntun,
  // yang terlihat seperti kesalahan render.
  let readmeBody = $derived.by(() => {
    const source = plugin.readme || plugin.description;
    if (!plugin.iconUrl) return source;
    return source
      .split("\n")
      .filter((line) => !(line.includes("![") && line.includes(plugin.iconUrl!)))
      .join("\n");
  });

  $effect(() => {
    const readme = readmeBody;
    const urls = extractImageUrls(readme);
    if (urls.length === 0) {
      untrack(() => (images = {}));
      return;
    }

    let cancelled = false;
    api
      .cacheImages(urls)
      .then((map) => {
        if (cancelled) return;
        const resolved: Record<string, string> = {};
        for (const [url, path] of Object.entries(map)) {
          resolved[url] = convertFileSrc(path);
        }
        untrack(() => (images = resolved));
      })
      .catch(() => {});

    return () => {
      cancelled = true;
    };
  });

  // Repo GitHub plugin ini. `sourceUrl` yang diutamakan karena itu memang repo
  // kode sumbernya; `homepageUrl` dipakai kalau ternyata hanya itu yang ada.
  let repoUrl = $derived(plugin.sourceUrl ?? plugin.homepageUrl);

  let starFailed = $state(false);

  // Membuka repo di browser, bukan menekan tombol bintang atas nama pengguna.
  // Memberi bintang memerlukan token GitHub, dan meminta kredensial hanya untuk
  // ini akan jauh lebih memaksa daripada nilainya. Membuka halamannya membiarkan
  // pengguna memutuskan, dengan sesi login yang sudah mereka punya.
  async function openRepo() {
    if (!repoUrl) return;
    starFailed = false;
    try {
      await api.openExternal(repoUrl);
    } catch {
      // Host di luar allowlist, atau tidak ada browser default. URL-nya sudah
      // tampil di bagian bawah halaman ini, jadi masih ada jalan manual.
      starFailed = true;
    }
  }

  function install() {
    requestInstall({
      pluginId: plugin.id,
      pluginName: plugin.name,
      // `undefined` berarti "latest" bagi backend; kirim versi eksplisit hanya
      // kalau pengguna benar-benar memilih yang bukan terbaru.
      version: chosen && !chosen.isLatest ? chosen.version : undefined,
    });
  }
</script>

<div class="detail">
  <button class="ghost back" onclick={onback}>← {$t("nav.explore")}</button>

  <header>
    <PluginIcon pluginId={plugin.id} name={plugin.name} size={72} />
    <div class="titles">
      <h1>{plugin.name}</h1>
      <div class="muted">
        {plugin.vendor}
        {#if plugin.categoryLabel}· {plugin.categoryLabel}{/if}
        {#if plugin.license}· {plugin.license}{/if}
      </div>
      <p class="tagline">{plugin.tagline}</p>
    </div>
  </header>

  {#if plugin.deprecated}
    <div class="notice warn">
      {$t("explore.deprecated")}
      {#if plugin.deprecationNotice}— {plugin.deprecationNotice}{/if}
    </div>
  {/if}

  <div class="panes">
  <section class="install-panel card">
    {#if !plugin.availableForPlatform || plugin.availableVersions.length === 0}
      <p class="muted">{$t("explore.unavailable")}</p>
    {:else if job}
      <JobProgress {job} />
    {:else}
      <div class="version-row">
        <label for="version-select">{$t("detail.chooseVersion")}</label>
        <select id="version-select" bind:value={selected}>
          {#each plugin.availableVersions as option (option.version)}
            <option value={option.version}>
              {option.version}
              {#if option.isLatest}· {$t("detail.latest")}{/if}
              {#if option.isInstalled}· {$t("explore.installed")}{/if}
            </option>
          {/each}
        </select>
      </div>

      {#if chosen}
        <div class="meta muted small">
          {formatBytes(chosen.downloadSizeBytes)}
          {#if chosen.releasedAt}
            · {$t("updates.releasedOn", { date: formatDate(chosen.releasedAt) })}
          {/if}
        </div>

        <div class="badges">
          {#if chosen.security}
            <span class="badge security">{$t("updates.security")}</span>
          {/if}
          {#if chosen.breaking}
            <span class="badge breaking">{$t("updates.breaking")}</span>
          {/if}
          {#if !chosen.isLatest}
            <!-- Memasang versi lama adalah pilihan sah (Persona B), tapi
                 pengguna harus tahu ia sedang meninggalkan yang terbaru. -->
            <span class="badge warn">{$t("detail.olderVersion")}</span>
          {/if}
        </div>

        {#if chosen.changelog}
          <details class="changelog">
            <summary class="small">{$t("detail.changelog")}</summary>
            <div class="prose selectable">{@html renderMarkdown(chosen.changelog)}</div>
          </details>
        {/if}

        <button class="primary get" onclick={install} disabled={chosen.isInstalled}>
          {chosen.isInstalled ? $t("explore.installed") : $t("common.getPlugin")}
        </button>
      {/if}
    {/if}

    <!-- Di luar rantai `{#if}` di atas: repo tetap layak dibuka meskipun
         plugin ini belum tersedia untuk platform pengguna atau sedang dipasang. -->
    {#if repoUrl}
      <button class="ghost star" onclick={openRepo}>
        <svg
          class="star-icon"
          viewBox="0 0 24 24"
          width="15"
          height="15"
          aria-hidden="true"
          focusable="false"
        >
          <path
            fill="currentColor"
            d="M12 2.6l2.9 5.9 6.5.9-4.7 4.6 1.1 6.4-5.8-3-5.8 3 1.1-6.4L2.6 9.4l6.5-.9z"
          />
        </svg>
        {$t("detail.starOnGithub")}
      </button>
      {#if starFailed}
        <p class="small muted star-failed">{$t("detail.starFailed")}</p>
      {/if}
    {/if}
  </section>

  <section class="readme card">
    <!-- Dirender lewat renderer allowlist: tidak ada HTML mentah atau link
         dari katalog, dan gambar hanya yang sudah divalidasi backend. -->
    <div class="prose selectable">
      {@html renderMarkdown(readmeBody, images)}
    </div>
  </section>
  </div>

  {#if plugin.sourceUrl}
    <p class="small muted selectable source">{plugin.sourceUrl}</p>
  {/if}
</div>

<style>
  /* Lebar mengikuti jendela, bukan angka tetap.
     Di jendela lebar, panel instalasi pindah ke kolom kanan yang menempel
     saat digulir — sebelumnya seluruh sisi kanan kosong sementara README
     terjepit di kolom sempit. */
  .detail {
    max-width: 1280px;
    margin: 0 auto;
  }

  .panes {
    display: grid;
    gap: 16px;
    align-items: start;
  }

  @media (min-width: 1040px) {
    .panes {
      grid-template-columns: minmax(0, 1fr) 340px;
    }

    .panes > .install-panel {
      order: 2;
      position: sticky;
      top: 0;
    }

    .panes > .readme {
      order: 1;
    }
  }

  .back {
    margin-bottom: 10px;
    padding-left: 0;
  }

  header {
    display: flex;
    gap: 16px;
    align-items: flex-start;
    margin-bottom: 16px;
  }

  .titles h1 {
    margin: 0 0 2px;
  }

  .tagline {
    margin: 8px 0 0;
  }

  .install-panel {
    padding: 14px 16px;
  }

  .version-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .version-row label {
    color: var(--text-muted);
    font-size: 13px;
    white-space: nowrap;
  }

  .version-row select {
    max-width: 280px;
  }

  .meta {
    margin-top: 8px;
  }

  .badges {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-top: 8px;
  }

  .badges:empty {
    display: none;
  }

  .changelog {
    margin-top: 10px;
  }

  .changelog summary {
    cursor: pointer;
    color: var(--text-muted);
  }

  .get {
    margin-top: 12px;
  }

  /* Lebar penuh seperti tombol Get, supaya keduanya terbaca sebagai satu
     kelompok aksi dan bukan tombol yang tercecer. */
  .star {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    width: 100%;
    margin-top: 8px;
  }

  /* Bintangnya ikut warna teks tombol, jadi ia tetap kontras di kedua tema
     tanpa warna yang ditulis terpisah. */
  .star-icon {
    flex: 0 0 auto;
  }

  .star-failed {
    margin-top: 6px;
  }

  .readme {
    padding: 18px 20px;
  }

  .prose {
    font-size: 13.5px;
  }

  .source {
    margin-top: 10px;
    word-break: break-all;
  }
</style>
