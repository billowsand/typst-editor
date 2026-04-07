import { invoke } from "@tauri-apps/api/core";

export interface OutlineItem {
  title: string;
  level: number;
  file?: string | null;
  offset?: number | null;
  position?: {
    page: number;
    x: number;
    y: number;
  } | null;
}

let outlineContainer: HTMLDivElement | null = null;
let onSelectOutlineItem: ((item: OutlineItem) => void) | null = null;

export function initOutlinePanel(
  container: HTMLDivElement,
  onSelect: (item: OutlineItem) => void,
): void {
  outlineContainer = container;
  onSelectOutlineItem = onSelect;
  renderOutlineState("empty", []);
}

export async function refreshOutlinePanel(): Promise<void> {
  if (!outlineContainer) return;

  renderOutlineState("loading", []);

  try {
    const items = await invoke<OutlineItem[]>("get_document_outline");
    renderOutlineState(items.length > 0 ? "ready" : "empty", items);
  } catch {
    renderOutlineState("error", []);
  }
}

export function clearOutlinePanel(): void {
  renderOutlineState("empty", []);
}

function renderOutlineState(
  state: "loading" | "empty" | "error" | "ready",
  items: OutlineItem[],
): void {
  if (!outlineContainer) return;
  outlineContainer.replaceChildren();

  const header = document.createElement("div");
  header.className = "outline-header";
  header.textContent = "Main File Outline";
  outlineContainer.appendChild(header);

  if (state !== "ready") {
    const empty = document.createElement("div");
    empty.className = "outline-empty";
    empty.textContent =
      state === "loading"
        ? "Loading outline..."
        : state === "error"
          ? "Failed to load outline"
          : "Set a main file to show outline";
    outlineContainer.appendChild(empty);
    return;
  }

  const list = document.createElement("div");
  list.className = "outline-list";

  for (const item of items) {
    const row = document.createElement("button");
    row.type = "button";
    row.className = `outline-item level-${Math.min(item.level, 6)}`;
    row.style.paddingLeft = `${10 + Math.max(0, item.level - 1) * 14}px`;
    row.title = item.title;

    const level = document.createElement("span");
    level.className = "outline-level";
    level.textContent = `H${item.level}`;

    const title = document.createElement("span");
    title.className = "outline-title";
    title.textContent = item.title;

    row.append(level, title);
    row.addEventListener("click", () => onSelectOutlineItem?.(item));
    list.appendChild(row);
  }

  outlineContainer.appendChild(list);
}
