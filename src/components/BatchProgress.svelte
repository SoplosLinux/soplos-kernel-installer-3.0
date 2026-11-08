<script lang="ts">
  import { afterUpdate, beforeUpdate } from "svelte";
  import { _ } from "svelte-i18n";
  import BuildStats from "./BuildStats.svelte";
  import CancelDialog from "./CancelDialog.svelte";
  import { cancelBuild, job } from "../lib/build";
  import { fitToWindow } from "../lib/fit";

  // The follow-up of a batch of kernels that is being built: where it is, the
  // stage of the build in course, everything the build says and what is already
  // finished. It is the same in the Batch page and in the Build page, so neither
  // is left empty while the queue goes on.
  let showLog = true;
  let confirmCancel = false;
  let logBox: HTMLPreElement | undefined;
  let stick = true;

  // The output follows the end of the log unless it was scrolled up to read it.
  beforeUpdate(() => {
    stick = !logBox || logBox.scrollHeight - logBox.scrollTop - logBox.clientHeight < 40;
  });
  afterUpdate(() => {
    if (logBox && stick) logBox.scrollTop = logBox.scrollHeight;
  });

  async function cancel(deleteFolder: boolean) {
    confirmCancel = false;
    await cancelBuild(deleteFolder);
  }

  $: batch = $job.batch;
  $: last = $job.lines[$job.lines.length - 1] ?? "";

  const secondary = "px-4 py-2 rounded border border-border text-[0.85rem] text-text-bright hover:bg-surface-2 disabled:opacity-40";
</script>

{#if batch}
  <section class="rounded border border-border bg-surface p-5">
    <h3 class="text-[1.05rem] font-semibold text-text-bright mb-1">
      {$_("batch.running", { values: { index: Math.max(batch.index, 1), total: batch.total } })}
    </h3>
    <p class="text-[0.9rem] text-primary mb-1">{batch.name}</p>
    <p class="text-[0.8rem] text-text-muted mb-2">
      {#if $job.stage}{$_(`build.progress.stages.${$job.stage}`)} · {Math.round($job.percent)}%{/if}
    </p>
    <div class="h-1.5 rounded bg-surface-2 overflow-hidden mb-3">
      <div class="h-full bg-primary transition-[width]" style="width: {Math.max(0, Math.min(100, $job.percent))}%"></div>
    </div>
    <p class="text-[0.8rem] text-text-muted truncate mb-3" title={last}>{last}</p>
    {#if batch.completed.length > 0}
      <p class="text-[0.78rem] text-text-muted mb-3 break-words">{$_("batch.finished", { values: { names: batch.completed.join(", ") } })}</p>
    {/if}
    <BuildStats />
    <div class="flex items-center gap-3">
      <button class={secondary} on:click={() => (confirmCancel = true)}>{$_("batch.cancel")}</button>
      <button class="text-[0.8rem] text-primary hover:underline" on:click={() => (showLog = !showLog)}>
        {showLog ? $_("batch.hideLog") : $_("batch.showLog")}
      </button>
    </div>
    {#if showLog}
      <pre bind:this={logBox} use:fitToWindow class="content-scroll mt-3 min-h-[140px] overflow-auto overscroll-contain rounded bg-surface-2 p-3 text-[0.72rem] text-text-muted select-text whitespace-pre-wrap">{$job.lines.join("\n")}</pre>
    {/if}
  </section>
{/if}

{#if confirmCancel}
  <CancelDialog title={$_("batch.cancelTitle")} text={$_("batch.cancelText")} onChoose={cancel} onClose={() => (confirmCancel = false)} />
{/if}
