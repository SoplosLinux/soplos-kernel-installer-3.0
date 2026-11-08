<script lang="ts">
  import { _ } from "svelte-i18n";
  import Modal from "./Modal.svelte";

  // What to do with the folder of the builds when a build is cancelled, as in the
  // 2.x program: delete it, keep it so the next build reuses what was downloaded,
  // or go on building.
  export let title: string;
  /** What cancelling does here, before the question about the folder. */
  export let text = "";
  export let onChoose: (deleteFolder: boolean) => void;
  export let onClose: () => void;

  const button = "px-3 py-2 rounded border border-border text-center text-[0.85rem] text-text-bright hover:bg-surface-2";
</script>

<Modal variant="warning" {title} {onClose} showFooter={false} maxWidth={620}>
  {#if text}<p class="mb-2">{text}</p>{/if}
  <p class="mb-4">{$_("cancel.question")}</p>
  <div class="grid grid-cols-3 gap-2">
    <button class={button} on:click={onClose}>{$_("build.progress.keepGoing")}</button>
    <button class={button} on:click={() => onChoose(false)}>{$_("cancel.keep")}</button>
    <button class="px-3 py-2 rounded bg-danger hover:bg-danger/85 text-center text-text-bright text-[0.85rem] font-semibold" on:click={() => onChoose(true)}>{$_("cancel.delete")}</button>
  </div>
</Modal>
