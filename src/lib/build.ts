import { listen } from "@tauri-apps/api/event";
import { derived, get, writable } from "svelte/store";
import { api } from "./api";
import { busy } from "./ops";

export interface Recipe {
  version: string;
  goal: string;
  patches: string[];
  march: string;
  name: string;
  sign: boolean;
  cores: number;
  reuseSources: boolean;
  schedExt: boolean;
}

export interface BatchProgress {
  index: number;
  total: number;
  name: string;
  completed: string[];
  dest: string;
  failed: string | null;
}

export interface BuildJob {
  running: boolean;
  stage: string;
  percent: number;
  lines: string[];
  result: "done" | "failed" | "cancelled" | null;
  error: string | null;
  release: string | null;
  recipe: Recipe | null;
  nvidia: string | null;
  batch: BatchProgress | null;
}

const EMPTY: BuildJob = { running: false, stage: "", percent: 0, lines: [], result: null, error: null, release: null, recipe: null, nvidia: null, batch: null };

/** The build that runs or ran last. It lives here so that it goes on when the page is left. */
export const job = writable<BuildJob>(EMPTY);
export const building = derived(job, (j) => j.running);

let started = false;
let lastRefresh = 0;

async function refresh() {
  try {
    job.set(await api.invoke<BuildJob>("get_build_job"));
  } catch (error) {
    console.error("Failed to read the build", error);
  }
}

/** Follows the build for the whole life of the window. */
export async function initBuild() {
  if (started) return;
  started = true;
  await refresh();
  await listen<{ stage: string; percent: number; level: string; line: string }>("build-event", (event) => {
    const e = event.payload;
    if (e.stage === "done" || e.stage === "failed" || e.stage === "cancelled") {
      refresh();
      return;
    }
    // The queue of a batch moves on without a terminal event: ask now and then.
    if (get(job).batch && Date.now() - lastRefresh > 2000) {
      lastRefresh = Date.now();
      refresh();
    }
    job.update((j) => ({
      ...j,
      running: true,
      stage: e.stage,
      percent: Math.max(j.percent, e.percent),
      lines: e.line ? [...j.lines, e.level === "info" ? e.line : `[${e.level}] ${e.line}`].slice(-600) : j.lines,
    }));
  });
  // Other pages keep their hands off apt while a kernel is built.
  building.subscribe((running) => busy.set(running));
}
