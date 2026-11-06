<script lang="ts">
  import { onMount } from "svelte";
  import { _ } from "svelte-i18n";
  import { ChevronDown, Info, RefreshCw } from "lucide-svelte";
  import Modal from "../components/Modal.svelte";
  import OtherKernels from "../components/OtherKernels.svelte";
  import { api } from "../lib/api";
  import { busy, opsLine, runOp } from "../lib/ops";

  interface KernelPackage {
    package: string;
    profile: string;
    march: string;
    description: string;
    version: string;
    installed: boolean;
    updateAvailable: boolean;
    isRunning: boolean;
  }
  interface RepoState {
    configured: boolean;
    packages: KernelPackage[];
    running: string;
  }
  interface Hardware {
    cpu: string;
    march: string;
    gpu: string;
    ramGb: number;
  }

  let state: RepoState = { configured: false, packages: [], running: "" };
  let loaded = false;
  let profileFilter = "all";
  let marchFilter = "all";

  let message: { title: string; text: string } | null = null;
  let confirm: { kind: "removeRepo" } | { kind: "removeKernel"; pkg: KernelPackage } | null = null;
  let hardware: Hardware | null = null;

  async function load() {
    try {
      state = await api.invoke<RepoState>("get_repo_state");
    } catch (error) {
      console.error("Failed to read the repository", error);
    }
    loaded = true;
  }

  onMount(load);

  $: profiles = [...new Set(state.packages.map((p) => p.profile))].sort();
  $: marches = [...new Set(state.packages.map((p) => p.march).filter(Boolean))].sort();
  // A filter that no package has any more falls back to all.
  $: if (profileFilter !== "all" && !profiles.includes(profileFilter)) profileFilter = "all";
  $: if (marchFilter !== "all" && !marches.includes(marchFilter)) marchFilter = "all";
  $: visible = state.packages.filter(
    (p) => (profileFilter === "all" || p.profile === profileFilter) && (marchFilter === "all" || p.march === marchFilter),
  );

  async function perform(call: () => Promise<unknown>, failTitle: string) {
    try {
      await runOp(call);
    } catch (error) {
      const text = String(error instanceof Error ? error.message : error);
      if (text !== "cancelled" && text !== "busy") message = { title: failTitle, text };
    }
    await load();
  }

  const addRepo = () => perform(() => api.invoke("repo_add"), $_("kernels.errors.addRepo"));
  const refreshRepo = () => perform(() => api.invoke("repo_refresh"), $_("kernels.errors.refresh"));
  const removeRepo = () => perform(() => api.invoke("repo_remove"), $_("kernels.errors.removeRepo"));
  const act = (pkg: KernelPackage, action: "install" | "remove" | "update") =>
    perform(() => api.invoke("kernel_package_action", { package: pkg.package, action }), $_("kernels.errors.operation"));

  function askRemoveKernel(pkg: KernelPackage) {
    confirm = { kind: "removeKernel", pkg };
  }

  function confirmed() {
    const c = confirm;
    confirm = null;
    if (!c) return;
    if (c.kind === "removeRepo") removeRepo();
    else act(c.pkg, "remove");
  }

  async function showHardware() {
    try {
      hardware = await api.invoke<Hardware>("get_hardware_info");
    } catch (error) {
      console.error("Failed to read the hardware", error);
    }
  }

  const selectClass =
    "appearance-none bg-surface-2 border border-border rounded pl-2.5 pr-7 py-1.5 text-[0.82rem] text-text-bright outline-none focus-visible:ring-2 focus-visible:ring-primary/60";
  const secondary = "px-3 py-1.5 rounded border border-border text-[0.8rem] text-text-bright hover:bg-surface-2 disabled:opacity-40";
  const primary = "px-3.5 py-1.5 rounded bg-primary hover:bg-primary-hover text-text-bright text-[0.8rem] font-semibold disabled:opacity-40";
  const danger = "px-3.5 py-1.5 rounded bg-danger hover:bg-danger/85 text-text-bright text-[0.8rem] font-semibold disabled:opacity-40";
</script>

