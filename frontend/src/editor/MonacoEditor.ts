import * as monaco from "monaco-editor";
import { invoke } from "@tauri-apps/api/core";

let editor: monaco.editor.IStandaloneCodeEditor | null = null;
let compileTimeout:    ReturnType<typeof setTimeout> | null = null;
let cursorSyncTimeout: ReturnType<typeof setTimeout> | null = null;
let selSyncTimeout:    ReturnType<typeof setTimeout> | null = null;
// Prevents sync loop when we programmatically move the editor cursor from a preview click
// Use a counter instead of boolean to handle nested operations correctly
let suppressCursorSync = 0;
let suppressProgrammaticCompile = false;

let currentFilePath         = "";
let previewEntryPath        = "";
let lastSuccessfulPageCount = 0;
let latestCompileRequestId  = 0;
let renderedPageHashes: number[] = [];
let tabsHost: HTMLDivElement | null = null;
let editorHost: HTMLDivElement | null = null;

interface OpenDocument {
  path: string;
  model: monaco.editor.ITextModel;
  viewState: monaco.editor.ICodeEditorViewState | null;
  savedContent: string;
  dirty: boolean;
}

const openDocuments = new Map<string, OpenDocument>();

const DEFAULT_CONTENT = `#set page(width: 10cm, height: auto)

= Hello, Typst!

This is a *live preview* editor built with _Tauri_ and _Typst_.

#lorem(50)
`;

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

  const untitledDoc = createDocument("untitled.typ", DEFAULT_CONTENT);

  editor = monaco.editor.create(editorHost, {
    model: untitledDoc.model,
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

  setCurrentFilePath(untitledDoc.path);
  renderTabs();

  // Initialize world with default content
  const result = await invoke<CompileResult>("open_file", {
    filePath: "untitled.typ",
    content: DEFAULT_CONTENT,
  });
  if (result.success) {
    previewEntryPath = "untitled.typ";
    lastSuccessfulPageCount = result.page_count;
    updatePageStatus(result.page_count);
    await renderAllPages(result.page_count, result.page_hashes);
    updateStatus("success", `Compiled · ${result.page_count} page(s)`);
  } else {
    updatePageStatus(0);
    updateStatus("error", result.errors.join("; "));
  }

  // ── Live compile (debounced 300 ms) ───────────────────────────
  editor.onDidChangeModelContent(() => {
    updateActiveDocumentState();
    if (suppressProgrammaticCompile) return;
    if (compileTimeout) clearTimeout(compileTimeout);
    compileTimeout = setTimeout(compileAndPreview, 300);
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
          syncCursorToPreview(e.position);
        }
      }, 120);
    }
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
      selSyncTimeout = setTimeout(() => syncSelectionToPreview(sel), 200);
    }
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
  const content = document.querySelector<HTMLElement>(".preview-content");
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
}

