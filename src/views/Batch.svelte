<script lang="ts">
  import { onMount } from "svelte";
  import { _ } from "svelte-i18n";
  import { open } from "@tauri-apps/plugin-dialog";
  import Modal from "../components/Modal.svelte";
  import NumberField from "../components/NumberField.svelte";
  import { api } from "../lib/api";
  import { building, job } from "../lib/build";
  import { progress } from "../lib/progress";

  interface KernelVersion {
    version: string;
    channel: "stable" | "longterm" | "rc";
    eol: boolean;
    recommended: boolean;
  }
  interface BatchJob {
    name: string;
    patches: string[];
    march: string;
  }

  let versions: KernelVersion[] = [];
  let version = "";
  let dest = "";
  let revisionValue: number | null = 0;
  let revision = 0;
  $: revision = revisionValue ?? 0;
  let resume = false;
  let cores = 1;
  let maxCores = 1;
  let queue: BatchJob[] = [];
  let selected = new Set<string>();
  let built = new Set<string>();
  let confirm = false;
  let showLog = false;
  let confirmCancel = false;
  let error = "";

  // A kernel of another channel never shares a name with the stable line.
  $: channel = ((): string => {
    const v = versions.find((k) => k.version === version);
    if (!v) return "";
    if (v.channel === "rc") return "rc";
    if (v.channel === "longterm") return "lts";
    return v.eol ? "eol" : "";
  })();

  onMount(async () => {
    try {
      const env = await api.invoke<{ buildDir: string; cpuThreads: number }>("get_build_environment");
      maxCores = env.cpuThreads;
      cores = env.cpuThreads;
      // The folder of the last queue, if there was one: each person has their own.
      try {
        dest = localStorage.getItem("soplos-kernel-installer-batch-dest") ?? "";
      } catch {
        dest = "";
      }
      versions = await api.invoke<KernelVersion[]>("list_kernel_versions");
      version = versions.find((k) => k.recommended)?.version ?? versions[0]?.version ?? "";
    } catch (e) {
      error = String(e);
    }
  });

  async function loadQueue(c: string) {
    try {
      queue = await api.invoke<BatchJob[]>("get_batch_queue", { channel: c });
      selected = new Set(queue.map((j) => j.name));
    } catch (e) {
      error = String(e);
    }
  }
  $: if (version) loadQueue(channel);

  // What is already in the destination, for this version and revision.
  async function loadBuilt() {
    if (!version || !dest.startsWith("/")) {
      built = new Set();
      return;
    }
    try {
      built = new Set(await api.invoke<string[]>("batch_built", { dest, version, channel, revision }));
    } catch {
      built = new Set();
    }
  }
  $: version, dest, revision, channel, loadBuilt();

  $: chosen = queue.filter((j) => selected.has(j.name) && !(resume && built.has(j.name)));
  $: destOk = dest.startsWith("/") && dest.length > 1;

  function toggle(name: string) {
    const next = new Set(selected);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    selected = next;
  }

  async function chooseFolder() {
    try {
      const chosen = await open({ multiple: false, directory: true, title: $_("batch.chooseTitle"), defaultPath: destOk ? dest : undefined });
      if (typeof chosen === "string") dest = chosen;
    } catch (e) {
      error = String(e);
    }
  }

  async function start() {
    confirm = false;
    error = "";
    try {
      localStorage.setItem("soplos-kernel-installer-batch-dest", dest);
    } catch {
      // Not remembering the folder is not a failure.
    }
    try {
      await api.invoke("start_batch", { spec: { version, channel, dest, revision, cores, resume, jobs: [...selected] } });
    } catch (e) {
      error = String(e);
    }
  }

  async function cancel() {
    confirmCancel = false;
    await api.invoke("cancel_build");
  }

  function again() {
    job.update((j) => ({ ...j, result: null, error: null, batch: null }));
  }

  $: progress.set($building && $job.batch ? ($job.batch.completed.length + $job.percent / 100) / Math.max($job.batch.total, 1) : null);
  $: inBatch = $job.batch !== null && ($job.running || $job.result !== null);

  const primary = "px-5 py-2 rounded bg-primary hover:bg-primary-hover text-text-bright text-[0.88rem] font-semibold disabled:opacity-40";
  const secondary = "px-4 py-2 rounded border border-border text-[0.85rem] text-text-bright hover:bg-surface-2 disabled:opacity-40";
  const card = "rounded border border-border bg-surface p-5";
  const input = "bg-surface-2 border border-border rounded px-2.5 py-1.5 text-[0.85rem] text-text-bright outline-none focus-visible:ring-2 focus-visible:ring-primary/60";
