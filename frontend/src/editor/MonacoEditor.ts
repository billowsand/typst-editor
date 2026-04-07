import * as monaco from "monaco-editor";
import { invoke } from "@tauri-apps/api/core";
import {
  clientPointToPagePoint,
  ensurePreviewPageNode,
  getPreviewContent,
  getPreviewPane,
  getPreviewPageSvg,
  pagePointToPanePoint,
  replacePreviewPageSvg,
  resetPreviewContent,
  scrollPreviewToPosition,
  trimPreviewPages,
} from "../preview/PreviewPanel";
import {
  clearOutlinePanel,
  refreshOutlinePanel,
  updateOutlineActiveItem,
} from "../outline/OutlinePanel";
import { showEmptyWorkspace, showWorkspaceLayout } from "../workspace/WorkspaceShell";

let editor: monaco.editor.IStandaloneCodeEditor | null = null;
let compileTimeout:    ReturnType<typeof setTimeout> | null = null;
let cursorSyncTimeout: ReturnType<typeof setTimeout> | null = null;
let selSyncTimeout:    ReturnType<typeof setTimeout> | null = null;
let hoverSyncTimeout:  ReturnType<typeof setTimeout> | null = null;
let outlineRefreshTimeout: ReturnType<typeof setTimeout> | null = null;
let compileStatusTimeout: ReturnType<typeof setTimeout> | null = null;
// Prevents sync loop when we programmatically move the editor cursor from a preview click
// Use a counter instead of boolean to handle nested operations correctly
let suppressCursorSync = 0;
let suppressProgrammaticCompile = false;
// Tracks last user edit timestamp — used to suppress expensive cursor sync during active typing
let lastEditTime = 0;

let currentFilePath         = "";
let previewEntryPath        = "";
let lastSuccessfulPageCount = 0;
let latestCompileRequestId  = 0;
let renderedPageHashes: number[] = [];
let tabsHost: HTMLDivElement | null = null;
let editorHost: HTMLDivElement | null = null;
let tabContextMenu: HTMLDivElement | null = null;
let previewHoverRequestId = 0;
let previewHoverTargetKey = "";
let sourceHoverDecorationIds: string[] = [];
let sourceHoverTarget: { file: string; offset: number } | null = null;

interface OpenDocument {
  path: string;
  model: monaco.editor.ITextModel;
  viewState: monaco.editor.ICodeEditorViewState | null;
  savedContent: string;
  dirty: boolean;
}

const openDocuments = new Map<string, OpenDocument>();

// ─── Monaco Theme Definitions ─────────────────────────────────────
function defineMonacoThemes(): void {
  monaco.editor.defineTheme("typst-light", {
    base: "vs",
    inherit: true,
    rules: [
      { token: "keyword",  foreground: "0064d2", fontStyle: "bold" },
      { token: "comment",  foreground: "8e8e93", fontStyle: "italic" },
      { token: "string",   foreground: "c0392b" },
      { token: "number",   foreground: "7d3eb5" },
      { token: "strong",   foreground: "1d1d1f", fontStyle: "bold" },
      { token: "emphasis", foreground: "1d1d1f", fontStyle: "italic" },
    ],
    colors: {
      "editor.background":               "#ffffff",
      "editor.foreground":               "#1d1d1f",
      "editorLineNumber.foreground":     "#c7c7cc",
      "editorLineNumber.activeForeground": "#6e6e73",
      "editor.selectionBackground":      "#0071e326",
      "editor.inactiveSelectionBackground": "#0071e314",
      "editor.lineHighlightBackground":  "#f5f5f7",
      "editor.lineHighlightBorder":      "#00000000",
      "editorCursor.foreground":         "#0071e3",
      "editorWhitespace.foreground":     "#d1d1d6",
      "editorIndentGuide.background1":   "#f0f0f0",
      "editorIndentGuide.activeBackground1": "#c7c7cc",
      "scrollbar.shadow":                "#00000014",
    },
  });

  monaco.editor.defineTheme("typst-dark", {
    base: "vs-dark",
    inherit: true,
    rules: [
      { token: "keyword",  foreground: "409cff", fontStyle: "bold" },
      { token: "comment",  foreground: "6e6e73", fontStyle: "italic" },
      { token: "string",   foreground: "ff7b72" },
      { token: "number",   foreground: "bf5af2" },
      { token: "strong",   foreground: "f5f5f7", fontStyle: "bold" },
      { token: "emphasis", foreground: "f5f5f7", fontStyle: "italic" },
    ],
    colors: {
      "editor.background":               "#1c1c1e",
      "editor.foreground":               "#f5f5f7",
      "editorLineNumber.foreground":     "#48484a",
      "editorLineNumber.activeForeground": "#8e8e93",
      "editor.selectionBackground":      "#0a84ff28",
      "editor.inactiveSelectionBackground": "#0a84ff14",
      "editor.lineHighlightBackground":  "#2c2c2e",
      "editor.lineHighlightBorder":      "#00000000",
      "editorCursor.foreground":         "#0a84ff",
      "editorWhitespace.foreground":     "#3a3a3c",
      "editorIndentGuide.background1":   "#2c2c2e",
      "editorIndentGuide.activeBackground1": "#48484a",
      "scrollbar.shadow":                "#00000040",
    },
  });
}

// ─── Public: Theme & Font ─────────────────────────────────────────
export function setMonacoTheme(theme: "light" | "dark"): void {
  monaco.editor.setTheme(theme === "dark" ? "typst-dark" : "typst-light");
}

export function setEditorFont(family: string, size: number): void {
  editor?.updateOptions({
    fontFamily: family,
    fontSize: size,
    fontLigatures: true,
  });
}

