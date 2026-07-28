<script lang="ts">
  import { onMount } from "svelte";

  import { t } from "./lib/i18n";
  import {
    bootstrap,
    catalog,
    catalogError,
    catalogLoading,
    hardRefresh,
    updateCount,
    view,
    type View,
  } from "./lib/store";
  import { formatDateTime } from "./lib/format";
  import { pageIn } from "./lib/motion";
  import Library from "./views/Library.svelte";
  import Explore from "./views/Explore.svelte";
  import Updates from "./views/Updates.svelte";
  import Settings from "./views/Settings.svelte";
  import InstallDialog from "./components/InstallDialog.svelte";
  import UninstallDialog from "./components/UninstallDialog.svelte";
  import NavIcon from "./components/NavIcon.svelte";
  import appLogo from "./assets/logo.png";

  let ready = $state(false);
  let bootError = $state<string | null>(null);

  // Penskalaan render otomatis.
  //
  // Seluruh UI memakai satuan piksel, jadi di monitor lebar ia tetap sekecil
  // di laptop dan sisanya jadi ruang kosong. `zoom` menskalakan tata letak
  // sungguhan di Chromium — bukan transform yang mengaburkan teks — dan media
  // query tetap bekerja karena viewport CSS ikut menyesuaikan.
  //
  // Batas atas 1,3 disengaja: lebih dari itu, satu baris teks jadi terlalu
  // panjang untuk dibaca nyaman.
  let viewportWidth = $state(1280);
  let uiScale = $derived(Math.min(1.3, Math.max(1, viewportWidth / 1500)));

  // Sidebar yang dilipat bertahan antar sesi: pengguna yang memilih layar
  // lapang tidak mau memilihnya lagi setiap kali membuka aplikasi.
  const COLLAPSE_KEY = "studiohub.sidebar.collapsed";
  let collapsed = $state(false);

  // Explore paling atas: menemukan plugin baru adalah alasan utama membuka
  // aplikasi ini, sedangkan Library hanya dilihat saat ada yang perlu diurus.
  const NAV: { id: View; labelKey: string; icon: "explore" | "library" | "updates" | "settings" }[] =
    [
      { id: "explore", labelKey: "nav.explore", icon: "explore" },
      { id: "library", labelKey: "nav.library", icon: "library" },
      { id: "updates", labelKey: "nav.updates", icon: "updates" },
      { id: "settings", labelKey: "nav.settings", icon: "settings" },
    ];

  onMount(async () => {
    collapsed = localStorage.getItem(COLLAPSE_KEY) === "1";
    try {
      await bootstrap();
    } catch (e) {
      bootError = String(e);
    } finally {
      ready = true;
    }
  });

  function toggleSidebar() {
    collapsed = !collapsed;
    localStorage.setItem(COLLAPSE_KEY, collapsed ? "1" : "0");
  }

  // Banner offline/stale muncul di semua layar: pengguna harus tahu bahwa yang
  // mereka lihat mungkin bukan yang terbaru (FR-1.2).
  let staleMessage = $derived.by(() => {
    if ($catalogError) return $t("catalog.offline");
    if ($catalog?.stale) {
      return $t("catalog.stale", { time: formatDateTime($catalog.lastSuccessAt) });
    }
    return null;
  });
</script>

<svelte:window bind:innerWidth={viewportWidth} />

<!-- `zoom` ikut menskalakan tinggi, jadi 100vh akan meluber sebesar faktor
     skalanya. Tingginya dibagi balik supaya tetap tepat satu layar. Dialog
     berada di dalam pembungkus ini agar ikut terskala — kalau tidak, ia akan
     terlihat mengecil dibanding aplikasi di belakangnya. -->
