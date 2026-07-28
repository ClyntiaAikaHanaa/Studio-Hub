<script lang="ts">
  // Konfirmasi uninstall (FR-5.1, FR-5.2).
  //
  // Checkbox "hapus juga preset saya" **default tidak dicentang**. Preset adalah
  // pekerjaan pengguna; menghapusnya diam-diam saat mereka hanya ingin membuang
  // plugin adalah kerusakan yang tidak dapat dibatalkan.

  import * as api from "../lib/api";
  import { BackendError } from "../lib/api";
  import { uninstallRequest } from "../lib/dialogs";
  import { t } from "../lib/i18n";
  import { refreshLibrary, refreshUpdates } from "../lib/store";
  import type { HubError } from "../lib/types";
  import Modal from "./Modal.svelte";
  import ErrorPanel from "./ErrorPanel.svelte";

  let removeUserData = $state(false);
  let busy = $state(false);
  let error = $state<HubError | null>(null);
  let failures = $state<string[]>([]);

  let request = $derived($uninstallRequest);

  $effect(() => {
    if (request) {
      removeUserData = false;
      error = null;
      failures = [];
    }
  });

  async function confirm() {
    if (!request) return;
    busy = true;
    error = null;
    try {
      failures = await api.uninstallStart(request.pluginId, removeUserData);
      if (failures.length === 0) {
        await close();
      }
    } catch (e) {
      if (e instanceof BackendError) error = e.hub;
    } finally {
      busy = false;
    }
  }

  async function close() {
    uninstallRequest.set(null);
    await refreshLibrary();
    await refreshUpdates();
  }
</script>

{#if request}
  <Modal
    title={$t("uninstall.title", { name: request.pluginName })}
    onclose={close}
    dismissible={!busy}
  >
    {#if error}
      <ErrorPanel {error} onclose={close} />
    {:else}
      <p>{$t("uninstall.body")}</p>

      {#if request.adopted}
        <div class="notice warn">{$t("uninstall.adoptedWarning")}</div>
      {/if}

      <label class="checkbox">
        <input type="checkbox" bind:checked={removeUserData} />
        <span>{$t("uninstall.removeUserData")}</span>
      </label>
      <p class="small muted">{$t("uninstall.keepUserDataHint")}</p>

      {#if failures.length}
        <div class="notice warn selectable small">
          <ul>
            {#each failures as failure (failure)}
              <li>{failure}</li>
            {/each}
          </ul>
        </div>
      {/if}
    {/if}

    {#snippet footer()}
      <button onclick={close} disabled={busy}>{$t("common.cancel")}</button>
      {#if !error}
        <button class="danger" onclick={confirm} disabled={busy}>
          {$t("common.uninstall")}
        </button>
      {/if}
    {/snippet}
  </Modal>
{/if}

<style>
  .checkbox {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 12px 0 2px;
  }

  .notice {
    margin: 10px 0;
  }
</style>
