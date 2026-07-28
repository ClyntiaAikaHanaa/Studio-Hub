<script lang="ts">
  // Ikon tab sidebar.
  //
  // SVG inline, bukan berkas: CSP memblokir pemuatan aset eksternal, dan lima
  // ikon tidak layak menambah pipeline aset. `currentColor` membuatnya ikut
  // warna teks tab, termasuk saat tab aktif.

  interface Props {
    name: "explore" | "library" | "updates" | "settings" | "menu" | "refresh";
  }

  let { name }: Props = $props();
</script>

<svg
  class="nav-icon"
  viewBox="0 0 16 16"
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#if name === "explore"}
    <!-- Kompas: menemukan sesuatu yang belum kamu punya. -->
    <circle cx="8" cy="8" r="6.2" />
    <path d="M10.6 5.4 9.3 9.3 5.4 10.6 6.7 6.7z" />
  {:else if name === "library"}
    <!-- Kotak bertumpuk: yang sudah ada di tanganmu. -->
    <path d="M2.2 5.6 8 2.6l5.8 3v4.8L8 13.4l-5.8-3z" />
    <path d="M2.2 5.6 8 8.6l5.8-3M8 8.6v4.8" />
  {:else if name === "updates"}
    <!-- Panah turun ke garis: sesuatu yang menunggu diambil. -->
    <path d="M8 2.4v7.2M5.2 6.9 8 9.7l2.8-2.8" />
    <path d="M2.8 12.4h10.4" />
  {:else if name === "settings"}
    <path d="M6.6 2.4h2.8l.3 1.7 1.3.7 1.6-.7 1.4 2.4-1.3 1.1v1.4l1.3 1.1-1.4 2.4-1.6-.7-1.3.7-.3 1.7H6.6l-.3-1.7-1.3-.7-1.6.7-1.4-2.4L3.3 8.6V7.2L2 6.1l1.4-2.4 1.6.7 1.3-.7z" />
    <circle cx="8" cy="7.9" r="1.9" />
  {:else if name === "menu"}
    <path d="M2.5 4.5h11M2.5 8h11M2.5 11.5h11" />
  {:else if name === "refresh"}
    <path d="M13.5 8a5.5 5.5 0 1 1-1.61-3.89" />
    <path d="M13.6 1.9v3.3h-3.3" />
  {/if}
</svg>

<style>
  .nav-icon {
    width: 16px;
    height: 16px;
    flex: 0 0 auto;
  }
</style>