<div class="root" style="zoom: {uiScale}; height: calc(100vh / {uiScale})">
<div class="app" class:collapsed>
  <nav aria-label={$t("nav.main")}>
    <div class="brand">
      <img src={appLogo} alt="" class="brand-mark" />
      {#if !collapsed}<span class="brand-name">Studio Hub</span>{/if}
    </div>

    <button
      class="nav-item toggle"
      onclick={toggleSidebar}
      title={collapsed ? $t("nav.expand") : $t("nav.collapse")}
      aria-label={collapsed ? $t("nav.expand") : $t("nav.collapse")}
      aria-expanded={!collapsed}
    >
      <NavIcon name="menu" />
      {#if !collapsed}<span class="label">{$t("nav.collapse")}</span>{/if}
    </button>

    {#each NAV as item (item.id)}
      <button
        class="nav-item"
        class:active={$view === item.id}
        aria-current={$view === item.id ? "page" : undefined}
        title={collapsed ? $t(item.labelKey) : undefined}
        onclick={() => view.set(item.id)}
      >
        <NavIcon name={item.icon} />
        {#if !collapsed}<span class="label">{$t(item.labelKey)}</span>{/if}
        {#if item.id === "updates" && $updateCount > 0}
          <span class="badge update count">{$updateCount}</span>
        {/if}
      </button>
    {/each}

    <div class="nav-spacer"></div>

    <button
      class="nav-item refresh"
      onclick={hardRefresh}
      disabled={$catalogLoading}
      title={$t("common.refresh")}
      aria-label={$t("common.refresh")}
    >
      <span class="spin-host" class:spinning={$catalogLoading}>
        <NavIcon name="refresh" />
      </span>
      {#if !collapsed}<span class="label">{$t("common.refresh")}</span>{/if}
    </button>
  </nav>

  <main>
    {#if staleMessage}
      <div class="notice warn stale-banner" role="status">
        {staleMessage}
        <button class="ghost small" onclick={hardRefresh}>
          {$t("common.retry")}
        </button>
      </div>
    {/if}

    {#if !ready}
      <p class="muted">{$t("common.loading")}</p>
    {:else if bootError}
      <div class="notice danger selectable">{bootError}</div>
    {:else}
      <!-- `{#key}` membuat blok ini dibongkar-pasang tiap tab berganti,
           sehingga transisi masuknya berjalan. Tanpa transisi keluar: dua
           halaman yang hidup bersamaan akan saling mendorong tata letak. -->
      {#key $view}
        <div class="page" in:pageIn>
          {#if $view === "library"}
            <Library />
          {:else if $view === "explore"}
            <Explore />
          {:else if $view === "updates"}
            <Updates />
          {:else}
            <Settings />
          {/if}
        </div>
      {/key}
    {/if}
  </main>
</div>

<InstallDialog />
<UninstallDialog />
</div>

<style>
  .app {
    display: grid;
    grid-template-columns: 212px 1fr;
    height: 100%;
    transition: grid-template-columns 160ms ease;
  }

  .app.collapsed {
    grid-template-columns: 60px 1fr;
  }

  nav {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 14px 10px;
    background: linear-gradient(180deg, var(--surface), var(--bg-accent));
    border-right: 1px solid var(--border);
    overflow: hidden;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 6px 14px;
    min-width: 0;
  }

  /* Logonya putih polos di atas transparan. Di tema terang ia akan hilang,
     jadi dibalik menjadi gelap; di tema gelap dibiarkan apa adanya. */
  .brand-mark {
    width: 26px;
    height: 26px;
    flex: 0 0 auto;
    filter: invert(1);
  }

  :global(:root[data-theme="dark"]) .brand-mark {
    filter: none;
  }

  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) .brand-mark {
      filter: none;
    }
  }

  .brand-name {
    font-weight: 750;
    font-size: 15px;
    letter-spacing: -0.01em;
    white-space: nowrap;
    color: var(--text);
  }

  /* Aksen khaki tipis memisahkan sidebar dari area konten — sekaligus satu
     tempat lagi warna palet benar-benar terlihat, bukan cuma di badge. */
  nav {
    box-shadow: inset -1px 0 0 color-mix(in srgb, var(--focus) 28%, transparent);
  }

  .nav-item {
    position: relative;
    display: flex;
    align-items: center;
    gap: 11px;
    border: none;
    background: transparent;
    box-shadow: none;
    text-align: left;
    padding: 9px 11px;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    font-weight: 500;
    white-space: nowrap;
  }

  .nav-item:hover:not(:disabled) {
    background: var(--surface-2);
    color: var(--text);
  }

  .nav-item.active {
    background: var(--accent-soft);
    color: var(--text);
    font-weight: 650;
  }

  /* Penanda di tepi kiri: menandai tab aktif tanpa mengandalkan warna latar
     saja, yang sulit dibedakan di tema gelap. Memakai `--focus`, bukan
     `--accent` — slate di atas latar gelap nyaris tidak terlihat, dan penanda
     yang tidak terlihat tidak menandai apa pun. */
  .nav-item.active::before {
    content: "";
    position: absolute;
    left: 0;
    top: 20%;
    bottom: 20%;
    width: 3px;
    border-radius: 0 3px 3px 0;
    background: var(--focus);
  }

  .label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .count {
    margin-left: auto;
  }

  /* Saat dilipat, badge jumlah update menempel di sudut ikon — angkanya tetap
     terlihat tanpa memaksa sidebar melebar. */
  .app.collapsed .count {
    position: absolute;
    top: 2px;
    right: 2px;
    padding: 0 5px;
    font-size: 10px;
    line-height: 15px;
  }

  .toggle {
    color: var(--text-muted);
    margin-bottom: 6px;
  }

  .nav-spacer {
    flex: 1;
  }

  .refresh {
    font-size: 13px;
  }

  .spin-host {
    display: inline-flex;
  }

  .spin-host.spinning {
    animation: spin 900ms linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  main {
    overflow-y: auto;
    padding: 26px 30px 48px;
    /* Kilau halus di puncak area konten: memberi kesan bidang, bukan warna
       rata yang membentang sampai tepi jendela. */
    background:
      radial-gradient(
        120% 60% at 50% 0%,
        color-mix(in srgb, var(--accent) 9%, transparent),
        transparent 70%
      ),
      var(--bg);
  }

  /* `will-change` memberi tahu compositor bahwa elemen ini akan bergerak,
     supaya frame pertamanya tidak tersendat. Hanya dipasang di pembungkus
     halaman, bukan di mana-mana — dipakai berlebihan, ia justru memakan
     memori GPU dan memperlambat. */
  .page {
    will-change: transform, opacity;
  }

  .stale-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 16px;
  }
</style>