// ─── Init ─────────────────────────────────────────────────────────
export async function initEditor(container: HTMLDivElement): Promise<void> {
  defineMonacoThemes();
  registerTypstLanguage();

  const savedTheme = (localStorage.getItem("typst-theme") ?? "dark") as "light" | "dark";
  const savedSize  = parseInt(localStorage.getItem("typst-font-size") ?? "14", 10);

  container.replaceChildren();

  tabsHost = document.createElement("div");
  tabsHost.className = "editor-tabs";
  container.appendChild(tabsHost);

  editorHost = document.createElement("div");
  editorHost.className = "editor-surface";
  container.appendChild(editorHost);

  editor = monaco.editor.create(editorHost, {
    language: "typst",
    theme: savedTheme === "dark" ? "typst-dark" : "typst-light",
    automaticLayout: true,
    minimap: { enabled: false },
    fontSize: savedSize,
    lineNumbers: "on",
    scrollBeyondLastLine: false,
    wordWrap: "on",
    padding: { top: 10, bottom: 8 },
    tabSize: 2,
    autoClosingBrackets: "always",
    autoClosingQuotes: "always",
    autoIndent: "full",
    renderLineHighlight: "all",
    fontLigatures: true,
    smoothScrolling: true,
    cursorSmoothCaretAnimation: "on",
    renderWhitespace: "selection",
    bracketPairColorization: { enabled: true },
  });

  setCurrentFilePath(null);
  renderTabs();
  updatePageStatus(0);
  clearOutlinePanel();
  updateStatus("success", "Ready");

  // ── Live compile (debounced 650 ms) ──────────────────────────
  editor.onDidChangeModelContent(() => {
    lastEditTime = Date.now();
    updateActiveDocumentState();
    if (suppressProgrammaticCompile) return;
    scheduleCompile();
  });

  // ── Cursor → Preview (user-initiated only, debounced 120 ms) ────
  // Explicit=3 covers keyboard; NotSet=0 covers mouse clicks.
  // We suppress sync when we move the cursor programmatically (preview→editor).
  editor.onDidChangeCursorPosition((e) => {
    updateCursorStatus(e.position.lineNumber, e.position.column);
    if (suppressCursorSync > 0) return;
    const r = monaco.editor.CursorChangeReason;
    if (e.reason === r.Explicit || e.reason === r.NotSet) {
      if (cursorSyncTimeout) clearTimeout(cursorSyncTimeout);
      cursorSyncTimeout = setTimeout(() => {
        const sel = editor?.getSelection();
        if (sel && sel.isEmpty()) {
          removeSyncOverlays("selection-hl");
          void syncCursorToPreview(e.position);
        }
      }, 120);
    }
    syncOutlineActiveFromEditor();
  });

  const initialPos = editor.getPosition();
  if (initialPos) updateCursorStatus(initialPos.lineNumber, initialPos.column);

  // ── Selection → Preview (debounced 200 ms) ────────────────────
  editor.onDidChangeCursorSelection((e) => {
    const sel = e.selection;
    if (sel.isEmpty()) {
      // Selection cleared — remove selection overlays
      removeSyncOverlays("selection-hl");
    } else {
      if (selSyncTimeout) clearTimeout(selSyncTimeout);
      selSyncTimeout = setTimeout(() => void syncSelectionToPreview(sel), 200);
    }
  });

  editor.onMouseMove((e) => {
    const { position } = e.target;
    if (!position) return;
    if (hoverSyncTimeout) clearTimeout(hoverSyncTimeout);
    hoverSyncTimeout = setTimeout(() => {
      void syncHoverToPreview(position);
    }, 150);
  });

  editor.onMouseLeave(() => {
    if (hoverSyncTimeout) clearTimeout(hoverSyncTimeout);
    clearPreviewSourceHover();
  });

  // ── Keyboard shortcuts ─────────────────────────────────────────
  editor.addAction({
    id: "save-file",
    label: "Save File",
    keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS],
    run: () => saveCurrentFile(),
  });
  editor.addAction({
    id: "force-compile",
    label: "Force Compile",
    keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter],
    run: () => compileAndPreview(),
  });
  editor.addAction({
    id: "export-pdf",
    label: "Export PDF",
    keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.KeyE],
    run: () => exportPdf(),
  });
  editor.addAction({
    id: "open-file",
    label: "Open File",
    keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyO],
    run: () => openFile(),
  });
  editor.addAction({
    id: "close-file-tab",
    label: "Close File Tab",
    keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyW],
    run: () => closeCurrentTab(),
  });
  editor.addAction({
    id: "set-preview-entry",
    label: "Set Preview Entry",
    keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.Enter],
    run: () => setCurrentDocumentAsPreviewEntry(),
  });
  editor.addAction({
    id: "clear-preview-entry-pin",
    label: "Clear Preview Entry Pin",
    keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyMod.Alt | monaco.KeyCode.Enter],
    run: () => clearPreviewEntryPin(),
  });
}

// ─── Accessors ────────────────────────────────────────────────────
export function getEditor(): monaco.editor.IStandaloneCodeEditor | null { return editor; }
export function getContent(): string { return editor?.getValue() ?? ""; }
export function setContent(content: string): void { editor?.setValue(content); }
export function getCurrentFilePath(): string | null { return currentFilePath || null; }
export function hasUnsavedDocuments(): boolean {
  return Array.from(openDocuments.values()).some((doc) => doc.dirty);
}

export function setCurrentFilePath(path: string | null): void {
  currentFilePath = path ?? "";
  const title = document.querySelector<HTMLElement>(".app-title");
  if (title) {
    title.textContent = path ? `Typst · ${path.split(/[\\/]/).pop()}` : "Typst";
  }
  updateStatusFile(path);
  renderTabs();
}

// ─── Zoom ─────────────────────────────────────────────────────────
let currentZoom = 1.0;

export function setZoom(zoom: number): void {
  currentZoom = zoom;
  const content = getPreviewContent();
  if (content) {
    content.style.transform = `scale(${zoom})`;
    content.style.transformOrigin = "top center";
  }
  updateZoomStatus(zoom);
}

