import {
  initEditor, getEditor, setZoom, getZoom,
  setMonacoTheme, setEditorFont,
  pickFilePath, openFileAtPath, hasUnsavedDocuments, jumpToFileOffset, revealSourceOffsetInPreview,
} from "./editor/MonacoEditor";
import { initPreview } from "./preview/PreviewPanel";
import { initFileTree, loadTree } from "./file-tree/FileTree";
import { initOutlinePanel, type OutlineItem } from "./outline/OutlinePanel";
import { initWorkspaceShell } from "./workspace/WorkspaceShell";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

// ─── Types ────────────────────────────────────────────────────────
type Theme = "light" | "dark";

// ─── Font catalogue ───────────────────────────────────────────────
const FONT_FAMILIES: Record<string, string> = {
  system:    "ui-monospace, 'SF Mono', 'Cascadia Code', Menlo, Consolas, monospace",
  fira:      "'Fira Code', monospace",
  jetbrains: "'JetBrains Mono', monospace",
  cascadia:  "'Cascadia Code', monospace",
  source:    "'Source Code Pro', monospace",
  ibm:       "'IBM Plex Mono', monospace",
  roboto:    "'Roboto Mono', monospace",
  menlo:     "Menlo, monospace",
  monaco:    "'Monaco', monospace",
  consolas:  "Consolas, monospace",
  courier:   "'Courier New', Courier, monospace",
  custom:    "",
};

// ─── Preference helpers ───────────────────────────────────────────
function pref(key: string, fallback: string): string {
  try { return localStorage.getItem(`typst-${key}`) ?? fallback; }
  catch { return fallback; }
}
function setPref(key: string, val: string): void {
  try { localStorage.setItem(`typst-${key}`, val); } catch { /* ignore */ }
}

function savedTheme(): Theme    { return pref("theme", "dark") as Theme; }
function savedFontKey(): string { return pref("font-key", "system"); }
function savedFontSize(): number { return parseInt(pref("font-size", "14"), 10); }
function savedCustomFont(): string { return pref("font-custom", ""); }

// ─── Theme ────────────────────────────────────────────────────────
function applyTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
  setPref("theme", theme);
  const icon = document.getElementById("theme-icon");
  if (icon) icon.textContent = theme === "dark" ? "☀" : "🌙";
  setMonacoTheme(theme);
}

// ─── Font ─────────────────────────────────────────────────────────
function applyFont(key: string, size: number, customName?: string): void {
  let css: string;
  if (key === "custom") {
    const name = customName ?? savedCustomFont();
    css = name ? `'${name}', monospace` : FONT_FAMILIES.system;
  } else {
    css = FONT_FAMILIES[key] ?? FONT_FAMILIES.system;
  }
  setPref("font-key", key);
  setPref("font-size", String(size));
  if (key === "custom" && customName) setPref("font-custom", customName);
  setPref("font-css", css);
  setEditorFont(css, size);
}