<div class="flex flex-col gap-3.5">
  <section class="rounded border border-border bg-surface p-4">
    <h3 class="text-[0.95rem] font-semibold text-text-bright mb-2">{$_("kernels.repo.title")}</h3>
    <div class="flex items-center gap-2 flex-wrap">
      <p class="flex-1 min-w-[160px] text-[0.85rem] text-text">
        {state.configured ? $_("kernels.repo.installed") : $_("kernels.repo.notConfigured")}
      </p>
      {#if state.configured}
        <button class="{secondary} inline-flex items-center gap-1.5" disabled={$busy} on:click={refreshRepo}>
          <RefreshCw size={13} />{$_("kernels.repo.refresh")}
        </button>
        <button class={secondary} disabled={$busy} on:click={() => (confirm = { kind: "removeRepo" })}>
          {$_("kernels.repo.remove")}
        </button>
      {:else}
        <button class={primary} disabled={$busy} on:click={addRepo}>{$_("kernels.repo.add")}</button>
      {/if}
      <button
        class="p-1.5 rounded text-text-muted hover:bg-surface-2 hover:text-text-bright"
        title={$_("kernels.hardware.tooltip")}
        aria-label={$_("kernels.hardware.tooltip")}
        on:click={showHardware}
      >
        <Info size={16} />
      </button>
    </div>
    {#if $busy && $opsLine}
      <p class="mt-2 text-[0.75rem] text-text-muted truncate">{$opsLine}</p>
    {/if}
  </section>

  <section class="rounded border border-border bg-surface p-4">
    <div class="flex items-center gap-4 flex-wrap">
      <label class="flex items-center gap-2 text-[0.82rem] text-text">
        {$_("kernels.filters.profile")}
        <span class="relative">
          <select class={selectClass} bind:value={profileFilter}>
            <option value="all">{$_("kernels.filters.all")}</option>
            {#each profiles as p}<option value={p}>{p}</option>{/each}
          </select>
          <ChevronDown size={14} class="absolute right-2 top-1/2 -translate-y-1/2 pointer-events-none text-text-muted" />
        </span>
      </label>
      <label class="flex items-center gap-2 text-[0.82rem] text-text">
        {$_("kernels.filters.level")}
        <span class="relative">
          <select class={selectClass} bind:value={marchFilter}>
            <option value="all">{$_("kernels.filters.all")}</option>
            {#each marches as m}<option value={m}>{m}</option>{/each}
          </select>
          <ChevronDown size={14} class="absolute right-2 top-1/2 -translate-y-1/2 pointer-events-none text-text-muted" />
        </span>
      </label>
    </div>
  </section>

  <section class="rounded border border-border bg-surface p-4">
    <h3 class="text-[0.95rem] font-semibold text-text-bright mb-1">{$_("kernels.list.title")}</h3>
    {#if !loaded}
      <p class="text-[0.85rem] text-text-muted py-3">{$_("kernels.list.loading")}</p>
    {:else if state.packages.length === 0}
      <p class="text-[0.85rem] text-text-muted py-3">{$_("kernels.list.empty")}</p>
    {:else if visible.length === 0}
      <p class="text-[0.85rem] text-text-muted py-3">{$_("kernels.list.noMatch")}</p>
    {:else}
      <ul>
        {#each visible as pkg (pkg.package)}
          <li class="flex items-center gap-3 py-2.5 border-t border-border first:border-t-0">
            <div class="flex-1 min-w-0">
              <p class="text-[0.9rem] text-text-bright">
                <span class="font-semibold">{pkg.march ? `${pkg.profile} ${pkg.march}` : pkg.profile}</span>
                {#if pkg.version}<span class="text-[0.75rem] text-text-muted ml-1.5">{pkg.version}</span>{/if}
                {#if pkg.isRunning}<span class="ml-2 text-[0.7rem] text-primary">{$_("kernels.list.running")}</span>{/if}
              </p>
              <p class="text-[0.78rem] text-text-muted truncate">{pkg.description}</p>
            </div>
            {#if pkg.installed && pkg.updateAvailable}
              <button class={primary} disabled={$busy} on:click={() => act(pkg, "update")}>{$_("kernels.list.update")}</button>
            {/if}
            {#if pkg.installed}
              <button class={danger} disabled={$busy} on:click={() => askRemoveKernel(pkg)}>{$_("kernels.list.remove")}</button>
            {:else}
              <button class={primary} disabled={$busy || !state.configured} on:click={() => act(pkg, "install")}>
                {$_("kernels.list.install")}
              </button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <OtherKernels />
</div>

{#if message}
  <Modal variant="danger" title={message.title} onClose={() => (message = null)}>
    <p class="text-[0.85rem] text-text break-words">{message.text}</p>
  </Modal>
{/if}

{#if confirm}
  <Modal
    variant={confirm.kind === "removeKernel" ? "danger" : "warning"}
    title={confirm.kind === "removeRepo" ? $_("kernels.confirm.removeRepoTitle") : $_("kernels.confirm.removeKernelTitle")}
    confirmLabel={confirm.kind === "removeRepo" ? $_("kernels.repo.remove") : $_("kernels.list.remove")}
    onConfirm={confirmed}
    onClose={() => (confirm = null)}
  >
    <p class="text-[0.85rem] text-text">
      {confirm.kind === "removeRepo" ? $_("kernels.confirm.removeRepoText") : $_("kernels.confirm.removeKernelText")}
    </p>
    {#if confirm.kind === "removeKernel" && confirm.pkg.isRunning}
      <p class="text-[0.85rem] text-warning mt-2">{$_("kernels.confirm.runningWarning")}</p>
    {/if}
  </Modal>
{/if}

{#if hardware}
  <Modal variant="info" title={$_("kernels.hardware.title")} onClose={() => (hardware = null)}>
    <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 mb-3">
      {#if hardware.cpu}
        <dt class="text-text-muted">{$_("kernels.hardware.cpu")}</dt>
        <dd class="text-text-bright font-medium">{hardware.cpu}</dd>
      {/if}
      <dt class="text-text-muted">{$_("kernels.hardware.level")}</dt>
      <dd>
        <span class="px-2 py-0.5 rounded-full bg-primary/15 text-primary font-semibold text-[0.8rem]">{hardware.march.toUpperCase()}</span>
      </dd>
      {#if hardware.gpu}
        <dt class="text-text-muted">{$_("kernels.hardware.gpu")}</dt>
        <dd class="text-text-bright font-medium">{hardware.gpu}</dd>
      {/if}
      {#if hardware.ramGb}
        <dt class="text-text-muted">{$_("kernels.hardware.ram")}</dt>
        <dd class="text-text-bright font-medium">{$_("kernels.hardware.ramValue", { values: { value: hardware.ramGb.toFixed(1) } })}</dd>
      {/if}
    </dl>
    <p class="text-text-muted text-[0.8rem]">{$_("kernels.hardware.advice")}</p>
  </Modal>
{/if}
