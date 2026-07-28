<script lang="ts">
  // Dialog konfirmasi instalasi (PRD §8.6).
  //
  // Yang ditampilkan sebelum tombol Install: ukuran unduhan, versi, lokasi
  // tujuan, dan apakah elevasi akan diminta (FR-3.2). Semua angka ini datang
  // dari `install_plan` — frontend tidak menghitung apa pun sendiri, karena
  // menghitungnya berarti butuh akses filesystem (ADR-5).

  import { untrack } from "svelte";

  import * as api from "../lib/api";
  import { BackendError } from "../lib/api";
  import { installRequest } from "../lib/dialogs";
  import { t } from "../lib/i18n";
  import { formatBytes } from "../lib/format";
  import { catalog, jobs, refreshLibrary, refreshUpdates, savePrefs, trackJob } from "../lib/store";
  import type { HubError, InstallPlan, InstallScope, JobEvent } from "../lib/types";
  import Modal from "./Modal.svelte";
  import JobProgress from "./JobProgress.svelte";
  import ErrorPanel from "./ErrorPanel.svelte";
  import PlanBlockers from "./PlanBlockers.svelte";

  let plan = $state<InstallPlan | null>(null);
  let loading = $state(false);
  let error = $state<HubError | null>(null);
  let jobId = $state<string | null>(null);
  let scope = $state<InstallScope>({ kind: "current_user" });

  let request = $derived($installRequest);
  let job = $derived<JobEvent | null>(jobId ? ($jobs[jobId] ?? null) : null);
  let finished = $derived(job?.kind === "succeeded");

  // Lisensi diambil dari katalog, bukan dari `InstallPlan`: teks 35 KB tidak
  // perlu melintasi IPC setiap kali dialog dibuka.
  let entry = $derived(
    request ? ($catalog?.plugins.find((p) => p.id === request.pluginId) ?? null) : null
  );
  let licenseText = $derived(entry?.licenseText ?? "");

  /// Instalasi memasang kode ke folder yang dimuat otomatis DAW. Persetujuan
  /// lisensi adalah aksi sadar, jadi tombol Install tetap mati sampai pengguna
  /// menyatakan sudah membacanya — bukan checkbox yang tercentang duluan.
  let licenseAccepted = $state(false);

  // Hanya `$installRequest` yang boleh memicu effect ini. Sisanya dibungkus
  // `untrack` karena badan effect MENULIS `scope`, sementara `loadPlan()`
  // MEMBACA `scope` — tanpa untrack, effect memicu dirinya sendiri, dan karena
  // `{ kind: "current_user" }` adalah objek baru setiap putaran, ia tidak
  // pernah mencapai titik diam. Gejalanya: dialog tersangkut di "Loading…"
  // sambil membanjiri backend dengan install_plan.
  $effect(() => {
    const current = $installRequest;
    untrack(() => {
      if (current) {
        scope = current.scope ?? { kind: "current_user" };
        // Persetujuan direset setiap dialog dibuka: ia berlaku untuk satu
        // pemasangan, bukan untuk sesi aplikasi.
        licenseAccepted = false;
        void loadPlan();
      } else {
        plan = null;
        error = null;
        jobId = null;
      }
    });
  });

  async function loadPlan() {
    if (!request) return;
    loading = true;
    error = null;
    try {
      plan = await api.installPlan(request.pluginId, request.version, scope);
    } catch (e) {
      if (e instanceof BackendError) error = e.hub;
    } finally {
      loading = false;
    }
  }

  async function changeScope(next: InstallScope) {
    scope = next;
    // Ganti scope = angka berubah (lokasi, apakah UAC muncul, ruang disk di
    // volume lain), jadi rencananya dihitung ulang, bukan ditambal.
    await loadPlan();
  }

  async function confirm() {
    if (!request || !plan?.blockers || plan.blockers.length > 0) return;
    error = null;
    try {
      const id = await api.installStart(request.pluginId, request.version, scope);
      jobId = id;
      trackJob(request.pluginId, id);
      // FR-3.9: scope yang dipilih menjadi default berikutnya.
      await savePrefs({ defaultInstallScope: scope });
    } catch (e) {
      if (e instanceof BackendError) error = e.hub;
    }
  }

  async function close() {
    installRequest.set(null);
    await refreshLibrary();
    await refreshUpdates();
  }

  async function cancelJob() {
    if (jobId) await api.jobCancel(jobId);
  }

  function retry() {
    error = null;
    jobId = null;
    void loadPlan();
  }

  /// §8.8: elevasi ditolak → tawarkan jalur per-user, bukan jalan buntu.
  async function fallbackToPerUser() {
    error = null;
    jobId = null;
    await changeScope({ kind: "current_user" });
  }

  let title = $derived(
    plan?.fromVersion
      ? $t("install.updateTitle", { name: plan.pluginName })
      : $t("install.title", { name: request?.pluginName ?? "" })
  );
</script>

