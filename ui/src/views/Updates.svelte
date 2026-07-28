<script lang="ts">
  // Layar Updates (PRD §8.5) — menjawab "apa yang perlu saya lakukan?"
  //
  // Changelog ditampilkan ter-expand, bukan disembunyikan di balik klik:
  // pengguna harus bisa membacanya tanpa usaha sebelum memutuskan.

  import { flip } from "svelte/animate";

  import * as api from "../lib/api";
  import { requestInstall } from "../lib/dialogs";
  import { REFLOW_MS, cardIn, cardOut } from "../lib/motion";
  import { t } from "../lib/i18n";
  import { formatBytes, formatDate } from "../lib/format";
  import { renderMarkdown } from "../lib/markdown";
  import { jobByPlugin, jobs, refreshLibrary, refreshUpdates, updates } from "../lib/store";
  import JobProgress from "../components/JobProgress.svelte";

  let busy = $state(false);

  async function updateAll() {
    busy = true;
    try {
      // FR-4.6: hanya non-breaking. Yang breaking tetap harus dikonfirmasi
      // satu per satu — itulah yang mencegah "Update all" merusak project
      // pengguna yang sedang berjalan.
      await api.updateAllStart(false);
    } finally {
      busy = false;
    }
  }

  async function skip(pluginId: string, version: string) {
    await api.versionSkip(pluginId, version);
    await refreshUpdates();
    await refreshLibrary();
  }
</script>

<div class="spread header">
  <h1>{$t("updates.title")}</h1>
  {#if $updates.nonBreakingCount > 0}
    <button class="primary" onclick={updateAll} disabled={busy}>
      {$t("updates.updateAll", { count: $updates.nonBreakingCount })}
    </button>
  {/if}
</div>

{#if $updates.items.length === 0}
  <div class="empty card">
    <h2>{$t("updates.none.title")}</h2>
    <p class="muted">{$t("updates.none.body")}</p>
  </div>
{:else}
  <div class="list">
    {#each $updates.items as item, i (item.pluginId)}
      {@const jobId = $jobByPlugin[item.pluginId]}
      {@const job = jobId ? $jobs[jobId] : null}
      <article
        class="card item"
        class:breaking={item.breaking}
        in:cardIn={{ index: i }}
        out:cardOut
        animate:flip={{ duration: REFLOW_MS }}
      >
        <div class="spread">
          <div>
            <h2>{item.name}</h2>
            <div class="small muted">
              {item.fromVersion} → {item.toVersion} ·
              {formatBytes(item.downloadSizeBytes)}
              {#if item.releasedAt}
                · {$t("updates.releasedOn", { date: formatDate(item.releasedAt) })}
              {/if}
            </div>
          </div>
          <div class="badges">
            {#if item.security}
              <span class="badge security">{$t("updates.security")}</span>
            {/if}
            {#if item.breaking}
              <span class="badge breaking">{$t("updates.breaking")}</span>
            {/if}
          </div>
        </div>

        {#if item.breaking}
          <div class="notice warn small">{$t("updates.breakingHelp")}</div>
        {/if}

        {#if item.changelog}
          <div class="prose selectable">{@html renderMarkdown(item.changelog)}</div>
        {/if}

        {#if job}
          <JobProgress {job} />
        {:else}
          <div class="actions">
            <button
              class="primary"
              onclick={() =>
                requestInstall({ pluginId: item.pluginId, pluginName: item.name })}
            >
              {$t("common.update")}
            </button>
            <button class="ghost" onclick={() => skip(item.pluginId, item.toVersion)}>
              {$t("updates.skipVersion")}
            </button>
          </div>
        {/if}
      </article>
    {/each}
  </div>
{/if}

<style>
  .header {
    margin-bottom: 14px;
  }

  .empty {
    padding: 32px;
    text-align: center;
    max-width: 460px;
    margin: 32px auto;
  }

  /* Sama seperti Library: `margin-bottom`, bukan `gap`, supaya ruang kartu
     yang pergi ikut mengatup mulus. */
  .list {
    display: flex;
    flex-direction: column;
  }

  .item {
    padding: 14px 16px;
    margin-bottom: 12px;
  }

  .item.breaking {
    border-color: color-mix(in srgb, var(--danger) 40%, var(--border));
  }

  .item h2 {
    margin: 0;
  }

  .badges {
    display: flex;
    gap: 6px;
  }

  .prose {
    font-size: 13.5px;
    margin-top: 8px;
  }

  .actions {
    display: flex;
    gap: 6px;
    margin-top: 10px;
  }

  .notice {
    margin-top: 10px;
  }
</style>
