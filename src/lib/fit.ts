/**
 * Makes a box as tall as the page has room left under it, so that the page does
 * not need its own scroll and the box is the only thing that scrolls. The room
 * is measured against the scrolling container of the pages, from where the box
 * starts inside it, so scrolling the page does not change the height.
 */
export function fitToWindow(node: HTMLElement, options: { min?: number; margin?: number } = {}) {
  const min = options.min ?? 140;
  // What is left under the box: the padding of the page (28) and the one of the
  // card that holds it (20, and its border).
  const margin = options.margin ?? 50;

  function fit() {
    const container = document.getElementById("main-scroll");
    if (!container) return;
    const top = node.getBoundingClientRect().top - container.getBoundingClientRect().top + container.scrollTop;
    const height = Math.max(min, Math.floor(container.clientHeight - top - margin));
    if (Math.abs(node.offsetHeight - height) > 2) node.style.height = `${height}px`;
  }

  fit();
  // What is above the box changes (a stage, a list of what is finished).
  const timer = setInterval(fit, 1000);
  window.addEventListener("resize", fit);
  return {
    destroy() {
      clearInterval(timer);
      window.removeEventListener("resize", fit);
    },
  };
}
