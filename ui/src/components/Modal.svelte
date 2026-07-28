<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    title: string;
    onclose: () => void;
    children: Snippet;
    footer?: Snippet;
    /** Dialog yang sedang menjalankan operasi tidak boleh ditutup Esc. */
    dismissible?: boolean;
  }

  let { title, onclose, children, footer, dismissible = true }: Props = $props();

  let dialog = $state<HTMLDivElement | null>(null);

  // NFR-4.2: fokus berpindah ke dialog saat terbuka, jadi pengguna keyboard
  // tidak tertinggal di belakang overlay.
  $effect(() => {
    dialog?.focus();
  });

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape" && dismissible) {
      event.stopPropagation();
      onclose();
    }
  }
</script>

<svelte:window on:keydown={onkeydown} />

<div class="overlay" role="presentation">
  <div
    class="dialog card"
    role="dialog"
    aria-modal="true"
    aria-label={title}
    tabindex="-1"
    bind:this={dialog}
  >
    <header>
      <h2>{title}</h2>
    </header>
    <div class="body">
      {@render children()}
    </div>
    {#if footer}
      <footer>
        {@render footer()}
      </footer>
    {/if}
  </div>
</div>

<style>
  /* Animasi CSS, bukan transisi Svelte: aturan `prefers-reduced-motion` global
     di styles.css sudah mematikannya, jadi tidak perlu pemeriksaan terpisah. */
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(6, 5, 2, 0.62);
    backdrop-filter: blur(3px);
    display: grid;
    place-items: center;
    padding: 24px;
    z-index: 50;
    animation: overlay-in 180ms ease-out;
  }

  /* Naik sedikit sambil membesar dari 96%: cukup untuk terasa muncul dari
     kedalaman, tidak sampai terasa melompat. Skala di bawah 0,9 membuat teks
     terlihat buram saat animasinya berjalan. */
  .dialog {
    width: min(560px, 100%);
    max-height: min(84vh, 760px);
    display: flex;
    flex-direction: column;
    outline: none;
    box-shadow: var(--shadow-lg);
    animation: dialog-in 220ms cubic-bezier(0.16, 1, 0.3, 1);
    will-change: transform, opacity;
  }

  @keyframes overlay-in {
    from {
      opacity: 0;
    }
  }

  @keyframes dialog-in {
    from {
      opacity: 0;
      transform: translateY(10px) scale(0.96);
    }
  }

  header {
    padding: 18px 22px 6px;
  }

  header h2 {
    font-size: 17px;
  }

  .body {
    padding: 10px 22px 18px;
    overflow-y: auto;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 14px 22px 18px;
    border-top: 1px solid var(--border);
    background: var(--surface-2);
    border-radius: 0 0 var(--radius) var(--radius);
  }
</style>
