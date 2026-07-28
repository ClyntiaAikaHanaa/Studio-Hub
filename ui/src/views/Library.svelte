<script lang="ts">
  // Layar Library (PRD §8.3) — menjawab "apa yang saya punya dan apakah sehat?"

  import { flip } from "svelte/animate";

  import * as api from "../lib/api";
  import { requestInstall, requestUninstall } from "../lib/dialogs";
  import { REFLOW_MS, cardIn, cardOut } from "../lib/motion";
  import { t } from "../lib/i18n";
  import { formatDate } from "../lib/format";
  import { jobByPlugin, jobs, library, refreshLibrary, refreshUpdates, view } from "../lib/store";
  import type { LibraryEntry } from "../lib/types";
  import JobProgress from "../components/JobProgress.svelte";

  function statusOf(entry: LibraryEntry): { key: string; tone: string } {
    if (entry.installed.health === "missing") {
      return { key: "library.status.missing", tone: "warn" };
    }
    if (entry.installed.health === "unknown_version") {
      return { key: "library.status.unknownVersion", tone: "neutral" };
    }
    switch (entry.update.state) {
      case "updateAvailable":
        return { key: "library.status.updateAvailable", tone: "update" };
      case "aheadOfCatalog":
        return { key: "library.status.aheadOfCatalog", tone: "neutral" };
      case "skipped":
        return { key: "library.status.skipped", tone: "neutral" };
      case "upToDate":
        return { key: "library.status.ok", tone: "ok" };
      default:
        return { key: "library.status.ok", tone: "neutral" };
    }
  }

  async function rollback(pluginId: string) {
    await api.rollbackStart(pluginId);
    await refreshLibrary();
    await refreshUpdates();
  }
</script>

<h1>{$t("library.title")}</h1>

{#if $library.length === 0}
  <div class="empty card">
    <h2>{$t("library.empty.title")}</h2>
    <p class="muted">{$t("library.empty.body")}</p>
    <button class="primary" onclick={() => view.set("explore")}>
      {$t("library.empty.cta")}
    </button>
  </div>
{:else}
  <div class="list">
    {#each $library as entry, i (entry.pluginId)}
      {@const status = statusOf(entry)}
      {@const jobId = $jobByPlugin[entry.pluginId]}
      {@const job = jobId ? $jobs[jobId] : null}
      <!-- `animate:flip` menggeser kartu yang tersisa ke posisi barunya. Tanpa
           itu, mengatupkan satu kartu tetap membuat sisanya melompat — yang
           beranimasi hanya kartu yang pergi, bukan yang bertahan. -->
      <article
        class="card item"
        in:cardIn={{ index: i }}
        out:cardOut
        animate:flip={{ duration: REFLOW_MS }}
      >
        <div class="spread">
          <div class="identity">
            <h2>{entry.name}</h2>
            <div class="small muted">
              {$t("common.version")} {entry.installed.version} ·
              {formatDate(entry.installed.installedAt)}
            </div>
            {#if entry.installed.adopted}
              <div class="small muted">{$t("library.adopted")}</div>
            {/if}
            {#if !entry.inCatalog}
              <div class="small muted">{$t("library.notInCatalog")}</div>
            {/if}
          </div>
          <span class="badge {status.tone}">{$t(status.key)}</span>
        </div>

        {#if job}
          <JobProgress {job} />
        {:else}
          <div class="actions">
            {#if entry.update.state === "updateAvailable"}
              <button
                class="primary"
                onclick={() =>
                  requestInstall({ pluginId: entry.pluginId, pluginName: entry.name })}
              >
                {$t("common.update")}
              </button>
            {:else if entry.installed.health === "missing" && entry.inCatalog}
              <button
                class="primary"
                onclick={() =>
                  requestInstall({ pluginId: entry.pluginId, pluginName: entry.name })}
              >
                {$t("common.reinstall")}
              </button>
            {/if}

            {#if entry.installed.hasBackup}
              <button onclick={() => rollback(entry.pluginId)}>
                {$t("common.rollback")}
                <span class="muted small">({entry.installed.backupVersion})</span>
              </button>
            {/if}

            <button class="ghost" onclick={() => api.revealInExplorer(entry.pluginId)}>
              {$t("common.revealInExplorer")}
            </button>

            <button
              class="ghost danger"
              onclick={() =>
                requestUninstall({
                  pluginId: entry.pluginId,
                  pluginName: entry.name,
                  adopted: entry.installed.adopted,
                })}
            >
              {$t("common.uninstall")}
            </button>
          </div>
        {/if}
      </article>
    {/each}
  </div>
{/if}

<style>
  .empty {
    padding: 32px;
    text-align: center;
    max-width: 460px;
    margin: 32px auto;
  }

  .empty button {
    margin-top: 12px;
  }

  /* Jarak antar kartu memakai `margin-bottom`, bukan `gap`.
     `gap` adalah properti kontainer dan tidak dapat dianimasikan per kartu,
     jadi kartu yang mengatup tetap menyisakan celah setinggi gap sampai
     transisinya selesai — terlihat sebagai lubang yang berkedip. */
  .list {
    display: flex;
    flex-direction: column;
    margin-top: 14px;
  }

  .item {
    padding: 14px 16px;
    margin-bottom: 10px;
  }

  .identity h2 {
    margin: 0;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 12px;
  }
</style>
