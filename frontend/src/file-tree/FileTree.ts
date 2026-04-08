import { invoke } from "@tauri-apps/api/core";
import { closeDocument, closeDocumentsUnderPath, openFileFromTree, setPreviewEntryAtPath } from "../editor/editorApi";

// ─── Types ────────────────────────────────────────────────────────

interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  is_preview_entry?: boolean;
}

interface TreeNode {
  path: string;
  name: string;
  isDir: boolean;
  depth: number;
  el: HTMLDivElement;
  childContainer: HTMLDivElement | null;
  expanded: boolean;
  parentPath: string;
}

// ─── State ────────────────────────────────────────────────────────

const nodeMap = new Map<string, TreeNode>();
let rootContainer: HTMLDivElement | null = null;
let currentRoot: string | null = null;
let activeFilePath: string | null = null;
let treeListEl: HTMLDivElement | null = null;
let previewEntryPath: string | null = null;

// ─── Public API ───────────────────────────────────────────────────

export async function initFileTree(container: HTMLDivElement): Promise<void> {
  rootContainer = container;
  renderEmpty("打开文件以显示项目树");

  try {
    const root = await invoke<string | null>("get_project_root");
    if (root) await loadTree(container, root, 0);
  } catch {
    // no project open yet
  }
}

export async function loadTree(
  container: HTMLDivElement,
  rootPath: string,
  _depth: number,
): Promise<void> {
  rootContainer = container;
  currentRoot = rootPath;
  nodeMap.clear();
  container.innerHTML = "";

  // ── Header ─────────────────────────────────────────────────────
  const header = document.createElement("div");
  header.className = "tree-header";

  const headerTitle = document.createElement("span");
  headerTitle.className = "tree-header-title";
  headerTitle.textContent = rootPath.split(/[\\/]/).pop() || rootPath;
  headerTitle.title = rootPath;

  const headerActions = document.createElement("div");
  headerActions.className = "tree-header-actions";

  const btnNewFile = makeHeaderBtn(SVG_NEW_FILE, "新建文件 (Alt+N)");
  btnNewFile.addEventListener("click", () => {
    if (treeListEl) startNewItem(currentRoot!, treeListEl, false);
  });

  const btnNewFolder = makeHeaderBtn(SVG_NEW_FOLDER, "新建文件夹");
  btnNewFolder.addEventListener("click", () => {
    if (treeListEl) startNewItem(currentRoot!, treeListEl, true);
  });

  const btnRefresh = makeHeaderBtn(SVG_REFRESH, "刷新");
  btnRefresh.addEventListener("click", () => loadTree(container, rootPath, 0));

  headerActions.append(btnNewFile, btnNewFolder, btnRefresh);
  header.append(headerTitle, headerActions);
  container.appendChild(header);

  // ── Tree body ──────────────────────────────────────────────────
  treeListEl = document.createElement("div");
  treeListEl.className = "tree-list";
  container.appendChild(treeListEl);

  await loadDirectory(treeListEl, rootPath, 0, rootPath);
}

export function setTreeActiveFile(path: string | null): void {
  activeFilePath = path;
  nodeMap.forEach((node) => node.el.classList.remove("active"));
  if (!path) return;
  const node = nodeMap.get(normPath(path));
  if (node) {
    node.el.classList.add("active");
    node.el.scrollIntoView({ block: "nearest" });
  }
}

export function setTreePreviewEntryPath(path: string | null): void {
  previewEntryPath = path ? normPath(path) : null;
  nodeMap.forEach((node) => {
    const isPreviewEntry = node.path === previewEntryPath;
    node.el.classList.toggle("preview-entry", isPreviewEntry);

    // Update main file badge
    let badge = node.el.querySelector<HTMLElement>(".preview-entry-badge");
    if (isPreviewEntry && !badge) {
      badge = document.createElement("span");
      badge.className = "preview-entry-badge";
      badge.innerHTML = SVG_STAR;
      badge.title = "主文件";
      node.el.appendChild(badge);
    } else if (!isPreviewEntry && badge) {
      badge.remove();
    }
  });
}

// ─── Directory Loading ────────────────────────────────────────────

async function loadDirectory(
  parent: HTMLElement,
  dirPath: string,
  depth: number,
  parentPath: string,
): Promise<void> {
  let entries: FileEntry[];
  try {
    entries = await invoke<FileEntry[]>("list_dir", { path: dirPath });
  } catch {
    const err = document.createElement("div");
    err.className = "tree-error";
    err.textContent = "无法读取目录";
    err.style.setProperty("--depth", String(depth));
    parent.appendChild(err);
    return;
  }

  for (const entry of entries) {
    appendTreeItem(parent, entry, depth, parentPath);
  }
}

