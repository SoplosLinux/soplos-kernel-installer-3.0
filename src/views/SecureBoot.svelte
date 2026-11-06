<script lang="ts">
  import { getContext, onMount, tick } from "svelte";
  import { _ } from "svelte-i18n";
  import { AlertTriangle, Check, Minus, ShieldCheck, ShieldOff } from "lucide-svelte";
  import Modal from "../components/Modal.svelte";
  import PasswordForm from "../components/PasswordForm.svelte";
  import { api } from "../lib/api";
  import { busy, opsLine, runOp } from "../lib/ops";

  interface KernelSigning {
    version: string;
    signer: string;
    trusted: boolean;
    isCurrent: boolean;
  }
  interface EnrolledKey {
    fingerprint: string;
    subject: string;
    own: boolean;
    dkms: boolean;
  }
  interface LoaderFile {
    path: string;
    dir: string;
    active: boolean;
    signer: string;
    trusted: boolean;
    shim: boolean;
  }
  interface LoaderState {
    id: string;
    active: boolean;
    version: string | null;
    files: LoaderFile[];
  }
  interface PayloadSigning {
    path: string;
    signer: string;
    trusted: boolean;
  }
  interface LimineState {
    configPath: string | null;
    total: number;
    matching: number;
    unresolved: number;
    enrolled: boolean;
    hardened: boolean;
    tool: boolean;
  }
  interface ChainItem {
    id: string;
    status: "ok" | "bad" | "unknown" | "off";
    count: number | null;
    total: number | null;
  }
  interface Chain {
    bootloader: string | null;
    items: ChainItem[];
  }
  interface SystemInfo {
    vendor: string | null;
    product: string | null;
    cpu: string | null;
    firmwareVendor: string | null;
    firmwareVersion: string | null;
    firmwareDate: string | null;
    tpm: boolean;
    setupMode: boolean | null;
    bootEntry: string | null;
  }
  interface State {
    efi: boolean;
    mokutil: boolean;
    openssl: boolean;
    sbsign: boolean;
    secureBootEnabled: boolean;
    keysExist: boolean;
    keysDir: string;
    enrolled: boolean;
    enrollRequested: boolean;
    dkmsModules: boolean;
    dkmsKeyExists: boolean;
    dkmsEnrolled: boolean;
    dkmsRequested: boolean;
    kernels: KernelSigning[];
    autoSign: boolean;
    keysListed: boolean;
    enrolledKeys: EnrolledKey[];
    system: SystemInfo;
    bootloaders: LoaderState[];
    payloads: PayloadSigning[];
    limine: LimineState | null;
    certExported: boolean;
    certPath: string;
    grubVersion: string | null;
    chain: Chain;
  }

  // Names that are the same in every language.
  const LOADER_NAMES: Record<string, string> = { grub: "GRUB", "systemd-boot": "systemd-boot", limine: "Limine" };
  const LOADER_KEYS: Record<string, string> = { grub: "grub", "systemd-boot": "systemdBoot", limine: "limine" };

  const navigate = getContext<(id: string) => void>("navigate");

  let state: State | null = null;
  let selected = "";
  let message: { title: string; text: string } | null = null;
  let dialog: "restart" | "delete" | "reset" | null = null;
  let showKeys = false;

  // What each key is doing right now, in words.
  $: ownKeyStatus = state
    ? !state.keysExist
      ? "none"
      : state.enrolled
        ? "trusted"
        : state.enrollRequested
          ? "waiting"
          : "notTrusted"
    : "none";
  $: dkmsKeyStatus = state
    ? !state.dkmsKeyExists
      ? "missing"
      : state.dkmsEnrolled
        ? "trusted"
        : state.dkmsRequested
          ? "waiting"
          : "notTrusted"
    : "missing";

  // Who signed a file can only be read with sbverify: without it nothing is known.
  $: signersKnown = state ? state.sbsign : false;
  $: tools = state ? state.mokutil && state.openssl && state.sbsign : false;
  $: steps = state
    ? [
        { id: "check", done: state.efi && tools },
        { id: "key", done: state.keysExist },
        { id: "enroll", done: state.enrolled },
        ...(state.dkmsModules ? [{ id: "dkms", done: state.dkmsEnrolled }] : []),
        { id: "sign", done: signDone },
        { id: "enable", done: state.secureBootEnabled },
      ]
    : [];
  $: blocked = state ? !state.efi : true;
  $: unsigned = state ? state.kernels.filter((k) => !k.trusted) : [];
  // What the signing step is about depends on the bootloader that starts the computer.
  $: checkedLoader = state ? state.chain.bootloader : null;
  $: signDone = state
    ? signersKnown && state.chain.items.length > 0 && state.chain.items.filter((i) => ["loader", "kernels", "payloads"].includes(i.id)).every((i) => i.status === "ok")
    : false;
  $: needsSigning = state
    ? state.bootloaders.some((l) => l.files.some((f) => !f.trusted)) || state.payloads.some((p) => !p.trusted)
    : false;
  // Something is signed with the key of the user, so an update would undo it.
  $: ownSigned = state
    ? state.kernels.some((k) => k.signer === "own") || state.bootloaders.some((l) => l.files.some((f) => f.signer === "own"))
    : false;
  $: chainOk = state ? signersKnown && state.chain.items.length > 0 && state.chain.items.every((i) => i.status === "ok" || i.status === "off") : false;

  // The step to show after something changed: the first one still to do.
  function advance() {
    const next = steps.find((s) => !s.done) ?? steps[steps.length - 1];
    if (next) selected = next.id;
  }

  async function load(move = false) {
    try {
      state = await api.invoke<State>("get_secure_boot_state");
    } catch (error) {
      console.error("Failed to read Secure Boot", error);
    }
    // The list of steps is worked out from the state in the next update, so
    // the step to show can only be chosen after it.
    await tick();
    if (selected === "") {
      // The first time, the guide opens at its first step, done or not.
      selected = steps[0]?.id ?? "";
    } else if (move) {
      advance();
    }
  }

  onMount(() => load());

  async function perform(call: () => Promise<unknown>, failTitle: string): Promise<boolean> {
    let ok = false;
    try {
      await runOp(call);
      ok = true;
    } catch (error) {
      const text = String(error instanceof Error ? error.message : error);
      if (text !== "cancelled" && text !== "busy") message = { title: failTitle, text };
    }
    await load(ok);
    return ok;
  }

  const installTools = () => perform(() => api.invoke("sb_install_tools"), $_("secureboot.errors.tools"));
  const createKey = () => perform(() => api.invoke("sb_generate_keys"), $_("secureboot.errors.key"));
  const enroll = (kind: "kernel" | "dkms", password: string) =>
    perform(() => api.invoke("sb_enroll", { kind, password }), $_("secureboot.errors.enroll"));
  const sign = (releases: string[]) => perform(() => api.invoke("sb_sign", { releases }), $_("secureboot.errors.sign"));
  const signBoot = () => perform(() => api.invoke("sb_sign_boot"), $_("secureboot.errors.sign"));
  const refreshLimine = () => perform(() => api.invoke("sb_limine_refresh"), $_("secureboot.errors.limine"));
  const exportCert = () => perform(() => api.invoke("sb_export_cert"), $_("secureboot.errors.cert"));
  const setAutoSign = (enabled: boolean) => perform(() => api.invoke("sb_set_auto_sign", { enabled }), $_("secureboot.errors.auto"));

  async function removeKey(password: string) {
    dialog = null;
    await perform(() => api.invoke("sb_delete_key", { password, enrolled: state?.enrolled ?? false }), $_("secureboot.errors.delete"));
  }

  async function resetDatabase(password: string) {
    dialog = null;
    await perform(() => api.invoke("sb_reset", { password }), $_("secureboot.errors.reset"));
  }

  async function restart() {
    dialog = null;
    try {
      await api.invoke("reboot_now");
    } catch (error) {
      message = { title: $_("secureboot.errors.restart"), text: String(error) };
    }
  }

  const primary = "px-4 py-1.5 rounded bg-primary hover:bg-primary-hover text-text-bright text-[0.82rem] font-semibold disabled:opacity-40";
  const secondary = "px-3.5 py-1.5 rounded border border-border text-[0.82rem] text-text-bright hover:bg-surface-2 disabled:opacity-40";
  const pill = "px-2 py-0.5 rounded-full text-[0.72rem] font-semibold";
