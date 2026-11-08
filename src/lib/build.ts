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
  /** How long each finished kernel took, in seconds, in the same order. */
  timings: number[];
  /** When the kernel being built started, in milliseconds since 1970. */
  startedAt: number;
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

/** When the build that runs was first seen, for the clock of the panel of what it costs. */
export const buildStartedAt = writable<number | null>(null);

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
    // The queue of a batch moves on without a terminal event, and a batch that
    // has just started is not known here until it is asked: ask now and then,
    // knowing it or not.
    if (Date.now() - lastRefresh > 2000) {
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
  building.subscribe((running) => {
    busy.set(running);
    buildStartedAt.update((t) => (running ? t ?? Date.now() : null));
  });
}

/**
 * Cancels the build that runs, and when `deleteFolder` is true deletes the folder
 * of the builds once the build has stopped, as the 2.x program did.
 */
export async function cancelBuild(deleteFolder: boolean) {
  await api.invoke("cancel_build");
  if (!deleteFolder) return;
  // The programs of the build stop a moment after the order: the folder is
  // deleted when the build says it is over, and not under their feet.
  for (let i = 0; i < 150 && get(job).running; i++) {
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  try {
    await api.invoke("clean_build_dir");
  } catch (error) {
    console.error("Failed to delete the folder of the builds", error);
  }
}
