<script lang="ts">
  import { setContext, onMount, type ComponentType } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { getVersion } from "@tauri-apps/api/app";
  import { _ } from "svelte-i18n";
  import { progress } from "./lib/progress";
  import Sidebar from "./components/Sidebar.svelte";
  import Hero from "./components/Hero.svelte";
  import { api } from "./lib/api";
  import { initBuild } from "./lib/build";
  import WindowControls from "./components/WindowControls.svelte";
  import AboutDialog from "./components/AboutDialog.svelte";
  import Build from "./views/Build.svelte";
  import SoplosKernels from "./views/SoplosKernels.svelte";
  import Installed from "./views/Installed.svelte";
  import SecureBoot from "./views/SecureBoot.svelte";
  import Batch from "./views/Batch.svelte";
  import { Hammer, Package, HardDrive, ShieldCheck, Layers } from "lucide-svelte";

  // The batch page is for advanced use: it stays out of the sidebar until
  // Ctrl+Shift+D shows it.
  let batchVisible = false;

  $: tabs = [
    { id: "build", icon: Hammer, label: $_("app.tabs.build.label"), title: $_("app.tabs.build.title") },
    { id: "kernels", icon: Package, label: $_("app.tabs.kernels.label"), title: $_("app.tabs.kernels.title") },
    { id: "installed", icon: HardDrive, label: $_("app.tabs.installed.label"), title: $_("app.tabs.installed.title") },
    { id: "secureboot", icon: ShieldCheck, label: $_("app.tabs.secureboot.label"), title: $_("app.tabs.secureboot.title") },
    ...(batchVisible
      ? [{ id: "batch", icon: Layers, label: $_("app.tabs.batch.label"), title: $_("app.tabs.batch.title") }]
      : []),
  ];

  const views: Record<string, ComponentType> = {
    build: Build,
    kernels: SoplosKernels,
    installed: Installed,
    secureboot: SecureBoot,
    batch: Batch,
  };

  let active = "build";
  let appVersion = "";
  let running = "";

  // With `decorations` off in tauri.conf.json (and `transparent` on) the system
  // draws no frame: the window is one card with rounded corners and the buttons
  // of the window float at its top right. With `decorations` on, nothing of
  // this shows, so that setting is the only switch between the two.
  let frameless = false;
  let maximized = false;

  onMount(() => {
    initBuild();
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        appVersion = await getVersion();
      } catch {
        appVersion = "";
      }
      try {
        running = await api.invoke<string>("get_running_kernel");
      } catch {
        running = "";
      }
      try {
        const win = getCurrentWindow();
        frameless = !(await win.isDecorated());
        if (frameless) {
          maximized = await win.isMaximized();
          unlisten = await win.onResized(async () => {
            maximized = await win.isMaximized();
          });
        }
      } catch {
        frameless = false;
      }
    })();
    return () => unlisten?.();
  });

  function select(id: string) {
    active = id;
    document.getElementById("main-scroll")?.scrollTo({ top: 0 });
  }

  // Lets any view switch tabs without prop-drilling.
  setContext("navigate", select);
  setContext("openAbout", () => (aboutOpen = true));

  $: activeTab = tabs.find((t) => t.id === active) ?? tabs[0];
  $: ActiveView = views[activeTab.id];

  // F1 -> About, Ctrl+Q -> quit, Ctrl+1..5 -> the pages, Ctrl+Shift+D -> batch page.
  let aboutOpen = false;

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "F1") {
      e.preventDefault();
      aboutOpen = true;
      return;
    }
    if (e.ctrlKey && (e.key === "q" || e.key === "Q")) {
      e.preventDefault();
      getCurrentWindow().close();
      return;
    }
    // Ctrl+1 to Ctrl+5: the pages, in the order of the sidebar.
    if (e.ctrlKey && !e.shiftKey && !e.altKey && /^[1-9]$/.test(e.key)) {
      const tab = tabs[Number(e.key) - 1];
      if (tab) {
        e.preventDefault();
        select(tab.id);
      }
      return;
    }
    if (e.ctrlKey && e.shiftKey && (e.key === "d" || e.key === "D")) {
      e.preventDefault();
      batchVisible = !batchVisible;
      if (!batchVisible && active === "batch") select("build");
    }
  }
</script>

<svelte:window on:keydown={handleKeydown} />

<div class="relative flex h-screen overflow-hidden bg-dark {frameless && !maximized ? 'window-card' : ''}">
  <Sidebar {tabs} active={activeTab.id} onSelect={select} {appVersion} />

  {#if frameless}
    <div class="absolute right-2 top-[5px] z-40"><WindowControls {maximized} /></div>
  {/if}

  <div class="flex-1 flex flex-col overflow-hidden min-w-0">
    <!-- Every page opens with the hero and has no header: the strip over it
         holds the window buttons and moves the window. -->
    {#if frameless}
      <div class="h-7 shrink-0" data-tauri-drag-region></div>
    {/if}
    <div id="main-scroll" class="content-scroll flex-1 overflow-y-auto p-7 {frameless ? 'pt-0' : ''}">
      <Hero
        title={activeTab.id === "build" ? $_("build.title") : activeTab.title}
        description={$_(`${activeTab.id}.description`)}
        chip={running ? $_("build.running", { values: { kernel: running } }) : ""}
      />
      <svelte:component this={ActiveView} />
    </div>

    <div class="h-1.5 bg-surface-2 overflow-hidden shrink-0" class:invisible={$progress === null}>
      {#if typeof $progress === "number"}
        <div class="h-full bg-primary transition-[width]" style="width: {$progress * 100}%"></div>
      {:else if $progress === "busy"}
        <div class="h-full w-1/3 bg-primary progress-fill"></div>
      {/if}
    </div>
  </div>

  {#if aboutOpen}
    <AboutDialog onClose={() => (aboutOpen = false)} />
  {/if}
</div>