</script>

{#if state}
  <div class="flex flex-col gap-3.5">
    <!-- Where everything stands, whatever step is open. -->
    <section class="rounded border border-border bg-surface p-4">
      <h3 class="text-[0.95rem] font-semibold text-text-bright mb-2">{$_("secureboot.status.title")}</h3>
      <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-[0.85rem]">
        <dt class="text-text-muted">{$_("secureboot.status.secureBoot")}</dt>
        <dd><span class="{pill} {state.secureBootEnabled ? 'bg-primary/15 text-primary' : 'bg-surface-2 text-text'}">{state.secureBootEnabled ? $_("secureboot.on") : $_("secureboot.off")}</span></dd>
        <dt class="text-text-muted">{$_("secureboot.status.yourKey")}</dt>
        <dd><span class="{pill} {ownKeyStatus === 'trusted' ? 'bg-primary/15 text-primary' : ownKeyStatus === 'waiting' ? 'bg-warning/15 text-warning' : 'bg-surface-2 text-text'}">{$_(`secureboot.status.${ownKeyStatus}`)}</span></dd>
        {#if state.dkmsModules}
          <dt class="text-text-muted">{$_("secureboot.status.dkmsKey")}</dt>
          <dd><span class="{pill} {dkmsKeyStatus === 'trusted' ? 'bg-primary/15 text-primary' : dkmsKeyStatus === 'waiting' || dkmsKeyStatus === 'missing' ? 'bg-warning/15 text-warning' : 'bg-surface-2 text-text'}">{$_(`secureboot.status.${dkmsKeyStatus}`)}</span></dd>
        {/if}
        <dt class="text-text-muted">{$_("secureboot.status.firmwareKeys")}</dt>
        <dd class="flex items-center gap-3">
          {#if state.keysListed}
            <span class="text-text-bright">{$_("secureboot.status.keyCount", { values: { count: state.enrolledKeys.length } })}</span>
            {#if state.enrolledKeys.length > 0}
              <button class="text-[0.78rem] text-primary hover:underline" on:click={() => (showKeys = !showKeys)}>
                {showKeys ? $_("secureboot.status.hide") : $_("secureboot.status.show")}
              </button>
            {/if}
          {:else}
            <span class="text-text-muted">{$_("secureboot.status.notListed")}</span>
          {/if}
        </dd>
      </dl>
      {#if showKeys && state.enrolledKeys.length > 0}
        <ul class="mt-3 border-t border-border">
          {#each state.enrolledKeys as key (key.fingerprint + key.subject)}
            <li class="flex items-center gap-3 py-2 border-b border-border last:border-b-0">
              <span class="flex-1 min-w-0 truncate text-[0.8rem] text-text-bright" title={key.fingerprint}>{key.subject || key.fingerprint}</span>
              {#if key.own}<span class="{pill} bg-primary/15 text-primary">{$_("secureboot.status.mine")}</span>{/if}
              {#if key.dkms}<span class="{pill} bg-primary/15 text-primary">{$_("secureboot.status.drivers")}</span>{/if}
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <!-- The steps, in the order they are done. -->
    <ol class="flex flex-wrap gap-2">
      {#each steps as step, index (step.id)}
        <li>
          <button
            class="flex items-center gap-2 rounded border px-3 py-1.5 text-[0.82rem] transition-colors
              {selected === step.id ? 'border-primary bg-primary/15 text-primary' : 'border-border bg-surface text-text hover:bg-surface-2'}"
            on:click={() => (selected = step.id)}
          >
            <span class="w-5 h-5 rounded-full grid place-items-center text-[0.7rem] font-bold {step.done ? 'bg-primary text-text-bright' : 'bg-surface-2 text-text-muted'}">
              {#if step.done}<Check size={12} />{:else}{index + 1}{/if}
            </span>
            {$_(`secureboot.steps.${step.id}`)}
          </button>
        </li>
      {/each}
    </ol>

    <section class="rounded border border-border bg-surface p-5">
      <h3 class="text-[1rem] font-semibold text-text-bright mb-1">{$_(`secureboot.steps.${selected}`)}</h3>

      {#if selected === "check"}
        <p class="text-[0.85rem] text-text mb-3">{$_("secureboot.check.intro")}</p>
        <h4 class="text-[0.9rem] font-semibold text-text-bright mb-2">{$_("secureboot.system.title")}</h4>
        <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 mb-3 text-[0.85rem]">
          {#if state.system.vendor || state.system.product}
            <dt class="text-text-muted">{$_("secureboot.system.computer")}</dt>
            <dd class="text-text-bright">{[state.system.vendor, state.system.product].filter(Boolean).join(" ")}</dd>
          {/if}
          {#if state.system.cpu}
            <dt class="text-text-muted">{$_("kernels.hardware.cpu")}</dt>
            <dd class="text-text-bright">{state.system.cpu}</dd>
          {/if}
          {#if state.system.firmwareVendor || state.system.firmwareVersion}
            <dt class="text-text-muted">{$_("secureboot.system.firmware")}</dt>
            <dd class="text-text-bright">{[state.system.firmwareVendor, state.system.firmwareVersion, state.system.firmwareDate ? `(${state.system.firmwareDate})` : ""].filter(Boolean).join(" ")}</dd>
          {/if}
          <dt class="text-text-muted">{$_("secureboot.system.bootMode")}</dt>
          <dd><span class="{pill} {state.efi ? 'bg-primary/15 text-primary' : 'bg-warning/15 text-warning'}">{state.efi ? $_("secureboot.system.uefi") : $_("secureboot.system.legacy")}</span></dd>
          {#if state.system.setupMode !== null}
            <dt class="text-text-muted">{$_("secureboot.system.setupMode")}</dt>
            <dd><span class="{pill} bg-surface-2 text-text">{state.system.setupMode ? $_("secureboot.system.setupOpen") : $_("secureboot.system.setupProtected")}</span></dd>
          {/if}
          <dt class="text-text-muted">{$_("secureboot.check.tools")}</dt>
          <dd><span class="{pill} {tools ? 'bg-primary/15 text-primary' : 'bg-warning/15 text-warning'}">{tools ? $_("secureboot.check.toolsOk") : $_("secureboot.check.toolsMissing")}</span></dd>
          <dt class="text-text-muted">{$_("secureboot.system.tpm")}</dt>
          <dd><span class="{pill} bg-surface-2 text-text">{state.system.tpm ? $_("secureboot.system.tpmYes") : $_("secureboot.system.tpmNo")}</span></dd>
          <dt class="text-text-muted">{$_("secureboot.system.bootloader")}</dt>
          <dd class="text-text-bright">
            {#if state.bootloaders.length === 0}
              <span class="text-text-muted">{$_("secureboot.sign.noBootloader")}</span>
            {:else}
              {#each state.bootloaders as loader (loader.id)}
                <span class="inline-block mr-3">
                  {LOADER_NAMES[loader.id] ?? loader.id}{#if loader.version}<span class="text-text-muted text-[0.78rem]"> {loader.version}</span>{/if}
                  {#if loader.active}<span class="text-[0.7rem] text-primary"> {$_("secureboot.sign.startedFrom")}</span>{/if}
                </span>
              {/each}
            {/if}
          </dd>
          {#if state.system.bootEntry}
            <dt class="text-text-muted">{$_("secureboot.system.entry")}</dt>
            <dd class="text-text-bright">{state.system.bootEntry}</dd>
          {/if}
        </dl>
        {#if !state.efi}
          <div class="flex items-start gap-3 rounded border border-warning/40 bg-warning/10 px-3 py-2 mb-3">
            <AlertTriangle size={16} class="text-warning shrink-0 mt-0.5" />
            <p class="text-[0.82rem] text-text">{$_("secureboot.check.legacy")}</p>
          </div>
        {/if}
        {#if !tools}
          <button class={primary} disabled={$busy} on:click={installTools}>{$_("secureboot.check.install")}</button>
        {/if}
      {:else if selected === "key"}
        <p class="text-[0.85rem] text-text mb-3">{$_("secureboot.key.text")}</p>
        {#if state.keysExist}
          <p class="text-[0.85rem] text-text-bright mb-1">{$_("secureboot.key.created")}</p>
          <p class="text-[0.75rem] text-text-muted select-text break-all">{state.keysDir}</p>
          <details class="mt-4">
            <summary class="cursor-pointer text-[0.82rem] text-text-muted hover:text-text-bright">{$_("secureboot.advanced.title")}</summary>
            <div class="mt-2">
              <p class="text-[0.85rem] font-semibold text-text-bright">{$_("secureboot.advanced.delete")}</p>
              <p class="text-[0.8rem] text-text-muted mb-2">{$_("secureboot.advanced.deleteText")}</p>
              <button class={secondary} disabled={$busy} on:click={() => (dialog = "delete")}>{$_("secureboot.advanced.delete")}</button>
            </div>
          </details>
        {:else}
          <button class={primary} disabled={$busy || blocked || !state.openssl} on:click={createKey}>{$_("secureboot.key.create")}</button>
          {#if !state.openssl}<p class="text-[0.78rem] text-text-muted mt-2">{$_("secureboot.key.needTools")}</p>{/if}
        {/if}
      {:else if selected === "enroll"}
        <p class="text-[0.85rem] text-text mb-3">{$_("secureboot.enroll.text")}</p>
        {#if state.enrolled}
          <p class="text-[0.85rem] text-text-bright">{$_("secureboot.enroll.done")}</p>
        {:else if state.enrollRequested}
          <p class="text-[0.85rem] text-text-bright mb-2">{$_("secureboot.enroll.pendingTitle")}</p>
          <ol class="list-decimal pl-5 text-[0.85rem] text-text flex flex-col gap-1 mb-3">
            <li>{$_("secureboot.enroll.blue1")}</li>
            <li>{$_("secureboot.enroll.blue2")}</li>
            <li>{$_("secureboot.enroll.blue3")}</li>
            <li>{$_("secureboot.enroll.blue4")}</li>
          </ol>
          <button class={primary} disabled={$busy} on:click={() => (dialog = "restart")}>{$_("secureboot.restart")}</button>
        {:else if !state.keysExist}
          <p class="text-[0.85rem] text-text-muted">{$_("secureboot.needKey")}</p>
        {:else}
          <p class="text-[0.85rem] text-text mb-2">{$_("secureboot.enroll.password")}</p>
          <PasswordForm submitLabel={$_("secureboot.enroll.register")} disabled={$busy || blocked || !state.mokutil} onSubmit={(pw) => enroll("kernel", pw)} />
        {/if}
        <details class="mt-5">
          <summary class="cursor-pointer text-[0.82rem] text-text-muted hover:text-text-bright">{$_("secureboot.advanced.title")}</summary>
          <div class="mt-2">
            <p class="text-[0.85rem] font-semibold text-text-bright">{$_("secureboot.advanced.reset")}</p>
            <p class="text-[0.8rem] text-text-muted mb-2">{$_("secureboot.advanced.resetText")}</p>
            <button class={secondary} disabled={$busy || !state.mokutil} on:click={() => (dialog = "reset")}>{$_("secureboot.advanced.reset")}</button>
          </div>
        </details>
      {:else if selected === "dkms"}
        <p class="text-[0.85rem] text-text mb-3">{$_("secureboot.dkms.text")}</p>
        {#if state.dkmsEnrolled}
          <p class="text-[0.85rem] text-text-bright">{$_("secureboot.dkms.done")}</p>
        {:else if !state.dkmsKeyExists}
          <div class="flex items-start gap-3 rounded border border-warning/40 bg-warning/10 px-3 py-2">
            <AlertTriangle size={16} class="text-warning shrink-0 mt-0.5" />
            <p class="text-[0.82rem] text-text">{$_("secureboot.dkms.missing")}</p>
          </div>
        {:else if state.dkmsRequested}
          <p class="text-[0.85rem] text-text-bright mb-2">{$_("secureboot.enroll.pendingTitle")}</p>
          <p class="text-[0.85rem] text-text mb-3">{$_("secureboot.dkms.sameScreen")}</p>
          <button class={primary} disabled={$busy} on:click={() => (dialog = "restart")}>{$_("secureboot.restart")}</button>
        {:else}
          <p class="text-[0.85rem] text-text mb-2">{$_("secureboot.enroll.password")}</p>
          <PasswordForm submitLabel={$_("secureboot.enroll.register")} disabled={$busy || blocked || !state.mokutil} onSubmit={(pw) => enroll("dkms", pw)} />
        {/if}
      {:else if selected === "sign"}
        <p class="text-[0.85rem] text-text mb-3">{$_("secureboot.sign.text")}</p>

        {#if !signersKnown}
          <div class="flex items-start gap-3 rounded border border-warning/40 bg-warning/10 px-3 py-2 mb-3">
            <AlertTriangle size={16} class="text-warning shrink-0 mt-0.5" />
            <p class="text-[0.82rem] text-text">{$_("secureboot.sign.needTools")}</p>
          </div>
          <button class={primary} disabled={$busy} on:click={installTools}>{$_("secureboot.check.install")}</button>
        {:else}
        <h4 class="text-[0.9rem] font-semibold text-text-bright mb-1">{$_("secureboot.sign.bootloaderTitle")}</h4>
        <p class="text-[0.8rem] text-text-muted mb-2">{$_("secureboot.sign.bootloaderText")}</p>
        {#if state.bootloaders.length === 0}
          <p class="text-[0.85rem] text-text-muted mb-4">{$_("secureboot.sign.noBootloader")}</p>
        {:else}
          <ul class="mb-3">
            {#each state.bootloaders as loader (loader.id)}
              <li class="py-3 border-t border-border first:border-t-0">
                <p class="text-[0.88rem] text-text-bright">
                  <span class="font-semibold">{LOADER_NAMES[loader.id] ?? loader.id}</span>
                  {#if loader.version}
                    <span class="text-[0.75rem] text-text-muted ml-1.5">{$_("secureboot.sign.version", { values: { version: loader.version } })}</span>
                  {/if}
                  {#if loader.active}<span class="ml-2 text-[0.7rem] text-primary">{$_("secureboot.sign.startedFrom")}</span>{/if}
                </p>
                <p class="text-[0.78rem] text-text-muted mb-1">{$_(`secureboot.loaders.${LOADER_KEYS[loader.id] ?? "grub"}`)}</p>
                {#each loader.files as file (file.path)}
                  <div class="flex items-center gap-3 mt-1">
                    <span class="flex-1 min-w-0 truncate text-[0.75rem] text-text-muted" title={file.path}>{file.path}</span>
                    {#if !file.shim && loader.id !== "limine"}
                      <span class="{pill} bg-warning/15 text-warning">{$_("secureboot.sign.shimMissing")}</span>
                    {/if}
                    <span class="{pill} inline-flex items-center gap-1 {file.trusted ? 'bg-primary/15 text-primary' : 'bg-surface-2 text-text-muted'}">
                      {#if file.trusted}<ShieldCheck size={12} />{:else}<ShieldOff size={12} />{/if}{$_(`secureboot.signer.${file.signer}`)}
                    </span>
                  </div>
                {/each}

                {#if loader.id === "systemd-boot"}
                  <p class="text-[0.78rem] text-text-muted mt-2">{$_("secureboot.loaders.shimNote")}</p>
                  <p class="text-[0.82rem] font-semibold text-text-bright mt-2">{$_("secureboot.loaders.payloadsTitle")}</p>
                  {#if state.payloads.length === 0}
                    <p class="text-[0.78rem] text-text-muted">{$_("secureboot.loaders.noPayloads")}</p>
                  {:else}
                    {#each state.payloads as payload (payload.path)}
                      <div class="flex items-center gap-3 mt-1">
                        <span class="flex-1 min-w-0 truncate text-[0.75rem] text-text-muted" title={payload.path}>{payload.path}</span>
                        <span class="{pill} inline-flex items-center gap-1 {payload.trusted ? 'bg-primary/15 text-primary' : 'bg-surface-2 text-text-muted'}">
                          {#if payload.trusted}<ShieldCheck size={12} />{:else}<ShieldOff size={12} />{/if}{$_(`secureboot.signer.${payload.signer}`)}
                        </span>
                      </div>
                    {/each}
                  {/if}
                {/if}

                {#if loader.id === "limine" && state.limine}
                  <div class="mt-3 rounded border border-border bg-surface-2/40 p-3">
                    <p class="text-[0.85rem] font-semibold text-text-bright mb-1">{$_("secureboot.limine.title")}</p>
                    <p class="text-[0.8rem] text-text mb-2">{$_("secureboot.limine.text")}</p>
                    {#if state.limine.configPath}
                      <p class="text-[0.75rem] text-text-muted truncate" title={state.limine.configPath}>{state.limine.configPath}</p>
                      <p class="text-[0.8rem] text-text mt-1">{$_("secureboot.limine.hashes", { values: { matching: state.limine.matching, total: state.limine.total } })}</p>
                      {#if state.limine.unresolved > 0}
                        <p class="text-[0.78rem] text-warning">{$_("secureboot.limine.unresolved", { values: { count: state.limine.unresolved } })}</p>
                      {/if}
                      <p class="mt-2">
                        <span class="{pill} {state.limine.enrolled ? 'bg-primary/15 text-primary' : 'bg-surface-2 text-text-muted'}">
                          {state.limine.enrolled ? $_("secureboot.limine.enrolled") : $_("secureboot.limine.notEnrolled")}
                        </span>
                      </p>
                    {:else}
                      <p class="text-[0.8rem] text-text-muted">{$_("secureboot.limine.noConfig")}</p>
                    {/if}
                    {#if !state.limine.tool}<p class="text-[0.78rem] text-warning mt-1">{$_("secureboot.limine.needTool")}</p>{/if}
                    <button
                      class="{primary} mt-3"
                      disabled={$busy || blocked || !state.keysExist || !state.limine.tool || !state.limine.configPath || !tools}
                      on:click={refreshLimine}
                    >
                      {$_("secureboot.limine.refresh")}
                    </button>

                    <p class="text-[0.85rem] font-semibold text-text-bright mt-4 mb-1">{$_("secureboot.limine.certTitle")}</p>
                    <p class="text-[0.8rem] text-text mb-2">{$_("secureboot.limine.certText")}</p>
                    <button class={secondary} disabled={$busy || blocked || !state.keysExist} on:click={exportCert}>{$_("secureboot.limine.exportCert")}</button>
                    {#if state.certExported}
                      <p class="text-[0.75rem] text-text-muted mt-1 break-all select-text">{$_("secureboot.limine.exported", { values: { path: state.certPath } })}</p>
                    {/if}
                  </div>
                {/if}
              </li>
            {/each}
          </ul>
          {#if needsSigning && state.keysExist}
            <button class="{primary} mb-4" disabled={$busy || blocked || !tools} on:click={signBoot}>{$_("secureboot.sign.signLoader")}</button>
          {:else if needsSigning}
            <p class="text-[0.82rem] text-text-muted mb-4">{$_("secureboot.needKey")}</p>
          {/if}
        {/if}

        {#if state.keysExist && (checkedLoader === "grub" || checkedLoader === null)}
          <h4 class="text-[0.9rem] font-semibold text-text-bright mb-1">{$_("secureboot.sign.kernelsTitle")}</h4>
          {#if state.kernels.length === 0}
            <p class="text-[0.85rem] text-text-muted">{$_("secureboot.sign.none")}</p>
          {:else}
            <ul class="mb-3">
              {#each state.kernels as kernel (kernel.version)}
                <li class="flex items-center gap-3 py-2 border-t border-border first:border-t-0">
                  <span class="flex-1 min-w-0 truncate text-[0.88rem] {kernel.isCurrent ? 'text-primary font-semibold' : 'text-text-bright'}">{kernel.version}</span>
                  {#if kernel.trusted}
                    <span class="{pill} bg-primary/15 text-primary inline-flex items-center gap-1"><ShieldCheck size={12} />{$_(`secureboot.signer.${kernel.signer}`)}</span>
                  {:else}
                    <span class="{pill} bg-surface-2 text-text-muted inline-flex items-center gap-1"><ShieldOff size={12} />{$_(`secureboot.signer.${kernel.signer}`)}</span>
                    <button class={secondary} disabled={$busy || blocked || !tools} on:click={() => sign([kernel.version])}>{$_("secureboot.sign.sign")}</button>
                  {/if}
                </li>
              {/each}
            </ul>
            {#if unsigned.length > 1}
              <button class={primary} disabled={$busy || blocked || !tools} on:click={() => sign(unsigned.map((k) => k.version))}>{$_("secureboot.sign.all")}</button>
            {/if}
          {/if}
        {/if}
        <p class="text-[0.78rem] text-text-muted mt-3">{$_("secureboot.sign.build")}</p>
        <button class="{secondary} mt-2" on:click={() => navigate("build")}>{$_("secureboot.sign.goBuild")}</button>

        {#if state.keysExist}
          <div class="mt-5 rounded border border-border bg-surface-2/40 p-4">
            <h4 class="text-[0.9rem] font-semibold text-text-bright mb-1">{$_("secureboot.auto.title")}</h4>
            <p class="text-[0.82rem] text-text mb-1">{$_("secureboot.auto.text")}</p>
            <p class="text-[0.78rem] text-text-muted mb-3">{$_("secureboot.auto.note")}</p>
            <div class="flex items-center gap-3">
              <span class="{pill} {state.autoSign ? 'bg-primary/15 text-primary' : 'bg-surface-2 text-text-muted'}">
                {state.autoSign ? $_("secureboot.auto.on") : $_("secureboot.auto.off")}
              </span>
              <button class={state.autoSign ? secondary : primary} disabled={$busy || blocked || !tools} on:click={() => setAutoSign(!state?.autoSign)}>
                {state.autoSign ? $_("secureboot.auto.disable") : $_("secureboot.auto.enable")}
              </button>
            </div>
          </div>
        {/if}
        {/if}
      {:else if selected === "enable"}
        <h4 class="text-[0.9rem] font-semibold text-text-bright mb-2">{$_("secureboot.chain.title")}</h4>
        {#if !signersKnown}
          <p class="text-[0.82rem] text-text-muted mb-4">{$_("secureboot.sign.needTools")}</p>
        {:else if !state.chain.bootloader}
          <p class="text-[0.82rem] text-text-muted mb-4">{$_("secureboot.sign.noBootloader")}</p>
        {:else}
          <p class="text-[0.78rem] text-text-muted mb-2">{LOADER_NAMES[state.chain.bootloader] ?? state.chain.bootloader}</p>
          <ul class="flex flex-col gap-1.5 mb-3 text-[0.85rem]">
            {#each state.chain.items as item (item.id)}
              <li class="flex items-center gap-2">
                <span class={item.status === "ok" ? "text-primary" : item.status === "off" ? "text-text-muted" : "text-warning"}>
                  {#if item.status === "ok"}<Check size={15} />{:else if item.status === "off"}<Minus size={15} />{:else}<AlertTriangle size={15} />{/if}
                </span>
                {$_(`secureboot.chain.${item.id}`)}{#if item.total !== null}<span class="text-text-muted"> ({item.count}/{item.total})</span>{/if}
              </li>
            {/each}
            {#if ownSigned}
              <li class="flex items-center gap-2">
                <span class={state.autoSign ? "text-primary" : "text-warning"}>{#if state.autoSign}<Check size={15} />{:else}<AlertTriangle size={15} />{/if}</span>
                {state.autoSign ? $_("secureboot.chain.autoSign") : $_("secureboot.chain.autoSignOff")}
              </li>
            {/if}
          </ul>
          <p class="text-[0.85rem] mb-4 {chainOk ? 'text-primary font-semibold' : 'text-warning'}">{chainOk ? $_("secureboot.chain.yes") : $_("secureboot.chain.no")}</p>
        {/if}

        <p class="text-[0.85rem] text-text mb-3">{$_("secureboot.enable.text")}</p>
        <ol class="list-decimal pl-5 text-[0.85rem] text-text flex flex-col gap-1 mb-3">
          <li>{$_("secureboot.enable.step1")}</li>
          <li>{$_("secureboot.enable.step2")}</li>
          <li>{$_("secureboot.enable.step3")}</li>
        </ol>
        <p class="text-[0.85rem] mb-3">
          <span class="{pill} {state.secureBootEnabled ? 'bg-primary/15 text-primary' : 'bg-surface-2 text-text'}">
            {state.secureBootEnabled ? $_("secureboot.enable.isOn") : $_("secureboot.enable.isOff")}
          </span>
        </p>
        <button class={secondary} disabled={$busy} on:click={() => (dialog = "restart")}>{$_("secureboot.restart")}</button>
      {/if}

      {#if $busy && $opsLine}
        <p class="mt-3 text-[0.75rem] text-text-muted truncate">{$opsLine}</p>
      {/if}
    </section>

  </div>
{/if}

{#if dialog === "restart"}
  <Modal
    variant="confirm"
    title={$_("secureboot.restartTitle")}
    confirmLabel={$_("secureboot.restart")}
    onConfirm={restart}
    onClose={() => (dialog = null)}
  >
    <p class="text-[0.85rem] text-text">{$_("secureboot.restartText")}</p>
  </Modal>
{:else if dialog === "delete"}
  <Modal variant="danger" title={$_("secureboot.advanced.deleteTitle")} showFooter={false} onClose={() => (dialog = null)}>
    <p class="text-[0.85rem] text-text mb-3">{state?.enrolled ? $_("secureboot.advanced.deleteEnrolled") : $_("secureboot.advanced.deleteLocal")}</p>
    {#if state?.enrolled}
      <PasswordForm danger submitLabel={$_("secureboot.advanced.delete")} onSubmit={removeKey} onCancel={() => (dialog = null)} />
    {:else}
      <div class="flex gap-2">
        <button class="px-4 py-1.5 rounded bg-danger hover:bg-danger/85 text-text-bright text-[0.82rem] font-semibold" on:click={() => removeKey("")}>{$_("secureboot.advanced.delete")}</button>
        <button class="px-4 py-1.5 rounded border border-border text-[0.82rem] text-text hover:bg-surface-2" on:click={() => (dialog = null)}>{$_("common.cancel")}</button>
      </div>
    {/if}
  </Modal>
{:else if dialog === "reset"}
  <Modal variant="danger" title={$_("secureboot.advanced.resetTitle")} showFooter={false} onClose={() => (dialog = null)}>
    <p class="text-[0.85rem] text-text mb-3">{$_("secureboot.advanced.resetConfirm")}</p>
    <PasswordForm danger submitLabel={$_("secureboot.advanced.reset")} onSubmit={resetDatabase} onCancel={() => (dialog = null)} />
  </Modal>
{/if}

{#if message}
  <Modal variant="danger" title={message.title} onClose={() => (message = null)}>
    <p class="text-[0.85rem] text-text break-words">{message.text}</p>
  </Modal>
{/if}
