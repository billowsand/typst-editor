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
let currentItems: OutlineItem[] = [];
let currentState: "loading" | "empty" | "error" | "ready" = "empty";
let activeFile: string | null = null;
let activeOffset: number | null = null;

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

  if (currentItems.length === 0) {
    renderOutlineState("loading", []);
  }

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

export function setOutlineItems(items: OutlineItem[]): void {
  renderOutlineState(items.length > 0 ? "ready" : "empty", items);
}

export function updateOutlineActiveItem(file: string | null, offset: number | null): void {
  activeFile = file;
  activeOffset = offset;

  if (!outlineContainer || currentState !== "ready") return;
  syncActiveOutlineDom();
}

function renderOutlineState(
  state: "loading" | "empty" | "error" | "ready",
  items: OutlineItem[],
): void {
  if (!outlineContainer) return;
  if (state === currentState && state === "ready" && isSameOutline(items, currentItems)) {
    currentItems = items;
    syncActiveOutlineDom();
    return;
  }
  currentState = state;
  currentItems = items;
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

    row.dataset.file = item.file ?? "";
    row.dataset.offset = item.offset != null ? String(item.offset) : "";
    row.append(level, title);
    row.addEventListener("click", () => onSelectOutlineItem?.(item));
    list.appendChild(row);
  }

  outlineContainer.appendChild(list);
  syncActiveOutlineDom();
}

function syncActiveOutlineDom(): void {
  if (!outlineContainer) return;

  const activeIndex = findActiveOutlineIndex(currentItems, activeFile, activeOffset);
  const rows = outlineContainer.querySelectorAll<HTMLButtonElement>(".outline-item");

  rows.forEach((row, index) => {
    const isActive = index === activeIndex;
    row.classList.toggle("active", isActive);
    if (isActive) {
      row.setAttribute("aria-current", "true");
    } else {
      row.removeAttribute("aria-current");
    }
  });
}

function findActiveOutlineIndex(
  items: OutlineItem[],
  file: string | null,
  offset: number | null,
): number {
  if (!file || offset == null) return -1;

  let activeIndex = -1;
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (item.file !== file || item.offset == null) continue;
    if (item.offset <= offset) activeIndex = i;
  }
  return activeIndex;
}

function isSameOutline(a: OutlineItem[], b: OutlineItem[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((item, index) => {
    const other = b[index];
    return item.title === other.title
      && item.level === other.level
      && item.file === other.file
      && item.offset === other.offset
      && item.position?.page === other.position?.page
      && item.position?.x === other.position?.x
      && item.position?.y === other.position?.y;
  });
}
