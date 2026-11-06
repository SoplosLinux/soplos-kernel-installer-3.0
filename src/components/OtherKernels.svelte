<script lang="ts">
  import { onMount } from "svelte";
  import { _ } from "svelte-i18n";
  import { AlertTriangle } from "lucide-svelte";
  import Modal from "./Modal.svelte";
  import { api } from "../lib/api";
  import { busy, opsLine, runOp } from "../lib/ops";

  interface DebianKernel {
    package: string;
    installed: boolean;
    inUse: boolean;
  }
  interface XanmodVariant {
    key: string;
    package: string | null;
    installed: boolean;
    inUse: boolean;
  }
  interface OtherKernels {
    debian: DebianKernel[];
    xanmod: XanmodVariant[];
    xanmodBrokenRepo: boolean;
    psabiLevel: number;
    liquorixInstalled: boolean;
    liquorixInUse: boolean;
  }

  let state: OtherKernels | null = null;
  let message: { title: string; text: string } | null = null;
  let confirm: { name: string; text: string; run: () => Promise<unknown> } | null = null;

  async function load() {
    try {
      state = await api.invoke<OtherKernels>("get_other_kernels");
    } catch (error) {
      console.error("Failed to read the kernels", error);
    }
  }

  onMount(load);

  async function perform(call: () => Promise<unknown>) {
    try {
      await runOp(call);
    } catch (error) {
      const text = String(error instanceof Error ? error.message : error);
      if (text !== "cancelled" && text !== "busy") message = { title: $_("others.error"), text };
    }
    await load();
  }

  function askRemove(name: string, text: string, run: () => Promise<unknown>) {
    confirm = { name, text, run };
  }

  function confirmed() {
    const c = confirm;
    confirm = null;
    if (c) perform(c.run);
  }

  const debianKey = (pkg: string) => (pkg.includes("-rt-") ? "rt" : "standard");
  const xanmodKey = (key: string) => key.replace("xanmod-", "");

  const primary = "px-3.5 py-1.5 rounded bg-primary hover:bg-primary-hover text-text-bright text-[0.8rem] font-semibold disabled:opacity-40";
  const danger = "px-3.5 py-1.5 rounded bg-danger hover:bg-danger/85 text-text-bright text-[0.8rem] font-semibold disabled:opacity-40";
  const secondary = "px-3 py-1.5 rounded border border-border text-[0.8rem] text-text-bright hover:bg-surface-2 disabled:opacity-40";
</script>