export async function compileAndPreview(): Promise<void> {
  if (!editor) return;
  if (!currentFilePath) return;
  const content   = editor.getValue();
  const requestId = ++latestCompileRequestId;
  updateStatus("compiling", "Compiling…");

  try {
    const result = await invoke<CompileResult>("compile_current", {
      filePath: currentFilePath,
      content,
    });
    if (requestId < latestCompileRequestId) return; // stale

    if (result.success) {
      lastSuccessfulPageCount = result.page_count;
      updatePageStatus(result.page_count);
      await renderAllPages(result.page_count, result.page_hashes);
      updateStatus("success",
        `Compiled in ${result.duration_ms ?? "?"}ms · ${result.page_count} page(s)`);
      clearDiagnostics();
    } else {
      updatePageStatus(0);
      updateStatus("error", `${result.errors.length} error(s)`);
      showDiagnostics(result.errors);
    }
  } catch (e) {
    updateStatus("error", `Error: ${e}`);
  }
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
    const result = path === previewEntryPath || !previewEntryPath
      ? await invoke<CompileResult>("open_file", { filePath: path, content })
      : await invoke<CompileResult>("compile_current", { filePath: path, content });
    if (!previewEntryPath) previewEntryPath = path;
    if (!result.success) {
      updatePageStatus(0);
      showDiagnostics(result.errors);
      updateStatus("error", result.errors.join("; "));
      return;
    }
    resetPreviewState();
    lastSuccessfulPageCount = result.page_count;
    updatePageStatus(result.page_count);
    await renderAllPages(result.page_count, result.page_hashes);
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

// ─── Bidirectional Sync ───────────────────────────────────────────

interface PagePos { page: number; x: number; y: number; }

/** Editor cursor → preview position indicator */
async function syncCursorToPreview(position: monaco.Position): Promise<void> {
  const model = editor?.getModel();
  if (!model) return;
  if (!currentFilePath) return;

  const offset = model.getOffsetAt(position);
  try {
    const positions = await invoke<PagePos[]>("jump_from_cursor", {
      filePath: currentFilePath,
      cursor: offset,
    });
    if (positions.length > 0) highlightCursorInPreview(positions[0]);
  } catch { /* ignore */ }
}

/** Editor text selection → preview range highlight */
async function syncSelectionToPreview(sel: monaco.Selection): Promise<void> {
  const model = editor?.getModel();
  if (!model) return;
  if (!currentFilePath) return;

  const startOff = model.getOffsetAt(sel.getStartPosition());
  const endOff   = model.getOffsetAt(sel.getEndPosition());
  if (startOff === endOff) return;

  try {
    const [startPositions, endPositions] = await Promise.all([
      invoke<PagePos[]>("jump_from_cursor", { filePath: currentFilePath, cursor: startOff }),
      invoke<PagePos[]>("jump_from_cursor", { filePath: currentFilePath, cursor: endOff }),
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
  const rect  = svgEl.getBoundingClientRect();
  const scaleX = parseFloat(svgEl.getAttribute("width")  || "1") / rect.width;
  const scaleY = parseFloat(svgEl.getAttribute("height") || "1") / rect.height;
  const ptX   = (e.clientX - rect.left) * scaleX;
  const ptY   = (e.clientY - rect.top)  * scaleY;

  try {
    const result = await invoke<{
      type: string;
      file?: string | null;
      offset?: number | null;
      url?: string | null;
      page?: number | null;
      x?: number | null;
      y?: number | null;
    }>("jump_from_click", { page: pageIndex + 1, x: ptX, y: ptY });

    if (result.type === "file" && result.offset != null && result.file && editor) {
      const targetPath = result.file;
      const targetDoc = openDocuments.get(targetPath);
      if (targetDoc) {
        await switchToDocument(targetDoc, false);
      } else {
        await openFileAtPath(targetPath);
      }

      const model = editor.getModel();
      if (model) {
        const pos = model.getPositionAt(result.offset);
        // Suppress cursor-sync so the editor jump doesn't echo back to the preview
        suppressCursorSync++;
        editor.setPosition(pos);
        editor.revealPositionInCenter(pos);
        editor.focus();
        setTimeout(() => { suppressCursorSync = Math.max(0, suppressCursorSync - 1); }, 50);
      }
    } else if (result.type === "url" && result.url) {
      window.open(result.url, "_blank", "noopener,noreferrer");
    } else if (result.type === "position" && result.page != null && result.x != null && result.y != null && editor) {
      // Position type: query the corresponding text offset and jump to it
      const positions = currentFilePath
        ? await invoke<PagePos[]>("jump_from_cursor", {
            filePath: currentFilePath,
            cursor: editor.getModel()?.getOffsetAt({ lineNumber: 1, column: 1 }) ?? 0,
          })
        : [];
      if (positions.length > 0) {
        // Find position on the target page, fallback to first position
        const targetPos = positions.find(p => p.page === result.page) ?? positions[0];
        const model = editor.getModel();
        if (model) {
          // Estimate offset from page position (fallback: use page * 1000 as approximation)
          const approxOffset = Math.max(0, (result.page! - 1) * 1000 + Math.floor(result.x!) + Math.floor(result.y!));
          const pos = model.getPositionAt(Math.min(approxOffset, model.getValueLength()));
          suppressCursorSync++;
          editor.setPosition(pos);
          editor.revealPositionInCenter(pos);
          editor.focus();
          setTimeout(() => { suppressCursorSync = Math.max(0, suppressCursorSync - 1); }, 50);
        }
      }
    }
  } catch { /* ignore */ }
}

// ─── Highlight Helpers ────────────────────────────────────────────

function removeSyncOverlays(cls: string): void {
  document.querySelectorAll(`.highlight-overlay.${cls}`).forEach(el => el.remove());
}

/**
 * Show a full-width horizontal highlight at the cursor position on the preview page.
 * Scroll the preview so the cursor position is centered.
 */
function highlightCursorInPreview(pos: PagePos): void {
  removeSyncOverlays("cursor-hl");

  const pane = document.getElementById("preview-pane");
  const svgs = pane?.querySelectorAll<SVGSVGElement>("svg");
  if (!svgs || svgs.length < pos.page) {
    console.warn(`[cursor sync] page ${pos.page} out of range (${svgs?.length ?? 0} pages rendered)`);
    return;
  }

  const svgEl    = svgs[pos.page - 1];
  const rect     = svgEl.getBoundingClientRect();
  const paneRect = pane!.getBoundingClientRect();
  const scaleY   = rect.height / parseFloat(svgEl.getAttribute("height") || "1");
  const screenY  = pos.y * scaleY;

  const overlay        = document.createElement("div");
  overlay.className    = "highlight-overlay cursor-hl";
  overlay.style.left   = `${rect.left - paneRect.left + pane!.scrollLeft}px`;
  overlay.style.top    = `${rect.top  - paneRect.top  + pane!.scrollTop + screenY - 12}px`;
  overlay.style.width  = `${rect.width}px`;
  overlay.style.height = "24px";

  pane!.querySelector(".preview-content")!.appendChild(overlay);

  // Smooth-scroll so the cursor line is roughly centred in the pane
  const targetTop = rect.top - paneRect.top + pane!.scrollTop + screenY - paneRect.height / 2;
  pane!.scrollTo({ top: Math.max(0, targetTop), behavior: "smooth" });

  setTimeout(() => overlay.classList.add("fade"), 1500);
  setTimeout(() => overlay.remove(), 2000);
}

/**
 * Draw a selection-range highlight from `start` to `end`, spanning pages if necessary.
 * Scroll the preview so the start of the selection is visible.
 * @param clearFirst - if true (default), clears existing selection overlays before drawing.
 *                     Set to false when drawing multiple ranges in a loop.
 */
function highlightRangeInPreview(start: PagePos, end: PagePos, clearFirst = true): void {
  if (clearFirst) removeSyncOverlays("selection-hl");

  const pane = document.getElementById("preview-pane");
  const svgs = pane?.querySelectorAll<SVGSVGElement>("svg");
  if (!svgs || svgs.length === 0) return;

  const paneRect  = pane!.getBoundingClientRect();
  const contentEl = pane!.querySelector<HTMLElement>(".preview-content");
  if (!contentEl) return;

  const firstPage = Math.min(start.page, end.page);
  const lastPage  = Math.max(start.page, end.page);

  for (let p = firstPage; p <= Math.min(lastPage, svgs.length); p++) {
    const svgEl = svgs[p - 1];
    if (!svgEl) continue;

    const rect  = svgEl.getBoundingClientRect();
    const svgH  = parseFloat(svgEl.getAttribute("height") || "1");
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
    overlay.style.left   = `${rect.left - paneRect.left + pane!.scrollLeft + 2}px`;
    overlay.style.top    = `${rect.top  - paneRect.top  + pane!.scrollTop  + topY}px`;
    overlay.style.width  = `${rect.width - 4}px`;
    overlay.style.height = `${height}px`;

    contentEl.appendChild(overlay);
  }

  // Scroll so the start of the selection is visible
  if (firstPage <= svgs.length) {
    const svgEl  = svgs[firstPage - 1];
    const rect   = svgEl.getBoundingClientRect();
    const scaleY = rect.height / parseFloat(svgEl.getAttribute("height") || "1");
    const targetTop = rect.top - paneRect.top + pane!.scrollTop + start.y * scaleY - 80;
    pane!.scrollTo({ top: Math.max(0, targetTop), behavior: "smooth" });
  }
}

// ─── Rendering ────────────────────────────────────────────────────
export async function renderAllPages(
  pageCount: number,
  pageHashes: number[]
): Promise<void> {
  const contentDiv = ensurePreviewContent();
  trimRemovedPages(contentDiv, pageCount);

  for (let i = 0; i < pageCount; i++) {
    const pageHash = pageHashes[i];
    const pageNode = ensurePageNode(contentDiv, i);
    if (renderedPageHashes[i] === pageHash) continue; // unchanged

    try {
      const svg = await invoke<string>("render_page", { page: i });
      // Discard if hash changed while awaiting (newer compile running)
      if (pageHash !== pageHashes[i]) continue;
      replacePageSvg(pageNode, svg, i);
      renderedPageHashes[i] = pageHash;
      pageNode.classList.add("page-updated");
      setTimeout(() => pageNode.classList.remove("page-updated"), 120);
    } catch { break; }
  }

  renderedPageHashes.length = pageCount;
}

function ensurePreviewContent(): HTMLDivElement {
  const pane = document.getElementById("preview-pane") as HTMLDivElement | null;
  if (!pane) throw new Error("Preview pane not found");
  let div = pane.querySelector<HTMLDivElement>(".preview-content");
  if (!div) {
    div = document.createElement("div");
    div.className = "preview-content";
    pane.appendChild(div);
  }
  return div;
}

function ensurePageNode(contentDiv: HTMLDivElement, index: number): HTMLDivElement {
  let node = contentDiv.querySelector<HTMLDivElement>(`[data-page-index="${index}"]`);
  if (!node) {
    node = document.createElement("div");
    node.className = "preview-page";
    node.dataset.pageIndex = String(index);
    contentDiv.appendChild(node);
  }
  return node;
}

function replacePageSvg(node: HTMLDivElement, svg: string, pageIndex: number): void {
  const wrapper     = document.createElement("div");
  wrapper.innerHTML = svg;
  const svgEl       = wrapper.firstElementChild as SVGSVGElement | null;
  if (!svgEl) return;
  svgEl.addEventListener("click", (e) => handlePreviewClick(e, svgEl, pageIndex));
  node.replaceChildren(svgEl);
}

function trimRemovedPages(contentDiv: HTMLDivElement, pageCount: number): void {
  contentDiv.querySelectorAll<HTMLDivElement>(".preview-page").forEach((n) => {
    if (Number(n.dataset.pageIndex ?? -1) >= pageCount) n.remove();
  });
}

function resetPreviewState(): void {
  latestCompileRequestId++;
  renderedPageHashes = [];
  document.querySelector(".preview-content")?.replaceChildren();
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
    const untitledDoc = createDocument("untitled.typ", DEFAULT_CONTENT);
    await switchToDocument(untitledDoc);
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
    const currentRoot = await invoke<string | null>("get_project_root").catch(() => null);
    const targetRoot = await invoke<string | null>("resolve_project_root", { filePath: doc.path }).catch(() => null);
    const shouldOpenAsEntry = !previewEntryPath || !currentRoot || !targetRoot || currentRoot !== targetRoot;
    const result = shouldOpenAsEntry
      ? await invoke<CompileResult>("open_file", { filePath: doc.path, content })
      : await invoke<CompileResult>("compile_current", { filePath: doc.path, content });

    if (shouldOpenAsEntry) previewEntryPath = doc.path;

    if (result.success) {
      lastSuccessfulPageCount = result.page_count;
      updatePageStatus(result.page_count);
      await renderAllPages(result.page_count, result.page_hashes);
      updateStatus("success", `Compiled · ${result.page_count} page(s)`);
      clearDiagnostics();
    } else {
      updatePageStatus(0);
      updateStatus("error", result.errors.join("; "));
      showDiagnostics(result.errors);
    }
  } catch (e) {
    updatePageStatus(0);
    updateStatus("error", `Failed to compile: ${e}`);
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

    tabsHost.appendChild(tab);
  }
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
