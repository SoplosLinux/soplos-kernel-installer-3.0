<script lang="ts">
  import { _ } from "svelte-i18n";

  export let submitLabel: string;
  export let danger = false;
  export let disabled = false;
  export let onSubmit: (password: string) => void;
  export let onCancel: (() => void) | undefined = undefined;

  let first = "";
  let second = "";
  let error = "";

  function submit() {
    if (first.length < 8) {
      error = $_("secureboot.password.tooShort");
      return;
    }
    if (first !== second) {
      error = $_("secureboot.password.mismatch");
      return;
    }
    error = "";
    const value = first;
    first = "";
    second = "";
    onSubmit(value);
  }

  const input =
    "w-full bg-surface-2 border border-border rounded px-2.5 py-1.5 text-[0.85rem] text-text-bright outline-none focus-visible:ring-2 focus-visible:ring-primary/60";
</script>

<form class="flex flex-col gap-2 max-w-[340px]" on:submit|preventDefault={submit}>
  <input class={input} type="password" autocomplete="new-password" placeholder={$_("secureboot.password.first")} bind:value={first} />
  <input class={input} type="password" autocomplete="new-password" placeholder={$_("secureboot.password.second")} bind:value={second} />
  {#if error}<p class="text-[0.78rem] text-danger">{error}</p>{/if}
  <div class="flex gap-2 mt-1">
    <button
      type="submit"
      class="px-4 py-1.5 rounded text-text-bright text-[0.82rem] font-semibold disabled:opacity-40 {danger ? 'bg-danger hover:bg-danger/85' : 'bg-primary hover:bg-primary-hover'}"
      {disabled}
    >
      {submitLabel}
    </button>
    {#if onCancel}
      <button type="button" class="px-4 py-1.5 rounded border border-border text-[0.82rem] text-text hover:bg-surface-2" on:click={onCancel}>
        {$_("common.cancel")}
      </button>
    {/if}
  </div>
</form>
