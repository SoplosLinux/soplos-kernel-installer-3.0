<script lang="ts">
  import { getContext, onDestroy, onMount } from "svelte";
  import { _ } from "svelte-i18n";
  import { open } from "@tauri-apps/plugin-dialog";
  import BatchProgress from "../components/BatchProgress.svelte";
  import BuildTimes from "../components/BuildTimes.svelte";
  import Modal from "../components/Modal.svelte";
  import NumberField from "../components/NumberField.svelte";
  import { api } from "../lib/api";
  import { building, job } from "../lib/build";
  import { duration } from "../lib/time";
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

  const navigate = getContext<(id: string) => void>("navigate");

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

  // The queue by architecture level, so each level reads as one group.
  const LEVELS = ["v1", "v2", "v3", "v4"];
  $: byLevel = LEVELS.map((march) => ({ march, jobs: queue.filter((j) => j.march === march) })).filter((g) => g.jobs.length > 0);

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

  // What a finished queue says is seen once: it goes when the page is left.
  onDestroy(() => {
    job.update((j) => (!j.running && j.batch !== null && j.result !== null ? { ...j, result: null, error: null, batch: null } : j));
  });

  // How long the queue took, and each kernel of it.
  $: queueTime = $job.batch ? $job.batch.timings.reduce((a, b) => a + b, 0) : 0;
  $: queueTimes = $job.batch ? $job.batch.completed.map((name, i) => `${name} (${duration($job.batch?.timings[i] ?? 0)})`).join(", ") : "";

  $: progress.set($building && $job.batch ? ($job.batch.completed.length + $job.percent / 100) / Math.max($job.batch.total, 1) : null);

  const primary = "px-5 py-2 rounded bg-primary hover:bg-primary-hover text-text-bright text-[0.88rem] font-semibold disabled:opacity-40";
  const secondary = "px-4 py-2 rounded border border-border text-[0.85rem] text-text-bright hover:bg-surface-2 disabled:opacity-40";
  const card = "rounded border border-border bg-surface p-5";
  const heading = "text-[0.95rem] font-semibold text-text-bright mb-3";
  const input = "bg-surface-2 border border-border rounded px-2.5 py-1.5 text-[0.85rem] text-text-bright outline-none focus-visible:ring-2 focus-visible:ring-primary/60";
</script>

