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
  setOutlineItems,
  updateOutlineActiveItem,
} from "../outline/OutlinePanel";
import { showEmptyWorkspace, showWorkspaceLayout } from "../workspace/WorkspaceShell";

let editor: monaco.editor.IStandaloneCodeEditor | null = null;
let onFileChangeCb: ((path: string | null) => void) | null = null;
let onPreviewEntryChangeCb: ((path: string | null) => void) | null = null;

/** Register a callback that fires whenever the active file path changes. */
export function onEditorFileChange(cb: (path: string | null) => void): void {
  onFileChangeCb = cb;
}

/** Register a callback that fires whenever the preview entry path changes. */
export function onPreviewEntryChange(cb: (path: string | null) => void): void {
  onPreviewEntryChangeCb = cb;
}
let compileTimeout:    ReturnType<typeof setTimeout> | null = null;
let cursorSyncTimeout: ReturnType<typeof setTimeout> | null = null;
let compileStatusTimeout: ReturnType<typeof setTimeout> | null = null;
// Prevents sync loop when we programmatically move the editor cursor from a preview click
// Use a counter instead of boolean to handle nested operations correctly
let suppressCursorSync = 0;
let suppressProgrammaticCompile = false;
// Tracks last user edit timestamp — used to suppress expensive cursor sync during active typing
let lastEditTime = 0;
// Tracks the last line number synced to the outline — outline sync skips column-only changes
let lastOutlineSyncLine = -1;

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
let previewHoverThrottleTimeout: ReturnType<typeof setTimeout> | null = null;
let sourceHoverDecorationIds: string[] = [];
let sourceHoverTarget: { file: string; offset: number } | null = null;
let latestCursorRequestId = 0;
let lastCursorPreviewPage: number | null = null;
// Optimized offset cache with line-level granularity
let offsetCache:
  | {
    model: monaco.editor.ITextModel;
    versionId: number;
    lineStartChars: number[];
    lineStartBytes: number[];
    eol: string;
    eolCharLength: number;
    eolByteLength: number;
    // Track which lines are dirty and need recalculation
    dirtyLines: Set<number>;
    // Total character and byte count for quick bounds checking
    totalChars: number;
    totalBytes: number;
  }
  | null = null;

interface OpenDocument {
  path: string;
  model: monaco.editor.ITextModel;
  viewState: monaco.editor.ICodeEditorViewState | null;
  savedContent: string;
  /** Monaco alternative version ID at the time of last save — O(1) dirty check */
  savedVersionId: number;
  dirty: boolean;
}

const openDocuments = new Map<string, OpenDocument>();

type CompileCommand = "compile_current" | "open_file" | "set_preview_entry";

interface CompileJob {
  intentId: number;
  command: CompileCommand;
  filePath: string;
  content: string;
  savedContent: string;
  successMessage: (result: CompileResult) => string;
  preservePagesOnError?: boolean;
  notifyPreviewEntryChange?: boolean;
}

let activeCompileJob: Promise<void> | null = null;
let pendingCompileJob: CompileJob | null = null;