function appendTreeItem(
  parent: HTMLElement,
  entry: FileEntry,
  depth: number,
  parentPath: string,
): TreeNode {
  const item = document.createElement("div");
  item.className = `tree-item ${entry.is_dir ? "dir" : "file"}`;
  item.style.setProperty("--depth", String(depth));
  item.setAttribute("tabindex", "0");
  item.title = entry.path;
  item.dataset.path = entry.path;

  // Arrow (dirs only)
  const arrow = document.createElement("span");
  arrow.className = "tree-arrow";
  if (entry.is_dir) arrow.innerHTML = SVG_CHEVRON;
  item.appendChild(arrow);

  // Icon
  const icon = document.createElement("span");
  icon.className = "tree-icon";
  if (entry.is_dir) {
    icon.innerHTML = SVG_FOLDER;
    icon.dataset.icon = "folder";
  } else {
    const ext = getExtension(entry.name);
    icon.innerHTML = buildFileIconSvg(ext);
    icon.dataset.ftype = ext;
  }
  item.appendChild(icon);

  // Label
  const label = document.createElement("span");
  label.className = "tree-label";
  label.textContent = entry.name;
  item.appendChild(label);

  const node: TreeNode = {
    path: normPath(entry.path),
    name: entry.name,
    isDir: entry.is_dir,
    depth,
    el: item,
    childContainer: null,
    expanded: false,
    parentPath: normPath(parentPath),
  };
  nodeMap.set(normPath(entry.path), node);

  const normalizedPath = normPath(entry.path);

  if (activeFilePath && normalizedPath === normPath(activeFilePath)) {
    item.classList.add("active");
  }

  // Check if this is the preview entry
  const isPreviewEntry = normalizedPath === previewEntryPath;
  if (isPreviewEntry) {
    item.classList.add("preview-entry");
    const badge = document.createElement("span");
    badge.className = "preview-entry-badge";
    badge.innerHTML = SVG_STAR;
    badge.title = "主文件";
    item.appendChild(badge);
  }

  if (entry.is_dir) {
    item.addEventListener("click", async (e) => {
      e.stopPropagation();
      await toggleDir(node, parent, entry.path, depth);
    });
  } else {
    item.addEventListener("click", (e) => {
      e.stopPropagation();
      nodeMap.forEach((n) => n.el.classList.remove("active"));
      item.classList.add("active");
      void openFileFromTree(entry.path);
    });
  }

  item.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    e.stopPropagation();
    showContextMenu(e.clientX, e.clientY, node, parent);
  });

  item.addEventListener("keydown", (e) => {
    if (e.key === "F2") { e.preventDefault(); startRename(node); }
    if (e.key === "Delete") { e.preventDefault(); void confirmDelete(node); }
    if (e.key === "Enter" && !entry.is_dir) void openFileFromTree(entry.path);
    if (e.key === "Enter" && entry.is_dir) void toggleDir(node, parent, entry.path, depth);
  });

  parent.appendChild(item);
  return node;
}

async function toggleDir(
  node: TreeNode,
  parent: HTMLElement,
  dirPath: string,
  depth: number,
): Promise<void> {
  const iconEl = node.el.querySelector<HTMLSpanElement>("[data-icon]");

  if (node.expanded && node.childContainer) {
    node.childContainer.remove();
    node.childContainer = null;
    node.expanded = false;
    node.el.classList.remove("expanded");
    if (iconEl) { iconEl.innerHTML = SVG_FOLDER; iconEl.dataset.icon = "folder"; }
    return;
  }

  node.expanded = true;
  node.el.classList.add("expanded");
  if (iconEl) { iconEl.innerHTML = SVG_FOLDER_OPEN; iconEl.dataset.icon = "folder-open"; }

  const childContainer = document.createElement("div");
  childContainer.className = "tree-children";
  node.childContainer = childContainer;
  node.el.after(childContainer);
  await loadDirectory(childContainer, dirPath, depth + 1, dirPath);
}

// ─── File Operations ──────────────────────────────────────────────

