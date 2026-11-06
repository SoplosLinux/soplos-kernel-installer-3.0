<script lang="ts">
  import { Cpu } from "lucide-svelte";

  export let title: string;
  export let description: string;
  export let chip: string = "";
</script>

<div
  class="rounded px-5 py-4 mb-3.5 flex flex-col items-center text-center border border-border"
  style="background: linear-gradient(rgba(20, 20, 22, 0.55), rgba(20, 20, 22, 0.82)),
    url('/assets/hero/kernel-installer-background.jpg'); background-size: cover; background-position: center;"
>
  <img src="/assets/icons/kernel-installer.png" alt="" class="w-12 h-12 object-contain mb-1.5" />
  <h2 class="text-[1.4rem] font-bold text-white mb-0.5">{title}</h2>
  {#if chip}
    <span class="inline-flex items-center gap-1.5 rounded-full border border-primary/50 bg-black/30 px-3 py-0.5 text-[0.78rem] text-primary mb-1.5">
      <Cpu size={13} />{chip}
    </span>
  {/if}
  <p class="text-[0.85rem] text-white/80 max-w-[560px]">{description}</p>
</div>