// ─── App Construction ─────────────────────────────────────────────
export function createApp(): {
  editorContainer: HTMLDivElement;
  previewContainer: HTMLDivElement;
  fileTreeContainer: HTMLDivElement;
  outlineContainer: HTMLDivElement;
} {
  const app = document.getElementById("app")!;
  document.documentElement.dataset.theme = savedTheme();

  const themeIcon = savedTheme() === "dark" ? "☀" : "🌙";
  const fontSize  = savedFontSize();
  const fontKey   = savedFontKey();

  // ── Toolbar ──────────────────────────────────────────────────────
  const toolbar = document.createElement("div");
  toolbar.className = "toolbar";

  // Windows-style SVG icons (Segoe-flavoured)
  const ICON_MIN  = `<svg viewBox="0 0 10 10"><rect x="0" y="4.5" width="10" height="1"/></svg>`;
  const ICON_MAX  = `<svg viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" stroke-width="1"/></svg>`;
  const ICON_REST = `<svg viewBox="0 0 10 10"><rect x="2" y="0" width="8" height="8" fill="none" stroke="currentColor" stroke-width="1"/><rect x="0" y="2" width="8" height="8" fill="var(--c-toolbar)" stroke="currentColor" stroke-width="1"/></svg>`;
  const ICON_CLS  = `<svg viewBox="0 0 10 10"><line x1="0" y1="0" x2="10" y2="10" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/><line x1="10" y1="0" x2="0" y2="10" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg>`;

  toolbar.innerHTML = `
    <!-- Draggable title area (left, before buttons) -->
    <div class="title-drag">
      <span class="app-title">Typst</span>
    </div>

    <div class="toolbar-separator"></div>

    <!-- File & edit buttons -->
    <button id="btn-files"  title="切换文件树 (Ctrl+B)">Files</button>
    <button id="btn-open"   title="打开 (Ctrl+O)">Open</button>
    <button id="btn-save"   title="保存 (Ctrl+S)">Save</button>
    <button id="btn-export" title="导出 PDF (Ctrl+Shift+E)">Export PDF</button>

    <div class="toolbar-separator"></div>

    <!-- Zoom -->
    <button id="btn-zoom-out" class="btn-icon" title="缩小">−</button>
    <span   id="zoom-level"   class="zoom-label">100%</span>
    <button id="btn-zoom-in"  class="btn-icon" title="放大">+</button>
    <button id="btn-zoom-fit" title="适合宽度">Fit</button>

    <div class="toolbar-separator"></div>

    <!-- Theme toggle -->
    <button id="btn-theme" class="btn-icon" title="切换主题">
      <span id="theme-icon">${themeIcon}</span>
    </button>

    <!-- Settings -->
    <div class="settings-container">
      <button id="btn-settings" class="btn-icon" title="设置">⚙</button>

      <div class="settings-dropdown" id="settings-dropdown" hidden>

        <div class="settings-group">
          <div class="settings-section-title">编辑器字体</div>
          <select id="font-family-select">
            <option value="system">System Default</option>
            <option value="fira">Fira Code</option>
            <option value="jetbrains">JetBrains Mono</option>
            <option value="cascadia">Cascadia Code</option>
            <option value="source">Source Code Pro</option>
            <option value="ibm">IBM Plex Mono</option>
            <option value="roboto">Roboto Mono</option>
            <option value="menlo">Menlo</option>
            <option value="monaco">Monaco</option>
            <option value="consolas">Consolas</option>
            <option value="courier">Courier New</option>
            <option value="custom">自定义…</option>
          </select>
          <input
            type="text"
            id="font-custom-input"
            placeholder="字体名称，例如 Hack"
            style="display:none; margin-top:4px"
          />
        </div>

        <div class="settings-divider"></div>

        <div class="settings-group">
          <div class="settings-row">
            <span class="settings-row-label">字体大小</span>
            <span class="settings-value-label" id="font-size-display">${fontSize}px</span>
          </div>
          <input
            type="range"
            id="font-size-slider"
            min="10" max="26" step="1"
            value="${fontSize}"
          />
        </div>

      </div>
    </div>

    <!-- Flex spacer: takes remaining space and acts as drag region -->
    <div class="toolbar-spacer"></div>

    <!-- Windows 11-style window controls (right edge, flush) -->
    <div class="win-controls">
      <button class="wc-btn wc-minimize" id="wc-minimize" title="最小化">
        ${ICON_MIN}
      </button>
      <button class="wc-btn wc-maximize" id="wc-maximize" title="最大化">
        ${ICON_MAX}
      </button>
      <button class="wc-btn wc-close"    id="wc-close"    title="关闭">
        ${ICON_CLS}
      </button>
    </div>
  `;
  app.appendChild(toolbar);

  // ── Split Container ───────────────────────────────────────────────
  const splitContainer = document.createElement("div");
  splitContainer.className = "split-container";

  const sidebarPane = document.createElement("div");
  sidebarPane.className = "sidebar-pane";
  sidebarPane.id = "file-tree-pane";

  const fileTreePane = document.createElement("div");
  fileTreePane.className = "file-tree-pane";
  fileTreePane.id = "project-tree-pane";

  const sidebarDivider = document.createElement("div");
  sidebarDivider.className = "sidebar-divider";

  const outlinePane = document.createElement("div");
  outlinePane.className = "outline-pane";
  outlinePane.id = "outline-pane";

  sidebarPane.append(fileTreePane, sidebarDivider, outlinePane);

  const treeGutter = document.createElement("div");
  treeGutter.className = "gutter tree-gutter";

  const editorPane = document.createElement("div");
  editorPane.className = "editor-pane";
  editorPane.id = "editor-pane";

  const gutter = document.createElement("div");
  gutter.className = "gutter";

  const previewPane = document.createElement("div");
  previewPane.className = "preview-pane";
  previewPane.id = "preview-pane";

  splitContainer.append(sidebarPane, treeGutter, editorPane, gutter, previewPane);
  app.appendChild(splitContainer);

  const statusBar = document.createElement("div");
  statusBar.className = "status-bar";
  statusBar.innerHTML = `
    <div class="status-group">
      <span id="status-file" class="status-chip wide">未打开文件</span>
      <span id="status-cursor" class="status-chip">Ln 1, Col 1</span>
    </div>
    <div class="status-group status-group-right">
      <span id="status-pages" class="status-chip">0 page(s)</span>
      <span id="status-zoom-bar" class="status-chip">100%</span>
      <span id="compile-status" class="status-chip status">Ready</span>
    </div>
  `;
  app.appendChild(statusBar);

  setupGutterDrag(treeGutter, sidebarPane, true, null);
  setupGutterDrag(gutter, editorPane, false, previewPane);

  // Restore font UI (needs DOM)
  setTimeout(() => restoreFontUI(fontKey), 0);

  return {
    editorContainer: editorPane,
    previewContainer: previewPane,
    fileTreeContainer: fileTreePane,
    outlineContainer: outlinePane,
  };
}