{#if $job.batch && $job.running}
  <BatchProgress />
{:else if $building}
  <!-- A build that is not of this page is going on: what it is doing, and the way to its output. -->
  <section class={card}>
    <p class="text-[0.88rem] text-text mb-2">{$_("batch.busy")}</p>
    {#if $job.stage}
      <p class="text-[0.82rem] text-primary mb-1">{$_(`build.progress.stages.${$job.stage}`)} · {Math.round($job.percent)}%</p>
    {/if}
    <p class="text-[0.8rem] text-text-muted truncate mb-3" title={$job.lines[$job.lines.length - 1] ?? ""}>{$job.lines[$job.lines.length - 1] ?? ""}</p>
    <button class={secondary} on:click={() => navigate("build")}>{$_("build.progress.showLog")}</button>
  </section>
{:else}
  <!-- How a queue ended (finished or failed) is shown once, on top of the page of the batch, which is always there. A queue that was cancelled says nothing: it was meant. -->
  {#if $job.batch && ($job.result === "done" || $job.result === "failed")}
    <section class="{card} mb-4">
      {#if $job.result === "done"}
        <h3 class="text-[1.05rem] font-semibold text-text-bright mb-1">{$_("batch.doneTitle")}</h3>
        <p class="text-[0.85rem] text-text break-all select-text">{$_("batch.doneText", { values: { n: $job.batch.completed.length, dest: $job.batch.dest } })}</p>
        <p class="text-[0.8rem] text-text-muted mt-2 break-words select-text">{$_("stats.time")}: <span class="font-mono text-text-bright">{duration(queueTime)}</span></p>
        <p class="text-[0.78rem] text-text-muted mt-1 break-words select-text">{$_("batch.finished", { values: { names: queueTimes } })}</p>
      {:else if $job.result === "failed"}
        <h3 class="text-[1.05rem] font-semibold text-danger mb-1">{$_("batch.failedTitle")}</h3>
        <p class="text-[0.85rem] text-text mb-2 break-words select-text">{$job.error}</p>
        <p class="text-[0.8rem] text-text-muted break-all select-text">
          {$_("batch.failedText", { values: { n: $job.batch.completed.length, folder: $job.batch.failed ?? "" } })}
        </p>
      {/if}
    </section>
  {/if}
  <div class="flex flex-col gap-4">
    <section class={card}>
      <h3 class={heading}>{$_("batch.version")}</h3>
      <div class="flex flex-wrap items-center gap-x-6 gap-y-3">
        <select class="{input} appearance-none pr-6" bind:value={version}>
          {#each versions as v (v.version)}
            <option value={v.version}>{v.version}{v.channel === "rc" ? " (rc)" : v.channel === "longterm" ? " (lts)" : v.eol ? " (eol)" : ""}</option>
          {/each}
        </select>
        <div class="flex items-center gap-2">
          <span class="text-[0.85rem] text-text-muted">{$_("batch.revision")}</span>
          <NumberField bind:value={revisionValue} min={0} max={98} />
        </div>
      </div>
      <p class="text-[0.75rem] text-text-muted mt-2 max-w-[560px]">{$_("batch.revisionHint")}</p>
    </section>

    <section class={card}>
      <h3 class={heading}>{$_("batch.dest")}</h3>
      <div class="flex items-center gap-2">
        <input class="{input} w-full max-w-[460px]" bind:value={dest} spellcheck="false" />
        <button class={secondary} on:click={chooseFolder}>{$_("batch.choose")}</button>
      </div>
      <p class="text-[0.75rem] text-text-muted mt-2">{$_("batch.destHint")}</p>
      {#if dest && !destOk}<p class="text-[0.75rem] text-warning">{$_("batch.destRelative")}</p>{/if}
    </section>

    <section class={card}>
      <h3 class={heading}>{$_("batch.options")}</h3>
      <div class="flex items-center gap-3 text-[0.85rem] mb-3">
        <span class="text-text-muted">{$_("batch.cores")}</span>
        <input type="range" min="1" max={maxCores} bind:value={cores} class="w-[200px]" />
        <span class="text-text-bright">{cores} / {maxCores}</span>
      </div>
      <label class="flex items-center gap-2 text-[0.85rem] text-text-bright">
        <input type="checkbox" bind:checked={resume} />{$_("batch.resume")}
      </label>
    </section>

    <section class={card}>
      <div class="flex flex-wrap items-center gap-3 mb-3">
        <h3 class="text-[0.95rem] font-semibold text-text-bright">{$_("batch.kernels")}</h3>
        <button class="text-[0.78rem] text-primary hover:underline" on:click={() => (selected = new Set(queue.map((j) => j.name)))}>{$_("batch.selectAll")}</button>
        <button class="text-[0.78rem] text-primary hover:underline" on:click={() => (selected = new Set())}>{$_("batch.selectNone")}</button>
        <span class="text-[0.78rem] text-text-muted">{$_("batch.selectedCount", { values: { n: selected.size, total: queue.length } })}</span>
      </div>
      <div class="flex flex-col gap-3 max-h-[340px] overflow-auto">
        {#each byLevel as group (group.march)}
          <div>
            <p class="text-[0.82rem] font-semibold text-text-muted mb-1">{$_("batch.kernels")} {group.march.toUpperCase()}</p>
            <ul class="grid grid-cols-1 sm:grid-cols-2 gap-x-6 gap-y-1">
              {#each group.jobs as j (j.name)}
                <li>
                  <label class="flex items-center gap-2 text-[0.82rem] text-text-bright">
                    <input type="checkbox" checked={selected.has(j.name)} on:change={() => toggle(j.name)} />
                    {j.name}
                    {#if built.has(j.name)}<span class="text-[0.7rem] text-primary">{$_("batch.built")}</span>{/if}
                  </label>
                </li>
              {/each}
            </ul>
          </div>
        {/each}
      </div>
    </section>

    <!-- The action is out of the boxes: they are the settings, this is what starts it. -->
    <div class="flex flex-col items-end">
      {#if error}<p class="text-[0.8rem] text-danger mb-2 break-words text-right">{error}</p>{/if}
      <button class={primary} disabled={chosen.length === 0 || !destOk || !version} on:click={() => (confirm = true)}>
        {$_("batch.start", { values: { n: chosen.length } })}
      </button>
      {#if resume && selected.size > 0 && chosen.length === 0}
        <p class="text-[0.78rem] text-text-muted mt-2 text-right">{$_("batch.allBuilt")}</p>
      {/if}
    </div>

    <BuildTimes />
  </div>
{/if}

{#if confirm}
  <Modal variant="confirm" title={$_("batch.confirmTitle")} confirmLabel={$_("batch.start", { values: { n: chosen.length } })} onConfirm={start} onClose={() => (confirm = false)}>
    <p class="text-[0.85rem] text-text break-all">{$_("batch.confirmText", { values: { n: chosen.length, version, dest } })}</p>
  </Modal>
{/if}
