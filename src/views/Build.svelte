<script lang="ts">
  import { getContext, onMount } from "svelte";
  import { _ } from "svelte-i18n";
  import { AlertTriangle, AudioLines, Briefcase, Check, ChevronDown, Gamepad2, Minus, Sparkles, Wand2 } from "lucide-svelte";
  import Modal from "../components/Modal.svelte";
  import { api } from "../lib/api";
  import { building, job, type Recipe } from "../lib/build";
  import { progress } from "../lib/progress";

  interface KernelVersion {
    version: string;
    channel: "stable" | "longterm" | "rc";
    eol: boolean;
    released: string;
    url: string;
    recommended: boolean;
  }
  interface Env {
    cpu: string | null;
    cpuThreads: number;
    march: string;
    freeBytes: number;
    buildDir: string;
    deps: { missing: string[]; broken: string[]; rustc: string | null; bindgen: string | null };
    nvidia: { present: boolean; driver: string | null };
    keysExist: boolean;
    x3d: boolean;
    goals: { id: string; patches: string[] }[];
  }

  const navigate = getContext<(id: string) => void>("navigate");

  // The estimate of what a build takes on disk, in GB: sources, objects and packages.
  const NEEDED_GB = 25;
  const STEPS = ["goal", "version", "computer", "recipe"];
  const GOALS: { id: string; icon: typeof Gamepad2 }[] = [
    { id: "gaming", icon: Gamepad2 },
    { id: "audio", icon: AudioLines },
    { id: "minimal", icon: Briefcase },
    { id: "optimized", icon: Sparkles },
    { id: "stock", icon: Wand2 },
  ];
  const PATCHES = ["bore", "rt", "zen", "ntsync", "x3d"];
  const STAGES = ["deps", "download", "verify", "extract", "patches", "config", "build", "packages", "install"];

  let step = 0;
  let versions: KernelVersion[] = [];
  let versionsError = "";
  let loadingVersions = true;
  let env: Env | null = null;
  let showRc = false;
  let showOther = false;
  let showLog = false;
  let showAdvanced = false;
  let confirmCancel = false;
  let failure = "";
  let cleaned = "";
  let sources = { exists: false, size: 0 };

  let recipe: Recipe = {
    version: "",
    goal: "",
    patches: [],
    march: "v1",
    name: "soplos",
    sign: false,
    cores: 1,
    reuseSources: false,
    schedExt: false,
  };

  onMount(async () => {
    try {
      env = await api.invoke<Env>("get_build_environment");
      recipe.march = env.march;
      recipe.cores = env.cpuThreads;
    } catch (error) {
      console.error("Failed to read the computer", error);
    }
    await loadVersions();
  });

  async function loadVersions() {
    loadingVersions = true;
    versionsError = "";
    try {
      versions = await api.invoke<KernelVersion[]>("list_kernel_versions");
      if (!recipe.version) recipe.version = versions.find((v) => v.recommended)?.version ?? versions[0]?.version ?? "";
    } catch (error) {
      versionsError = String(error);
    }
    loadingVersions = false;
  }

  $: recommended = versions.find((v) => v.recommended);
  $: longterm = versions.filter((v) => v.channel === "longterm");
  $: otherStable = versions.filter((v) => v.channel === "stable" && !v.recommended);
  $: rcs = versions.filter((v) => v.channel === "rc");
  $: chosen = versions.find((v) => v.version === recipe.version);
  $: release = `${recipe.version}${localVersion(recipe)}`;
  $: lowSpace = env ? env.freeBytes < NEEDED_GB * 1024 ** 3 : false;
  $: toInstall = env ? [...env.deps.missing, ...env.deps.broken] : [];
  $: signNeedsKey = recipe.sign && env ? !env.keysExist : false;

  // The Rust Binder came with 6.18: a kernel before it is built without Binder.
  function lacksRustBinder(version: string): boolean {
    const m = version.match(/^(\d+)\.(\d+)/);
    if (!m) return false;
    return Number(m[1]) < 6 || (Number(m[1]) === 6 && Number(m[2]) < 18);
  }

  const SUFFIX: Record<string, string> = { gaming: "gaming", audio: "lowlatency", minimal: "minimal", optimized: "optimized", stock: "" };

  // The same name the program gives the release.
  function localVersion(r: Recipe): string {
    let name = r.name;
    if (r.goal !== "stock" && r.march !== "v1") name = `${name}-${r.march}`;
    const suffix = SUFFIX[r.goal] ?? "";
    return suffix ? `-${name}-${suffix}` : `-${name}`;
  }

  function chooseGoal(id: string) {
    recipe.goal = id;
    recipe.patches = [...(env?.goals.find((g) => g.id === id)?.patches ?? [])];
    // The X3D patch only makes sense on the processors it is for.
    if (env?.x3d && id !== "stock" && !recipe.patches.includes("x3d")) recipe.patches = [...recipe.patches, "x3d"];
  }

  function togglePatch(id: string) {
    recipe.patches = recipe.patches.includes(id) ? recipe.patches.filter((p) => p !== id) : [...recipe.patches, id];
  }

  async function chooseVersion(v: string) {
    recipe.version = v;
    try {
      sources = await api.invoke<{ exists: boolean; size: number }>("get_build_sources", { version: v });
    } catch {
      sources = { exists: false, size: 0 };
    }
    recipe.reuseSources = false;
  }

  const gb = (bytes: number) => (bytes / 1024 ** 3).toFixed(1);

  function next() {
    if (step === 1) chooseVersion(recipe.version);
    step = Math.min(step + 1, STEPS.length - 1);
  }
  const back = () => (step = Math.max(step - 1, 0));

  $: canNext = step === 0 ? recipe.goal !== "" : step === 1 ? recipe.version !== "" : true;
  $: nameOk = /^[A-Za-z0-9._-]{1,40}$/.test(recipe.name);

  async function start() {
    failure = "";
    cleaned = "";
    try {
      await api.invoke("start_build", { recipe });
    } catch (error) {
      failure = String(error);
    }
  }

  async function cancel() {
    confirmCancel = false;
    await api.invoke("cancel_build");
  }

  async function clean() {
    try {
      const freed = await api.invoke<number>("clean_build_files", { version: $job.recipe?.version ?? recipe.version });
      cleaned = gb(freed);
    } catch (error) {
      failure = String(error);
    }
  }

  async function restart() {
    try {
      await api.invoke("reboot_now");
    } catch (error) {
      failure = String(error);
    }
  }

  function again() {
    job.update((j) => ({ ...j, result: null, error: null }));
    step = 3;
  }

  $: progress.set($building ? $job.percent / 100 : null);
  $: stageIndex = STAGES.indexOf($job.stage === "build" || $job.stage === "packages" ? $job.stage : $job.stage);
  // A batch has its own page: here it only says so.
  $: shown = ($job.running || $job.result !== null) && !$job.batch;

  const primary = "px-5 py-2 rounded bg-primary hover:bg-primary-hover text-text-bright text-[0.88rem] font-semibold disabled:opacity-40";
  const secondary = "px-4 py-2 rounded border border-border text-[0.85rem] text-text-bright hover:bg-surface-2 disabled:opacity-40";
  const card = "rounded border border-border bg-surface p-5";
  const input = "bg-surface-2 border border-border rounded px-2.5 py-1.5 text-[0.85rem] text-text-bright outline-none focus-visible:ring-2 focus-visible:ring-primary/60";
