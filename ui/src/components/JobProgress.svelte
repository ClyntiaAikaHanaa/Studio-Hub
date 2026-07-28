<script lang="ts">
  // Progres job (PRD §8.6, §12.4).
  //
  // Setiap tahap punya teksnya sendiri. "Memeriksa integritas…" bukan detail
  // implementasi yang bocor ke UI — ia adalah alasan launcher ini layak
  // dipercaya, dan pengguna berhak melihatnya terjadi.

  import { t } from "../lib/i18n";
  import { formatBytes, formatSpeed, percent } from "../lib/format";
  import { presentError } from "../lib/errors";
  import type { JobEvent } from "../lib/types";

  interface Props {
    job: JobEvent;
  }

  let { job }: Props = $props();

  let pct = $derived(job.kind === "downloading" ? percent(job.received, job.total) : 0);
</script>

{#if job.kind === "succeeded"}
  <div class="notice info success" role="status">
    <strong>{$t("success.title")}</strong>
    {#if job.needsRescan}
      <div>{$t("success.rescan")}</div>
    {/if}
  </div>
{:else if job.kind === "failed"}
  {@const view = presentError(job.error)}
  <div class="notice danger" role="alert">
    <strong>{view.title}</strong>
    <div>{view.body}</div>
    {#if view.detail}
      <details class="small">
        <summary>{$t("common.details")}</summary>
        <code class="selectable">{view.detail}</code>
      </details>
    {/if}
  </div>
{:else if job.kind === "cancelled"}
  <p class="muted">{$t("error.cancelled")}</p>
{:else}
  <div class="progress">
    <div class="label">
      {#if job.kind === "downloading"}
        {$t("progress.downloading", {
          percent: pct,
          received: formatBytes(job.received),
          total: formatBytes(job.total),
          speed: formatSpeed(job.bytesPerSec),
        })}
      {:else if job.kind === "verifying"}
        {$t("progress.verifying")}
      {:else if job.kind === "extracting"}
        {$t("progress.extracting")}
      {:else if job.kind === "elevating"}
        {$t("progress.elevating")}
      {:else if job.kind === "backingUp"}
        {$t("progress.backingUp")}
      {:else if job.kind === "installing"}
        {$t("progress.installing")}
      {:else if job.kind === "rollingBack"}
        {$t("progress.rollingBack")}
      {:else}
        {$t("progress.queued")}
      {/if}
    </div>

    <div
      class="progress-track"
      role="progressbar"
      aria-valuemin="0"
      aria-valuemax="100"
      aria-valuenow={job.kind === "downloading" ? pct : undefined}
    >
      {#if job.kind === "downloading"}
        <div class="progress-fill" style="width: {pct}%"></div>
      {:else}
        <!-- Tahap tanpa progres terukur: bar indeterminate lebih jujur
             daripada persentase yang dikarang. -->
        <div class="progress-fill indeterminate"></div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .progress {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 4px 0 8px;
  }

  .label {
    font-size: 13px;
    font-variant-numeric: tabular-nums;
  }

  .success {
    background: var(--success-bg);
    color: var(--success);
  }

  .notice strong {
    display: block;
    margin-bottom: 2px;
  }

  details {
    margin-top: 6px;
  }

  summary {
    cursor: pointer;
  }
</style>