</script>

{#if inBatch && $job.batch}
  <section class={card}>
    {#if $job.result === "done"}
      <h3 class="text-[1.05rem] font-semibold text-text-bright mb-1">{$_("batch.doneTitle")}</h3>
      <p class="text-[0.85rem] text-text mb-3 break-all select-text">{$_("batch.doneText", { values: { n: $job.batch.completed.length, dest: $job.batch.dest } })}</p>
      <button class={primary} on:click={again}>{$_("batch.again")}</button>
    {:else if $job.result === "failed"}
      <h3 class="text-[1.05rem] font-semibold text-danger mb-1">{$_("batch.failedTitle")}</h3>
      <p class="text-[0.85rem] text-text mb-2 break-words select-text">{$job.error}</p>
      <p class="text-[0.8rem] text-text-muted mb-3 break-all select-text">
        {$_("batch.failedText", { values: { n: $job.batch.completed.length, folder: $job.batch.failed ?? "" } })}
      </p>
      <button class={primary} on:click={again}>{$_("batch.again")}</button>
    {:else if $job.result === "cancelled"}
      <h3 class="text-[1.05rem] font-semibold text-text-bright mb-1">{$_("batch.cancelledTitle")}</h3>
      <p class="text-[0.85rem] text-text mb-3">{$_("batch.cancelledText", { values: { n: $job.batch.completed.length } })}</p>
      <button class={primary} on:click={again}>{$_("batch.again")}</button>
    {:else}
      <h3 class="text-[1.05rem] font-semibold text-text-bright mb-1">
        {$_("batch.running", { values: { index: Math.max($job.batch.index, 1), total: $job.batch.total } })}
      </h3>
      <p class="text-[0.9rem] text-primary mb-3">{$job.batch.name}</p>
      <p class="text-[0.8rem] text-text-muted truncate mb-3" title={$job.lines[$job.lines.length - 1] ?? ""}>{$job.lines[$job.lines.length - 1] ?? ""}</p>
      {#if $job.batch.completed.length > 0}
        <p class="text-[0.78rem] text-text-muted mb-3 break-words">{$_("batch.finished", { values: { names: $job.batch.completed.join(", ") } })}</p>
      {/if}
      <div class="flex items-center gap-3">
        <button class={secondary} on:click={() => (confirmCancel = true)}>{$_("batch.cancel")}</button>
        <button class="text-[0.8rem] text-primary hover:underline" on:click={() => (showLog = !showLog)}>
          {showLog ? $_("batch.hideLog") : $_("batch.showLog")}
        </button>
      </div>
      {#if showLog}
        <pre class="mt-3 max-h-[260px] overflow-auto rounded bg-surface-2 p-3 text-[0.72rem] text-text-muted select-text whitespace-pre-wrap">{$job.lines.join("\n")}</pre>
      {/if}
    {/if}
  </section>
{:else if $building}
  <section class={card}><p class="text-[0.88rem] text-text">{$_("batch.busy")}</p></section>
{:else}
  <section class={card}>
    <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-3 text-[0.85rem] mb-4">
      <dt class="text-text-muted">{$_("batch.version")}</dt>
      <dd>
        <select class="{input} appearance-none pr-6" bind:value={version}>
          {#each versions as v (v.version)}
            <option value={v.version}>{v.version}{v.channel === "rc" ? " (rc)" : v.channel === "longterm" ? " (lts)" : v.eol ? " (eol)" : ""}</option>
          {/each}
        </select>
      </dd>

      <dt class="text-text-muted">{$_("batch.dest")}</dt>
      <dd>
        <div class="flex items-center gap-2">
          <input class="{input} w-full max-w-[460px]" bind:value={dest} spellcheck="false" />
          <button class={secondary} on:click={chooseFolder}>{$_("batch.choose")}</button>
        </div>
        <p class="text-[0.75rem] text-text-muted mt-1">{$_("batch.destHint")}</p>
        {#if dest && !destOk}<p class="text-[0.75rem] text-warning">{$_("batch.destRelative")}</p>{/if}
      </dd>

      <dt class="text-text-muted">{$_("batch.revision")}</dt>
      <dd>
        <NumberField bind:value={revisionValue} min={0} max={98} />
        <p class="text-[0.75rem] text-text-muted mt-1 max-w-[520px]">{$_("batch.revisionHint")}</p>
      </dd>

      <dt class="text-text-muted">{$_("batch.cores")}</dt>
      <dd class="flex items-center gap-3">
        <input type="range" min="1" max={maxCores} bind:value={cores} class="w-[200px]" />
        <span class="text-text-bright">{cores} / {maxCores}</span>
      </dd>
    </dl>

    <label class="flex items-center gap-2 text-[0.85rem] text-text-bright mb-4">
      <input type="checkbox" bind:checked={resume} />{$_("batch.resume")}
    </label>

    <div class="flex items-center gap-3 mb-2">
      <h3 class="text-[0.95rem] font-semibold text-text-bright">{$_("batch.kernels")}</h3>
      <button class="text-[0.78rem] text-primary hover:underline" on:click={() => (selected = new Set(queue.map((j) => j.name)))}>{$_("batch.selectAll")}</button>
      <button class="text-[0.78rem] text-primary hover:underline" on:click={() => (selected = new Set())}>{$_("batch.selectNone")}</button>
      <span class="text-[0.78rem] text-text-muted">{$_("batch.selectedCount", { values: { n: selected.size, total: queue.length } })}</span>
    </div>
    <ul class="grid grid-cols-1 sm:grid-cols-2 gap-x-6 gap-y-1 max-h-[300px] overflow-auto mb-4">
      {#each queue as j (j.name)}
        <li>
          <label class="flex items-center gap-2 text-[0.82rem] text-text-bright">
            <input type="checkbox" checked={selected.has(j.name)} on:change={() => toggle(j.name)} />
            {j.name}
            {#if built.has(j.name)}<span class="text-[0.7rem] text-primary">{$_("batch.built")}</span>{/if}
          </label>
        </li>
      {/each}
    </ul>

    {#if error}<p class="text-[0.8rem] text-danger mb-2 break-words">{error}</p>{/if}
    <button class={primary} disabled={chosen.length === 0 || !destOk || !version} on:click={() => (confirm = true)}>
      {$_("batch.start", { values: { n: chosen.length } })}
    </button>
    {#if resume && selected.size > 0 && chosen.length === 0}
      <p class="text-[0.78rem] text-text-muted mt-2">{$_("batch.allBuilt")}</p>
    {/if}
  </section>
{/if}

{#if confirm}
  <Modal variant="confirm" title={$_("batch.confirmTitle")} confirmLabel={$_("batch.start", { values: { n: chosen.length } })} onConfirm={start} onClose={() => (confirm = false)}>
    <p class="text-[0.85rem] text-text break-all">{$_("batch.confirmText", { values: { n: chosen.length, version, dest } })}</p>
  </Modal>
{/if}

{#if confirmCancel}
  <Modal variant="warning" title={$_("batch.cancelTitle")} confirmLabel={$_("batch.cancel")} cancelLabel={$_("batch.keepGoing")} onConfirm={cancel} onClose={() => (confirmCancel = false)}>
    <p class="text-[0.85rem] text-text">{$_("batch.cancelText")}</p>
  </Modal>
{/if}