export async function initApp(): Promise<void> {
  const { editorContainer, previewContainer, fileTreeContainer, outlineContainer } = createApp();
  initPreview(previewContainer);
  initOutlinePanel(outlineContainer, handleOutlineSelect);
  initWorkspaceShell({
    splitContainer: document.querySelector(".split-container") as HTMLDivElement,
    sidebarPane: document.getElementById("file-tree-pane") as HTMLDivElement,
    treeGutter: document.querySelector(".tree-gutter") as HTMLDivElement,
    editorPane: editorContainer,
    previewGutter: document.querySelectorAll(".gutter")[1] as HTMLDivElement,
    previewPane: previewContainer,
    onOpen: () => promptOpenFile(fileTreeContainer),
  });
  await initEditor(editorContainer);
  bindToolbarButtons(fileTreeContainer);
  setupPreviewZoom();
  setupSettingsPanel();
  setupWindowControls();          // ← frameless window controls
  await initFileTree(fileTreeContainer);

  // Apply saved font
  applyFont(savedFontKey(), savedFontSize(), savedCustomFont());
}

async function handleOutlineSelect(item: OutlineItem): Promise<void> {
  // Outline navigates the editor only — preview follows the editor cursor via its own sync path
  if (item.file && item.offset != null) {
    await jumpToFileOffset(item.file, item.offset, false);
  }
}

// ─── Frameless Window Controls ────────────────────────────────────

// SVG icons for maximize vs restore
const SVG_MAX  = `<svg viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" stroke-width="1"/></svg>`;
const SVG_REST = `<svg viewBox="0 0 10 10"><rect x="2" y="0" width="8" height="8" fill="none" stroke="currentColor" stroke-width="1"/><rect x="0" y="2" width="8" height="8" fill="var(--c-toolbar)" stroke="currentColor" stroke-width="1"/></svg>`;

function setupWindowControls(): void {
  const win = getCurrentWindow();

  const toolbar = document.querySelector(".toolbar") as HTMLDivElement | null;
  const closeBtn    = document.getElementById("wc-close")    as HTMLButtonElement | null;
  const minimizeBtn = document.getElementById("wc-minimize") as HTMLButtonElement | null;
  const maximizeBtn = document.getElementById("wc-maximize") as HTMLButtonElement | null;

  closeBtn?.addEventListener("click", (e) => {
    e.stopPropagation();
    if (hasUnsavedDocuments() && !window.confirm("仍有未保存文件，确定关闭应用吗？")) {
      return;
    }
    win.close();
  });

  minimizeBtn?.addEventListener("click", (e) => {
    e.stopPropagation();
    win.minimize();
  });

  maximizeBtn?.addEventListener("click", async (e) => {
    e.stopPropagation();
    await win.toggleMaximize();
    await syncMaximizeState(win, maximizeBtn);
  });

  toolbar?.addEventListener("mousedown", async (e) => {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement | null;
    if (!target || isToolbarInteractiveTarget(target)) return;
    if (e.detail > 1) return;
    e.preventDefault();
    try {
      await win.startDragging();
    } catch { /* non-Tauri preview context */ }
  });

  toolbar?.addEventListener("dblclick", async (e) => {
    const target = e.target as HTMLElement | null;
    if (!target || isToolbarInteractiveTarget(target)) return;
    await win.toggleMaximize();
    await syncMaximizeState(win, maximizeBtn);
  });

  // Re-sync icon when OS snaps or resizes the window externally
  win.onResized(async () => {
    await syncMaximizeState(win, maximizeBtn);
  });

  // Dim controls when window loses focus (Windows standard behaviour)
  win.onFocusChanged(({ payload: focused }) => {
    document.querySelector<HTMLElement>(".toolbar")
      ?.classList.toggle("window-inactive", !focused);
  });

  // Initial icon sync
  syncMaximizeState(win, maximizeBtn);
}