export function getZoom(): number { return currentZoom; }

// ─── Compilation ──────────────────────────────────────────────────
interface CompileResult {
  success: boolean;
  page_count: number;
  page_hashes: number[];
  errors: string[];
  duration_ms?: number;
  entry_path?: string | null;
}

interface JumpResult {
  type: string;
  file?: string | null;
  offset?: number | null;
  url?: string | null;
  position?: PagePos | null;
}

export async function compileAndPreview(): Promise<void> {
  if (!editor) return;
  if (!currentFilePath) return;
  const content   = editor.getValue();
  const requestId = ++latestCompileRequestId;
  if (compileStatusTimeout) clearTimeout(compileStatusTimeout);
  compileStatusTimeout = setTimeout(() => {
    if (requestId === latestCompileRequestId) {
      updateStatus("compiling", "Syncing preview…");
    }
  }, 180);

  try {
    const result = await invoke<CompileResult>("compile_current", {
      filePath: currentFilePath,
      content,
    });
    if (requestId < latestCompileRequestId) return; // stale
    if (compileStatusTimeout) {
      clearTimeout(compileStatusTimeout);
      compileStatusTimeout = null;
    }

    if (result.success) {
      previewEntryPath = result.entry_path ?? previewEntryPath;
      lastSuccessfulPageCount = result.page_count;
      updatePageStatus(result.page_count);
      await renderAllPages(result.page_count, result.page_hashes);
      scheduleOutlineRefresh();
      updateStatus("success",
        `Compiled in ${result.duration_ms ?? "?"}ms · ${result.page_count} page(s)`);
      clearDiagnostics();
    } else {
      updatePageStatus(0);
      clearOutlinePanel();
      updateStatus("error", `${result.errors.length} error(s)`);
      showDiagnostics(result.errors);
    }
  } catch (e) {
    if (compileStatusTimeout) {
      clearTimeout(compileStatusTimeout);
      compileStatusTimeout = null;
    }
    updateStatus("error", `Error: ${e}`);
  }
}

function scheduleCompile(delay = 650): void {
  if (compileTimeout) clearTimeout(compileTimeout);
  compileTimeout = setTimeout(() => {
    compileTimeout = null;
    void compileAndPreview();
  }, delay);
}

function scheduleOutlineRefresh(delay = 120): void {
  if (outlineRefreshTimeout) clearTimeout(outlineRefreshTimeout);
  outlineRefreshTimeout = setTimeout(() => {
    outlineRefreshTimeout = null;
    void refreshOutlinePanel().then(() => {
      syncOutlineActiveFromEditor();
    });
  }, delay);
}

// ─── File Operations ──────────────────────────────────────────────
export async function openFile(): Promise<void> {
  const path = await pickFilePath();
  if (!path) return;
  await openFileAtPath(path);
}

export async function pickFilePath(): Promise<string | null> {
  return invoke<string | null>("open_file_dialog");
}

export async function openFileAtPath(path: string): Promise<void> {
  try {
    const existingDoc = openDocuments.get(path);
    if (existingDoc) {
      await switchToDocument(existingDoc);
      return;
    }

    const content = await invoke<string>("read_text_file", { path });
    const doc = createDocument(path, content);
    await switchToDocument(doc);
  } catch (e) {
    suppressProgrammaticCompile = false;
    updateStatus("error", `Failed to open: ${e}`);
  }
}

async function saveCurrentFile(): Promise<void> {
  if (!editor) return;
  let path = currentFilePath;
  let activeDoc = getActiveDocument();

  if (!path) {
    try {
      const picked = await invoke<string | null>("save_file_dialog");
      if (!picked) return;
      setCurrentFilePath(picked);
      path = picked;
      activeDoc = getActiveDocument();
    } catch (e) {
      updateStatus("error", `Save dialog failed: ${e}`);
      return;
    }
  }

  const content = editor.getValue();
  try {
    await invoke("save_file", { content, filePath: path });
    if (activeDoc) {
      activeDoc.savedContent = content;
      activeDoc.dirty = false;
      renderTabs();
    }
    const result = await invoke<CompileResult>("compile_current", { filePath: path, content });
    if (!result.success) {
      updatePageStatus(0);
      showDiagnostics(result.errors);
      updateStatus("error", result.errors.join("; "));
      return;
    }
    resetPreviewState();
    previewEntryPath = result.entry_path ?? previewEntryPath;
    lastSuccessfulPageCount = result.page_count;
    updatePageStatus(result.page_count);
    await renderAllPages(result.page_count, result.page_hashes);
    scheduleOutlineRefresh();
    updateStatus("success", "Saved");
  } catch (e) {
    updateStatus("error", `Save failed: ${e}`);
  }
}

async function exportPdf(): Promise<void> {
  try {
    const result = await invoke<"Success" | "Cancelled">("export_pdf_dialog");
    if (result !== "Cancelled") updateStatus("success", "PDF exported");
  } catch (e) {
    updateStatus("error", `Export failed: ${e}`);
  }
}

// ─── File Tree Integration ────────────────────────────────────────
export async function openFileFromTree(path: string): Promise<void> {
  if (!editor) return;
  await openFileAtPath(path);
}

export async function jumpToFileOffset(
  path: string,
  offset: number,
  refreshPreview = false,
): Promise<void> {
  const targetDoc = openDocuments.get(path);
  if (targetDoc) {
    await switchToDocument(targetDoc, refreshPreview);
  } else {
    await openFileAtPath(path);
  }

  if (!editor) return;
  const model = editor.getModel();
  if (!model) return;

  // offset from Rust is a UTF-8 byte offset; Monaco needs a UTF-16 char offset
  const charOffset = utf8ByteOffsetToMonacoOffset(model.getValue(), offset);
  const pos = model.getPositionAt(charOffset);
  suppressCursorSync++;
  editor.setPosition(pos);
  editor.revealPositionInCenter(pos);
  editor.focus();
  syncOutlineActiveFromEditor();
  setTimeout(() => {
    suppressCursorSync = Math.max(0, suppressCursorSync - 1);
  }, 50);
}

