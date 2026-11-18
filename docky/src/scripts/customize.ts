import { prefersReducedMotion } from "@/lib/motion";

/** The preview desk is smaller than a screen, so the dock is drawn at this share of its size. */
const PREVIEW_SCALE = 0.7;
/** Horizontal room the dock needs besides its icons: gaps, padding, border and a margin to the desk edge. */
const DOCK_CHROME = 72;
/** Width of the magnified area around the pointer, in preview pixels at `PREVIEW_SCALE`. */
const SIGMA = 46;
/** How close to an edge the pointer must be to reveal an auto-hidden bar. */
const EDGE = 34;
/** Extra reach that keeps a revealed dock open while the pointer travels over it. */
const DOCK_REACH = 60;
const CHECK_MS = 1100;
/** How long the dock stays magnified after a magnification setting changes. */
const DEMO_MS = 1400;
/** Gap between preview icons; it must match the gap of `.preview-dock` in the stylesheet. */
const ICON_GAP = 6;

type Strings = Record<string, string>;

/**
 * The Settings replica: switches, sliders, the language picker and the update check
 * all behave like the app, and the preview desk follows them as the dock would.
 */
export function initCustomize(root: HTMLElement) {
  const scene = root.querySelector<HTMLElement>("[data-customize]");
  const preview = root.querySelector<HTMLElement>("[data-preview]");
  const dock = root.querySelector<HTMLElement>("[data-preview-dock]");
  const bar = root.querySelector<HTMLElement>("[data-preview-bar]");
  const island = root.querySelector<HTMLElement>("[data-preview-island]");
  const textNode = root.querySelector<HTMLScriptElement>("[data-settings-text]");
  if (!scene || !preview || !dock || !bar || !textNode) return;

  const texts = JSON.parse(textNode.textContent ?? "{}") as Record<"en" | "es", Strings>;
  let strings = texts.en;
  const icons = [...dock.querySelectorAll<HTMLImageElement>("img")];
  const ranges = new Map(
    [...scene.querySelectorAll<HTMLInputElement>("input[data-setting]")].map((input) => [
      input.dataset.setting ?? "",
      input,
    ]),
  );
  const switches = new Map(
    [...scene.querySelectorAll<HTMLButtonElement>("button[role=switch]")].map((button) => [
      button.dataset.setting ?? "",
      button,
    ]),
  );

  const isOn = (key: string) => switches.get(key)?.getAttribute("aria-checked") === "true";
  const value = (key: string) => Number(ranges.get(key)?.value ?? 0);

  const paintRange = (input: HTMLInputElement) => {
    const min = Number(input.min);
    const max = Number(input.max);
    input.style.setProperty("--p", String((Number(input.value) - min) / (max - min || 1)));
  };

  /* ---- Preview -------------------------------------------------------- */

  let pointerX: number | null = null;
  let nearBottom = false;
  let nearTop = false;
  // Stands in for the pointer while a magnification setting is being changed, so the
  // slider gives feedback even though the pointer is over the Settings window.
  let demoX: number | null = null;
  let demoTimer = 0;

  const layoutDock = () => {
    // Narrow desks (phones) shrink the dock so even the largest icon size fits.
    const largest = icons.length * Number(ranges.get("iconSize")?.max ?? 0);
    const scale = Math.min(PREVIEW_SCALE, (preview.clientWidth - DOCK_CHROME) / (largest || 1));
    const sigma = SIGMA * (scale / PREVIEW_SCALE);
    const size = value("iconSize") * scale;
    const peak = value("magnifiedSize") * scale;
    const focusX = pointerX ?? demoX;
    const magnify = isOn("magnification") && focusX !== null && !prefersReducedMotion();

    // Centres of the resting row: the magnified widths are derived from these.
    const rowWidth = icons.length * size + (icons.length - 1) * ICON_GAP;
    const left = preview.clientWidth / 2 - rowWidth / 2;
    icons.forEach((icon, index) => {
      let width = size;
      if (magnify && focusX !== null) {
        const center = left + index * (size + ICON_GAP) + size / 2;
        const distance = focusX - center;
        width = size + (Math.max(peak, size) - size) * Math.exp(-(distance * distance) / (2 * sigma * sigma));
      }
      icon.style.width = `${width}px`;
      icon.style.height = `${width}px`;
    });
    dock.style.setProperty("--rest", `${size}px`);
  };

  // Pointer moves arrive faster than frames; one layout per frame is enough.
  let queued = 0;
  const schedule = () => {
    if (queued) return;
    queued = requestAnimationFrame(() => {
      queued = 0;
      layoutDock();
    });
  };

  const demoMagnify = () => {
    window.clearTimeout(demoTimer);
    demoX = isOn("magnification") ? preview.clientWidth / 2 : null;
    layoutDock();
    if (demoX === null) return;
    demoTimer = window.setTimeout(() => {
      demoX = null;
      layoutDock();
    }, DEMO_MS);
  };

  const applyBars = () => {
    const dockHidden = isOn("autoHide") && !nearBottom;
    const barShown = isOn("menuBar") && (!isOn("menuBarAutoHide") || nearTop);
    dock.classList.toggle("is-hidden", dockHidden);
    bar.classList.toggle("is-hidden", !barShown);
    // Options of the menu bar fold away with it, as in the app.
    scene.querySelectorAll<HTMLElement>(".settings-row-sub").forEach((row) => {
      row.hidden = !isOn("menuBar");
    });
    island?.toggleAttribute("hidden", !isOn("aiUsage"));
  };

  preview.addEventListener("pointermove", (event) => {
    const rect = preview.getBoundingClientRect();
    const y = event.clientY - rect.top;
    // Once revealed, the dock stays open over its whole height: otherwise it would hide
    // while the pointer climbs towards its icons.
    const reach = !isOn("autoHide") || nearBottom ? EDGE + DOCK_REACH : EDGE;
    nearBottom = rect.height - y < reach;
    nearTop = y < EDGE;
    pointerX = nearBottom ? event.clientX - rect.left : null;
    applyBars();
    schedule();
  });
  preview.addEventListener("pointerleave", () => {
    pointerX = null;
    nearBottom = false;
    nearTop = false;
    applyBars();
    schedule();
  });

  /* ---- Controls ------------------------------------------------------- */

  const syncMagnified = () => {
    const magnified = ranges.get("magnifiedSize");
    if (!magnified) return;
    // As in the app: the magnified size never goes below the icon size.
    const floor = Math.min(value("iconSize"), Number(magnified.max));
    magnified.min = String(floor);
    if (Number(magnified.value) < floor) magnified.value = String(floor);
    magnified.disabled = !isOn("magnification");
    paintRange(magnified);
  };

  ranges.forEach((input) => {
    paintRange(input);
    input.addEventListener("input", () => {
      paintRange(input);
      syncMagnified();
      if (input.dataset.setting === "magnifiedSize") demoMagnify();
      else layoutDock();
    });
  });

  switches.forEach((button) => {
    button.addEventListener("click", () => {
      button.setAttribute("aria-checked", String(!isOn(button.dataset.setting ?? "")));
      syncMagnified();
      applyBars();
      if (button.dataset.setting === "magnification") demoMagnify();
      else layoutDock();
    });
  });

  /* ---- Panes and language ---------------------------------------------- */

  const title = scene.querySelector<HTMLElement>("[data-pane-title]");
  let pane = "dock";
  const showPane = (next: string) => {
    pane = next;
    scene.querySelectorAll<HTMLElement>("[data-view]").forEach((view) => {
      view.hidden = view.dataset.view !== next;
    });
    scene.querySelectorAll<HTMLButtonElement>("[data-pane]").forEach((button) => {
      if (button.dataset.pane === next) button.setAttribute("aria-current", "page");
      else button.removeAttribute("aria-current");
    });
    if (title) title.textContent = strings[next] ?? next;
  };
  scene.querySelectorAll<HTMLButtonElement>("[data-pane]").forEach((button) => {
    button.addEventListener("click", () => showPane(button.dataset.pane ?? "dock"));
  });

  const status = scene.querySelector<HTMLElement>("[data-update-status]");
  const setLanguage = (choice: string) => {
    const resolved = choice === "auto" ? (navigator.language.toLowerCase().startsWith("es") ? "es" : "en") : choice;
    strings = texts[resolved === "es" ? "es" : "en"];
    scene.lang = resolved === "es" ? "es" : "en";
    scene.querySelectorAll<HTMLElement>("[data-t]").forEach((node) => {
      const text = strings[node.dataset.t ?? ""];
      if (text) node.textContent = text;
    });
    scene.querySelectorAll<HTMLElement>("[data-label], [data-setting]").forEach((node) => {
      const key = node.dataset.label ?? node.dataset.setting ?? "";
      if (strings[key]) node.setAttribute("aria-label", strings[key]);
    });
    if (status) status.textContent = strings.updatesCurrent;
    scene.querySelectorAll<HTMLButtonElement>("[data-lang]").forEach((button) => {
      button.setAttribute("aria-checked", String(button.dataset.lang === choice));
    });
    showPane(pane);
  };
  scene.querySelectorAll<HTMLButtonElement>("[data-lang]").forEach((button) => {
    button.addEventListener("click", () => setLanguage(button.dataset.lang ?? "auto"));
  });

  const check = scene.querySelector<HTMLButtonElement>("[data-update]");
  check?.addEventListener("click", () => {
    if (!status) return;
    check.disabled = true;
    status.textContent = strings.updatesChecking;
    window.setTimeout(() => {
      status.textContent = strings.updatesCurrent;
      check.disabled = false;
    }, CHECK_MS);
  });

  syncMagnified();
  applyBars();
  layoutDock();
  window.addEventListener("resize", layoutDock, { passive: true });
}