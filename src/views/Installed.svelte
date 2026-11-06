<script lang="ts">
  import { onMount } from "svelte";
  import { _ } from "svelte-i18n";
  import { Check, ShieldCheck, Trash2 } from "lucide-svelte";
  import Modal from "../components/Modal.svelte";
  import PasswordForm from "../components/PasswordForm.svelte";
  import { api } from "../lib/api";
  import { busy, opsLine, runOp } from "../lib/ops";

  interface InstalledKernel {
    version: string;
    profile: string;
    patches: string;
    date: string;
    isCurrent: boolean;
    secureBoot: boolean;
  }

  let kernels: InstalledKernel[] = [];
  let loaded = false;
  let toRemove: InstalledKernel | null = null;
  let failure = "";
  // The key of the user is no longer used by anything: offer to take it away.
  let revokeOffer = false;
  let revokeDone = false;
  let revokeFailure = "";

  // Names that are the same in every language.
  const PROPER: Record<string, string> = {
    soplos: "Soplos",
    xanmod: "XanMod",
    liquorix: "Liquorix",
    zen: "Zen",
    rt: "PREEMPT_RT",
  };

  function profileLabel(id: string): string {
    return PROPER[id] ?? $_(`installed.profiles.${id}`);
  }

  async function load() {
    try {
      kernels = await api.invoke<InstalledKernel[]>("list_installed_kernels");
    } catch (error) {
      console.error("Failed to list the kernels", error);
    }
    loaded = true;
  }

  onMount(load);

  interface SigningState {
    keysExist: boolean;
    enrolled: boolean;
    kernels: { signer: string }[];
    bootloaders: { files: { signer: string }[] }[];
    payloads: { signer: string }[];
  }

  // Revoking is only offered when nothing is signed with the key any more: a
  // bootloader signed with it that stayed would not start without it.
  async function checkRevoke() {
    try {
      const state = await api.invoke<SigningState>("get_secure_boot_state");
      const inUse =
        state.kernels.some((k) => k.signer === "own") ||
        state.bootloaders.some((l) => l.files.some((f) => f.signer === "own")) ||
        state.payloads.some((p) => p.signer === "own");
      revokeOffer = state.keysExist && state.enrolled && !inUse;
    } catch (error) {
      console.error("Failed to read Secure Boot", error);
    }
  }

  async function revoke(password: string) {
    revokeOffer = false;
    try {
      await runOp(() => api.invoke("sb_delete_key", { password, enrolled: true }));
      revokeDone = true;
    } catch (error) {
      const text = String(error instanceof Error ? error.message : error);
      if (text !== "cancelled" && text !== "busy") revokeFailure = text;
    }
  }

  async function remove() {
    const kernel = toRemove;
    toRemove = null;
    if (!kernel) return;
    let removed = false;
    try {
      await runOp(() => api.invoke("remove_installed_kernel", { release: kernel.version }));
      removed = true;
    } catch (error) {
      const text = String(error instanceof Error ? error.message : error);
      if (text !== "cancelled" && text !== "busy") failure = text;
    }
    await load();
    if (removed) await checkRevoke();
  }
</script>

<section class="rounded border border-border bg-surface p-4">
  <h3 class="text-[0.95rem] font-semibold text-text-bright mb-1">{$_("installed.title")}</h3>
  {#if $busy && $opsLine}
    <p class="text-[0.75rem] text-text-muted truncate mb-1">{$opsLine}</p>
  {/if}
  {#if !loaded}
    <p class="text-[0.85rem] text-text-muted py-3">{$_("installed.loading")}</p>
  {:else if kernels.length === 0}
    <p class="text-[0.85rem] text-text-muted py-3">{$_("installed.empty")}</p>
  {:else}
    <ul>
      {#each kernels as kernel (kernel.version)}
        <li class="flex items-center gap-3 py-2.5 border-t border-border first:border-t-0">
          <span class="w-5 shrink-0 text-primary" title={kernel.isCurrent ? $_("installed.current") : ""}>
            {#if kernel.isCurrent}<Check size={16} />{/if}
          </span>
          <span class="flex-1 min-w-0 truncate text-[0.9rem] {kernel.isCurrent ? 'text-primary font-semibold' : 'text-text-bright'}">
            {kernel.version}
          </span>
          <span class="w-[110px] shrink-0 text-center rounded-full bg-surface-2 px-2 py-0.5 text-[0.75rem] text-text">
            {profileLabel(kernel.profile)}
          </span>
          <span class="w-[110px] shrink-0 truncate text-[0.75rem] text-text-muted" title={kernel.patches ? $_("installed.patches", { values: { value: kernel.patches } }) : ""}>
            {kernel.patches}
          </span>
          <span class="w-5 shrink-0 text-text-muted" title={kernel.secureBoot ? $_("installed.signed") : ""}>
            {#if kernel.secureBoot}<ShieldCheck size={15} />{/if}
          </span>
          <span class="w-[78px] shrink-0 text-right text-[0.75rem] text-text-muted">{kernel.date}</span>
          <button
            class="w-8 shrink-0 p-1.5 rounded text-text-muted hover:bg-surface-2 hover:text-danger disabled:opacity-0 disabled:pointer-events-none"
            disabled={kernel.isCurrent || $busy}
            title={$_("installed.remove")}
            aria-label={$_("installed.remove")}
            on:click={() => (toRemove = kernel)}
          >
            <Trash2 size={15} />
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

{#if toRemove}
  <Modal
    variant="danger"
    title={$_("installed.confirmTitle", { values: { release: toRemove.version } })}
    confirmLabel={$_("installed.remove")}
    onConfirm={remove}
    onClose={() => (toRemove = null)}
  >
    <p class="text-[0.85rem] text-text">{$_("installed.confirmText")}</p>
  </Modal>
{/if}

{#if failure}
  <Modal variant="danger" title={$_("installed.error")} onClose={() => (failure = "")}>
    <p class="text-[0.85rem] text-text break-words">{failure}</p>
  </Modal>
{/if}

{#if revokeOffer}
  <Modal variant="danger" title={$_("installed.revoke.title")} showFooter={false} onClose={() => (revokeOffer = false)}>
    <p class="text-[0.85rem] text-text mb-3">{$_("installed.revoke.text")}</p>
    <PasswordForm danger submitLabel={$_("installed.revoke.action")} onSubmit={revoke} onCancel={() => (revokeOffer = false)} />
  </Modal>
{/if}

{#if revokeDone}
  <Modal variant="info" title={$_("installed.revoke.title")} onClose={() => (revokeDone = false)}>
    <p class="text-[0.85rem] text-text">{$_("installed.revoke.done")}</p>
  </Modal>
{/if}

{#if revokeFailure}
  <Modal variant="danger" title={$_("installed.revoke.error")} onClose={() => (revokeFailure = "")}>
    <p class="text-[0.85rem] text-text break-words">{revokeFailure}</p>
  </Modal>
{/if}