export async function revealSourceOffsetInPreview(
  path: string,
  offset: number,
  behavior: ScrollBehavior = "auto",
): Promise<void> {
  if (!editor) return;
  const model = editor.getModel();
  if (!model || currentFilePath !== path) return;

  // offset from Rust is a UTF-8 byte offset; Monaco needs a UTF-16 char offset
  const charOffset = utf8ByteOffsetToMonacoOffset(model.getValue(), offset);
  const pos = model.getPositionAt(charOffset);
  try {
    const positions = await invoke<PagePos[]>("jump_from_cursor", {
      filePath: path,
      line: pos.lineNumber - 1,
      character: pos.column - 1,
    });
    const previewPos = pickBestPreviewPosition(positions);
    if (previewPos) {
      highlightCursorInPreview(previewPos, { scroll: true, behavior, persistent: true });
    }
  } catch {
    // ignore sync failures
  }
}

// ─── Offset Conversion ────────────────────────────────────────────
//
// Rust (Typst) reports source offsets as **UTF-8 byte offsets**.
// Monaco internally uses **UTF-16 code-unit offsets** (same as JS string indices).
// For pure-ASCII files these are identical, but CJK characters are 3 UTF-8 bytes
// and 1 UTF-16 code unit, so without conversion outline navigation lands at the
// wrong position in files with Chinese content.

/**
 * Convert a UTF-8 byte offset (from Rust/Typst) to a Monaco/JS character offset.
 * Walks the text character-by-character, counting UTF-8 bytes, until reaching
 * the target byte count.
 */
function utf8ByteOffsetToMonacoOffset(text: string, byteOffset: number): number {
  let bytes = 0;
  let i = 0;
  while (i < text.length && bytes < byteOffset) {
    const code = text.charCodeAt(i);
    if (code < 0x80) {
      bytes += 1;
    } else if (code < 0x800) {
      bytes += 2;
    } else if (code >= 0xD800 && code < 0xDC00) {
      // Surrogate pair — 4 UTF-8 bytes, 2 UTF-16 code units
      bytes += 4;
      i++; // skip low surrogate (outer loop adds 1 more → total advance = 2)
    } else {
      bytes += 3;
    }
    i++;
  }
  return i;
}

/**
 * Convert a Monaco/JS character offset to a UTF-8 byte offset (for Rust/Typst).
 */
function monacoOffsetToUtf8ByteOffset(text: string, charOffset: number): number {
  let bytes = 0;
  for (let i = 0; i < charOffset && i < text.length; i++) {
    const code = text.charCodeAt(i);
    if (code < 0x80) {
      bytes += 1;
    } else if (code < 0x800) {
      bytes += 2;
    } else if (code >= 0xD800 && code < 0xDC00) {
      bytes += 4; // surrogate pair
      i++;        // skip low surrogate
    } else {
      bytes += 3;
    }
  }
  return bytes;
}

// ─── Bidirectional Sync ───────────────────────────────────────────

interface PagePos { page: number; x: number; y: number; }

/** Editor cursor → preview position indicator */
async function syncCursorToPreview(position: monaco.Position): Promise<void> {
  const model = editor?.getModel();
  if (!model) return;
  if (!currentFilePath) return;
  // Skip sync if no compiled output exists yet, or if the user is actively typing
  if (lastSuccessfulPageCount === 0) return;
  if (Date.now() - lastEditTime < 400) return;

  try {
    const positions = await invoke<PagePos[]>("jump_from_cursor", {
      filePath: currentFilePath,
      line: position.lineNumber - 1,
      character: position.column - 1,
    });
    const previewPos = pickBestPreviewPosition(positions);
    if (previewPos) {
      highlightCursorInPreview(previewPos, { scroll: false, persistent: true });
    }
  } catch { /* ignore */ }
}

/** Editor text selection → preview range highlight */
async function syncSelectionToPreview(sel: monaco.Selection): Promise<void> {
  const model = editor?.getModel();
  if (!model) return;
  if (!currentFilePath) return;

  const startPos = sel.getStartPosition();
  const endPos   = sel.getEndPosition();
  if (startPos.equals(endPos)) return;

  try {
    const [startPositions, endPositions] = await Promise.all([
      invoke<PagePos[]>("jump_from_cursor", {
        filePath: currentFilePath,
        line: startPos.lineNumber - 1,
        character: startPos.column - 1,
      }),
      invoke<PagePos[]>("jump_from_cursor", {
        filePath: currentFilePath,
        line: endPos.lineNumber - 1,
        character: endPos.column - 1,
      }),
    ]);

    if (startPositions.length > 0 && endPositions.length > 0) {
      removeSyncOverlays("cursor-hl");
      // Process each position pair. highlightRangeInPreview handles multi-page ranges,
      // and by passing clearFirst=false for subsequent calls, we accumulate all highlights.
      const maxLen = Math.max(startPositions.length, endPositions.length);
      for (let i = 0; i < maxLen; i++) {
        const start = startPositions[i] ?? startPositions[0];
        const end = endPositions[i] ?? endPositions[0];
        highlightRangeInPreview(start, end, i === 0);
      }
    }
  } catch { /* ignore */ }
}

/** Preview click → editor cursor jump */
async function handlePreviewClick(
  e: MouseEvent,
  svgEl: SVGSVGElement,
  pageIndex: number
): Promise<void> {
  const pagePoint = clientPointToPagePoint(svgEl, e.clientX, e.clientY);

  try {
    const result = await invoke<JumpResult>("jump_from_click", {
      page: pageIndex + 1,
      x: pagePoint.x,
      y: pagePoint.y,
    });

    if (result.type === "file" && result.offset != null && result.file && editor) {
      applySourceHoverDecoration(result.file, result.offset);
      await jumpToFileOffset(result.file, result.offset, false);
    } else if (result.type === "url" && result.url) {
      window.open(result.url, "_blank", "noopener,noreferrer");
    } else if (result.type === "position" && result.position) {
      scrollPreviewToPosition(result.position);
    }
  } catch { /* ignore */ }
}