function isToolbarInteractiveTarget(target: HTMLElement): boolean {
  return Boolean(target.closest(
    "button, input, select, textarea, a, .settings-container, .settings-dropdown"
  ));
}

async function syncMaximizeState(
  win: ReturnType<typeof getCurrentWindow>,
  btn: HTMLButtonElement | null
): Promise<void> {
  if (!btn) return;
  try {
    const maximized = await win.isMaximized();
    btn.innerHTML   = maximized ? SVG_REST : SVG_MAX;
    btn.title       = maximized ? "还原" : "最大化";
  } catch { /* non-Tauri preview context */ }
}

// ─── Font UI Restore ──────────────────────────────────────────────
function restoreFontUI(key: string): void {
  const sel = document.getElementById("font-family-select") as HTMLSelectElement | null;
  if (sel) sel.value = key;
  if (key === "custom") {
    const inp = document.getElementById("font-custom-input") as HTMLInputElement | null;
    if (inp) {
      inp.style.display = "block";
      inp.value = savedCustomFont();
    }
  }
}

// ─── Settings Panel ───────────────────────────────────────────────
function setupSettingsPanel(): void {
  const btn      = document.getElementById("btn-settings");
  const dropdown = document.getElementById("settings-dropdown");
  if (!btn || !dropdown) return;

  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    const hidden = dropdown.hasAttribute("hidden");
    if (hidden) {
      dropdown.removeAttribute("hidden");
      btn.classList.add("panel-open");
    } else {
      dropdown.setAttribute("hidden", "");
      btn.classList.remove("panel-open");
    }
  });

  document.addEventListener("click", (e) => {
    if (!dropdown.hasAttribute("hidden") &&
        !dropdown.contains(e.target as Node) &&
        e.target !== btn) {
      dropdown.setAttribute("hidden", "");
      btn.classList.remove("panel-open");
    }
  });

  const fontSel   = document.getElementById("font-family-select") as HTMLSelectElement | null;
  const custInput = document.getElementById("font-custom-input")  as HTMLInputElement | null;
  const slider    = document.getElementById("font-size-slider")   as HTMLInputElement | null;
  const sizeLabel = document.getElementById("font-size-display");

  fontSel?.addEventListener("change", () => {
    const key = fontSel.value;
    if (key === "custom") {
      if (custInput) custInput.style.display = "block";
    } else {
      if (custInput) custInput.style.display = "none";
      applyFont(key, slider ? parseInt(slider.value, 10) : savedFontSize());
    }
  });

  custInput?.addEventListener("input", () => {
    const name = custInput.value.trim();
    if (name) applyFont("custom", slider ? parseInt(slider.value, 10) : savedFontSize(), name);
  });

  slider?.addEventListener("input", () => {
    const size = parseInt(slider.value, 10);
    if (sizeLabel) sizeLabel.textContent = `${size}px`;
    const key  = fontSel?.value ?? savedFontKey();
    const name = custInput?.value.trim();
    applyFont(key, size, name);
  });
}

// ─── Toolbar Buttons ──────────────────────────────────────────────
function bindToolbarButtons(fileTreeContainer: HTMLDivElement): void {
  document.getElementById("btn-theme")?.addEventListener("click", () => {
    const cur = (document.documentElement.dataset.theme ?? "dark") as Theme;
    applyTheme(cur === "dark" ? "light" : "dark");
  });

  document.getElementById("btn-files")?.addEventListener("click", () => {
    const pane   = document.getElementById("file-tree-pane") as HTMLElement | null;
    const gutter = document.querySelector<HTMLElement>(".tree-gutter");
    if (!pane || !gutter) return;
    const hidden = pane.style.display === "none";
    pane.style.display   = hidden ? "" : "none";
    gutter.style.display = hidden ? "" : "none";
  });

  document.getElementById("btn-open")?.addEventListener("click", async () => {
    await promptOpenFile(fileTreeContainer);
  });

  document.getElementById("btn-save")?.addEventListener("click", () => {
    getEditor()?.getAction("save-file")?.run();
  });

  document.getElementById("btn-export")?.addEventListener("click", () => {
    getEditor()?.getAction("export-pdf")?.run();
  });

  document.getElementById("btn-zoom-in")?.addEventListener("click", () => {
    setZoom(Math.min(getZoom() + 0.25, 3.0));
    updateZoomLabel();
  });

  document.getElementById("btn-zoom-out")?.addEventListener("click", () => {
    setZoom(Math.max(getZoom() - 0.25, 0.25));
    updateZoomLabel();
  });

  document.getElementById("btn-zoom-fit")?.addEventListener("click", () => {
    setZoom(1.0);
    updateZoomLabel();
  });
}

