<script lang="ts">
  // Layar Explore (PRD §8.4) — menjawab "apa lagi yang tersedia?"
  //
  // Kartu tidak lagi memasang langsung. Kliknya membuka halaman detail berisi
  // README dan pemilih versi, karena keputusan "versi mana" tidak bisa diambil
  // dari kartu seukuran ini — dan memasang versi lama adalah kebutuhan nyata
  // Persona B (project lama yang tidak boleh berubah suaranya).

  import { t } from "../lib/i18n";
  import { formatBytes } from "../lib/format";
  import { catalog, catalogError, catalogLoading, jobByPlugin, jobs } from "../lib/store";
  import { cardIn, detailIn } from "../lib/motion";
  import type { PluginView } from "../lib/types";
  import JobProgress from "../components/JobProgress.svelte";
  import PluginIcon from "../components/PluginIcon.svelte";
  import PluginDetail from "./PluginDetail.svelte";

  let query = $state("");
  let category = $state("");
  let selectedId = $state<string | null>(null);

  let plugins = $derived.by(() => {
    const all = $catalog?.plugins ?? [];
    const needle = query.trim().toLowerCase();
    return all.filter((plugin) => {
      if (category && plugin.category !== category) return false;
      if (!needle) return true;
      return (
        plugin.name.toLowerCase().includes(needle) ||
        plugin.tagline.toLowerCase().includes(needle) ||
        plugin.vendor.toLowerCase().includes(needle)
      );
    });
  });

  // Dicari ulang dari katalog, bukan disimpan sebagai objek: katalog dapat
  // diperbarui saat halaman detail terbuka, dan menyimpan salinan lama akan
  // menampilkan versi yang sudah tidak ada.
  let selected = $derived<PluginView | null>(
    selectedId ? ($catalog?.plugins.find((p) => p.id === selectedId) ?? null) : null
  );
</script>

{#if selected}
  <div in:detailIn>
    <PluginDetail plugin={selected} onback={() => (selectedId = null)} />
  </div>
{:else}
  <div class="explore">
  <h1>{$t("explore.title")}</h1>

  <div class="filters">
    <input
      type="search"
      placeholder={$t("explore.search")}
      aria-label={$t("explore.search")}
      bind:value={query}
    />
    <select bind:value={category} aria-label={$t("explore.allCategories")}>
      <option value="">{$t("explore.allCategories")}</option>
      {#each $catalog?.categories ?? [] as cat (cat.id)}
        <option value={cat.id}>{cat.label}</option>
      {/each}
    </select>
  </div>

  {#if $catalogLoading && !$catalog}
    <p class="muted">{$t("common.loading")}</p>
  {:else if $catalogError && !$catalog}
    <div class="notice danger">{$t("error.network.noCache")}</div>
  {:else if plugins.length === 0}
    <p class="muted">{$t("explore.noResults")}</p>
  {:else}
    <div class="grid">
      {#each plugins as plugin, i (plugin.id)}
        {@const jobId = $jobByPlugin[plugin.id]}
        {@const job = jobId ? $jobs[jobId] : null}
        <!-- Berkunci `plugin.id`: kartu yang sudah ada tidak beranimasi ulang
             saat katalog diperbarui, hanya yang benar-benar baru. -->
        <article class="card tile" in:cardIn={{ index: i }}>
          <button class="tile-body" onclick={() => (selectedId = plugin.id)}>
            <div class="head">
              <PluginIcon pluginId={plugin.id} name={plugin.name} size={44} />
              <div class="head-text">
                <h2>{plugin.name}</h2>
                <div class="small muted">{plugin.categoryLabel ?? plugin.category}</div>
              </div>
              {#if plugin.installed}
                <span class="badge ok">{$t("explore.installed")}</span>
              {/if}
            </div>

            <p class="tagline">{plugin.tagline}</p>
            <div class="small muted">
              v{plugin.latestVersion} · {formatBytes(plugin.downloadSizeBytes)}
            </div>
            {#if plugin.deprecated}
              <div class="small warn-text">{$t("explore.deprecated")}</div>
            {/if}
          </button>

          {#if job}
            <div class="tile-foot"><JobProgress {job} /></div>
          {:else if !plugin.availableForPlatform}
            <div class="tile-foot small muted">{$t("explore.unavailable")}</div>
          {:else}
            <div class="tile-foot">
              <button class="primary full" onclick={() => (selectedId = plugin.id)}>
                {$t("common.getPlugin")}
              </button>
            </div>
          {/if}
        </article>
      {/each}
    </div>
  {/if}
  </div>
{/if}

<style>
  /* Judul, filter, dan grid berbagi lebar yang sama supaya tepinya sejajar.
     Batas 1140 px ada karena kartu yang melebar tanpa henti di monitor lebar
     justru terlihat kosong, bukan lapang. */
  .explore {
    max-width: 1180px;
    margin: 0 auto;
  }

  .filters {
    display: grid;
    grid-template-columns: 1fr 200px;
    gap: 8px;
    margin: 12px 0 16px;
  }

  /* `auto-fit`, bukan `auto-fill`: trek yang tidak terisi diciutkan, sehingga
     dua plugin melebar mengisi seluruh baris alih-alih menyisakan slot ketiga
     yang menganga. Batas tiga kolom datang dari `max-width` di `.explore` —
     kolom keempat butuh 1244 px dan tidak akan pernah muat. */
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    gap: 14px;
  }

  .tile {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    transition: transform 140ms ease, box-shadow 140ms ease, border-color 140ms ease;
  }

  /* Kartu terangkat saat disentuh kursor: sinyal paling murah bahwa ia dapat
     diklik, tanpa menambah satu pun elemen di layar. */
  .tile:hover {
    transform: translateY(-2px);
    box-shadow: var(--shadow-lg);
    border-color: var(--border-strong);
  }

  .tile-body {
    border: none;
    background: transparent;
    box-shadow: none;
    text-align: left;
    padding: 16px 16px 10px;
    border-radius: 0;
    flex: 1;
  }

  .tile-body:hover {
    background: transparent;
  }

  /* Tinggi baris header dikunci supaya nama plugin sejajar di seluruh baris,
     tidak peduli label kategorinya satu kata atau tiga. */
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 44px;
  }

  .head-text {
    min-width: 0;
    flex: 1;
  }

  .head-text h2 {
    margin: 0;
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Label kategori tidak boleh membungkus jadi dua baris — itu yang membuat
     header kartu di sebelahnya bergeser turun. */
  .head-text .small {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Tagline dipatok dua baris: yang lebih pendek tetap memakan ruang yang
     sama, yang lebih panjang dipotong. Tanpa ini, baris versi dan tombol di
     bawahnya berada di ketinggian berbeda antar kartu — dan itu terbaca
     sebagai kartu yang tidak rata, bukan sebagai teks yang beragam. */
  .tagline {
    margin: 10px 0 6px;
    color: var(--text-muted);
    font-size: 13px;
    line-height: 1.45;
    height: calc(2 * 1.45em);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .tile-foot {
    padding: 8px 12px 12px;
  }

  .full {
    width: 100%;
  }

  .warn-text {
    color: var(--warning);
  }
</style>