function startNewItem(dirPath: string, listEl: HTMLElement, isDir: boolean): void {
  dismissInlineCreate();

  const row = document.createElement("div");
  row.className = "tree-item tree-inline-create";
  row.style.setProperty("--depth", "0");

  const arrow = document.createElement("span");
  arrow.className = "tree-arrow";
  row.appendChild(arrow);

  const icon = document.createElement("span");
  icon.className = "tree-icon";
  icon.innerHTML = isDir ? SVG_FOLDER : buildFileIconSvg("typ");
  row.appendChild(icon);

  const input = document.createElement("input");
  input.className = "tree-inline-input";
  input.type = "text";
  input.placeholder = isDir ? "文件夹名称…" : "文件名.typ";
  row.appendChild(input);

  listEl.insertBefore(row, listEl.firstChild);
  input.focus();

  const commit = async (): Promise<void> => {
    const name = input.value.trim();
    row.remove();
    if (!name) return;
    const newPath = joinPath(dirPath, name);
    try {
      await invoke(isDir ? "create_directory" : "create_file", { path: newPath });
      await loadTree(rootContainer!, currentRoot!, 0);
      if (!isDir) void openFileFromTree(newPath);
    } catch (e) {
      showTreeError(`创建失败: ${e}`);
    }
  };

  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") { e.preventDefault(); void commit(); }
    if (e.key === "Escape") { row.remove(); }
  });
  input.addEventListener("blur", () => {
    setTimeout(() => { if (row.isConnected) row.remove(); }, 200);
  });
}

function dismissInlineCreate(): void {
  document.querySelectorAll(".tree-inline-create").forEach((el) => el.remove());
}

function startRename(node: TreeNode): void {
  const labelEl = node.el.querySelector<HTMLSpanElement>(".tree-label");
  if (!labelEl) return;

  const input = document.createElement("input");
  input.className = "tree-inline-input";
  input.type = "text";
  input.value = node.name;
  labelEl.replaceWith(input);
  input.focus();
  input.select();

  const restore = (): void => { input.replaceWith(labelEl); };

  const commit = async (): Promise<void> => {
    const newName = input.value.trim();
    if (!newName || newName === node.name) { restore(); return; }
    const sep = node.path.includes("\\") ? "\\" : "/";
    const dir = node.path.substring(0, node.path.lastIndexOf(sep));
    const newPath = joinPath(dir, newName);
    try {
      // If the renamed file/directory is currently open, close it (and all children for directories)
      const closed = node.isDir
        ? await closeDocumentsUnderPath(node.path)
        : await closeDocument(node.path);
      if (!closed) {
        // User cancelled closing - abort rename
        restore();
        return;
      }
      await invoke("rename_path", { oldPath: node.path, newPath });
      await loadTree(rootContainer!, currentRoot!, 0);
    } catch (e) {
      restore();
      showTreeError(`重命名失败: ${e}`);
    }
  };

  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") { e.preventDefault(); void commit(); }
    if (e.key === "Escape") { restore(); }
  });
  input.addEventListener("blur", () => {
    setTimeout(() => { if (input.isConnected) void commit(); }, 150);
  });
}

async function confirmDelete(node: TreeNode): Promise<void> {
  const kind = node.isDir ? "文件夹" : "文件";
  if (!window.confirm(`确定要删除${kind} "${node.name}" 吗？\n此操作不可撤销。`)) return;
  try {
    // If the deleted file/directory is currently open, close it (and all children for directories)
    const closed = node.isDir
      ? await closeDocumentsUnderPath(node.path)
      : await closeDocument(node.path);
    if (!closed) {
      // User cancelled closing - abort delete
      return;
    }
    await invoke("delete_path", { path: node.path });
    await loadTree(rootContainer!, currentRoot!, 0);
  } catch (e) {
    showTreeError(`删除失败: ${e}`);
  }
}

// ─── Context Menu ─────────────────────────────────────────────────

let ctxMenu: HTMLDivElement | null = null;

