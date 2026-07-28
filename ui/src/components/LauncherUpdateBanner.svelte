<script lang="ts">
  // Banner update launcher (PRD §15.4, FR-7.3).
  //
  // Muncul di atas seluruh layar karena dua alasannya berlaku di mana saja:
  // ada versi baru, atau katalog menuntut versi yang lebih baru sebelum plugin
  // tertentu boleh dipasang. Yang kedua tidak dapat ditutup, karena menutupnya
  // hanya akan membuat tombol Install mati tanpa penjelasan.

  import * as api from "../lib/api";
  import { t } from "../lib/i18n";
  import { prefs } from "../lib/store";
  import type { LauncherUpdate } from "../lib/types";

  let update = $state<LauncherUpdate | null>(null);
  let installing = $state(false);
  let failed = $state(false);
  let dismissed = $state(false);

  // Menunggu prefs terisi, bukan memakai `onMount`. Komponen ini dipasang
  // sebelum bootstrap selesai, jadi saat itu prefs masih null dan membaca
  // `checkUpdatesOnLaunch` terlalu dini akan mengabaikan pilihan pengguna.
  let started = false;
  $effect(() => {
    const current = $prefs;
    if (!current || started) return;
    started = true;

    // Kalau pemeriksaan otomatis dimatikan, tuntutan katalog tetap dibaca:
    // itu tidak menyentuh jaringan sama sekali, dan tanpa itu instalasi yang
    // diblokir jadi tidak punya penjelasan.
    const query = current.checkUpdatesOnLaunch
      ? api.launcherUpdateCheck()
      : api.launcherUpdateRequired();

    // Pemeriksaan update tidak pernah boleh menghalangi pemakaian aplikasi.
    query.then((found) => (update = found)).catch(() => (update = null));
  });

  async function install() {
    installing = true;
    failed = false;
    try {
      await api.launcherUpdateInstall();
      // Kalau berhasil, proses ini digantikan dan baris berikutnya tidak
      // pernah dijalankan.
    } catch {
      failed = true;
      installing = false;
    }
  }

  let visible = $derived(update !== null && (update.required || !dismissed));
</script>

{#if visible && update}
  <div
    class="notice launcher-banner"
    class:danger={update.required}
    class:warn={!update.required}
    role="status"
  >
    <div class="text">
      <strong>
        {update.required
          ? $t("launcherUpdate.required", { version: update.availableVersion })
          : $t("launcherUpdate.available", { version: update.availableVersion })}
      </strong>
      {#if failed}
        <div class="small">{$t("launcherUpdate.failed")}</div>
      {/if}
    </div>

    <div class="actions">
      <button class="primary" onclick={install} disabled={installing}>
        {installing ? $t("launcherUpdate.installing") : $t("launcherUpdate.install")}
      </button>
      {#if !update.required}
        <button class="ghost small" onclick={() => (dismissed = true)}>
          {$t("launcherUpdate.later")}
        </button>
      {/if}
    </div>
  </div>
{/if}

<style>
  .launcher-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 16px;
  }

  .text {
    min-width: 0;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 0 0 auto;
  }
</style>
