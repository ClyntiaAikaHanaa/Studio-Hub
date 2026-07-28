<script lang="ts">
  // Thumbnail plugin.
  //
  // Ikon TIDAK dimuat langsung dari internet: backend mengunduhnya, memvalidasi
  // magic bytes-nya, dan menyimpannya di cache; di sini kita hanya mengubah
  // path lokal itu menjadi URL `asset:` (PRD §14.5, mitigasi T7). CSP memblokir
  // `img-src` ke host mana pun, jadi jalur lain memang tidak ada.

  import { convertFileSrc } from "@tauri-apps/api/core";

  import * as api from "../lib/api";
  import { catalog } from "../lib/store";

  interface Props {
    pluginId: string;
    name: string;
    size?: number;
  }

  let { pluginId, name, size = 48 }: Props = $props();

  let src = $state<string | null>(null);
  let failed = $state(false);

  $effect(() => {
    const id = pluginId;
    // `icon_url` hidup di katalog. Kartu tidak dibuat ulang saat katalog
    // diperbarui — kuncinya tetap `plugin.id` — jadi tanpa dependensi ini,
    // plugin yang baru mendapat logo tidak akan pernah menampilkannya sampai
    // aplikasi di-restart.
    void $catalog?.generatedAt;

    let cancelled = false;
    src = null;
    failed = false;

    api
      .pluginIcon(id)
      .then((path) => {
        if (cancelled || !path) return;
        src = convertFileSrc(path);
      })
      .catch(() => {
        if (!cancelled) failed = true;
      });

    return () => {
      cancelled = true;
    };
  });

  // Fallback: inisial nama. Kartu tanpa gambar tetap harus terlihat rapi,
  // dan plugin baru sering belum punya logo.
  let initials = $derived(
    name
      .split(/\s+/)
      .slice(0, 2)
      .map((w) => w[0] ?? "")
      .join("")
      .toUpperCase()
  );
</script>

<div class="icon" style="--size: {size}px" aria-hidden="true">
  {#if src && !failed}
    <img {src} alt="" onerror={() => (failed = true)} />
  {:else}
    <span class="fallback">{initials}</span>
  {/if}
</div>

<style>
  .icon {
    width: var(--size);
    height: var(--size);
    flex: 0 0 auto;
    border-radius: 10px;
    overflow: hidden;
    background: var(--surface-2);
    border: 1px solid var(--border);
    display: grid;
    place-items: center;
  }

  img {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  .fallback {
    font-weight: 700;
    font-size: calc(var(--size) * 0.34);
    color: var(--text-muted);
    letter-spacing: 0.02em;
  }
</style>