function showContextMenu(x: number, y: number, node: TreeNode, parent: HTMLElement): void {
  dismissContextMenu();
  ctxMenu = document.createElement("div");
  ctxMenu.className = "tree-context-menu";

  const addItem = (label: string, key: string, action: () => void, danger = false): void => {
    const el = document.createElement("button");
    el.className = `ctx-item${danger ? " ctx-danger" : ""}`;
    const labelSpan = document.createElement("span");
    labelSpan.className = "ctx-label";
    labelSpan.textContent = label;
    const keySpan = document.createElement("span");
    keySpan.className = "ctx-key";
    keySpan.textContent = key;
    el.append(labelSpan, keySpan);
    el.addEventListener("mousedown", (e) => {
      e.preventDefault();
      dismissContextMenu();
      action();
    });
    ctxMenu!.appendChild(el);
  };

  const addSep = (): void => {
    const el = document.createElement("div");
    el.className = "ctx-sep";
    ctxMenu!.appendChild(el);
  };

  if (node.isDir) {
    addItem("新建文件", "Alt+N", () => {
      const childList = node.childContainer ?? treeListEl!;
      if (!node.expanded) void toggleDir(node, parent, node.path, node.depth);
      setTimeout(() => startNewItem(node.path, node.childContainer ?? treeListEl!, false), 200);
    });
    addItem("新建文件夹", "", () => {
      if (!node.expanded) void toggleDir(node, parent, node.path, node.depth);
      setTimeout(() => startNewItem(node.path, node.childContainer ?? treeListEl!, true), 200);
    });
  } else {
    addItem("在编辑器中打开", "Enter", () => void openFileFromTree(node.path));
    if (node.name.endsWith(".typ")) {
      addItem("设为预览入口", "Ctrl+Shift+↵", () => void setAsPreviewEntry(node.path));
    }
  }

  addSep();
  addItem("重命名", "F2", () => startRename(node));
  addItem("删除", "Del", () => void confirmDelete(node), true);
  addSep();
  addItem("复制路径", "", () => {
    navigator.clipboard.writeText(node.path).catch(() => {});
  });
  addItem("在文件管理器中显示", "", () => {
    void invoke("reveal_in_explorer", { path: node.path });
  });

  document.body.appendChild(ctxMenu);

  // Viewport-aware placement
  const r = ctxMenu.getBoundingClientRect();
  ctxMenu.style.left = `${Math.min(x, window.innerWidth  - r.width  - 6)}px`;
  ctxMenu.style.top  = `${Math.min(y, window.innerHeight - r.height - 6)}px`;

  // Auto-dismiss
  setTimeout(() => {
    document.addEventListener("mousedown", dismissContextMenu, { once: true });
  }, 0);
  document.addEventListener("keydown", ctxKeyHandler);
}

function ctxKeyHandler(e: KeyboardEvent): void {
  if (e.key === "Escape") dismissContextMenu();
}

function dismissContextMenu(): void {
  ctxMenu?.remove();
  ctxMenu = null;
  document.removeEventListener("keydown", ctxKeyHandler);
}

async function setAsPreviewEntry(path: string): Promise<void> {
  await setPreviewEntryAtPath(path);
}

// ─── Helpers ─────────────────────────────────────────────────────

function normPath(p: string): string {
  return p.replace(/\\/g, "/");
}

function joinPath(...parts: string[]): string {
  return parts.map((p) => p.replace(/[\\/]+$/, "")).join("/");
}