function scheduleCursorPreviewSync(delay = 120): void {
  if (cursorSyncTimeout) clearTimeout(cursorSyncTimeout);
  cursorSyncTimeout = setTimeout(() => {
    const currentEditor = editor;
    const sel = currentEditor?.getSelection();
    const pos = currentEditor?.getPosition();
    if (!currentEditor || !sel || !pos) return;
    removeSyncOverlays("selection-hl");
    void syncCursorToPreview(pos);
  }, delay);
}

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
export function setEditorTheme(theme: "light" | "dark"): void {
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
    // "keep" just preserves the current line's indentation on Enter — far cheaper than
    // "full" which runs a full syntax analysis pass on every keystroke
    autoIndent: "keep",
    renderLineHighlight: "line",
    fontLigatures: true,
    smoothScrolling: true,
    cursorSmoothCaretAnimation: "on",
    renderWhitespace: "none",
    // Bracket colorization requires a full bracket-matching pass on every edit;
    // the Typst tokenizer doesn't benefit from it enough to justify the cost
    bracketPairColorization: { enabled: false },
    occurrencesHighlight: "off",
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

  // ── Cursor → Preview (double-click only) ────
  // Sync preview position only on double-click to avoid performance impact
  let lastMouseUpTime = 0;
  editor.onDidChangeCursorPosition((e: monaco.editor.ICursorPositionChangedEvent) => {
    updateCursorStatus(e.position.lineNumber, e.position.column);
    // Outline heading boundaries are line-level — skip the O(n) byte-offset conversion
    // when only the column changed (e.g. typing characters on the same line)
    if (e.position.lineNumber !== lastOutlineSyncLine) {
      lastOutlineSyncLine = e.position.lineNumber;
      syncOutlineActiveFromEditor();
    }
  });

  // Double-click handler for cursor sync
  editor.onMouseUp((e: monaco.editor.IEditorMouseEvent) => {
    const now = Date.now();
    if (now - lastMouseUpTime < 300 && e.event.leftButton) {
      // Double-click detected
      const pos = editor?.getPosition();
      if (pos && !editor?.getSelection()?.isEmpty()) {
        // Only sync on double-click with selection
        scheduleCursorPreviewSync(0); // Immediate sync on double-click
      }
    }
    lastMouseUpTime = now;
  });

  const initialPos = editor.getPosition();
  if (initialPos) updateCursorStatus(initialPos.lineNumber, initialPos.column);

  // ── Keyboard shortcuts ─────────────────────────────────────────
  editor.addAction({
    id: "save-file",
    label: "保存文件",
    keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS],
    run: () => saveCurrentFile(),
  });
  editor.addAction({
    id: "force-compile",
    label: "强制编译",
    keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter],
    run: () => compileAndPreview(),
  });
  editor.addAction({
    id: "export-pdf",
    label: "导出 PDF",
    keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.KeyE],
    run: () => exportPdf(),
  });
  editor.addAction({
    id: "open-file",
    label: "打开文件",
    keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyO],
    run: () => openFile(),
  });
  editor.addAction({
    id: "close-file-tab",
    label: "关闭文件标签",
    keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyW],
    run: () => closeCurrentTab(),
  });
  editor.addAction({
    id: "set-preview-entry",
    label: "设置预览入口",
    keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.Enter],
    run: () => setCurrentDocumentAsPreviewEntry(),
  });
  editor.addAction({
    id: "clear-preview-entry-pin",
    label: "清除预览入口固定",
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
  onFileChangeCb?.(path);
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
  outline: import("../outline/OutlinePanel").OutlineItem[];
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

function enqueueCompile(job: Omit<CompileJob, "intentId">): void {
  pendingCompileJob = {
    ...job,
    intentId: ++latestCompileRequestId,
  };
  void drainCompileQueue();
}

async function drainCompileQueue(): Promise<void> {
  if (activeCompileJob || !pendingCompileJob) return;

  const job = pendingCompileJob;
  pendingCompileJob = null;
  activeCompileJob = runCompileJob(job).finally(() => {
    activeCompileJob = null;
    if (pendingCompileJob) {
      void drainCompileQueue();
    }
  });

  await activeCompileJob;
}

async function runCompileJob(job: CompileJob): Promise<void> {
  if (compileStatusTimeout) clearTimeout(compileStatusTimeout);
  compileStatusTimeout = setTimeout(() => {
    if (job.intentId === latestCompileRequestId) {
      updateStatus("compiling", "Syncing preview…");
    }
  }, 180);

  try {
    const result = await invoke<CompileResult>(job.command, {
      filePath: job.filePath,
      content: job.content,
    });

    // Always apply the result - if intentId doesn't match anymore, applyCompileResult
    // will handle it by only updating essential state (previewEntryPath, page count, etc.)
    // without triggering UI updates that might conflict with a newer request.
    await applyCompileResult(job, result);
  } catch (e) {
    // For errors, always apply the result even if intentId changed,
    // because the error state needs to be displayed to the user.
    if (job.intentId !== latestCompileRequestId) {
      finalizeCompileStatusTimeout();
      removeSyncOverlays("unsaved-add-hl");
      updateStatus("error", `错误: ${e}`);
      return;
    }
    finalizeCompileStatusTimeout();
    removeSyncOverlays("unsaved-add-hl");
    if (!job.preservePagesOnError) {
      updatePageStatus(0);
      clearOutlinePanel();
    }
    updateStatus("error", `错误: ${e}`);
  }
}

async function applyCompileResult(job: CompileJob, result: CompileResult): Promise<void> {
  finalizeCompileStatusTimeout();

  if (result.success) {
    const oldPreviewEntryPath = previewEntryPath;
    const entryPathChanged = result.entry_path != null && result.entry_path !== previewEntryPath;
    previewEntryPath = result.entry_path ?? previewEntryPath;
    lastSuccessfulPageCount = result.page_count;
    updatePageStatus(result.page_count);
    await renderAllPages(result.page_count, result.page_hashes);
    if (job.intentId !== latestCompileRequestId) return;
    await renderUnsavedInsertHighlights(job.filePath, job.savedContent, job.content);
    if (job.intentId !== latestCompileRequestId) return;
    setOutlineItems(result.outline ?? []);
    syncOutlineActiveFromEditor();
    updateStatus("success", job.successMessage(result));
    clearDiagnostics();
    if (job.notifyPreviewEntryChange && entryPathChanged) {
      onPreviewEntryChangeCb?.(previewEntryPath);
    }
    return;
  }

  if (isTransientCompileResult(result)) {
    updateStatus("compiling", "正在同步预览…");
    return;
  }

  removeSyncOverlays("unsaved-add-hl");
  if (!job.preservePagesOnError) {
    updatePageStatus(0);
    clearOutlinePanel();
  }
  updateStatus("error", `${result.errors.length} 个错误`);
  showDiagnostics(result.errors);
}

function finalizeCompileStatusTimeout(): void {
  if (!compileStatusTimeout) return;
  clearTimeout(compileStatusTimeout);
  compileStatusTimeout = null;
}

export async function compileAndPreview(): Promise<void> {
  if (!editor) return;
  if (!currentFilePath) return;
  const activeDoc = getActiveDocument();
  enqueueCompile({
    command: "compile_current",
    filePath: currentFilePath,
    content: editor.getValue(),
    savedContent: activeDoc?.savedContent ?? editor.getValue(),
    preservePagesOnError: false,
    notifyPreviewEntryChange: true,
    successMessage: (result) => `Compiled in ${result.duration_ms ?? "?"}ms · ${result.page_count} page(s)`,
  });
}

function scheduleCompile(delay = 650): void {
  if (compileTimeout) clearTimeout(compileTimeout);
  compileTimeout = setTimeout(() => {
    compileTimeout = null;
    void compileAndPreview();
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
      updateStatus("error", `保存对话框失败: ${e}`);
      return;
    }
  }

  const content = editor.getValue();
  try {
    await invoke("save_file", { content, filePath: path });
    if (activeDoc) {
      activeDoc.savedContent = content;
      activeDoc.savedVersionId = activeDoc.model.getAlternativeVersionId();
      activeDoc.dirty = false;
      renderTabs();
    }
    updateStatus("success", "已保存");
    enqueueCompile({
      command: "compile_current",
      filePath: path,
      content,
      savedContent: content,
      preservePagesOnError: true,
      notifyPreviewEntryChange: true,
      successMessage: () => "已保存",
    });
  } catch (e) {
    updateStatus("error", `保存失败: ${e}`);
  }
}

async function exportPdf(): Promise<void> {
  try {
    const result = await invoke<"Success" | "Cancelled">("export_pdf_dialog");
    if (result !== "Cancelled") updateStatus("success", "PDF 已导出");
  } catch (e) {
    updateStatus("error", `导出失败: ${e}`);
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
  const charOffset = utf8ByteOffsetToMonacoOffset(model, offset);
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
  const charOffset = utf8ByteOffsetToMonacoOffset(model, offset);
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

interface TextRange {
  startLine: number;
  startColumn: number;
  endLine: number;
  endColumn: number;
}

function splitLinesPreserveFinalEmpty(text: string): string[] {
  return text.split(/\r\n|\n|\r/);
}

function offsetToLineColumn(text: string, offset: number): { line: number; column: number } {
  const clampedOffset = Math.max(0, Math.min(offset, text.length));
  const lines = splitLinesPreserveFinalEmpty(text);
  let remaining = clampedOffset;

  for (let index = 0; index < lines.length; index++) {
    const line = lines[index] ?? "";
    if (remaining <= line.length) {
      return { line: index + 1, column: remaining + 1 };
    }
    remaining -= line.length;
    if (index < lines.length - 1) {
      if (remaining === 0) {
        return { line: index + 1, column: line.length + 1 };
      }
      remaining -= 1;
    }
  }

  const lastLine = lines[lines.length - 1] ?? "";
  return { line: Math.max(1, lines.length), column: lastLine.length + 1 };
}

function computeUnsavedChangedTextRanges(savedContent: string, compiledContent: string): TextRange[] {
  if (savedContent === compiledContent) return [];

  let prefix = 0;
  const sharedPrefixLimit = Math.min(savedContent.length, compiledContent.length);
  while (prefix < sharedPrefixLimit && savedContent[prefix] === compiledContent[prefix]) {
    prefix++;
  }

  let savedSuffix = savedContent.length;
  let compiledSuffix = compiledContent.length;
  while (
    savedSuffix > prefix &&
    compiledSuffix > prefix &&
    savedContent[savedSuffix - 1] === compiledContent[compiledSuffix - 1]
  ) {
    savedSuffix--;
    compiledSuffix--;
  }

  if (compiledSuffix <= prefix) return [];

  const start = offsetToLineColumn(compiledContent, prefix);
  const end = offsetToLineColumn(compiledContent, compiledSuffix);
  return [{
    startLine: start.line,
    startColumn: start.column,
    endLine: end.line,
    endColumn: end.column,
  }];
}

async function renderUnsavedInsertHighlights(
  filePath: string,
  savedContent: string,
  compiledContent: string,
): Promise<void> {
  removeSyncOverlays("unsaved-add-hl");
  if (!currentFilePath || currentFilePath !== filePath || lastSuccessfulPageCount === 0) return;

  const ranges = computeUnsavedChangedTextRanges(savedContent, compiledContent);
  if (ranges.length === 0) return;

  for (let rangeIndex = 0; rangeIndex < ranges.length; rangeIndex++) {
    const range = ranges[rangeIndex];
    try {
      const [startPositions, endPositions] = await Promise.all([
        invoke<PagePos[]>("jump_from_cursor", {
          filePath,
          line: range.startLine - 1,
          character: Math.max(0, range.startColumn - 1),
        }),
        invoke<PagePos[]>("jump_from_cursor", {
          filePath,
          line: range.endLine - 1,
          character: Math.max(0, range.endColumn - 1),
        }),
      ]);

      if (startPositions.length === 0 || endPositions.length === 0) continue;

      const maxLen = Math.max(startPositions.length, endPositions.length);
      for (let idx = 0; idx < maxLen; idx++) {
        const start = startPositions[idx] ?? startPositions[0];
        const end = endPositions[idx] ?? endPositions[0];
        highlightRangeInPreview(start, end, {
          clearFirst: idx === 0 && rangeIndex === 0,
          overlayClass: "unsaved-add-hl",
          scroll: false,
        });
      }
    } catch {
      // ignore individual unsaved highlight mapping failures
    }
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
 * Uses cached line start indices so large documents don't rescan the whole text.
 */
function utf8ByteOffsetToMonacoOffset(model: monaco.editor.ITextModel, byteOffset: number): number {
  const cache = getOffsetCache(model);
  const lineIndex = findFloorIndex(cache.lineStartBytes, byteOffset);
  const lineNumber = lineIndex + 1;
  const lineText = model.getLineContent(lineNumber);
  const lineStartChar = cache.lineStartChars[lineIndex] ?? 0;
  const lineStartByte = cache.lineStartBytes[lineIndex] ?? 0;
  const lineByteLength = utf8BytesForText(lineText);
  const localByteOffset = Math.max(0, byteOffset - lineStartByte);

  if (localByteOffset <= lineByteLength) {
    return lineStartChar + utf8ByteOffsetInTextToCharOffset(lineText, localByteOffset);
  }

  const eolByteOffset = Math.min(localByteOffset - lineByteLength, cache.eolByteLength);
  return lineStartChar + lineText.length + utf8ByteOffsetInTextToCharOffset(cache.eol, eolByteOffset);
}

/**
 * Convert a Monaco/JS character offset to a UTF-8 byte offset (for Rust/Typst).
 */
function monacoOffsetToUtf8ByteOffset(model: monaco.editor.ITextModel, charOffset: number): number {
  const cache = getOffsetCache(model);
  const lineIndex = findFloorIndex(cache.lineStartChars, charOffset);
  const lineNumber = lineIndex + 1;
  const lineText = model.getLineContent(lineNumber);
  const lineStartChar = cache.lineStartChars[lineIndex] ?? 0;
  const lineStartByte = cache.lineStartBytes[lineIndex] ?? 0;
  const localCharOffset = Math.max(0, charOffset - lineStartChar);

  if (localCharOffset <= lineText.length) {
    return lineStartByte + utf8BytesForText(lineText.slice(0, localCharOffset));
  }

  const eolCharOffset = Math.min(localCharOffset - lineText.length, cache.eolCharLength);
  return lineStartByte + utf8BytesForText(lineText) + utf8BytesForText(cache.eol.slice(0, eolCharOffset));
}

function getOffsetCache(model: monaco.editor.ITextModel): NonNullable<typeof offsetCache> {
  const versionId = model.getVersionId();
  const lineCount = model.getLineCount();
  const eol = model.getEOL();
  const eolCharLength = eol.length;
  const eolByteLength = utf8BytesForText(eol);

  // Check if we can reuse existing cache
  if (offsetCache && offsetCache.model === model && offsetCache.versionId === versionId) {
    // Cache is still valid - ensure arrays are the right size
    if (offsetCache.lineStartChars.length === lineCount) {
      return offsetCache;
    }
    // Size mismatch (lines added/removed), need to rebuild
  }

  // Build or rebuild cache
  const lineStartChars = new Array<number>(lineCount);
  const lineStartBytes = new Array<number>(lineCount);
  let charCursor = 0;
  let byteCursor = 0;

  for (let i = 1; i <= lineCount; i++) {
    const index = i - 1;
    lineStartChars[index] = charCursor;
    lineStartBytes[index] = byteCursor;
    const lineText = model.getLineContent(i);
    charCursor += lineText.length;
    byteCursor += utf8BytesForText(lineText);
    if (i !== lineCount) {
      charCursor += eolCharLength;
      byteCursor += eolByteLength;
    }
  }

  offsetCache = {
    model,
    versionId,
    lineStartChars,
    lineStartBytes,
    eol,
    eolCharLength,
    eolByteLength,
    dirtyLines: new Set(),
    totalChars: charCursor,
    totalBytes: byteCursor
  };
  return offsetCache;
}

function utf8BytesForText(text: string): number {
  let bytes = 0;
  for (let i = 0; i < text.length; i++) {
    const code = text.charCodeAt(i);
    if (code < 0x80) {
      bytes += 1;
    } else if (code < 0x800) {
      bytes += 2;
    } else if (code >= 0xD800 && code < 0xDC00) {
      bytes += 4;
      i++;
    } else {
      bytes += 3;
    }
  }
  return bytes;
}

function utf8ByteOffsetInTextToCharOffset(text: string, byteOffset: number): number {
  let bytes = 0;
  let i = 0;
  while (i < text.length && bytes < byteOffset) {
    const code = text.charCodeAt(i);
    if (code < 0x80) {
      bytes += 1;
    } else if (code < 0x800) {
      bytes += 2;
    } else if (code >= 0xD800 && code < 0xDC00) {
      bytes += 4;
      i++;
    } else {
      bytes += 3;
    }
    i++;
  }
  return i;
}

function findFloorIndex(values: number[], target: number): number {
  let low = 0;
  let high = values.length - 1;
  let best = 0;

  while (low <= high) {
    const mid = (low + high) >> 1;
    if (values[mid] <= target) {
      best = mid;
      low = mid + 1;
    } else {
      high = mid - 1;
    }
  }

  return best;
}

// ─── Bidirectional Sync ───────────────────────────────────────────

interface PagePos { page: number; x: number; y: number; }

/** Editor cursor → preview position indicator */
async function syncCursorToPreview(position: monaco.Position): Promise<void> {
  const model = editor?.getModel();
  if (!model) return;
  if (!currentFilePath) return;
  // Skip sync if no compiled output exists yet or pages aren't rendered yet
  if (lastSuccessfulPageCount === 0) return;
  if (renderedPageHashes.length === 0) return;

  const requestId = ++latestCursorRequestId;
  try {
    const positions = await invoke<PagePos[]>("jump_from_cursor", {
      filePath: currentFilePath,
      line: position.lineNumber - 1,
      character: position.column - 1,
    });
    // Discard stale responses
    if (requestId !== latestCursorRequestId) return;
    const previewPos = pickBestPreviewPosition(positions);
    if (previewPos && previewPos.page <= renderedPageHashes.length) {
      const shouldScroll = lastCursorPreviewPage == null || lastCursorPreviewPage !== previewPos.page;
      lastCursorPreviewPage = previewPos.page;
      highlightCursorInPreview(previewPos, {
        scroll: shouldScroll,
        behavior: "auto",
        persistent: true,
      });
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
      await revealSourceOffsetInPreview(result.file, result.offset, "auto");
    } else if (result.type === "url" && result.url) {
      window.open(result.url, "_blank", "noopener,noreferrer");
    } else if (result.type === "position" && result.position) {
      scrollPreviewToPosition(result.position);
    }
  } catch { /* ignore */ }
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

  // Throttle hover events to avoid excessive backend calls
  if (previewHoverThrottleTimeout) clearTimeout(previewHoverThrottleTimeout);
  previewHoverThrottleTimeout = setTimeout(async () => {
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
  }, 50); // 50ms throttle delay
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
    scrollPreviewToPosition(pos, options.behavior ?? "auto");
  }
  if (!options.persistent) {
    setTimeout(() => overlay.classList.add("fade"), 1500);
    setTimeout(() => overlay.remove(), 2000);
  }
}

interface PreviewPageMetrics {
  pageLeft: number;
  pageTop: number;
  pageWidth: number;
  pageHeight: number;
  x: number;
  y: number;
}

function getPreviewPageMetrics(pos: PagePos): PreviewPageMetrics | null {
  const svgEl = getPreviewPageSvg(pos.page);
  const panePoint = pagePointToPanePoint(pos);
  const pageOrigin = pagePointToPanePoint({ page: pos.page, x: 0, y: 0 });
  if (!svgEl || !panePoint || !pageOrigin) return null;

  const rect = svgEl.getBoundingClientRect();
  return {
    pageLeft: pageOrigin.left,
    pageTop: pageOrigin.top,
    pageWidth: rect.width,
    pageHeight: rect.height,
    x: panePoint.left,
    y: panePoint.top,
  };
}

function appendRangeOverlay(
  overlayClass: string,
  left: number,
  top: number,
  width: number,
  height: number,
): void {
  const contentEl = getPreviewContent();
  if (!contentEl) return;

  const overlay = document.createElement("div");
  overlay.className = `highlight-overlay ${overlayClass}`;
  overlay.style.left = `${Math.max(0, left)}px`;
  overlay.style.top = `${Math.max(0, top)}px`;
  overlay.style.width = `${Math.max(6, width)}px`;
  overlay.style.height = `${Math.max(16, height)}px`;
  contentEl.appendChild(overlay);
}

/**
 * Draw a selection-range highlight from `start` to `end`, spanning pages if necessary.
 * Scroll the preview so the start of the selection is visible.
 */
function highlightRangeInPreview(
  start: PagePos,
  end: PagePos,
  options: {
    clearFirst?: boolean;
    overlayClass?: string;
    scroll?: boolean;
  } = {},
): void {
  const clearFirst = options.clearFirst ?? true;
  const overlayClass = options.overlayClass ?? "selection-hl";
  const scroll = options.scroll ?? true;
  if (clearFirst) removeSyncOverlays(overlayClass);

  const pageCount = getPreviewContent()?.querySelectorAll<SVGSVGElement>("svg").length ?? 0;
  if (pageCount === 0) return;

  const firstPage = Math.min(start.page, end.page);
  const lastPage = Math.max(start.page, end.page);
  const startMetrics = getPreviewPageMetrics(start);
  const endMetrics = getPreviewPageMetrics(end);
  if (!startMetrics || !endMetrics) return;
  const linePaddingTop = 12;
  const linePaddingBottom = 8;
  const lineHeight = 20;

  for (let page = firstPage; page <= Math.min(lastPage, pageCount); page++) {
    const pageMetrics = getPreviewPageMetrics({ page, x: 0, y: 0 });
    if (!pageMetrics) continue;

    if (firstPage === lastPage) {
      const sameLine = Math.abs(endMetrics.y - startMetrics.y) < lineHeight;
      if (sameLine) {
        appendRangeOverlay(
          overlayClass,
          startMetrics.x,
          startMetrics.y - linePaddingTop,
          endMetrics.x - startMetrics.x,
          lineHeight + linePaddingTop + linePaddingBottom,
        );
      } else {
        appendRangeOverlay(
          overlayClass,
          startMetrics.x,
          startMetrics.y - linePaddingTop,
          pageMetrics.pageLeft + pageMetrics.pageWidth - startMetrics.x,
          lineHeight + linePaddingTop + linePaddingBottom,
        );
        if (endMetrics.y - startMetrics.y > lineHeight) {
          appendRangeOverlay(
            overlayClass,
            pageMetrics.pageLeft + 2,
            startMetrics.y + linePaddingBottom,
            pageMetrics.pageWidth - 4,
            endMetrics.y - startMetrics.y - lineHeight,
          );
        }
        appendRangeOverlay(
          overlayClass,
          pageMetrics.pageLeft + 2,
          endMetrics.y - linePaddingTop,
          endMetrics.x - pageMetrics.pageLeft - 2,
          lineHeight + linePaddingTop + linePaddingBottom,
        );
      }
      break;
    }

    if (page === firstPage) {
      appendRangeOverlay(
        overlayClass,
        startMetrics.x,
        startMetrics.y - linePaddingTop,
        pageMetrics.pageLeft + pageMetrics.pageWidth - startMetrics.x,
        lineHeight + linePaddingTop + linePaddingBottom,
      );
      appendRangeOverlay(
        overlayClass,
        pageMetrics.pageLeft + 2,
        startMetrics.y + linePaddingBottom,
        pageMetrics.pageWidth - 4,
        pageMetrics.pageTop + pageMetrics.pageHeight - startMetrics.y - linePaddingBottom,
      );
      continue;
    }

    if (page === lastPage) {
      appendRangeOverlay(
        overlayClass,
        pageMetrics.pageLeft + 2,
        pageMetrics.pageTop,
        pageMetrics.pageWidth - 4,
        Math.max(0, endMetrics.y - pageMetrics.pageTop - linePaddingTop),
      );
      appendRangeOverlay(
        overlayClass,
        pageMetrics.pageLeft + 2,
        endMetrics.y - linePaddingTop,
        endMetrics.x - pageMetrics.pageLeft - 2,
        lineHeight + linePaddingTop + linePaddingBottom,
      );
      continue;
    }

    appendRangeOverlay(
      overlayClass,
      pageMetrics.pageLeft + 2,
      pageMetrics.pageTop,
      pageMetrics.pageWidth - 4,
      pageMetrics.pageHeight,
    );
  }

  if (scroll) {
    scrollPreviewToPosition(start, "auto");
  }
}

function pickBestPreviewPosition(positions: PagePos[]): PagePos | null {
  if (positions.length === 0) return null;
  return positions[positions.length - 1] ?? positions[0] ?? null;
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
  const charStart = utf8ByteOffsetToMonacoOffset(model, offset);
  const charEnd   = utf8ByteOffsetToMonacoOffset(model, offset + 1);
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
  if (previewHoverThrottleTimeout) {
    clearTimeout(previewHoverThrottleTimeout);
    previewHoverThrottleTimeout = null;
  }
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
  const byteOffset = monacoOffsetToUtf8ByteOffset(model, charOffset);
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
  lastCursorPreviewPage = null;
  clearPreviewSourceHover();
  removeSyncOverlays("cursor-hl");
  removeSyncOverlays("selection-hl");
  removeSyncOverlays("unsaved-add-hl");
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
    savedVersionId: model.getAlternativeVersionId(),
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
  // O(1) version counter — avoids full string copy + comparison on every keystroke
  const wasDirty = doc.dirty;
  doc.dirty = doc.model.getAlternativeVersionId() !== doc.savedVersionId;
  // Only touch the DOM when dirty state actually flips (first edit, or after save)
  if (doc.dirty !== wasDirty) renderTabs();
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
  lastOutlineSyncLine = -1; // force outline sync on next cursor event for this document
  updateActiveDocumentState();
  syncOutlineActiveFromEditor();
  if (sourceHoverTarget?.file === doc.path) {
    applySourceHoverDecoration(sourceHoverTarget.file, sourceHoverTarget.offset);
  } else {
    clearSourceHoverDecoration();
  }
  if (refreshPreview) {
    resetPreviewState();
    compileOpenDocument(doc);
  }
}

async function closeCurrentTab(): Promise<void> {
  const doc = getActiveDocument();
  if (!doc) return;
  await closeDocument(doc.path);
}

/**
 * Close a document and return true if successful, false if cancelled by user.
 * This function is exported for use by FileTree during rename/delete operations.
 */
async function closeDocument(path: string): Promise<boolean> {
  const doc = openDocuments.get(path);
  if (!doc) return true;

  if (doc.dirty && !window.confirm(`"${doc.path.split(/[\\/]/).pop()}" 有未保存修改，确定关闭吗？`)) {
    return false; // User cancelled
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
    return true;
  }

  if (wasActive) {
    const remainingDocs = Array.from(openDocuments.values());
    const nextDoc = remainingDocs[Math.max(0, Math.min(currentIndex, remainingDocs.length - 1))];
    await switchToDocument(nextDoc);
  } else {
    renderTabs();
  }
  return true;
}

/**
 * Close all documents under a given directory path (including subdirectories).
 * Returns true if all documents were closed successfully, false if any closure was cancelled.
 * This function is exported for use by FileTree during directory rename/delete operations.
 */
export async function closeDocumentsUnderPath(dirPath: string): Promise<boolean> {
  const sep = dirPath.includes("\\") ? "\\" : "/";
  const normalizedDirPath = dirPath.endsWith(sep) ? dirPath : dirPath + sep;

  // Find all documents under this directory
  const docsToClose: string[] = [];
  for (const [path, doc] of openDocuments.entries()) {
    const normalizedDocPath = path.endsWith(sep) ? path : path + sep;
    if (normalizedDocPath.startsWith(normalizedDirPath)) {
      docsToClose.push(path);
    }
  }

  // Close documents in reverse order (so the currently open one is closed last)
  for (const path of docsToClose.reverse()) {
    const closed = await closeDocument(path);
    if (!closed) {
      // User cancelled - abort the operation
      return false;
    }
  }

  return true;
}

function compileOpenDocument(doc: OpenDocument): void {
  const content = doc.model.getValue();
  showWorkspaceLayout();
  enqueueCompile({
    command: previewEntryPath ? "compile_current" : "open_file",
    filePath: doc.path,
    content,
    savedContent: doc.savedContent,
    preservePagesOnError: false,
    notifyPreviewEntryChange: true,
    successMessage: (result) => `Compiled · ${result.page_count} page(s)`,
  });
}

async function setCurrentDocumentAsPreviewEntry(): Promise<void> {
  if (!editor || !currentFilePath) return;
  const activeDoc = getActiveDocument();
  const content = editor.getValue();

  resetPreviewState();
  showWorkspaceLayout();
  enqueueCompile({
    command: "set_preview_entry",
    filePath: currentFilePath,
    content,
    savedContent: activeDoc?.savedContent ?? content,
    preservePagesOnError: false,
    notifyPreviewEntryChange: true,
    successMessage: () => "Preview entry updated",
  });
}

export async function setPreviewEntryAtPath(path: string): Promise<void> {
  if (!editor) return;

  const targetDoc = openDocuments.get(path);
  if (targetDoc) {
    if (currentFilePath !== path) {
      await switchToDocument(targetDoc, false);
    }
  } else {
    await openFileAtPath(path);
  }

  await setCurrentDocumentAsPreviewEntry();
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
    const isPreviewEntry = doc.path === previewEntryPath;
    const tab = document.createElement("button");
    tab.className = `editor-tab${doc.path === currentFilePath ? " active" : ""}${isPreviewEntry ? " preview-entry" : ""}`;
    tab.title = doc.path;

    const label = document.createElement("span");
    label.className = "editor-tab-label";
    label.textContent = doc.path.split(/[\\/]/).pop() || doc.path;
    tab.appendChild(label);

    if (isPreviewEntry) {
      const star = document.createElement("span");
      star.className = "editor-tab-star";
      star.innerHTML = `<svg width="11" height="11" viewBox="0 0 12 12" fill="none" xmlns="http://www.w3.org/2000/svg">
        <path d="M6 1l1.5 3h3.3l-2.6 2 1 3.3L6 8l-3.2 1.3 1-3.3-2.6-2h3.3L6 1Z" fill="currentColor"/>
      </svg>`;
      star.title = "主文件";
      tab.appendChild(star);
    }

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

  const isMainFile = path === previewEntryPath;

  if (isMainFile) {
    // File is already the main file - show only "取消主文件固定"
    const clearMain = document.createElement("button");
    clearMain.type = "button";
    clearMain.className = "tab-context-menu-item";
    clearMain.textContent = "取消主文件固定";
    clearMain.addEventListener("click", async () => {
      hideTabContextMenu();
      await clearPreviewEntryPin();
    });
    menu.appendChild(clearMain);
  } else {
    // File is not the main file - show only "设为主文件"
    const setMain = document.createElement("button");
    setMain.type = "button";
    setMain.className = "tab-context-menu-item";
    setMain.textContent = "设为主文件";
    setMain.addEventListener("click", async () => {
      hideTabContextMenu();
      const targetDoc = openDocuments.get(path);
      if (!targetDoc || !editor) return;
      if (currentFilePath !== path) {
        await switchToDocument(targetDoc, false);
      }
      await setCurrentDocumentAsPreviewEntry();
    });
    menu.appendChild(setMain);
  }

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
    provideCompletionItems: (model: monaco.editor.ITextModel, position: monaco.Position) => {
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

// ─── Re-exports for FileTree ──────────────────────────────────────
export { closeDocument };

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

function isTransientCompileResult(result: CompileResult): boolean {
  if (result.success || result.errors.length === 0) return false;
  return result.errors.some((err) =>
    err.includes("Stale compilation request")
    || err.includes("Stale compilation result discarded")
    || err.includes("No pending content available")
    || err.includes("Lock acquisition failed for pending compilation")
  );
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