async function syncHoverToPreview(position: monaco.Position): Promise<void> {
  const model = editor?.getModel();
  if (!model || !currentFilePath) return;
  // Skip hover sync if no compiled output, or if actively typing
  if (lastSuccessfulPageCount === 0) return;
  if (Date.now() - lastEditTime < 600) return;

  try {
    const positions = await invoke<PagePos[]>("jump_from_cursor", {
      filePath: currentFilePath,
      line: position.lineNumber - 1,
      character: position.column - 1,
    });
    const previewPos = pickBestPreviewPosition(positions);
    if (previewPos) {
      applyHoverAtPreviewPosition(previewPos);
    }
  } catch {
    // ignore sync failures
  }
}

async function handlePreviewHover(
  event: MouseEvent,
  svgEl: SVGSVGElement,
  pageIndex: number,
): Promise<void> {
  const pagePoint = clientPointToPagePoint(svgEl, event.clientX, event.clientY);
  const hoverKey = `${pageIndex}:${Math.round(pagePoint.x)}:${Math.round(pagePoint.y)}`;
  if (hoverKey === previewHoverTargetKey) return;
  previewHoverTargetKey = hoverKey;

  const requestId = ++previewHoverRequestId;
  try {
    const result = await invoke<JumpResult>("jump_from_click", {
      page: pageIndex + 1,
      x: pagePoint.x,
      y: pagePoint.y,
    });
    if (requestId !== previewHoverRequestId) return;

    if (result.type === "file" && result.offset != null && result.file) {
      // Show a subtle hover line at the hovered position (overlay approach, no SVG fill changes)
      highlightHoverInPreview({ page: pageIndex + 1, x: pagePoint.x, y: pagePoint.y });
      applySourceHoverDecoration(result.file, result.offset);
    } else {
      clearPreviewSourceHover();
    }
  } catch {
    if (requestId === previewHoverRequestId) clearPreviewSourceHover();
  }
}

// ─── Highlight Helpers ────────────────────────────────────────────

function removeSyncOverlays(cls: string): void {
  document.querySelectorAll(`.highlight-overlay.${cls}`).forEach(el => el.remove());
}

function highlightCursorInPreview(
  pos: PagePos,
  options: { scroll?: boolean; behavior?: ScrollBehavior; persistent?: boolean } = {},
): void {
  removeSyncOverlays("cursor-hl");
  const pane = getPreviewPane();
  const svgEl = getPreviewPageSvg(pos.page);
  const panePoint = pagePointToPanePoint(pos);
  if (!pane || !svgEl || !panePoint) {
    const totalPages = getPreviewContent()?.querySelectorAll<SVGSVGElement>("svg").length ?? 0;
    console.warn(`[cursor sync] page ${pos.page} out of range (${totalPages} pages rendered)`);
    return;
  }

  const overlay        = document.createElement("div");
  overlay.className    = "highlight-overlay cursor-hl";
  overlay.style.left   = `${panePoint.left}px`;
  overlay.style.top    = `${panePoint.top - 12}px`;
  overlay.style.width  = `${panePoint.pageWidth}px`;
  overlay.style.height = "24px";

  getPreviewContent()?.appendChild(overlay);
  if (options.scroll) {
    scrollPreviewToPosition(pos, options.behavior ?? "smooth");
  }
  if (!options.persistent) {
    setTimeout(() => overlay.classList.add("fade"), 1500);
    setTimeout(() => overlay.remove(), 2000);
  }
}

/**
 * Draw a selection-range highlight from `start` to `end`, spanning pages if necessary.
 * Scroll the preview so the start of the selection is visible.
 * @param clearFirst - if true (default), clears existing selection overlays before drawing.
 *                     Set to false when drawing multiple ranges in a loop.
 */
function highlightRangeInPreview(start: PagePos, end: PagePos, clearFirst = true): void {
  if (clearFirst) removeSyncOverlays("selection-hl");

  const pane = getPreviewPane();
  const contentEl = getPreviewContent();
  const svgs = contentEl?.querySelectorAll<SVGSVGElement>("svg");
  if (!svgs || svgs.length === 0) return;
  if (!pane || !contentEl) return;

  const firstPage = Math.min(start.page, end.page);
  const lastPage  = Math.max(start.page, end.page);

  for (let p = firstPage; p <= Math.min(lastPage, svgs.length); p++) {
    const svgEl = svgs[p - 1];
    if (!svgEl) continue;

    const rect  = svgEl.getBoundingClientRect();
    const panePoint = pagePointToPanePoint({ page: p, x: 0, y: 0 });
    if (!panePoint) continue;
    const svgH  = svgEl.viewBox.baseVal.height || parseFloat(svgEl.getAttribute("height") || "1");
    const scaleY = rect.height / svgH;

    let topY: number;
    let botY: number;

    if (p === firstPage && p === lastPage) {
      topY = start.y * scaleY - 12;
      botY = end.y   * scaleY + 8;
    } else if (p === firstPage) {
      topY = start.y * scaleY - 12;
      botY = rect.height;
    } else if (p === lastPage) {
      topY = 0;
      botY = end.y * scaleY + 8;
    } else {
      topY = 0;
      botY = rect.height;
    }

    const height = Math.max(20, botY - topY);

    const overlay        = document.createElement("div");
    overlay.className    = "highlight-overlay selection-hl";
    overlay.style.left   = `${panePoint.left + 2}px`;
    overlay.style.top    = `${panePoint.top + topY}px`;
    overlay.style.width  = `${rect.width - 4}px`;
    overlay.style.height = `${height}px`;

    contentEl.appendChild(overlay);
  }

  scrollPreviewToPosition(start);
}

