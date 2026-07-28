<script lang="ts">
  // Menampilkan error sebagai judul / penjelasan / aksi (PRD §18.3).
  // Kode teknis ada di balik "Detail", bukan sebagai hal pertama yang dilihat.

  import * as api from "../lib/api";
  import { t } from "../lib/i18n";
  import { presentError } from "../lib/errors";
  import type { HubError } from "../lib/types";

  interface Props {
    error: HubError;
    onretry?: () => void;
    oninstallperuser?: () => void;
    onclose?: () => void;
  }

  let { error, onretry, oninstallperuser, onclose }: Props = $props();

  let view = $derived(presentError(error));

  async function run(action: string, href?: string) {
    switch (action) {
      case "retry":
        onretry?.();
        break;
      case "installPerUser":
        oninstallperuser?.();
        break;
      case "openHelp":
        // Backend memvalidasi skema dan host; frontend hanya meneruskan URL.
        if (href) await api.openExternal(href);
        break;
      case "viewLog":
        await api.logsOpen();
        break;
      case "close":
        onclose?.();
        break;
      default:
        onclose?.();
    }
  }
</script>

<div class="notice danger" role="alert">
  <strong>{view.title}</strong>
  {#if view.body}
    <div>{view.body}</div>
  {/if}
  {#if view.detail}
    <details class="small">
      <summary>{$t("common.details")}</summary>
      <code class="selectable">{view.detail}</code>
    </details>
  {/if}
</div>

{#if view.actions.length}
  <div class="actions">
    {#each view.actions as action (action.labelKey)}
      <button
        class:primary={action.action === "retry" || action.action === "installPerUser"}
        onclick={() => run(action.action, action.href)}
      >
        {$t(action.labelKey)}
      </button>
    {/each}
  </div>
{/if}

<style>
  .notice strong {
    display: block;
    margin-bottom: 2px;
  }

  .actions {
    display: flex;
    gap: 8px;
    margin-top: 12px;
    flex-wrap: wrap;
  }

  details {
    margin-top: 6px;
  }

  summary {
    cursor: pointer;
  }
</style>
