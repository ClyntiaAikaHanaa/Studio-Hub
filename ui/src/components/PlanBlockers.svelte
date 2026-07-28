<script lang="ts">
  // Blockers dan warnings dari `InstallPlan` (PRD §12.3).
  //
  // Blocker dihitung sebelum eksekusi, sehingga kegagalan yang dapat diprediksi
  // muncul di dialog konfirmasi — bukan sebagai error di tengah progress bar.

  import { t } from "../lib/i18n";
  import { formatBytes } from "../lib/format";
  import type { InstallPlan } from "../lib/types";

  interface Props {
    plan: InstallPlan;
  }

  let { plan }: Props = $props();
</script>

{#each plan.blockers as blocker (JSON.stringify(blocker))}
  <div class="notice danger" role="alert">
    {#if blocker.kind === "insufficientDisk"}
      <strong>{$t("error.disk.title")}</strong>
      <div>
        {$t("error.disk.body", {
          required: formatBytes(blocker.required),
          available: formatBytes(blocker.available),
          volume: blocker.volume,
        })}
      </div>
    {:else if blocker.kind === "cpuFeatureMissing"}
      <strong>{$t("error.prereq.title")}</strong>
      <div>{blocker.feature}</div>
    {:else if blocker.kind === "osTooOld"}
      <strong>{$t("error.prereq.title")}</strong>
      <div>Windows build {blocker.required}</div>
    {:else if blocker.kind === "launcherTooOld"}
      <strong>{$t("error.launcherTooOld.title")}</strong>
      <div>{$t("error.launcherTooOld.body", { required: blocker.required })}</div>
    {:else if blocker.kind === "fileLocked"}
      {@const named = blocker.holders.map((h) => h.name).filter(Boolean)}
      <strong>
        {named.length
          ? $t("blocked.dawRunning.title", { daw: String(named[0]) })
          : $t("blocked.dawUnknown.title")}
      </strong>
      <div>
        {named.length ? $t("blocked.dawRunning.body") : $t("blocked.dawUnknown.body")}
      </div>
    {:else}
      <strong>{$t("explore.unavailable")}</strong>
    {/if}
  </div>
{/each}

{#each plan.warnings as warning (JSON.stringify(warning))}
  <div class="notice warn">
    {#if warning.kind === "breakingChange"}
      <strong>{$t("updates.breaking")}</strong>
      <div>{$t("updates.breakingHelp")}</div>
      {#if warning.summary}
        <div class="small">{warning.summary}</div>
      {/if}
    {:else if warning.kind === "elevationWillBeRequested"}
      {$t("warning.elevation")}
    {:else if warning.kind === "perUserLocationMayNeedDawConfig"}
      {$t("warning.perUserPath", { path: warning.path })}
    {:else if warning.kind === "replacingAdoptedInstall"}
      {$t("warning.adopted")}
    {:else if warning.kind === "rollbackMayBreakPresets"}
      {$t("warning.rollbackPresets")}
    {:else if warning.kind === "prereqMissing"}
      <strong>{warning.name}</strong>
      <div>{warning.detail}</div>
    {/if}
  </div>
{/each}

<style>
  .notice {
    margin-bottom: 10px;
  }

  .notice strong {
    display: block;
    margin-bottom: 2px;
  }
</style>