function pickBestPreviewPosition(positions: PagePos[]): PagePos | null {
  if (positions.length === 0) return null;
  return positions[positions.length - 1] ?? positions[0] ?? null;
}

function applyHoverAtPreviewPosition(pos: PagePos): void {
  // Use the cursor-hl overlay (already persistent) — no separate SVG element coloring needed
  highlightCursorInPreview(pos, { scroll: false, persistent: true });
}

/**
 * Draw a subtle "hover" overlay at the given page position.
 * Uses the .hover-hl overlay class so it doesn't affect SVG element fill/stroke.
 */
function highlightHoverInPreview(pos: PagePos): void {
  removeSyncOverlays("hover-hl");
  const pane = getPreviewPane();
  const svgEl = getPreviewPageSvg(pos.page);
  const panePoint = pagePointToPanePoint(pos);
  if (!pane || !svgEl || !panePoint) return;

  const overlay = document.createElement("div");
  overlay.className = "highlight-overlay hover-hl";
  overlay.style.left   = `${panePoint.left}px`;
  overlay.style.top    = `${panePoint.top - 10}px`;
  overlay.style.width  = `${panePoint.pageWidth}px`;
  overlay.style.height = "22px";

  getPreviewContent()?.appendChild(overlay);
}

function applySourceHoverDecoration(file: string, offset: number): void {
  sourceHoverTarget = { file, offset };
  if (!editor || currentFilePath !== file) {
    clearSourceHoverDecoration();
    return;
  }

  const model = editor.getModel();
  if (!model) return;

  // offset from Rust is UTF-8 bytes; convert to Monaco UTF-16 char offset
  const text = model.getValue();
  const charStart = utf8ByteOffsetToMonacoOffset(text, offset);
  const charEnd   = utf8ByteOffsetToMonacoOffset(text, offset + 1);
  const start = model.getPositionAt(charStart);
  const end   = model.getPositionAt(charEnd);
  sourceHoverDecorationIds = editor.deltaDecorations(sourceHoverDecorationIds, [{
    range: new monaco.Range(
      start.lineNumber,
      start.column,
      end.lineNumber,
      Math.max(start.column + 1, end.column),
    ),
    options: {
      className: "editor-source-hover",
      stickiness: monaco.editor.TrackedRangeStickiness.NeverGrowsWhenTypingAtEdges,
    },
  }]);
}

function clearSourceHoverDecoration(): void {
  sourceHoverDecorationIds = editor?.deltaDecorations(sourceHoverDecorationIds, []) ?? [];
}

function clearPreviewSourceHover(): void {
  previewHoverTargetKey = "";
  sourceHoverTarget = null;
  clearSourceHoverDecoration();
  removeSyncOverlays("hover-hl");
}

function syncOutlineActiveFromEditor(): void {
  const model = editor?.getModel();
  const pos = editor?.getPosition();
  if (!model || !pos || !currentFilePath) {
    updateOutlineActiveItem(null, null);
    return;
  }

  // getOffsetAt returns a UTF-16 char offset; convert to UTF-8 byte offset
  // so it can be compared correctly against Rust/Typst outline item offsets
  const charOffset = model.getOffsetAt(pos);
  const byteOffset = monacoOffsetToUtf8ByteOffset(model.getValue(), charOffset);
  updateOutlineActiveItem(currentFilePath, byteOffset);
}

// ─── Rendering ────────────────────────────────────────────────────
export async function renderAllPages(
  pageCount: number,
  pageHashes: number[]
): Promise<void> {
  trimPreviewPages(pageCount);

  // Collect indices of pages that actually changed (hash-based skip)
  const changedPages: number[] = [];
  for (let i = 0; i < pageCount; i++) {
    ensurePreviewPageNode(i);
    if (renderedPageHashes[i] !== pageHashes[i]) changedPages.push(i);
  }

  // Render changed pages in parallel — render_page only reads from the compiled document
  await Promise.all(
    changedPages.map((i) => renderSinglePage(i, pageHashes))
  );

  renderedPageHashes.length = pageCount;
}

async function renderSinglePage(i: number, pageHashes: number[]): Promise<void> {
  const capturedHash = pageHashes[i];
  const pageNode = ensurePreviewPageNode(i);
  try {
    const svg = await invoke<string>("render_page", { page: i });
    // Discard result if a newer compile has already produced a different hash
    if (capturedHash !== pageHashes[i]) return;
    if (!replacePreviewPageSvg(i, svg, {
      onClick: handlePreviewClick,
      onHover: handlePreviewHover,
      onLeave: () => clearPreviewSourceHover(),
    })) return;
    renderedPageHashes[i] = capturedHash;
    pageNode.classList.add("page-updated");
    setTimeout(() => pageNode.classList.remove("page-updated"), 80);
  } catch { /* ignore individual page render errors */ }
}
function resetPreviewState(): void {
  latestCompileRequestId++;
  renderedPageHashes = [];
  clearPreviewSourceHover();
  removeSyncOverlays("cursor-hl");
  removeSyncOverlays("selection-hl");
  resetPreviewContent();
}

function createDocument(path: string, content: string): OpenDocument {
  const uri = monaco.Uri.parse(`file:///${path.replace(/\\/g, "/").replace(/^\/+/, "")}`);
  const existingModel = monaco.editor.getModel(uri);
  const model = existingModel ?? monaco.editor.createModel(content, "typst", uri);
  if (existingModel && existingModel.getValue() !== content) {
    existingModel.setValue(content);
  }

  const doc: OpenDocument = {
    path,
    model,
    viewState: null,
    savedContent: content,
    dirty: false,
  };

  openDocuments.set(path, doc);
  return doc;
}

function getActiveDocument(): OpenDocument | null {
  return currentFilePath ? (openDocuments.get(currentFilePath) ?? null) : null;
}