async function promptOpenFile(fileTreeContainer: HTMLDivElement): Promise<void> {
  const path = await pickFilePath();
  if (!path) return;

  // Fire-and-forget: load tree independently, don't block file opening
  void invoke<string | null>("resolve_project_root", { filePath: path })
    .then((root) => root ? loadTree(fileTreeContainer, root, 0) : undefined)
    .catch(() => undefined);

  await openFileAtPath(path);
}

function setupPreviewZoom(): void {
  document.getElementById("preview-pane")?.addEventListener(
    "wheel",
    (e) => {
      if (e.ctrlKey) {
        e.preventDefault();
        const d = e.deltaY > 0 ? -0.1 : 0.1;
        setZoom(Math.max(0.25, Math.min(3.0, getZoom() + d)));
        updateZoomLabel();
      }
    },
    { passive: false }
  );
}

function updateZoomLabel(): void {
  const el = document.getElementById("zoom-level");
  if (el) el.textContent = `${Math.round(getZoom() * 100)}%`;
}

// ─── Gutter Drag ──────────────────────────────────────────────────
function setupGutterDrag(
  gutter: HTMLDivElement,
  pane: HTMLDivElement,
  isFixed: boolean,
  previewPane: HTMLDivElement | null
): void {
  let dragging = false;
  let startMouseX = 0;
  let startGutterLeft = 0;
  let startEditorWidth = 0;
  let startPreviewWidth = 0;
  let gutterWidth = 0;

  gutter.addEventListener("mousedown", (e) => {
    dragging = true;
    startMouseX = e.clientX;
    const gutterRect = gutter.getBoundingClientRect();
    startGutterLeft = gutterRect.left;
    gutterWidth = gutterRect.width;
    startEditorWidth = pane.offsetWidth;
    if (previewPane) {
      startPreviewWidth = previewPane.offsetWidth;
    }
    gutter.classList.add("dragging");
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    e.preventDefault();
  });

  document.addEventListener("mousemove", (e) => {
    if (!dragging) return;
    const parent = pane.parentElement!;
    const parentRect = parent.getBoundingClientRect();
    const delta = e.clientX - startMouseX;

    if (isFixed) {
      // Sidebar starts at container left, so gutter.left - container.left = sidebar width
      const w = startGutterLeft + delta - parentRect.left;
      pane.style.flexBasis = `${Math.max(120, Math.min(420, w))}px`;
      pane.style.flexShrink = "0";
    } else {
      // Use startEditorWidth + delta — avoids including sibling widths in the calculation
      const minW = Math.max(parentRect.width * 0.2, 200);
      const maxW = Math.min(parentRect.width * 0.8, 800);
      const clampedW = Math.max(minW, Math.min(maxW, startEditorWidth + delta));
      pane.style.flexBasis = `${clampedW}px`;
      pane.style.flexShrink = "0";

      if (previewPane) {
        const totalWidth = startEditorWidth + gutterWidth + startPreviewWidth;
        const newPreviewWidth = totalWidth - gutterWidth - clampedW;
        previewPane.style.flexBasis = `${Math.max(200, newPreviewWidth)}px`;
        previewPane.style.flexShrink = "0";
        previewPane.style.flexGrow = "0";
      }
    }
  });

  document.addEventListener("mouseup", () => {
    if (!dragging) return;
    dragging = false;
    gutter.classList.remove("dragging");
    document.body.style.cursor = "";
    document.body.style.userSelect = "";

    // Let preview pane fill remaining space via flex: 1 (CSS default)
    // Editor pane keeps its flexBasis to maintain the split position
    if (previewPane) {
      previewPane.style.flexBasis = "";
      previewPane.style.flexShrink = "";
      previewPane.style.flexGrow = "";
    }
  });
}