</script>

{#if $job.batch && $job.running}
  <section class={card}>
    <p class="text-[0.88rem] text-text">{$_("build.batchRunning")}</p>
  </section>
{:else if shown}
  <section class={card}>
    {#if $job.result === "done"}
      <h3 class="text-[1.05rem] font-semibold text-text-bright mb-1">{$_("build.result.doneTitle", { values: { release: $job.release ?? "" } })}</h3>
      <p class="text-[0.85rem] text-text mb-3">{$_("build.result.doneText")}</p>
      {#if $job.nvidia}
        <p class="text-[0.82rem] mb-3 {$job.nvidia === 'installed' ? 'text-primary' : 'text-warning'}">
          {$job.nvidia === "installed" ? $_("build.result.nvidiaOk") : $_("build.result.nvidiaBad")}
        </p>
      {/if}
      <div class="flex flex-wrap gap-2">
        <button class={primary} on:click={restart}>{$_("build.result.restart")}</button>
        <button class={secondary} on:click={() => navigate("installed")}>{$_("build.result.installed")}</button>
        <button class={secondary} on:click={again}>{$_("build.result.again")}</button>
        {#if !cleaned}
          <button class={secondary} on:click={clean}>{$_("build.result.clean")}</button>
        {:else}
          <span class="self-center text-[0.8rem] text-text-muted">{$_("build.result.cleaned", { values: { size: cleaned } })}</span>
        {/if}
      </div>
    {:else if $job.result === "failed"}
      <h3 class="text-[1.05rem] font-semibold text-danger mb-1">{$_("build.result.failedTitle")}</h3>
      <p class="text-[0.85rem] text-text mb-3 break-words select-text">{$job.error}</p>
      <button class={primary} on:click={again}>{$_("build.result.backRecipe")}</button>
    {:else if $job.result === "cancelled"}
      <h3 class="text-[1.05rem] font-semibold text-text-bright mb-1">{$_("build.result.cancelledTitle")}</h3>
      <button class="{primary} mt-2" on:click={again}>{$_("build.result.backRecipe")}</button>
    {:else}
      <h3 class="text-[1.05rem] font-semibold text-text-bright mb-1">
        {$_("build.progress.title", { values: { release: $job.recipe ? `${$job.recipe.version}${localVersion($job.recipe)}` : "" } })}
      </h3>
      <p class="text-[0.8rem] text-text-muted mb-4">{$_("build.progress.passwordNote")}</p>
      <ol class="flex flex-col gap-2 mb-4">
        {#each STAGES as stage, i (stage)}
          <li class="flex items-center gap-3 text-[0.88rem] {i < stageIndex ? 'text-text-muted' : i === stageIndex ? 'text-primary font-semibold' : 'text-text-muted/60'}">
            <span class="w-5 h-5 rounded-full grid place-items-center {i < stageIndex ? 'bg-primary/20 text-primary' : i === stageIndex ? 'bg-primary text-text-bright' : 'bg-surface-2'}">
              {#if i < stageIndex}<Check size={12} />{:else if i === stageIndex}<Minus size={12} />{/if}
            </span>
            {$_(`build.progress.stages.${stage}`)}
          </li>
        {/each}
      </ol>
      <p class="text-[0.8rem] text-text-muted truncate mb-3" title={$job.lines[$job.lines.length - 1] ?? ""}>{$job.lines[$job.lines.length - 1] ?? ""}</p>
      <div class="flex items-center gap-3">
        <button class={secondary} on:click={() => (confirmCancel = true)}>{$_("build.progress.cancel")}</button>
        <button class="text-[0.8rem] text-primary hover:underline" on:click={() => (showLog = !showLog)}>
          {showLog ? $_("build.progress.hideLog") : $_("build.progress.showLog")}
        </button>
      </div>
      {#if showLog}
        <pre class="mt-3 max-h-[260px] overflow-auto rounded bg-surface-2 p-3 text-[0.72rem] text-text-muted select-text whitespace-pre-wrap">{$job.lines.join("\n")}</pre>
      {/if}
    {/if}
  </section>
{:else}
  <ol class="flex flex-wrap gap-2 mb-3.5">
    {#each STEPS as id, i (id)}
      <li>
        <button
          class="flex items-center gap-2 rounded border px-3 py-1.5 text-[0.82rem] {step === i ? 'border-primary bg-primary/15 text-primary' : 'border-border bg-surface text-text'}"
          disabled={i > step}
          on:click={() => (step = i)}
        >
          <span class="w-5 h-5 rounded-full grid place-items-center text-[0.7rem] font-bold {i < step ? 'bg-primary text-text-bright' : 'bg-surface-2 text-text-muted'}">
            {#if i < step}<Check size={12} />{:else}{i + 1}{/if}
          </span>
          {$_(`build.steps.${id}`)}
        </button>
      </li>
    {/each}
  </ol>

  <section class={card}>
    {#if step === 0}
      <h3 class="text-[1.05rem] font-semibold text-text-bright mb-3">{$_("build.goal.question")}</h3>
      <div class="grid grid-cols-1 sm:grid-cols-2 gap-3">
        {#each GOALS as goal (goal.id)}
          <button
            class="flex items-start gap-3 rounded border p-4 text-left transition-colors {recipe.goal === goal.id ? 'border-primary bg-primary/10' : 'border-border bg-surface-2/40 hover:bg-surface-2'}"
            on:click={() => chooseGoal(goal.id)}
          >
            <span class="mt-0.5 text-primary"><svelte:component this={goal.icon} size={22} /></span>
            <span>
              <span class="block text-[0.95rem] font-semibold text-text-bright">{$_(`build.goal.${goal.id}.title`)}</span>
              <span class="block text-[0.8rem] text-text-muted">{$_(`build.goal.${goal.id}.text`)}</span>
            </span>
          </button>
        {/each}
      </div>
    {:else if step === 1}
      <h3 class="text-[1.05rem] font-semibold text-text-bright mb-3">{$_("build.version.question")}</h3>
      {#if loadingVersions}
        <p class="text-[0.85rem] text-text-muted">{$_("build.version.loading")}</p>
      {:else if versionsError}
        <p class="text-[0.85rem] text-warning mb-3 break-words">{versionsError}</p>
        <button class={secondary} on:click={loadVersions}>{$_("build.version.retry")}</button>
      {:else}
        {#if recommended}
          <button
            class="w-full text-left rounded border p-4 mb-3 {recipe.version === recommended.version ? 'border-primary bg-primary/10' : 'border-border bg-surface-2/40 hover:bg-surface-2'}"
            on:click={() => (recipe.version = recommended.version)}
          >
            <span class="text-[0.7rem] font-semibold text-primary">{$_("build.version.recommended")}</span>
            <span class="block text-[1.05rem] font-semibold text-text-bright">Linux {recommended.version}</span>
            <span class="block text-[0.8rem] text-text-muted">{$_("build.version.recommendedText")} · {$_("build.version.released", { values: { date: recommended.released } })}</span>
          </button>
        {/if}

        <p class="text-[0.85rem] font-semibold text-text-bright mb-1">{$_("build.version.longterm")}</p>
        <p class="text-[0.78rem] text-text-muted mb-2">{$_("build.version.longtermText")}</p>
        <div class="flex flex-wrap gap-2 mb-3">
          {#each longterm as v (v.version)}
            <button
              class="rounded border px-3 py-1.5 text-[0.82rem] {recipe.version === v.version ? 'border-primary bg-primary/10 text-primary' : 'border-border bg-surface-2/40 text-text hover:bg-surface-2'}"
              on:click={() => (recipe.version = v.version)}
            >
              {v.version}
            </button>
          {/each}
        </div>

        {#if otherStable.length > 0}
          <button class="flex items-center gap-1 text-[0.82rem] text-text-muted hover:text-text-bright mb-2" on:click={() => (showOther = !showOther)}>
            <ChevronDown size={14} class={showOther ? "rotate-180" : ""} />{$_("build.version.other")}
          </button>
          {#if showOther}
            <div class="flex flex-wrap gap-2 mb-3">
              {#each otherStable as v (v.version)}
                <button
                  class="rounded border px-3 py-1.5 text-[0.82rem] {recipe.version === v.version ? 'border-primary bg-primary/10 text-primary' : 'border-border bg-surface-2/40 text-text hover:bg-surface-2'}"
                  title={v.eol ? $_("build.version.eol") : ""}
                  on:click={() => (recipe.version = v.version)}
                >
                  {v.version}{v.eol ? " *" : ""}
                </button>
              {/each}
            </div>
          {/if}
        {/if}

        {#if rcs.length > 0}
          <label class="flex items-center gap-2 text-[0.85rem] text-text-bright mt-2">
            <input type="checkbox" bind:checked={showRc} />
            {$_("build.version.rcToggle")}
          </label>
          <p class="text-[0.78rem] text-text-muted mb-2">{$_("build.version.rcText")}</p>
          {#if showRc}
            <div class="flex flex-wrap gap-2 mb-3">
              {#each rcs as v (v.version)}
                <button
                  class="rounded border px-3 py-1.5 text-[0.82rem] {recipe.version === v.version ? 'border-primary bg-primary/10 text-primary' : 'border-border bg-surface-2/40 text-text hover:bg-surface-2'}"
                  on:click={() => (recipe.version = v.version)}
                >
                  {v.version}
                </button>
              {/each}
            </div>
          {/if}
        {/if}

        {#if recipe.version && lacksRustBinder(recipe.version)}
          <div class="flex items-start gap-3 rounded border border-warning/40 bg-warning/10 px-3 py-2 mt-2">
            <AlertTriangle size={16} class="text-warning shrink-0 mt-0.5" />
            <p class="text-[0.82rem] text-text">{$_("build.version.noRustBinder")}</p>
          </div>
        {/if}
      {/if}
    {:else if step === 2}
      <h3 class="text-[1.05rem] font-semibold text-text-bright mb-3">{$_("build.computer.title")}</h3>
      {#if env}
        <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-3 text-[0.85rem] mb-3">
          <dt class="text-text-muted">{$_("kernels.hardware.cpu")}</dt>
          <dd class="text-text-bright">{env.cpu ?? "-"}</dd>

          <dt class="text-text-muted">{$_("build.computer.level")}</dt>
          <dd>
            <div class="flex items-center gap-1.5">
              {#each ["v1", "v2", "v3", "v4"] as level (level)}
                <button
                  class="px-2.5 py-0.5 rounded-full text-[0.78rem] font-semibold {recipe.march === level ? 'bg-primary/20 text-primary' : 'bg-surface-2 text-text-muted hover:text-text-bright'}"
                  disabled={level > env.march}
                  title={level > env.march ? $_("build.computer.levelNo") : ""}
                  on:click={() => (recipe.march = level)}
                >
                  {level.toUpperCase()}
                </button>
              {/each}
            </div>
            <p class="text-[0.75rem] text-text-muted mt-1">{$_("build.computer.levelText")}</p>
          </dd>

          <dt class="text-text-muted">{$_("build.computer.cores")}</dt>
          <dd class="flex items-center gap-3">
            <input type="range" min="1" max={env.cpuThreads} bind:value={recipe.cores} class="w-[200px]" />
            <span class="text-text-bright">{recipe.cores} / {env.cpuThreads}</span>
          </dd>

          <dt class="text-text-muted">{$_("build.computer.space")}</dt>
          <dd>
            <span class="text-text-bright">{gb(env.freeBytes)} GB</span>
            <span class="text-[0.78rem] text-text-muted"> · {$_("build.computer.spaceNeed", { values: { gb: NEEDED_GB } })}</span>
            {#if lowSpace}<p class="text-[0.78rem] text-warning">{$_("build.computer.spaceLow")}</p>{/if}
          </dd>

          <dt class="text-text-muted">{$_("build.computer.tools")}</dt>
          <dd>
            {#if toInstall.length === 0}
              <span class="text-primary">{$_("build.computer.toolsOk")}</span>
            {:else}
              <span class="text-text-bright">{$_("build.computer.toolsInstall", { values: { count: toInstall.length } })}</span>
              <p class="text-[0.75rem] text-text-muted break-words">{toInstall.join(", ")}</p>
            {/if}
          </dd>

          <dt class="text-text-muted">Rust</dt>
          <dd class="text-text-bright">{env.deps.rustc ?? $_("build.computer.rustMissing")}</dd>

          {#if env.nvidia.present}
            <dt class="text-text-muted">{$_("build.computer.nvidia")}</dt>
            <dd>
              <span class="text-text-bright">{env.nvidia.driver ? $_("build.computer.nvidiaDriver", { values: { version: env.nvidia.driver } }) : "NVIDIA"}</span>
              <p class="text-[0.75rem] text-text-muted">{$_("build.computer.nvidiaNote")}</p>
            </dd>
          {/if}

          <dt class="text-text-muted">{$_("build.computer.sign")}</dt>
          <dd>
            {#if env.keysExist}
              <label class="flex items-center gap-2 text-text-bright">
                <input type="checkbox" bind:checked={recipe.sign} />{$_("build.computer.signText")}
              </label>
            {:else}
              <span class="text-text-muted">{$_("build.computer.signNoKey")}</span>
              <button class="ml-2 text-[0.8rem] text-primary hover:underline" on:click={() => navigate("secureboot")}>{$_("build.computer.signGo")}</button>
            {/if}
          </dd>
        </dl>
      {/if}
    {:else}
      <h3 class="text-[1.05rem] font-semibold text-text-bright mb-3">{$_("build.recipe.title")}</h3>
      <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-3 text-[0.85rem] mb-3">
        <dt class="text-text-muted">{$_("build.steps.goal")}</dt>
        <dd class="text-text-bright">{$_(`build.goal.${recipe.goal}.title`)}</dd>
        <dt class="text-text-muted">{$_("build.steps.version")}</dt>
        <dd class="text-text-bright">Linux {recipe.version}{chosen?.channel === "rc" ? ` (${$_("build.version.rcTag")})` : ""}</dd>
        <dt class="text-text-muted">{$_("build.recipe.name")}</dt>
        <dd>
          <input class="{input} w-[220px]" bind:value={recipe.name} maxlength="40" />
          {#if !nameOk}<p class="text-[0.75rem] text-danger">{$_("build.recipe.nameHint")}</p>{/if}
        </dd>
        <dt class="text-text-muted">{$_("build.recipe.patches")}</dt>
        <dd class="flex flex-col gap-1.5">
          {#each PATCHES.filter((p) => p !== "x3d" || env?.x3d || recipe.patches.includes("x3d")) as patch (patch)}
            <label class="flex items-start gap-2">
              <input type="checkbox" class="mt-1" checked={recipe.patches.includes(patch)} on:change={() => togglePatch(patch)} />
              <span>
                <span class="text-text-bright">{$_(`build.recipe.patch.${patch}.name`)}</span>
                <span class="block text-[0.75rem] text-text-muted">{$_(`build.recipe.patch.${patch}.text`)}</span>
              </span>
            </label>
          {/each}
        </dd>
        <dt class="text-text-muted">{$_("build.computer.level")}</dt>
        <dd class="text-text-bright">{recipe.march.toUpperCase()}</dd>
        <dt class="text-text-muted">{$_("build.computer.sign")}</dt>
        <dd class="text-text-bright">{recipe.sign ? $_("build.recipe.yes") : $_("build.recipe.no")}</dd>
      </dl>

      <button class="flex items-center gap-1 text-[0.82rem] text-text-muted hover:text-text-bright mb-2" on:click={() => (showAdvanced = !showAdvanced)}>
        <ChevronDown size={14} class={showAdvanced ? "rotate-180" : ""} />{$_("build.recipe.advanced")}
      </button>
      {#if showAdvanced}
        <div class="flex flex-col gap-2 mb-3">
          <label class="flex items-center gap-2 text-[0.85rem] text-text-bright">
            <input type="checkbox" bind:checked={recipe.schedExt} />{$_("build.recipe.schedExt")}
          </label>
          {#if sources.exists}
            <label class="flex items-center gap-2 text-[0.85rem] text-text-bright">
              <input type="checkbox" bind:checked={recipe.reuseSources} />{$_("build.recipe.reuse", { values: { size: gb(sources.size) } })}
            </label>
          {/if}
        </div>
      {/if}

      <p class="text-[0.85rem] text-text mb-1">{$_("build.recipe.release", { values: { release } })}</p>
      {#if lacksRustBinder(recipe.version)}
        <p class="text-[0.78rem] text-warning mb-1">{$_("build.version.noRustBinder")}</p>
      {/if}
      {#if lowSpace}<p class="text-[0.78rem] text-warning mb-1">{$_("build.computer.spaceLow")}</p>{/if}
      {#if signNeedsKey}<p class="text-[0.78rem] text-warning mb-1">{$_("build.computer.signNoKey")}</p>{/if}
      {#if failure}<p class="text-[0.8rem] text-danger mb-1 break-words">{failure}</p>{/if}
    {/if}

    <div class="flex items-center gap-2 mt-5">
      {#if step > 0}<button class={secondary} on:click={back}>{$_("build.recipe.back")}</button>{/if}
      {#if step < STEPS.length - 1}
        <button class={primary} disabled={!canNext} on:click={next}>{$_("build.recipe.next")}</button>
      {:else}
        <button class={primary} disabled={!nameOk || signNeedsKey || $building} on:click={start}>{$_("build.recipe.start")}</button>
      {/if}
    </div>
  </section>
{/if}

{#if confirmCancel}
  <Modal variant="warning" title={$_("build.progress.cancelTitle")} confirmLabel={$_("build.progress.cancel")} cancelLabel={$_("build.progress.keepGoing")} onConfirm={cancel} onClose={() => (confirmCancel = false)}>
    <p class="text-[0.85rem] text-text">{$_("build.progress.cancelText")}</p>
  </Modal>
{/if}