{#if request}
  <Modal {title} onclose={close} dismissible={!job || finished}>
    {#if loading}
      <p class="muted">{$t("common.loading")}</p>
    {:else if error}
      <ErrorPanel
        {error}
        onretry={retry}
        oninstallperuser={fallbackToPerUser}
        onclose={close}
      />
    {:else if job}
      <JobProgress {job} />
    {:else if plan}
      <dl class="facts">
        {#if plan.fromVersion}
          <dt>{$t("common.version")}</dt>
          <dd>{$t("install.fromTo", { from: plan.fromVersion, to: plan.toVersion })}</dd>
        {:else}
          <dt>{$t("common.version")}</dt>
          <dd>{plan.toVersion}</dd>
        {/if}

        <dt>{$t("install.downloadSize")}</dt>
        <dd>
          {formatBytes(plan.download.sizeBytes)}
          {#if plan.download.cached}
            <span class="muted small">· {$t("install.cached")}</span>
          {/if}
        </dd>

        <dt>{$t("install.willInstallTo")}</dt>
        <dd class="path selectable">{plan.target.installDir}</dd>
      </dl>

      <fieldset class="scope">
        <legend>{$t("install.scope")}</legend>
        <label>
          <input
            type="radio"
            name="scope"
            checked={scope.kind === "current_user"}
            onchange={() => changeScope({ kind: "current_user" })}
          />
          {$t("install.scope.currentUser")}
        </label>
        <label>
          <input
            type="radio"
            name="scope"
            checked={scope.kind === "all_users"}
            onchange={() => changeScope({ kind: "all_users" })}
          />
          {$t("install.scope.allUsers")}
        </label>
      </fieldset>

      <PlanBlockers {plan} />

      {#if plan.backupWillBeCreated}
        <p class="small muted">{$t("install.backupWillBeCreated")}</p>
      {/if}

      {#if plan.userDataPreserved.length}
        <details class="preserved">
          <summary class="small muted">{$t("install.userDataPreserved")}</summary>
          <ul class="small muted selectable">
            {#each plan.userDataPreserved as path (path)}
              <li>{path}</li>
            {/each}
          </ul>
        </details>
      {/if}

      <section class="license">
        <header class="spread">
          <h3>{$t("install.license")}</h3>
          {#if entry?.license}
            <span class="badge neutral">{entry.license}</span>
          {/if}
        </header>

        {#if licenseText}
          <!-- `region` + `tabindex="0"` + label adalah pola yang justru
               dianjurkan WAI-ARIA untuk kotak bergulir: tanpa fokus, isinya
               mustahil digulir dengan keyboard — dan lisensi adalah hal yang
               paling tidak boleh terkunci dari pembaca keyboard (NFR-4.2).
               Aturan lint Svelte tidak mengenali pengecualian ini. -->
          <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
          <div
            class="license-body selectable"
            tabindex="0"
            role="region"
            aria-label={$t("install.license")}
          >
            <pre>{licenseText}</pre>
          </div>
        {:else}
          <p class="small muted">{$t("install.licenseUnavailable")}</p>
        {/if}

        <button
          class="accept"
          class:accepted={licenseAccepted}
          onclick={() => (licenseAccepted = true)}
          disabled={licenseAccepted}
        >
          {licenseAccepted ? $t("install.licenseAccepted") : $t("install.licenseAccept")}
        </button>
      </section>
    {/if}

    {#snippet footer()}
      {#if finished}
        <button class="primary" onclick={close}>{$t("common.done")}</button>
      {:else if job}
        <button onclick={cancelJob}>{$t("common.cancel")}</button>
      {:else if !error}
        <button onclick={close}>{$t("common.cancel")}</button>
        <button
          class="primary"
          disabled={!plan || plan.blockers.length > 0 || loading || !licenseAccepted}
          title={licenseAccepted ? undefined : $t("install.licenseRequired")}
          onclick={confirm}
        >
          {$t("install.confirm")}
        </button>
      {/if}
    {/snippet}
  </Modal>
{/if}

<style>
  .facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 6px 16px;
    margin: 0 0 14px;
  }

  .facts dt {
    color: var(--text-muted);
    font-size: 13px;
  }

  .facts dd {
    margin: 0;
  }

  .path {
    font-family: "Cascadia Mono", ui-monospace, monospace;
    font-size: 12.5px;
    word-break: break-all;
  }

  .scope {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 10px 12px;
    margin: 0 0 12px;
  }

  .scope legend {
    padding: 0 6px;
    color: var(--text-muted);
    font-size: 13px;
  }

  .scope label {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 0;
  }

  .preserved {
    margin-top: 10px;
  }

  .preserved summary {
    cursor: pointer;
  }

  .license {
    margin-top: 16px;
    border-top: 1px solid var(--border);
    padding-top: 14px;
  }

  .license h3 {
    margin: 0;
    font-size: 13px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }

  /* Tinggi tetap dan bergulir: teks lisensi puluhan ribu karakter tidak boleh
     mendorong tombol Install keluar dari layar. */
  .license-body {
    max-height: 180px;
    overflow-y: auto;
    margin: 10px 0;
    padding: 10px 12px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }

  .license-body pre {
    margin: 0;
    font-family: "Cascadia Mono", ui-monospace, monospace;
    font-size: 11.5px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--text-muted);
  }

  .accept {
    width: 100%;
  }

  .accept.accepted {
    background: var(--success-bg);
    border-color: color-mix(in srgb, var(--success) 40%, transparent);
    color: var(--success);
    font-weight: 600;
  }
</style>