function updateActiveDocumentState(): void {
  const doc = getActiveDocument();
  if (!doc) return;
  doc.dirty = doc.model.getValue() !== doc.savedContent;
  renderTabs();
}

async function switchToDocument(doc: OpenDocument, refreshPreview = true): Promise<void> {
  if (!editor) return;
  showWorkspaceLayout();

  const currentDoc = getActiveDocument();
  if (currentDoc && currentDoc.path !== doc.path) {
    currentDoc.viewState = editor.saveViewState();
  }

  suppressProgrammaticCompile = true;
  editor.setModel(doc.model);
  suppressProgrammaticCompile = false;

  setCurrentFilePath(doc.path);
  if (doc.viewState) {
    editor.restoreViewState(doc.viewState);
  }
  editor.focus();
  updateActiveDocumentState();
  syncOutlineActiveFromEditor();
  if (sourceHoverTarget?.file === doc.path) {
    applySourceHoverDecoration(sourceHoverTarget.file, sourceHoverTarget.offset);
  } else {
    clearSourceHoverDecoration();
  }
  if (refreshPreview) {
    resetPreviewState();
    await compileOpenDocument(doc);
  }
}

async function closeCurrentTab(): Promise<void> {
  const doc = getActiveDocument();
  if (!doc) return;
  await closeDocument(doc.path);
}

async function closeDocument(path: string): Promise<void> {
  const doc = openDocuments.get(path);
  if (!doc) return;

  if (doc.dirty && !window.confirm(`"${doc.path.split(/[\\/]/).pop()}" 有未保存修改，确定关闭吗？`)) {
    return;
  }

  const wasActive = currentFilePath === path;
  const docs = Array.from(openDocuments.values());
  const currentIndex = docs.findIndex((item) => item.path === path);
  openDocuments.delete(path);
  doc.model.dispose();

  if (openDocuments.size === 0) {
    editor?.setModel(null);
    setCurrentFilePath(null);
    previewEntryPath = "";
    lastSuccessfulPageCount = 0;
    resetPreviewState();
    renderTabs();
    clearDiagnostics();
    clearOutlinePanel();
    updateOutlineActiveItem(null, null);
    updatePageStatus(0);
    updateStatus("success", "Ready");
    await invoke("reset_session");
    showEmptyWorkspace();
    return;
  }

  if (wasActive) {
    const remainingDocs = Array.from(openDocuments.values());
    const nextDoc = remainingDocs[Math.max(0, Math.min(currentIndex, remainingDocs.length - 1))];
    await switchToDocument(nextDoc);
  } else {
    renderTabs();
  }
}

async function compileOpenDocument(doc: OpenDocument): Promise<void> {
  try {
    const content = doc.model.getValue();
    const result = !previewEntryPath
      ? await invoke<CompileResult>("open_file", { filePath: doc.path, content })
      : await invoke<CompileResult>("compile_current", { filePath: doc.path, content });

    if (result.success) {
      showWorkspaceLayout();
      previewEntryPath = result.entry_path ?? previewEntryPath;
      lastSuccessfulPageCount = result.page_count;
      updatePageStatus(result.page_count);
      await renderAllPages(result.page_count, result.page_hashes);
      scheduleOutlineRefresh();
      updateStatus("success", `Compiled · ${result.page_count} page(s)`);
      clearDiagnostics();
      syncOutlineActiveFromEditor();
    } else {
      updatePageStatus(0);
      clearOutlinePanel();
      updateStatus("error", result.errors.join("; "));
      showDiagnostics(result.errors);
    }
  } catch (e) {
    updatePageStatus(0);
    updateStatus("error", `Failed to compile: ${e}`);
  }
}

async function setCurrentDocumentAsPreviewEntry(): Promise<void> {
  if (!editor || !currentFilePath) return;

  try {
    const result = await invoke<CompileResult>("set_preview_entry", {
      filePath: currentFilePath,
      content: editor.getValue(),
    });

    if (!result.success) {
      updatePageStatus(0);
      updateStatus("error", result.errors.join("; "));
      showDiagnostics(result.errors);
      return;
    }

    resetPreviewState();
    showWorkspaceLayout();
    previewEntryPath = result.entry_path ?? currentFilePath;
    lastSuccessfulPageCount = result.page_count;
    updatePageStatus(result.page_count);
    await renderAllPages(result.page_count, result.page_hashes);
    scheduleOutlineRefresh();
    updateStatus("success", "Preview entry updated");
    clearDiagnostics();
    syncOutlineActiveFromEditor();
  } catch (e) {
    updateStatus("error", `Failed to set preview entry: ${e}`);
  }
}

async function clearPreviewEntryPin(): Promise<void> {
  if (!editor || !currentFilePath) return;

  try {
    await invoke("clear_preview_entry_pin");
    await compileAndPreview();
  } catch (e) {
    updateStatus("error", `Failed to clear preview entry pin: ${e}`);
  }
}

function renderTabs(): void {
  if (!tabsHost) return;
  tabsHost.replaceChildren();

  for (const doc of openDocuments.values()) {
    const tab = document.createElement("button");
    tab.className = `editor-tab${doc.path === currentFilePath ? " active" : ""}`;
    tab.title = doc.path;

    const label = document.createElement("span");
    label.className = "editor-tab-label";
    label.textContent = doc.path.split(/[\\/]/).pop() || doc.path;
    tab.appendChild(label);

    if (doc.dirty) {
      const dirty = document.createElement("span");
      dirty.className = "editor-tab-dirty";
      dirty.textContent = "●";
      tab.appendChild(dirty);
    }

    const close = document.createElement("span");
    close.className = "editor-tab-close";
    close.textContent = "×";
    close.title = "关闭";
    close.addEventListener("click", async (event) => {
      event.stopPropagation();
      await closeDocument(doc.path);
    });
    tab.appendChild(close);

    tab.addEventListener("click", async () => {
      if (doc.path === currentFilePath) return;
      await switchToDocument(doc);
    });

    tab.addEventListener("contextmenu", (event) => {
      event.preventDefault();
      showTabContextMenu(event.clientX, event.clientY, doc.path);
    });

    tabsHost.appendChild(tab);
  }
}

