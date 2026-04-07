import { invoke } from "@tauri-apps/api/core";
import { closeDocument, closeDocumentsUnderPath, openFileFromTree } from "../editor/editorApi";

// ─── Types ────────────────────────────────────────────────────────

interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
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

  if (activeFilePath && normPath(entry.path) === normPath(activeFilePath)) {
    item.classList.add("active");
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
  try {
    const content = await invoke<string>("read_text_file", { path });
    await invoke("set_preview_entry", { filePath: path, content });
  } catch { /* ignore */ }
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

// ─── File Icon Colors ─────────────────────────────────────────────

const EXT_COLOR: Record<string, string> = {
  typ:  "#3b82f6",
  bib:  "#22c55e",
  toml: "#f97316",
  yaml: "#06b6d4",
  yml:  "#06b6d4",
  json: "#eab308",
  md:   "#8b5cf6",
  txt:  "#94a3b8",
  pdf:  "#ef4444",
  svg:  "#fb923c",
  img:  "#a855f7",
  rs:   "#dc2626",
  ts:   "#1d4ed8",
  js:   "#ca8a04",
  py:   "#16a34a",
  lua:  "#4338ca",
  sh:   "#334155",
  css:  "#0ea5e9",
  html: "#ea580c",
};

function buildFileIconSvg(ext: string): string {
  const c = EXT_COLOR[ext] ?? "#64748b";
  return `<svg width="13" height="15" viewBox="0 0 13 15" fill="none" xmlns="http://www.w3.org/2000/svg">
    <path d="M1.5 0h6L12 3.5V13.5A1.5 1.5 0 0 1 10.5 15h-9A1.5 1.5 0 0 1 0 13.5V1.5A1.5 1.5 0 0 1 1.5 0z" fill="${c}"/>
    <path d="M7.5 0L12 3.5H9A1.5 1.5 0 0 1 7.5 2V0z" fill="rgba(0,0,0,0.2)"/>
  </svg>`;
}

// ─── SVG Strings ──────────────────────────────────────────────────

const SVG_FOLDER = `<svg width="16" height="13" viewBox="0 0 16 13" fill="none">
  <path d="M0 3.5A1.5 1.5 0 0 1 1.5 2H6l1.5 1.5h7A1.5 1.5 0 0 1 16 5v6a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 0 11V3.5Z" fill="#c4933f"/>
</svg>`;

const SVG_FOLDER_OPEN = `<svg width="16" height="13" viewBox="0 0 16 13" fill="none">
  <path d="M0 3.5A1.5 1.5 0 0 1 1.5 2H6l1.5 1.5h7A1.5 1.5 0 0 1 16 5v1H0V3.5Z" fill="#c4933f" opacity="0.75"/>
  <path d="M0 6h16L14 12.5A1.5 1.5 0 0 1 12.5 13.5h-11A1.5 1.5 0 0 1 0 12V6Z" fill="#e6aa4e"/>
</svg>`;

const SVG_CHEVRON = `<svg width="8" height="8" viewBox="0 0 8 8" fill="none">
  <path d="M2 1.5 5 4 2 6.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>
</svg>`;

const SVG_NEW_FILE = `<svg width="15" height="15" viewBox="0 0 15 15" fill="none">
  <path d="M3 1.5h5.5L12 5v8a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1v-11a1 1 0 0 1 1-1Z" stroke="currentColor" stroke-width="1.2" fill="none"/>
  <path d="M8.5 1.5V5H12" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>
  <path d="M6 9.5h3M7.5 8v3" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>
</svg>`;

const SVG_NEW_FOLDER = `<svg width="15" height="15" viewBox="0 0 15 15" fill="none">
  <path d="M1 4.5A1 1 0 0 1 2 3.5h3.5L7 5h6.5A1 1 0 0 1 14.5 6v6a1 1 0 0 1-1 1H2a1 1 0 0 1-1-1V4.5Z" stroke="currentColor" stroke-width="1.2" fill="none"/>
  <path d="M7.75 8v3M6 9.5h3.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>
</svg>`;

const SVG_REFRESH = `<svg width="13" height="13" viewBox="0 0 13 13" fill="none">
  <path d="M11.5 2A6 6 0 1 0 12.5 7" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>
  <path d="M9.5 2H12V4.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/>
</svg>`;