function getExtension(name: string): string {
  const dot = name.lastIndexOf(".");
  if (dot < 0 || dot >= name.length - 1) return "";
  const ext = name.slice(dot + 1).toLowerCase();
  if (["png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "tiff"].includes(ext)) return "img";
  return ext;
}

function renderEmpty(msg: string): void {
  if (!rootContainer) return;
  rootContainer.innerHTML = `<div class="tree-empty">${msg}</div>`;
}

function showTreeError(msg: string): void {
  const el = document.getElementById("compile-status");
  if (!el) return;
  const prev = el.className;
  const prevText = el.textContent ?? "";
  el.className = "status-chip status error";
  el.textContent = msg;
  setTimeout(() => { el.className = prev; el.textContent = prevText; }, 3500);
}

function makeHeaderBtn(svg: string, title: string): HTMLButtonElement {
  const btn = document.createElement("button");
  btn.className = "tree-header-btn";
  btn.title = title;
  btn.innerHTML = svg;
  return btn;
}

// ─── File Icon Colors (Lucide-inspired palette) ────────────────────────

const EXT_COLOR: Record<string, string> = {
  typ:  "#0ea5e9",      // Typst - Sky blue
  bib:  "#22c55e",      // BibTeX - Green
  toml: "#f97316",      // TOML - Orange
  yaml: "#06b6d4",      // YAML - Cyan
  yml:  "#06b6d4",      // YAML - Cyan
  json: "#eab308",      // JSON - Yellow
  md:   "#8b5cf6",      // Markdown - Purple
  txt:  "#64748b",      // Text - Slate
  pdf:  "#ef4444",      // PDF - Red
  svg:  "#f59e0b",      // SVG - Amber
  png:  "#ec4899",      // PNG - Pink
  jpg:  "#ec4899",      // JPG - Pink
  jpeg: "#ec4899",      // JPEG - Pink
  gif:  "#ec4899",      // GIF - Pink
  webp: "#ec4899",      // WebP - Pink
  avif: "#ec4899",      // AVIF - Pink
  bmp:  "#ec4899",      // BMP - Pink
  tiff: "#ec4899",      // TIFF - Pink
  img:  "#a855f7",      // Generic image - Purple
  rs:   "#f97316",      // Rust - Orange
  ts:   "#3b82f6",      // TypeScript - Blue
  js:   "#eab308",      // JavaScript - Yellow
  py:   "#22c55e",      // Python - Green
  lua:  "#06b6d4",      // Lua - Cyan
  sh:   "#64748b",      // Shell - Slate
  css:  "#8b5cf6",      // CSS - Purple
  html: "#f97316",      // HTML - Orange
  scss: "#f472b6",      // SCSS - Pink
  less: "#8b5cf6",      // Less - Purple
  xml:  "#06b6d4",      // XML - Cyan
  csv:  "#22c55e",      // CSV - Green
};

function buildFileIconSvg(ext: string): string {
  const c = EXT_COLOR[ext] ?? "#94a3b8";
  return `<svg width="14" height="16" viewBox="0 0 14 16" fill="none" xmlns="http://www.w3.org/2000/svg">
    <path d="M2 1h7l3 3v9a1 1 0 0 1-1 1H2a1 1 0 0 1-1-1V2a1 1 0 0 1 1-1z" fill="${c}" fill-opacity="0.15" stroke="${c}" stroke-width="1"/>
    <path d="M9 1l3 3H9V1z" fill="${c}" fill-opacity="0.3"/>
  </svg>`;
}

// ─── SVG Strings (Lucide-inspired) ──────────────────────────────────────

const SVG_FOLDER = `<svg width="16" height="14" viewBox="0 0 16 14" fill="none" xmlns="http://www.w3.org/2000/svg">
  <path d="M1 3.5C1 2.67 1.67 2 2.5 2h5l1.5 1.5h4.5c.83 0 1.5.67 1.5 1.5v6c0 .83-.67 1.5-1.5 1.5h-10c-.83 0-1.5-.67-1.5-1.5v-7.5Z" fill="currentColor" fill-opacity="0.25" stroke="currentColor" stroke-width="1"/>
</svg>`;

const SVG_FOLDER_OPEN = `<svg width="16" height="14" viewBox="0 0 16 14" fill="none" xmlns="http://www.w3.org/2000/svg">
  <path d="M1 3.5C1 2.67 1.67 2 2.5 2h5l1.5 1.5h4.5c.83 0 1.5.67 1.5 1.5v1.5H1v-2Z" fill="currentColor" fill-opacity="0.15" stroke="currentColor" stroke-width="1"/>
  <path d="M1 7h13l-1.5 5H2.5L1 7Z" fill="currentColor" fill-opacity="0.25" stroke="currentColor" stroke-width="1"/>
</svg>`;

const SVG_CHEVRON = `<svg width="10" height="10" viewBox="0 0 10 10" fill="none" xmlns="http://www.w3.org/2000/svg">
  <path d="M3 2l4 3-4 3" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/>
</svg>`;

const SVG_STAR = `<svg width="12" height="12" viewBox="0 0 12 12" fill="none" xmlns="http://www.w3.org/2000/svg">
  <path d="M6 1l1.5 3h3.3l-2.6 2 1 3.3L6 8l-3.2 1.3 1-3.3-2.6-2h3.3L6 1Z" fill="currentColor"/>
</svg>`;

const SVG_NEW_FILE = `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" xmlns="http://www.w3.org/2000/svg">
  <path d="M3 1h6l2 2h3c.55 0 1 .45 1 1v8c0 .55-.45 1-1 1H3c-.55 0-1-.45-1-1V2c0-.55.45-1 1-1z" stroke="currentColor" stroke-width="1.2" fill="none"/>
  <path d="M9 1l2 2H9V1z" stroke="currentColor" stroke-width="1.2"/>
  <path d="M5 7h3M6.5 5.5v3" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>
</svg>`;

const SVG_NEW_FOLDER = `<svg width="14" height="14" viewBox="0 0 14 14" fill="none" xmlns="http://www.w3.org/2000/svg">
  <path d="M1 3.5C1 2.67 1.67 2 2.5 2h5l1.5 1.5h4.5c.83 0 1.5.67 1.5 1.5v6c0 .83-.67 1.5-1.5 1.5h-10c-.83 0-1.5-.67-1.5-1.5v-7.5Z" stroke="currentColor" stroke-width="1.2" fill="none"/>
  <path d="M7 6.5h3M8.5 5v3" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>
</svg>`;

const SVG_REFRESH = `<svg width="12" height="12" viewBox="0 0 12 12" fill="none" xmlns="http://www.w3.org/2000/svg">
  <path d="M10 1.5a5 5 0 1 0 .5 5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>
  <path d="M8.5 1.5h2.5V4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"/>
</svg>`;