function showTabContextMenu(x: number, y: number, path: string): void {
  hideTabContextMenu();

  const menu = document.createElement("div");
  menu.className = "tab-context-menu";
  menu.style.left = `${x}px`;
  menu.style.top = `${y}px`;

  const setMain = document.createElement("button");
  setMain.type = "button";
  setMain.className = "tab-context-menu-item";
  setMain.textContent = path === previewEntryPath ? "当前已是主文件" : "设为主文件";
  setMain.disabled = path === previewEntryPath;
  setMain.addEventListener("click", async () => {
    hideTabContextMenu();
    const targetDoc = openDocuments.get(path);
    if (!targetDoc || !editor) return;
    if (currentFilePath !== path) {
      await switchToDocument(targetDoc, false);
    }
    await setCurrentDocumentAsPreviewEntry();
  });

  const clearMain = document.createElement("button");
  clearMain.type = "button";
  clearMain.className = "tab-context-menu-item";
  clearMain.textContent = "取消主文件固定";
  clearMain.addEventListener("click", async () => {
    hideTabContextMenu();
    await clearPreviewEntryPin();
  });

  menu.append(setMain, clearMain);
  document.body.appendChild(menu);
  tabContextMenu = menu;

  setTimeout(() => {
    document.addEventListener("mousedown", handleTabContextMenuDismiss, { once: true });
    document.addEventListener("blur", handleTabContextMenuDismiss, { once: true });
  }, 0);
}

function handleTabContextMenuDismiss(): void {
  hideTabContextMenu();
}

function hideTabContextMenu(): void {
  tabContextMenu?.remove();
  tabContextMenu = null;
}

// ─── Typst Language ───────────────────────────────────────────────
function registerTypstLanguage(): void {
  monaco.languages.register({ id: "typst" });

  monaco.languages.setMonarchTokensProvider("typst", {
    tokenizer: {
      root: [
        [/^\s*={1,6}\s/, "keyword"],
        [/\/\/.*$/, "comment"],
        [/\/\*/, "comment", "@comment"],
        [/\*[^*]+\*/, "strong"],
        [/_[^_]+_/, "emphasis"],
        [/#([a-zA-Z_]\w*)/, "keyword"],
        [/"/, "string", "@string"],
        [/\d+(\.\d+)?/, "number"],
        [/[{}()\[\]]/, "@brackets"],
        [/[@$]/, "keyword"],
      ],
      comment: [
        [/\*\//, "comment", "@pop"],
        [/./, "comment"],
      ],
      string: [
        [/"/, "string", "@pop"],
        [/\\./, "string.escape"],
        [/./, "string"],
      ],
    },
  });

  monaco.languages.registerCompletionItemProvider("typst", {
    provideCompletionItems: (model, position) => {
      const word  = model.getWordUntilPosition(position);
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber:   position.lineNumber,
        startColumn:     word.startColumn,
        endColumn:       word.endColumn,
      };
      const kw = (label: string, insertText: string): monaco.languages.CompletionItem => ({
        label, kind: monaco.languages.CompletionItemKind.Keyword, insertText, range,
      });
      const fn = (label: string, insertText: string): monaco.languages.CompletionItem => ({
        label, kind: monaco.languages.CompletionItemKind.Function, insertText, range,
      });
      return {
        suggestions: [
          kw("set",     "set "),
          kw("show",    "show "),
          kw("let",     "let "),
          kw("if",      "if "),
          kw("for",     "for  in "),
          kw("while",   "while "),
          kw("import",  "import "),
          kw("include", "include "),
          fn("page",    "page(width: auto, height: auto)"),
          fn("text",    "text(size: )"),
          fn("align",   "align(center)"),
          fn("image",   'image("")'),
          fn("lorem",   "lorem(50)"),
        ],
      };
    },
  });
}

// ─── UI Helpers ───────────────────────────────────────────────────
export function updateStatus(type: "success" | "error" | "compiling", message: string): void {
  const el = document.getElementById("compile-status");
  if (el) {
    el.className = `status-chip status ${type}`;
    el.textContent = message;
  }
}

function updateStatusFile(path: string | null): void {
  const el = document.getElementById("status-file");
  if (!el) return;
  el.textContent = path ? path.split(/[\\/]/).pop() || path : "未打开文件";
  el.setAttribute("title", path ?? "未打开文件");
}

function updateCursorStatus(line: number, column: number): void {
  const el = document.getElementById("status-cursor");
  if (el) el.textContent = `Ln ${line}, Col ${column}`;
}

function updatePageStatus(pageCount: number): void {
  const el = document.getElementById("status-pages");
  if (el) el.textContent = `${pageCount} page(s)`;
}

function updateZoomStatus(zoom: number): void {
  const el = document.getElementById("status-zoom-bar");
  if (el) el.textContent = `${Math.round(zoom * 100)}%`;
}

function clearDiagnostics(): void {
  document.querySelector(".diagnostics-panel")?.remove();
}

function showDiagnostics(errors: string[]): void {
  clearDiagnostics();
  const panel = document.createElement("div");
  panel.className = "diagnostics-panel";
  errors.forEach((err) => {
    const item = document.createElement("div");
    item.className = "diagnostic-item error";
    item.textContent = err;
    item.addEventListener("click", () => {
      // Jump to the error line if we can parse file:line from the message
      const match = err.match(/:(\d+):/);
      if (match && editor) {
        const line = parseInt(match[1], 10);
        editor.revealLineInCenter(line);
        editor.setPosition({ lineNumber: line, column: 1 });
        editor.focus();
      }
    });
    panel.appendChild(item);
  });
  document.getElementById("editor-pane")?.appendChild(panel);
}
