<script lang="ts">
  // Layar Settings (PRD §8.2, §6.8).
  //
  // Sengaja pendek. Setiap pilihan yang ditampilkan adalah pilihan yang harus
  // dipahami pengguna, dan sebagian besar tidak punya jawaban yang benar-benar
  // berbeda — URL katalog dan beta channel dihapus karena keduanya lebih
  // sering jadi cara menyalahsetel aplikasi daripada cara memakainya.

  import { onMount } from "svelte";

  import * as api from "../lib/api";
  import { BackendError } from "../lib/api";
  import { t } from "../lib/i18n";
  import { clearAllCache, prefs, savePrefs } from "../lib/store";
  import type { DiagnosticsSummary, HubError, InstallScope } from "../lib/types";
  import ErrorPanel from "../components/ErrorPanel.svelte";

  let diagnostics = $state<DiagnosticsSummary | null>(null);
  let error = $state<HubError | null>(null);
  let clearing = $state(false);
  let cleared = $state(false);

  // Pemeriksaan manual. Tanpa ini, mematikan pemeriksaan otomatis di atas akan
  // membuat pengguna terjebak di versi lama selamanya tanpa jalan keluar.
  let checking = $state(false);
  let checkResult = $state<"none" | "upToDate" | "found">("none");
  let foundVersion = $state("");

  async function checkNow() {
    checking = true;
    checkResult = "none";
    try {
      const found = await api.launcherUpdateCheck();
      if (found) {
        foundVersion = found.availableVersion;
        checkResult = "found";
      } else {
        checkResult = "upToDate";
      }
    } catch (e) {
      if (e instanceof BackendError) error = e.hub;
    } finally {
      checking = false;
    }
  }

  async function installUpdate() {
    checking = true;
    try {
      await api.launcherUpdateInstall();
    } catch (e) {
      if (e instanceof BackendError) error = e.hub;
      checking = false;
    }
  }

  onMount(async () => {
    diagnostics = await api.diagnosticsSummary();
  });

  async function patch(values: Parameters<typeof savePrefs>[0]) {
    error = null;
    try {
      await savePrefs(values);
    } catch (e) {
      if (e instanceof BackendError) error = e.hub;
    }
  }

  async function setScope(kind: string) {
    const scope: InstallScope =
      kind === "all_users" ? { kind: "all_users" } : { kind: "current_user" };
    await patch({ defaultInstallScope: scope });
  }

  async function clearCache() {
    clearing = true;
    cleared = false;
    try {
      await clearAllCache();
      cleared = true;
    } catch (e) {
      if (e instanceof BackendError) error = e.hub;
    } finally {
      clearing = false;
    }
  }
</script>

<h1>{$t("settings.title")}</h1>

{#if error}
  <ErrorPanel {error} onclose={() => (error = null)} />
{/if}

{#if $prefs}
  <section class="card">
    <h2>{$t("settings.installLocation")}</h2>
    <p class="small muted">{$t("settings.installLocationHelp")}</p>
    <select
      value={$prefs.defaultInstallScope.kind}
      onchange={(e) => setScope(e.currentTarget.value)}
    >
      <option value="current_user">{$t("install.scope.currentUser")}</option>
      <option value="all_users">{$t("install.scope.allUsers")}</option>
    </select>
  </section>

  <section class="card">
    <h2>{$t("settings.updates")}</h2>
    <label class="checkbox">
      <input
        type="checkbox"
        checked={$prefs.checkUpdatesOnLaunch}
        onchange={(e) => patch({ checkUpdatesOnLaunch: e.currentTarget.checked })}
      />
      <span>{$t("settings.checkOnLaunch")}</span>
    </label>

    <div class="check-row">
      <button onclick={checkNow} disabled={checking}>
        {checking ? $t("launcherUpdate.installing") : $t("settings.checkNow")}
      </button>
      {#if checkResult === "upToDate"}
        <span class="small muted">{$t("settings.upToDate")}</span>
      {:else if checkResult === "found"}
        <span class="small">{$t("launcherUpdate.available", { version: foundVersion })}</span>
        <button class="primary small" onclick={installUpdate} disabled={checking}>
          {$t("launcherUpdate.install")}
        </button>
      {/if}
    </div>
  </section>

  <section class="card">
    <h2>{$t("settings.privacy")}</h2>
    <label class="checkbox">
      <input
        type="checkbox"
        checked={$prefs.telemetryEnabled}
        onchange={(e) => patch({ telemetryEnabled: e.currentTarget.checked })}
      />
      <span>{$t("settings.telemetry")}</span>
    </label>
    <!-- Penjelasan konkret tentang apa yang dikirim, bukan janji umum
         (PRD §17.1). -->
    <p class="small muted">{$t("settings.telemetryHelp")}</p>
    <button class="ghost small" onclick={() => api.telemetryResetId()}>
      {$t("settings.telemetryResetId")}
    </button>
  </section>

  <section class="card">
    <h2>{$t("settings.language")}</h2>
    <select value={$prefs.locale} onchange={(e) => patch({ locale: e.currentTarget.value })}>
      <option value="id">Bahasa Indonesia</option>
      <option value="en">English</option>
    </select>

    <h2 class="spaced">{$t("settings.theme")}</h2>
    <select value={$prefs.theme} onchange={(e) => patch({ theme: e.currentTarget.value })}>
      <option value="system">{$t("settings.theme.system")}</option>
      <option value="light">{$t("settings.theme.light")}</option>
      <option value="dark">{$t("settings.theme.dark")}</option>
    </select>
  </section>
{/if}

<section class="card">
  <h2>{$t("settings.storage")}</h2>
  <p class="small muted">{$t("settings.clearCacheHelp")}</p>
  <div class="row">
    <button onclick={clearCache} disabled={clearing}>
      {clearing ? $t("common.loading") : $t("settings.clearCache")}
    </button>
    {#if cleared}
      <span class="badge ok">{$t("settings.cacheCleared")}</span>
    {/if}
  </div>
</section>

<section class="card">
  <h2>{$t("settings.about")}</h2>
  {#if diagnostics}
    <dl class="facts small selectable">
      <dt>{$t("common.version")}</dt>
      <dd>{diagnostics.launcherVersion}</dd>
      <dt>Windows</dt>
      <dd>build {diagnostics.osBuild ?? "—"} · {diagnostics.arch}</dd>
      <dt>VC++ Redistributable</dt>
      <dd>{diagnostics.vcRedist ? "OK" : "—"}</dd>
      <dt>Plugin</dt>
      <dd>{diagnostics.installedCount}</dd>
      <dt>DAW</dt>
      <dd>{diagnostics.detectedDaws.join(", ") || "—"}</dd>
    </dl>
  {/if}
  <button class="ghost small" onclick={() => api.logsOpen()}>
    {$t("settings.openLogs")}
  </button>
</section>

<style>
  section {
    padding: 16px 18px;
    margin-bottom: 12px;
  }

  h2 {
    margin: 0 0 6px;
  }

  h2.spaced {
    margin-top: 16px;
  }

  select {
    max-width: 340px;
  }

  .checkbox {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 0;
  }

  .facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 16px;
    margin: 8px 0 12px;
  }

  .facts dt {
    color: var(--text-muted);
  }

  .facts dd {
    margin: 0;
  }

  .check-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 10px;
  }
</style>
