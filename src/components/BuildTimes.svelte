<script lang="ts">
  import { _ } from "svelte-i18n";
  import { api } from "../lib/api";
  import { building } from "../lib/build";
  import { duration } from "../lib/time";

  // How long each kernel took to build, the newest first: the history that the
  // program keeps. It is read again each time a build ends.
  interface BuildTime {
    name: string;
    version: string;
    seconds: number;
    date: string;
    cores: number;
  }

  let times: BuildTime[] = [];

  async function load() {
    try {
      times = await api.invoke<BuildTime[]>("get_build_times");
    } catch {
      times = [];
    }
  }
  $: if (!$building) load();

  const when = (iso: string) => {
    const date = new Date(iso);
    return Number.isNaN(date.getTime()) ? iso : date.toLocaleString();
  };
</script>

{#if times.length > 0}
  <section class="rounded border border-border bg-surface p-5">
    <h3 class="text-[0.95rem] font-semibold text-text-bright mb-3">{$_("times.title")}</h3>
    <div class="content-scroll max-h-[260px] overflow-auto overscroll-contain">
      <table class="w-full text-[0.8rem]">
        <tbody>
          {#each times as t, i (i)}
            <tr class="border-t border-border/60 first:border-t-0">
              <td class="py-1.5 pr-4 text-text-muted whitespace-nowrap">{when(t.date)}</td>
              <td class="py-1.5 pr-4 text-text-bright">{t.name}</td>
              <td class="py-1.5 pr-4 text-text-muted">{t.version}</td>
              <td class="py-1.5 text-right font-mono text-text-bright">{duration(t.seconds)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </section>
{/if}
