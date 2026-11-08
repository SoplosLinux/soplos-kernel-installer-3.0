<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { _ } from "svelte-i18n";
  import { api } from "../lib/api";
  import { buildStartedAt } from "../lib/build";

  // What a build costs the computer while it runs: the time, the load of the CPU,
  // the memory, the disk of the folder of builds and the temperature. It is the
  // panel of the 2.x program: the numbers are read every two seconds.
  interface Stats {
    load1: number;
    load5: number;
    load15: number;
    cpuCount: number;
    memUsedGb: number;
    memTotalGb: number;
    memPercent: number;
    diskFreeGb: number;
    diskTotalGb: number;
    diskPercent: number;
    cpuTemp: number | null;
  }

  let stats: Stats | null = null;
  let now = Date.now();
  let clock: ReturnType<typeof setInterval> | undefined;
  let poll: ReturnType<typeof setInterval> | undefined;
  let reading = false;

  async function read() {
    if (reading) return;
    reading = true;
    try {
      stats = await api.invoke<Stats>("get_system_stats");
    } catch {
      // The panel is a courtesy: a failed reading leaves the last one.
    }
    reading = false;
  }

  onMount(() => {
    read();
    clock = setInterval(() => (now = Date.now()), 1000);
    poll = setInterval(read, 2000);
  });
  onDestroy(() => {
    clearInterval(clock);
    clearInterval(poll);
  });

  const pad = (n: number) => String(n).padStart(2, "0");
  $: elapsed = $buildStartedAt ? Math.max(0, Math.floor((now - $buildStartedAt) / 1000)) : 0;
  $: time = `${pad(Math.floor(elapsed / 3600))}:${pad(Math.floor((elapsed % 3600) / 60))}:${pad(elapsed % 60)}`;
  $: cpu = stats ? Math.min(100, (stats.load1 / Math.max(stats.cpuCount, 1)) * 100) : 0;
  $: tiles = stats
    ? [
        { key: "cpu", label: $_("stats.cpu"), value: `${cpu.toFixed(0)}%`, percent: cpu },
        { key: "memory", label: $_("stats.memory"), value: `${stats.memUsedGb.toFixed(1)} / ${stats.memTotalGb.toFixed(1)} GB`, percent: stats.memPercent },
        { key: "disk", label: $_("stats.disk"), value: $_("stats.free", { values: { n: stats.diskFreeGb.toFixed(1) } }), percent: stats.diskPercent },
        { key: "temperature", label: $_("stats.temperature"), value: stats.cpuTemp === null ? "--°C" : `${stats.cpuTemp.toFixed(0)}°C`, percent: stats.cpuTemp ?? 0 },
      ]
    : [];
</script>

<div class="rounded border border-border bg-surface-2/40 p-3 mb-3">
  <div class="flex items-center justify-between mb-2 text-[0.8rem]">
    <span class="text-text-muted">{$_("stats.time")}</span>
    <span class="font-mono text-text-bright">{time}</span>
  </div>
  {#if stats}
    <div class="grid grid-cols-2 sm:grid-cols-4 gap-x-5 gap-y-3">
      {#each tiles as tile (tile.key)}
        <div>
          <div class="flex items-baseline justify-between gap-2 text-[0.78rem] mb-1">
            <span class="text-text-muted">{tile.label}</span>
            <span class="text-text-bright truncate">{tile.value}</span>
          </div>
          <div class="h-1.5 rounded bg-surface-2 overflow-hidden">
            <div class="h-full bg-primary" style="width: {Math.max(0, Math.min(100, tile.percent))}%"></div>
          </div>
        </div>
      {/each}
    </div>
    <p class="text-[0.72rem] text-text-muted mt-2">
      {$_("stats.load", { values: { a: stats.load1.toFixed(2), b: stats.load5.toFixed(2), c: stats.load15.toFixed(2) } })}
    </p>
  {/if}
</div>
