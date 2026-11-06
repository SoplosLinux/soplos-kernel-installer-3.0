import { listen } from "@tauri-apps/api/event";
import { get, writable } from "svelte/store";
import { progress } from "./progress";

/** The last line the administrator step printed, for the status of a page. */
export const opsLine = writable("");
/** True while an operation runs: the pages use it to disable their buttons. */
export const busy = writable(false);

// The share of the work that apt reports, read from its own lines (it always
// runs with LC_ALL=C, so they are English). Never goes back.
function share(line: string, current: number): number {
  const m = line.match(/(\d+)\s*%/);
  let next = current;
  if (m) {
    const value = Math.min(Math.max(parseInt(m[1], 10) / 100, 0), 1);
    next = Math.max(next, line.includes("Reading database") ? value * 0.3 : value);
  } else if (line.includes("Unpacking")) {
    next = Math.max(next, 0.4);
  } else if (/dracut|initramfs/i.test(line)) {
    next = Math.max(next, 0.8);
  } else if (/grub/i.test(line)) {
    next = Math.max(next, 0.9);
  } else if (line.includes("Setting up")) {
    next = Math.max(next, current >= 0.9 ? 0.95 : 0.6);
  }
  return next;
}

/**
 * Runs one operation of the administrator and follows its output.
 * Rejects with the message of the error; "cancelled" means the password window
 * was closed, which is not a failure to show.
 */
export async function runOp<T>(call: () => Promise<T>): Promise<T> {
  if (get(busy)) throw new Error("busy");
  busy.set(true);
  opsLine.set("");
  progress.set("busy");
  let fraction = 0;
  const unlisten = await listen<string>("ops-line", (event) => {
    const line = event.payload;
    opsLine.set(line.slice(0, 120));
    const next = share(line, fraction);
    if (next > fraction) {
      fraction = next;
      progress.set(fraction);
    }
  });
  try {
    return await call();
  } finally {
    unlisten();
    progress.set(null);
    busy.set(false);
    opsLine.set("");
  }
}