{#if state}
  <section class="rounded border border-border bg-surface p-4">
    <h3 class="text-[0.95rem] font-semibold text-text-bright">{$_("others.debian.title")}</h3>
    <p class="text-[0.8rem] text-text-muted mb-1">{$_("others.debian.description")}</p>
    <ul>
      {#each state.debian as k (k.package)}
        {@const key = debianKey(k.package)}
        <li class="flex items-center gap-3 py-2.5 border-t border-border first:border-t-0">
          <div class="flex-1 min-w-0">
            <p class="text-[0.9rem] text-text-bright">
              <span class="font-semibold">{$_(`others.debian.${key}.name`)}</span>
              {#if k.inUse}<span class="ml-2 text-[0.7rem] text-primary">{$_("kernels.list.running")}</span>{/if}
            </p>
            <p class="text-[0.78rem] text-text-muted truncate">{$_(`others.debian.${key}.description`)}</p>
          </div>
          {#if k.installed}
            <button
              class={danger}
              disabled={$busy}
              on:click={() =>
                askRemove($_(`others.debian.${key}.name`), $_("others.debian.removeText"), () =>
                  api.invoke("debian_kernel_action", { package: k.package, action: "remove" }),
                )}>{$_("kernels.list.remove")}</button
            >
          {:else}
            <button class={primary} disabled={$busy} on:click={() => perform(() => api.invoke("debian_kernel_action", { package: k.package, action: "install" }))}>
              {$_("kernels.list.install")}
            </button>
          {/if}
        </li>
      {/each}
    </ul>
  </section>

  <section class="rounded border border-border bg-surface p-4">
    <h3 class="text-[0.95rem] font-semibold text-text-bright">XanMod</h3>
    <p class="text-[0.8rem] text-text-muted">{$_("others.xanmod.description")}</p>
    <p class="text-[0.8rem] text-text-muted mb-1">{$_("others.xanmod.level", { values: { level: `V${Math.min(state.psabiLevel, 3)}` } })}</p>
    {#if state.xanmodBrokenRepo}
      <div class="flex items-center gap-3 rounded border border-warning/40 bg-warning/10 px-3 py-2 my-2">
        <AlertTriangle size={16} class="text-warning shrink-0" />
        <p class="flex-1 text-[0.8rem] text-text">{$_("others.xanmod.brokenRepo")}</p>
        <button class={secondary} disabled={$busy} on:click={() => perform(() => api.invoke("xanmod_action", { variant: "", action: "repair" }))}>
          {$_("others.xanmod.repair")}
        </button>
      </div>
    {/if}
    <ul>
      {#each state.xanmod as v (v.key)}
        {@const key = xanmodKey(v.key)}
        <li class="flex items-center gap-3 py-2.5 border-t border-border first:border-t-0">
          <div class="flex-1 min-w-0">
            <p class="text-[0.9rem] text-text-bright">
              <span class="font-semibold">{$_(`others.xanmod.${key}.name`)}</span>
              {#if v.inUse}<span class="ml-2 text-[0.7rem] text-primary">{$_("kernels.list.running")}</span>{/if}
            </p>
            <p class="text-[0.78rem] text-text-muted truncate">
              {v.package ? $_(`others.xanmod.${key}.description`) : $_("others.xanmod.tooOld")}
            </p>
          </div>
          {#if v.installed}
            <button
              class={danger}
              disabled={$busy}
              on:click={() =>
                askRemove(`XanMod ${$_(`others.xanmod.${key}.name`)}`, $_("others.xanmod.removeText"), () =>
                  api.invoke("xanmod_action", { variant: v.key, action: "remove" }),
                )}>{$_("kernels.list.remove")}</button
            >
          {:else}
            <button
              class={primary}
              disabled={$busy || !v.package}
              on:click={() => perform(() => api.invoke("xanmod_action", { variant: v.key, action: "install" }))}
            >
              {$_("kernels.list.install")}
            </button>
          {/if}
        </li>
      {/each}
    </ul>
  </section>

  <section class="rounded border border-border bg-surface p-4">
    <h3 class="text-[0.95rem] font-semibold text-text-bright">Liquorix</h3>
    <p class="text-[0.8rem] text-text-muted mb-1">{$_("others.liquorix.description")}</p>
    <div class="flex items-center gap-3 py-2.5">
      <p class="flex-1 min-w-0 text-[0.9rem] text-text-bright">
        <span class="font-semibold">Liquorix</span>
        {#if state.liquorixInUse}<span class="ml-2 text-[0.7rem] text-primary">{$_("kernels.list.running")}</span>{/if}
      </p>
      {#if state.liquorixInstalled}
        <button
          class={danger}
          disabled={$busy}
          on:click={() => askRemove("Liquorix", $_("others.liquorix.removeText"), () => api.invoke("liquorix_action", { action: "remove" }))}
        >
          {$_("kernels.list.remove")}
        </button>
      {:else}
        <button class={primary} disabled={$busy} on:click={() => perform(() => api.invoke("liquorix_action", { action: "install" }))}>
          {$_("kernels.list.install")}
        </button>
      {/if}
    </div>
  </section>

  <p class="text-[0.78rem] text-text-muted px-1">{$_("others.secureBootNote")}</p>
  {#if $busy && $opsLine}
    <p class="text-[0.75rem] text-text-muted truncate px-1">{$opsLine}</p>
  {/if}
{/if}

{#if message}
  <Modal variant="danger" title={message.title} onClose={() => (message = null)}>
    <p class="text-[0.85rem] text-text break-words">{message.text}</p>
  </Modal>
{/if}

{#if confirm}
  <Modal
    variant="danger"
    title={$_("others.confirmTitle", { values: { name: confirm.name } })}
    confirmLabel={$_("kernels.list.remove")}
    onConfirm={confirmed}
    onClose={() => (confirm = null)}
  >
    <p class="text-[0.85rem] text-text">{confirm.text}</p>
  </Modal>
{/if}
