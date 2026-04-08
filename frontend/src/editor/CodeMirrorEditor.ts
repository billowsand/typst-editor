import { invoke } from "@tauri-apps/api/core";
import { basicSetup } from "codemirror";
import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  type Completion,
  type CompletionContext,
} from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { Compartment, EditorSelection, EditorState, RangeSetBuilder, StateEffect, StateField, type Extension } from "@codemirror/state";
import {
  Decoration,
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  rectangularSelection,
  ViewPlugin,
  ViewUpdate,
} from "@codemirror/view";
import {
  clientPointToPagePoint,
  ensurePreviewPageNode,
  getPreviewContent,
  getPreviewPageSvg,
  getPreviewPane,
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
  type OutlineItem,
} from "../outline/OutlinePanel";
import { showEmptyWorkspace, showWorkspaceLayout } from "../workspace/WorkspaceShell";

let editorView: EditorView | null = null;
let onFileChangeCb: ((path: string | null) => void) | null = null;
let onPreviewEntryChangeCb: ((path: string | null) => void) | null = null;

let compileTimeout: ReturnType<typeof setTimeout> | null = null;
let cursorSyncTimeout: ReturnType<typeof setTimeout> | null = null;
let selSyncTimeout: ReturnType<typeof setTimeout> | null = null;
let hoverSyncTimeout: ReturnType<typeof setTimeout> | null = null;
let compileStatusTimeout: ReturnType<typeof setTimeout> | null = null;

let suppressCursorSync = 0;
let suppressProgrammaticCompile = false;
let lastEditTime = 0;
let lastOutlineSyncLine = -1;

let currentFilePath = "";
let activePreviewEntryPath = "";
let pinnedPreviewEntryPath = "";
let lastSuccessfulPageCount = 0;
let latestCompileRequestId = 0;
let renderedPageHashes: number[] = [];
let tabsHost: HTMLDivElement | null = null;
let editorHost: HTMLDivElement | null = null;
let tabContextMenu: HTMLDivElement | null = null;
let previewHoverRequestId = 0;
let previewHoverTargetKey = "";
let sourceHoverTarget: { file: string; offset: number } | null = null;
let latestCursorRequestId = 0;
let lastCursorPreviewPage: number | null = null;

type Theme = "light" | "dark";
type CompileCommand = "compile_current" | "set_preview_entry";

interface CompileResult {
  success: boolean;
  page_count: number;
  page_hashes: number[];
  outline: OutlineItem[];
  errors: string[];
  duration_ms?: number;
  entry_path?: string | null;
  pinned_entry_path?: string | null;
}

interface JumpResult {
  type: string;
  file?: string | null;
  offset?: number | null;
  url?: string | null;
  position?: PagePos | null;
}

interface PagePos {
  page: number;
  x: number;
  y: number;
}

interface OpenDocument {
  path: string;
  state: EditorState;
  savedContent: string;
  version: number;
  savedVersion: number;
  dirty: boolean;
  scrollTop: number;
  scrollLeft: number;
}

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

interface PreviewPageMetrics {
  pageLeft: number;
  pageTop: number;
  pageWidth: number;
  pageHeight: number;
  x: number;
  y: number;
}

interface SyntaxToken {
  from: number;
  to: number;
  className: string;
}

const openDocuments = new Map<string, OpenDocument>();

const themeCompartment = new Compartment();
const fontCompartment = new Compartment();

let activeCompileJob: Promise<void> | null = null;
let pendingCompileJob: CompileJob | null = null;

const setSourceHoverEffect = StateEffect.define<{ from: number; to: number } | null>({
  map(value, mapping) {
    if (!value) return null;
    return {
      from: mapping.mapPos(value.from),
      to: mapping.mapPos(value.to),
    };
  },
});

const sourceHoverField = StateField.define({
  create() {
    return Decoration.none;
  },
  update(decorations, transaction) {
    decorations = decorations.map(transaction.changes);
    for (const effect of transaction.effects) {
      if (!effect.is(setSourceHoverEffect)) continue;
      if (!effect.value) {
        return Decoration.none;
      }
      const builder = new RangeSetBuilder<Decoration>();
      builder.add(
        effect.value.from,
        effect.value.to,
        Decoration.mark({ class: "editor-source-hover" }),
      );
      return builder.finish();
    }
    return decorations;
  },
  provide(field) {
    return EditorView.decorations.from(field);
  },
});

const TYPOGRAPHIC_COMPLETIONS: Completion[] = [
  { label: "set", type: "keyword", apply: "set " },
  { label: "show", type: "keyword", apply: "show " },
  { label: "let", type: "keyword", apply: "let " },
  { label: "if", type: "keyword", apply: "if " },
  { label: "for", type: "keyword", apply: "for  in " },
  { label: "while", type: "keyword", apply: "while " },
  { label: "import", type: "keyword", apply: "import " },
  { label: "include", type: "keyword", apply: "include " },
  { label: "page", type: "function", apply: "page(width: auto, height: auto)" },
  { label: "text", type: "function", apply: "text(size: )" },
  { label: "align", type: "function", apply: "align(center)" },
  { label: "image", type: "function", apply: 'image("")' },
  { label: "lorem", type: "function", apply: "lorem(50)" },
];

const typstKeywordPattern = /\b(set|show|let|if|for|while|import|include)\b/g;
const typstDirectivePattern = /#[A-Za-z_]\w*/g;
const typstNumberPattern = /\b\d+(?:\.\d+)?\b/g;
const typstStringPattern = /"(?:\\.|[^"\\])*"/g;
const typstStrongPattern = /\*[^*\n]+\*/g;
const typstEmphasisPattern = /_[^_\n]+_/g;

const typstSyntaxHighlighting = ViewPlugin.fromClass(class {
  decorations;

  constructor(view: EditorView) {
    this.decorations = buildSyntaxDecorations(view);
  }

  update(update: ViewUpdate) {
    if (update.docChanged || update.viewportChanged) {
      this.decorations = buildSyntaxDecorations(update.view);
    }
  }
}, {
  decorations: (plugin) => plugin.decorations,
});

function typstCompletionSource(context: CompletionContext) {
  const word = context.matchBefore(/[A-Za-z_]\w*/);
  if (!word && !context.explicit) return null;
  return {
    from: word?.from ?? context.pos,
    options: TYPOGRAPHIC_COMPLETIONS,
  };
}

function buildSyntaxDecorations(view: EditorView) {
  const builder = new RangeSetBuilder<Decoration>();

  for (const { from, to } of view.visibleRanges) {
    let line = view.state.doc.lineAt(from);
    while (line.from <= to) {
      for (const token of getLineSyntaxTokens(line.text, line.from)) {
        builder.add(
          token.from,
          token.to,
          Decoration.mark({ class: token.className }),
        );
      }

      if (line.to >= to || line.number >= view.state.doc.lines) break;
      line = view.state.doc.line(line.number + 1);
    }
  }

  return builder.finish();
}

function getLineSyntaxTokens(text: string, lineFrom: number): SyntaxToken[] {
  const tokens: SyntaxToken[] = [];
  const occupied: Array<{ from: number; to: number }> = [];

  const pushToken = (from: number, to: number, className: string) => {
    if (from >= to || overlapsOccupied(occupied, from, to)) return;
    occupied.push({ from, to });
    tokens.push({ from: lineFrom + from, to: lineFrom + to, className });
  };

  const headingMatch = text.match(/^\s*={1,6}\s+[^\n]*/);
  if (headingMatch && headingMatch[0].length > 0) {
    pushToken(0, headingMatch[0].length, "cm-typst-heading");
  }

  const lineCommentStart = text.indexOf("//");
  if (lineCommentStart >= 0) {
    pushToken(lineCommentStart, text.length, "cm-typst-comment");
  }

  addRegexTokens(text, typstStringPattern, "cm-typst-string", pushToken);
  addRegexTokens(text, typstDirectivePattern, "cm-typst-directive", pushToken);
  addRegexTokens(text, typstKeywordPattern, "cm-typst-keyword", pushToken);
  addRegexTokens(text, typstNumberPattern, "cm-typst-number", pushToken);
  addRegexTokens(text, typstStrongPattern, "cm-typst-strong", pushToken);
  addRegexTokens(text, typstEmphasisPattern, "cm-typst-emphasis", pushToken);

  tokens.sort((left, right) => left.from - right.from);
  return tokens;
}

function addRegexTokens(
  text: string,
  pattern: RegExp,
  className: string,
  pushToken: (from: number, to: number, className: string) => void,
): void {
  pattern.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    pushToken(match.index, match.index + match[0].length, className);
    if (match[0].length === 0) {
      pattern.lastIndex++;
    }
  }
}

function overlapsOccupied(
  occupied: Array<{ from: number; to: number }>,
  from: number,
  to: number,
): boolean {
  return occupied.some((range) => from < range.to && to > range.from);
}

export function onEditorFileChange(cb: (path: string | null) => void): void {
  onFileChangeCb = cb;
}

export function onPreviewEntryChange(cb: (path: string | null) => void): void {
  onPreviewEntryChangeCb = cb;
}

function scheduleCursorPreviewSync(delay = 120): void {
  if (cursorSyncTimeout) clearTimeout(cursorSyncTimeout);
  cursorSyncTimeout = setTimeout(() => {
    const view = editorView;
    if (!view) return;
    const selection = view.state.selection.main;
    if (!selection.empty) return;
    void syncCursorToPreview(selection.head);
  }, delay);
}

function createEditorTheme(theme: Theme) {
  const dark = theme === "dark";
  return EditorView.theme({
    "&": {
      height: "100%",
      backgroundColor: dark ? "#1c1c1e" : "#ffffff",
      color: dark ? "#f5f5f7" : "#1d1d1f",
      fontSize: "14px",
    },
    ".cm-scroller": {
      fontFamily: "var(--editor-font-family)",
      lineHeight: "1.6",
      overflow: "auto",
    },
    ".cm-content": {
      padding: "10px 0 8px",
      caretColor: dark ? "#d4a855" : "#b5843a",
    },
    ".cm-line": {
      padding: "0 12px",
    },
    ".cm-gutters": {
      backgroundColor: dark ? "#1c1c1e" : "#ffffff",
      color: dark ? "#48484a" : "#c7c7cc",
      borderRight: "1px solid transparent",
    },
    ".cm-activeLineGutter": {
      color: dark ? "#8e8e93" : "#6e6e73",
    },
    ".cm-activeLine": {
      backgroundColor: dark ? "#2c2c2e" : "#f5f5f7",
    },
    ".cm-selectionBackground, &.cm-focused .cm-selectionBackground, ::selection": {
      backgroundColor: dark ? "#d4a85530" : "#b5843a25",
    },
    ".cm-selectionBackground": {
      borderRadius: "2px",
    },
    ".cm-cursor, .cm-dropCursor": {
      borderLeftColor: dark ? "#d4a855" : "#b5843a",
    },
    ".cm-tooltip": {
      backgroundColor: "var(--c-toolbar)",
      border: "1px solid var(--c-border)",
      color: "var(--c-text)",
      backdropFilter: "blur(18px)",
    },
    ".cm-tooltip-autocomplete > ul > li[aria-selected]": {
      backgroundColor: "var(--c-item-active)",
      color: "var(--c-accent)",
    },
    ".cm-panels": {
      backgroundColor: "var(--c-editor)",
      color: "var(--c-text)",
    },
  }, { dark });
}

function createFontTheme(family: string, size: number) {
  return EditorView.theme({
    "&": {
      "--editor-font-family": family || "ui-monospace, 'Cascadia Code', Menlo, Consolas, monospace",
    },
    ".cm-scroller": {
      fontSize: `${size}px`,
    },
    ".cm-content": {
      fontSize: `${size}px`,
    },
    ".cm-gutters": {
      fontSize: `${size}px`,
    },
  });
}

function createBaseExtensions(): Extension[] {
  return [
    lineNumbers(),
    highlightActiveLineGutter(),
    highlightActiveLine(),
    drawSelection(),
    rectangularSelection(),
    history(),
    closeBrackets(),
    basicSetup,
    EditorView.lineWrapping,
    sourceHoverField,
    typstSyntaxHighlighting,
    autocompletion({ override: [typstCompletionSource] }),
    themeCompartment.of(createEditorTheme(savedTheme())),
    fontCompartment.of(createFontTheme(savedFontFamilyCss(), savedFontSize())),
    keymap.of([
      ...closeBracketsKeymap,
      ...defaultKeymap,
      ...historyKeymap,
      {
        key: "Mod-s",
        run: () => {
          void saveCurrentFile();
          return true;
        },
      },
      {
        key: "Mod-Enter",
        run: () => {
          void compileAndPreview();
          return true;
        },
      },
      {
        key: "Mod-Shift-e",
        run: () => {
          void exportPdf();
          return true;
        },
      },
      {
        key: "Mod-o",
        run: () => {
          void openFile();
          return true;
        },
      },
      {
        key: "Mod-w",
        run: () => {
          void closeCurrentTab();
          return true;
        },
      },
      {
        key: "Mod-Shift-Enter",
        run: () => {
          void setCurrentDocumentAsPreviewEntry();
          return true;
        },
      },
      {
        key: "Mod-Alt-Enter",
        run: () => {
          void clearPreviewEntryPin();
          return true;
        },
      },
    ]),
    EditorView.updateListener.of((update) => {
      handleViewUpdate(update);
    }),
    EditorView.domEventHandlers({
      mousemove(event, view) {
        if (hoverSyncTimeout) clearTimeout(hoverSyncTimeout);
        hoverSyncTimeout = setTimeout(() => {
          const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
          if (pos == null) return;
          void syncHoverToPreview(pos);
        }, 150);
      },
      mouseleave() {
        if (hoverSyncTimeout) clearTimeout(hoverSyncTimeout);
        clearPreviewSourceHover();
      },
    }),
  ];
}

function createEditorState(content: string): EditorState {
  return EditorState.create({
    doc: content,
    extensions: createBaseExtensions(),
  });
}

export async function initEditor(container: HTMLDivElement): Promise<void> {
  container.replaceChildren();

  tabsHost = document.createElement("div");
  tabsHost.className = "editor-tabs";
  container.appendChild(tabsHost);

  editorHost = document.createElement("div");
  editorHost.className = "editor-surface";
  container.appendChild(editorHost);

  editorView = new EditorView({
    state: createEditorState(""),
    parent: editorHost,
  });

  setCurrentFilePath(null);
  renderTabs();
  updatePageStatus(0);
  clearOutlinePanel();
  updateStatus("success", "Ready");
  updateCursorStatus(1, 1);
}

function handleViewUpdate(update: ViewUpdate): void {
  const doc = getActiveDocument();
  if (!doc) return;

  doc.state = update.state;
  doc.scrollTop = update.view.scrollDOM.scrollTop;
  doc.scrollLeft = update.view.scrollDOM.scrollLeft;

  if (update.docChanged) {
    doc.version++;
    lastEditTime = Date.now();
    updateActiveDocumentState();
    if (!suppressProgrammaticCompile) {
      scheduleCompile();
    }
  }

  if (!update.docChanged && !update.selectionSet) return;

  const selection = update.state.selection.main;
  const line = update.state.doc.lineAt(selection.head);
  const column = selection.head - line.from + 1;
  updateCursorStatus(line.number, column);

  if (suppressCursorSync > 0) return;

  if (selection.empty) {
    scheduleCursorPreviewSync(selectionSetDelay(update));
  } else {
    if (selSyncTimeout) clearTimeout(selSyncTimeout);
  }

  if (line.number !== lastOutlineSyncLine) {
    lastOutlineSyncLine = line.number;
    syncOutlineActiveFromEditor();
  }
}

function selectionSetDelay(update: ViewUpdate): number {
  return update.selectionSet && !update.docChanged ? 80 : 120;
}

export function setEditorTheme(theme: Theme): void {
  editorView?.dispatch({
    effects: themeCompartment.reconfigure(createEditorTheme(theme)),
  });

  for (const doc of openDocuments.values()) {
    doc.state = doc.state.update({
      effects: themeCompartment.reconfigure(createEditorTheme(theme)),
    }).state;
  }
}

export function setEditorFont(family: string, size: number): void {
  const fontTheme = createFontTheme(family, size);
  editorView?.dispatch({
    effects: fontCompartment.reconfigure(fontTheme),
  });

  for (const doc of openDocuments.values()) {
    doc.state = doc.state.update({
      effects: fontCompartment.reconfigure(fontTheme),
    }).state;
  }
}

export function getContent(): string {
  return editorView?.state.doc.toString() ?? "";
}

export function setContent(content: string): void {
  const view = editorView;
  if (!view) return;
  view.dispatch({
    changes: {
      from: 0,
      to: view.state.doc.length,
      insert: content,
    },
  });
}

export function getCurrentFilePath(): string | null {
  return currentFilePath || null;
}

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

export function getZoom(): number {
  return currentZoom;
}

function enqueueCompile(job: Omit<CompileJob, "intentId">): void {
  pendingCompileJob = {
    ...job,
    intentId: ++latestCompileRequestId,
  };
  void drainCompileQueue();
}

function launchCompile(job: Omit<CompileJob, "intentId">): void {
  const compileJob: CompileJob = {
    ...job,
    intentId: ++latestCompileRequestId,
  };
  pendingCompileJob = null;
  void runCompileJob(compileJob);
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
    if (job.intentId !== latestCompileRequestId) return;
    await applyCompileResult(job, result);
  } catch (error) {
    if (job.intentId !== latestCompileRequestId) return;
    finalizeCompileStatusTimeout();
    if (!job.preservePagesOnError) {
      updatePageStatus(0);
      clearOutlinePanel();
    }
    updateStatus("error", `Error: ${error}`);
  }
}

async function applyCompileResult(job: CompileJob, result: CompileResult): Promise<void> {
  finalizeCompileStatusTimeout();
  const pinnedEntryChanged = applyCompileEntryState(result);

  if (result.success) {
    lastSuccessfulPageCount = result.page_count;
    updatePageStatus(result.page_count);
    await renderAllPages(result.page_count, result.page_hashes);
    if (job.intentId !== latestCompileRequestId) return;
    setOutlineItems(result.outline ?? []);
    syncOutlineActiveFromEditor();
    updateStatus("success", job.successMessage(result));
    clearDiagnostics();
    if (job.notifyPreviewEntryChange && pinnedEntryChanged) {
      onPreviewEntryChangeCb?.(pinnedPreviewEntryPath || null);
    }
    return;
  }

  if (isTransientCompileResult(result)) {
    updateStatus("compiling", "Syncing preview…");
    return;
  }
  if (!job.preservePagesOnError) {
    updatePageStatus(0);
    clearOutlinePanel();
  }
  updateStatus("error", `${result.errors.length} error(s)`);
  showDiagnostics(result.errors);
  if (job.notifyPreviewEntryChange && pinnedEntryChanged) {
    onPreviewEntryChangeCb?.(pinnedPreviewEntryPath || null);
  }
}

function applyCompileEntryState(result: CompileResult): boolean {
  const nextActivePreviewEntryPath = result.entry_path ?? activePreviewEntryPath;
  const nextPinnedPreviewEntryPath = result.pinned_entry_path ?? pinnedPreviewEntryPath;
  const pinnedEntryChanged = nextPinnedPreviewEntryPath !== pinnedPreviewEntryPath;
  const activeEntryChanged = nextActivePreviewEntryPath !== activePreviewEntryPath;

  activePreviewEntryPath = nextActivePreviewEntryPath;
  pinnedPreviewEntryPath = nextPinnedPreviewEntryPath;

  if (pinnedEntryChanged || activeEntryChanged) {
    renderTabs();
  }

  return pinnedEntryChanged;
}

function finalizeCompileStatusTimeout(): void {
  if (!compileStatusTimeout) return;
  clearTimeout(compileStatusTimeout);
  compileStatusTimeout = null;
}

export async function compileAndPreview(): Promise<void> {
  const view = editorView;
  if (!view || !currentFilePath) return;
  const activeDoc = getActiveDocument();
  const content = view.state.doc.toString();
  enqueueCompile({
    command: "compile_current",
    filePath: currentFilePath,
    content,
    savedContent: activeDoc?.savedContent ?? content,
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
  } catch (error) {
    suppressProgrammaticCompile = false;
    updateStatus("error", `Failed to open: ${error}`);
  }
}

async function saveCurrentFile(): Promise<void> {
  const view = editorView;
  if (!view) return;
  let path = currentFilePath;
  let activeDoc = getActiveDocument();

  if (!path) {
    try {
      const picked = await invoke<string | null>("save_file_dialog");
      if (!picked) return;
      setCurrentFilePath(picked);
      path = picked;
      activeDoc = getActiveDocument();
    } catch (error) {
      updateStatus("error", `Save dialog failed: ${error}`);
      return;
    }
  }

  const content = view.state.doc.toString();
  try {
    await invoke("save_file", { content, filePath: path });
    if (activeDoc) {
      activeDoc.savedContent = content;
      activeDoc.savedVersion = activeDoc.version;
      activeDoc.dirty = false;
      renderTabs();
    }
    updateStatus("success", "Saved");
    enqueueCompile({
      command: "compile_current",
      filePath: path,
      content,
      savedContent: content,
      preservePagesOnError: true,
      notifyPreviewEntryChange: true,
      successMessage: () => "Saved",
    });
  } catch (error) {
    updateStatus("error", `Save failed: ${error}`);
  }
}

async function exportPdf(): Promise<void> {
  try {
    const result = await invoke<"Success" | "Cancelled">("export_pdf_dialog");
    if (result !== "Cancelled") updateStatus("success", "PDF exported");
  } catch (error) {
    updateStatus("error", `Export failed: ${error}`);
  }
}

export async function openFileFromTree(path: string): Promise<void> {
  if (!editorView) return;
  await openFileAtPath(path);
}

export async function setPreviewEntryAtPath(path: string): Promise<void> {
  if (!editorView) return;

  let targetDoc = openDocuments.get(path);
  if (!targetDoc) {
    const content = await invoke<string>("read_text_file", { path });
    targetDoc = createDocument(path, content);
  }

  if (currentFilePath !== path) {
    await switchToDocument(targetDoc, false);
  }

  await setCurrentDocumentAsPreviewEntry();
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

  const view = editorView;
  if (!view) return;
  const text = view.state.doc.toString();
  const charOffset = utf8ByteOffsetToCharOffset(text, offset);
  suppressCursorSync++;
  view.dispatch({
    selection: { anchor: charOffset },
    effects: EditorView.scrollIntoView(charOffset, { y: "center" }),
  });
  view.focus();
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
  const view = editorView;
  if (!view || currentFilePath !== path) return;
  const charOffset = utf8ByteOffsetToCharOffset(view.state.doc.toString(), offset);
  const pos = offsetToLineColumn(view.state.doc.toString(), charOffset);
  try {
    const positions = await invoke<PagePos[]>("jump_from_cursor", {
      filePath: path,
      line: pos.line - 1,
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
      remaining -= 1;
    }
  }
  const lastLine = lines[lines.length - 1] ?? "";
  return { line: Math.max(1, lines.length), column: lastLine.length + 1 };
}

function utf8BytesForText(text: string): number {
  let bytes = 0;
  for (let index = 0; index < text.length; index++) {
    const code = text.charCodeAt(index);
    if (code < 0x80) {
      bytes += 1;
    } else if (code < 0x800) {
      bytes += 2;
    } else if (code >= 0xd800 && code < 0xdc00) {
      bytes += 4;
      index++;
    } else {
      bytes += 3;
    }
  }
  return bytes;
}

function utf8ByteOffsetInTextToCharOffset(text: string, byteOffset: number): number {
  let bytes = 0;
  let index = 0;
  while (index < text.length && bytes < byteOffset) {
    const code = text.charCodeAt(index);
    if (code < 0x80) {
      bytes += 1;
    } else if (code < 0x800) {
      bytes += 2;
    } else if (code >= 0xd800 && code < 0xdc00) {
      bytes += 4;
      index++;
    } else {
      bytes += 3;
    }
    index++;
  }
  return index;
}

function utf8ByteOffsetToCharOffset(text: string, byteOffset: number): number {
  if (byteOffset <= 0) return 0;
  return utf8ByteOffsetInTextToCharOffset(text, byteOffset);
}

function charOffsetToUtf8ByteOffset(text: string, charOffset: number): number {
  return utf8BytesForText(text.slice(0, Math.max(0, Math.min(charOffset, text.length))));
}

async function syncCursorToPreview(head: number): Promise<void> {
  const view = editorView;
  if (!view || !currentFilePath) return;
  if (lastSuccessfulPageCount === 0) return;
  if (renderedPageHashes.length === 0) return;
  if (Date.now() - lastEditTime < 400) return;

  const byteOffset = charOffsetToUtf8ByteOffset(view.state.doc.toString(), head);
  const requestId = ++latestCursorRequestId;
  try {
    const positions = await invoke<PagePos[]>("jump_from_offset", {
      filePath: currentFilePath,
      offset: byteOffset,
    });
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
  } catch {
    // ignore
  }
}

async function handlePreviewClick(
  event: MouseEvent,
  svgEl: SVGSVGElement,
  pageIndex: number,
): Promise<void> {
  const pagePoint = clientPointToPagePoint(svgEl, event.clientX, event.clientY);
  try {
    const result = await invoke<JumpResult>("jump_from_click", {
      page: pageIndex + 1,
      x: pagePoint.x,
      y: pagePoint.y,
    });
    if (result.type === "file" && result.offset != null && result.file) {
      applySourceHoverDecoration(result.file, result.offset);
      await jumpToFileOffset(result.file, result.offset, false);
      await revealSourceOffsetInPreview(result.file, result.offset, "auto");
    } else if (result.type === "url" && result.url) {
      window.open(result.url, "_blank", "noopener,noreferrer");
    } else if (result.type === "position" && result.position) {
      scrollPreviewToPosition(result.position);
    }
  } catch {
    // ignore
  }
}

async function syncHoverToPreview(offset: number): Promise<void> {
  if (!currentFilePath) return;
  if (lastSuccessfulPageCount === 0) return;
  if (renderedPageHashes.length === 0) return;
  if (Date.now() - lastEditTime < 600) return;

  const view = editorView;
  if (!view) return;
  const byteOffset = charOffsetToUtf8ByteOffset(view.state.doc.toString(), offset);
  try {
    const positions = await invoke<PagePos[]>("jump_from_offset", {
      filePath: currentFilePath,
      offset: byteOffset,
    });
    const previewPos = pickBestPreviewPosition(positions);
    if (previewPos && previewPos.page <= renderedPageHashes.length) {
      applyHoverAtPreviewPosition(previewPos);
    }
  } catch {
    // ignore
  }
}

async function handlePreviewHover(event: MouseEvent, svgEl: SVGSVGElement, pageIndex: number): Promise<void> {
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
      highlightHoverInPreview({ page: pageIndex + 1, x: pagePoint.x, y: pagePoint.y });
      applySourceHoverDecoration(result.file, result.offset);
    } else {
      clearPreviewSourceHover();
    }
  } catch {
    if (requestId === previewHoverRequestId) clearPreviewSourceHover();
  }
}

function removeSyncOverlays(cls: string): void {
  document.querySelectorAll(`.highlight-overlay.${cls}`).forEach((element) => element.remove());
}

function highlightCursorInPreview(
  pos: PagePos,
  options: { scroll?: boolean; behavior?: ScrollBehavior; persistent?: boolean } = {},
): void {
  removeSyncOverlays("cursor-hl");
  const panePoint = pagePointToPanePoint(pos);
  const svgEl = getPreviewPageSvg(pos.page);
  if (!panePoint || !svgEl) return;

  const overlay = document.createElement("div");
  overlay.className = "highlight-overlay cursor-hl";
  overlay.style.left = `${Math.max(0, panePoint.left - 18)}px`;
  overlay.style.top = `${Math.max(0, panePoint.top - 14)}px`;
  overlay.style.width = "36px";
  overlay.style.height = "28px";
  getPreviewContent()?.appendChild(overlay);
  if (options.scroll) {
    scrollPreviewToPosition(pos, options.behavior ?? "auto");
  }
  if (!options.persistent) {
    setTimeout(() => overlay.classList.add("fade"), 1500);
    setTimeout(() => overlay.remove(), 2000);
  }
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

function appendRangeOverlay(overlayClass: string, left: number, top: number, width: number, height: number): void {
  const content = getPreviewContent();
  if (!content) return;
  const overlay = document.createElement("div");
  overlay.className = `highlight-overlay ${overlayClass}`;
  overlay.style.left = `${Math.max(0, left)}px`;
  overlay.style.top = `${Math.max(0, top)}px`;
  overlay.style.width = `${Math.max(6, width)}px`;
  overlay.style.height = `${Math.max(16, height)}px`;
  content.appendChild(overlay);
}

function pickBestPreviewPosition(positions: PagePos[]): PagePos | null {
  if (positions.length === 0) return null;
  return positions[positions.length - 1] ?? positions[0] ?? null;
}

function applyHoverAtPreviewPosition(pos: PagePos): void {
  highlightCursorInPreview(pos, { scroll: false, persistent: true });
}

function highlightHoverInPreview(pos: PagePos): void {
  removeSyncOverlays("hover-hl");
  const panePoint = pagePointToPanePoint(pos);
  const svgEl = getPreviewPageSvg(pos.page);
  if (!panePoint || !svgEl) return;
  const overlay = document.createElement("div");
  overlay.className = "highlight-overlay hover-hl";
  overlay.style.left = `${Math.max(0, panePoint.left - 14)}px`;
  overlay.style.top = `${Math.max(0, panePoint.top - 10)}px`;
  overlay.style.width = "28px";
  overlay.style.height = "20px";
  getPreviewContent()?.appendChild(overlay);
}

function applySourceHoverDecoration(file: string, offset: number): void {
  sourceHoverTarget = { file, offset };
  const view = editorView;
  if (!view || currentFilePath !== file) {
    clearSourceHoverDecoration();
    return;
  }

  const text = view.state.doc.toString();
  const from = utf8ByteOffsetToCharOffset(text, offset);
  const to = Math.max(from + 1, utf8ByteOffsetToCharOffset(text, offset + 1));
  view.dispatch({
    effects: setSourceHoverEffect.of({ from, to }),
  });
}

function clearSourceHoverDecoration(): void {
  editorView?.dispatch({
    effects: setSourceHoverEffect.of(null),
  });
}

function clearPreviewSourceHover(): void {
  previewHoverTargetKey = "";
  sourceHoverTarget = null;
  clearSourceHoverDecoration();
  removeSyncOverlays("hover-hl");
}

function syncOutlineActiveFromEditor(): void {
  const view = editorView;
  if (!view || !currentFilePath) {
    updateOutlineActiveItem(null, null);
    return;
  }

  const head = view.state.selection.main.head;
  const byteOffset = charOffsetToUtf8ByteOffset(view.state.doc.toString(), head);
  updateOutlineActiveItem(currentFilePath, byteOffset);
}

export async function renderAllPages(pageCount: number, pageHashes: number[]): Promise<void> {
  trimPreviewPages(pageCount);
  const changedPages: number[] = [];
  for (let index = 0; index < pageCount; index++) {
    ensurePreviewPageNode(index);
    if (renderedPageHashes[index] !== pageHashes[index]) changedPages.push(index);
  }
  await Promise.all(changedPages.map((index) => renderSinglePage(index, pageHashes)));
  renderedPageHashes.length = pageCount;
}

async function renderSinglePage(index: number, pageHashes: number[]): Promise<void> {
  const capturedHash = pageHashes[index];
  const pageNode = ensurePreviewPageNode(index);
  try {
    const svg = await invoke<string>("render_page", { page: index });
    if (capturedHash !== pageHashes[index]) return;
    if (!replacePreviewPageSvg(index, svg, {
      onClick: handlePreviewClick,
      onHover: handlePreviewHover,
      onLeave: () => clearPreviewSourceHover(),
    })) return;
    renderedPageHashes[index] = capturedHash;
    pageNode.classList.add("page-updated");
    setTimeout(() => pageNode.classList.remove("page-updated"), 80);
  } catch {
    // ignore
  }
}

function resetPreviewState(): void {
  latestCompileRequestId++;
  renderedPageHashes = [];
  lastCursorPreviewPage = null;
  clearPreviewSourceHover();
  removeSyncOverlays("cursor-hl");
  resetPreviewContent();
}

function createDocument(path: string, content: string): OpenDocument {
  const doc: OpenDocument = {
    path,
    state: createEditorState(content),
    savedContent: content,
    version: 0,
    savedVersion: 0,
    dirty: false,
    scrollTop: 0,
    scrollLeft: 0,
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
  const wasDirty = doc.dirty;
  doc.dirty = doc.version !== doc.savedVersion;
  if (doc.dirty !== wasDirty) renderTabs();
}

async function switchToDocument(doc: OpenDocument, refreshPreview = true): Promise<void> {
  const view = editorView;
  if (!view) return;
  showWorkspaceLayout();

  const currentDoc = getActiveDocument();
  if (currentDoc && currentDoc.path !== doc.path) {
    currentDoc.state = view.state;
    currentDoc.scrollTop = view.scrollDOM.scrollTop;
    currentDoc.scrollLeft = view.scrollDOM.scrollLeft;
  }

  suppressProgrammaticCompile = true;
  view.setState(doc.state);
  suppressProgrammaticCompile = false;

  setCurrentFilePath(doc.path);
  requestAnimationFrame(() => {
    if (!editorView) return;
    editorView.scrollDOM.scrollTop = doc.scrollTop;
    editorView.scrollDOM.scrollLeft = doc.scrollLeft;
  });
  view.focus();
  lastOutlineSyncLine = -1;
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

async function closeDocument(path: string): Promise<boolean> {
  const doc = openDocuments.get(path);
  if (!doc) return true;

  if (doc.dirty && !window.confirm(`"${doc.path.split(/[\\/]/).pop()}" 有未保存修改，确定关闭吗？`)) {
    return false;
  }

  const wasActive = currentFilePath === path;
  const docs = Array.from(openDocuments.values());
  const currentIndex = docs.findIndex((item) => item.path === path);
  openDocuments.delete(path);

  if (openDocuments.size === 0) {
    editorView?.setState(createEditorState(""));
    setCurrentFilePath(null);
    const pinnedEntryChanged = !!pinnedPreviewEntryPath;
    activePreviewEntryPath = "";
    pinnedPreviewEntryPath = "";
    lastSuccessfulPageCount = 0;
    resetPreviewState();
    renderTabs();
    clearDiagnostics();
    clearOutlinePanel();
    updateOutlineActiveItem(null, null);
    updatePageStatus(0);
    updateStatus("success", "Ready");
    await invoke("reset_session");
    if (pinnedEntryChanged) {
      onPreviewEntryChangeCb?.(null);
    }
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

export async function closeDocumentsUnderPath(dirPath: string): Promise<boolean> {
  const sep = dirPath.includes("\\") ? "\\" : "/";
  const normalizedDirPath = dirPath.endsWith(sep) ? dirPath : dirPath + sep;
  const docsToClose: string[] = [];

  for (const [path] of openDocuments.entries()) {
    const normalizedDocPath = path.endsWith(sep) ? path : path + sep;
    if (normalizedDocPath.startsWith(normalizedDirPath)) {
      docsToClose.push(path);
    }
  }

  for (const path of docsToClose.reverse()) {
    const closed = await closeDocument(path);
    if (!closed) return false;
  }
  return true;
}

function compileOpenDocument(doc: OpenDocument): void {
  const content = doc.state.doc.toString();
  showWorkspaceLayout();
  enqueueCompile({
    command: "compile_current",
    filePath: doc.path,
    content,
    savedContent: doc.savedContent,
    preservePagesOnError: false,
    notifyPreviewEntryChange: true,
    successMessage: (result) => `Compiled · ${result.page_count} page(s)`,
  });
}

async function setCurrentDocumentAsPreviewEntry(): Promise<void> {
  const view = editorView;
  if (!view || !currentFilePath) return;
  const activeDoc = getActiveDocument();
  const content = view.state.doc.toString();
  resetPreviewState();
  showWorkspaceLayout();
  launchCompile({
    command: "set_preview_entry",
    filePath: currentFilePath,
    content,
    savedContent: activeDoc?.savedContent ?? content,
    preservePagesOnError: false,
    notifyPreviewEntryChange: true,
    successMessage: () => "Preview entry updated",
  });
}

async function clearPreviewEntryPin(): Promise<void> {
  if (!editorView || !currentFilePath) return;
  try {
    await invoke("clear_preview_entry_pin");
    await compileAndPreview();
  } catch (error) {
    updateStatus("error", `Failed to clear preview entry pin: ${error}`);
  }
}

function renderTabs(): void {
  if (!tabsHost) return;
  tabsHost.replaceChildren();

  for (const doc of openDocuments.values()) {
    const isPinnedPreviewEntry = doc.path === pinnedPreviewEntryPath;
    const tab = document.createElement("button");
    tab.className = `editor-tab${doc.path === currentFilePath ? " active" : ""}${isPinnedPreviewEntry ? " preview-entry" : ""}`;
    tab.title = doc.path;

    const label = document.createElement("span");
    label.className = "editor-tab-label";
    label.textContent = doc.path.split(/[\\/]/).pop() || doc.path;
    tab.appendChild(label);

    if (isPinnedPreviewEntry) {
      const star = document.createElement("span");
      star.className = "editor-tab-star";
      star.innerHTML = `<svg width="11" height="11" viewBox="0 0 12 12" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M6 1l1.5 3h3.3l-2.6 2 1 3.3L6 8l-3.2 1.3 1-3.3-2.6-2h3.3L6 1Z" fill="currentColor"/></svg>`;
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

  const setMain = document.createElement("button");
  setMain.type = "button";
  setMain.className = "tab-context-menu-item";
  setMain.textContent = path === pinnedPreviewEntryPath ? "当前已是主文件" : "设为主文件";
  setMain.disabled = path === pinnedPreviewEntryPath;
  setMain.addEventListener("click", async () => {
    hideTabContextMenu();
    await setPreviewEntryAtPath(path);
  });

  const clearMain = document.createElement("button");
  clearMain.type = "button";
  clearMain.className = "tab-context-menu-item";
  clearMain.textContent = "取消主文件固定";
  clearMain.disabled = !pinnedPreviewEntryPath;
  clearMain.addEventListener("click", async () => {
    hideTabContextMenu();
    await clearPreviewEntryPin();
  });

  menu.append(setMain, clearMain);
  document.body.appendChild(menu);
  tabContextMenu = menu;

  setTimeout(() => {
    document.addEventListener("mousedown", handleTabContextMenuDismiss, { once: true });
    window.addEventListener("blur", handleTabContextMenuWindowBlur, { once: true });
  }, 0);
}

function handleTabContextMenuDismiss(event: MouseEvent): void {
  if (tabContextMenu?.contains(event.target as Node)) {
    document.addEventListener("mousedown", handleTabContextMenuDismiss, { once: true });
    return;
  }
  hideTabContextMenu();
}

function handleTabContextMenuWindowBlur(): void {
  hideTabContextMenu();
}

function hideTabContextMenu(): void {
  tabContextMenu?.remove();
  tabContextMenu = null;
}

export async function runEditorAction(actionId: string): Promise<void> {
  switch (actionId) {
    case "save-file":
      await saveCurrentFile();
      break;
    case "force-compile":
      await compileAndPreview();
      break;
    case "export-pdf":
      await exportPdf();
      break;
    case "open-file":
      await openFile();
      break;
    case "close-file-tab":
      await closeCurrentTab();
      break;
    case "set-preview-entry":
      await setCurrentDocumentAsPreviewEntry();
      break;
    case "clear-preview-entry-pin":
      await clearPreviewEntryPin();
      break;
    default:
      break;
  }
}

function savedTheme(): Theme {
  try {
    return (localStorage.getItem("typst-theme") ?? "dark") as Theme;
  } catch {
    return "dark";
  }
}

function savedFontSize(): number {
  try {
    return Number.parseInt(localStorage.getItem("typst-font-size") ?? "14", 10);
  } catch {
    return 14;
  }
}

function savedFontFamilyCss(): string {
  try {
    const family = localStorage.getItem("typst-font-family") ?? "";
    return family ? `'${family}', monospace` : "ui-monospace, 'Cascadia Code', Menlo, Consolas, monospace";
  } catch {
    return "ui-monospace, 'Cascadia Code', Menlo, Consolas, monospace";
  }
}

export function updateStatus(type: "success" | "error" | "compiling", message: string): void {
  const element = document.getElementById("compile-status");
  if (!element) return;
  element.className = `status-chip status ${type}`;

  const icons = {
    success: `<svg viewBox="0 0 12 12" fill="none" xmlns="http://www.w3.org/2000/svg">
      <circle cx="6" cy="6" r="4.5" stroke="currentColor" stroke-width="1.1"/>
      <path d="M4 6l1.5 1.5L8 4.5" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" stroke-linejoin="round"/>
    </svg>`,
    error: `<svg viewBox="0 0 12 12" fill="none" xmlns="http://www.w3.org/2000/svg">
      <circle cx="6" cy="6" r="4.5" stroke="currentColor" stroke-width="1.1"/>
      <path d="M4.5 4.5l3 3M7.5 4.5l-3 3" stroke="currentColor" stroke-width="1.1" stroke-linecap="round"/>
    </svg>`,
    compiling: `<svg viewBox="0 0 12 12" fill="none" xmlns="http://www.w3.org/2000/svg">
      <circle cx="6" cy="6" r="4.5" stroke="currentColor" stroke-width="1.1" stroke-dasharray="3 2"/>
    </svg>`,
  };

  element.innerHTML = `${icons[type]} ${message}`;
}

function updateStatusFile(path: string | null): void {
  const element = document.getElementById("status-file");
  if (!element) return;
  const icon = `<svg viewBox="0 0 12 12" fill="none" xmlns="http://www.w3.org/2000/svg">
    <path d="M2 1.5h5.5l2 2v7.5H2V1.5z" stroke="currentColor" stroke-width="1.1" stroke-linejoin="round"/>
    <path d="M7.5 1.5v2h2" stroke="currentColor" stroke-width="1.1" stroke-linejoin="round"/>
  </svg>`;
  element.innerHTML = `${icon} ${path ? path.split(/[\\/]/).pop() || path : "未打开文件"}`;
  element.setAttribute("title", path ?? "未打开文件");
}

function updateCursorStatus(line: number, column: number): void {
  const element = document.getElementById("status-cursor");
  if (!element) return;
  const icon = `<svg viewBox="0 0 12 12" fill="none" xmlns="http://www.w3.org/2000/svg">
    <path d="M3 1v10M3 1l4 4M3 1l4-1" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" stroke-linejoin="round"/>
  </svg>`;
  element.innerHTML = `${icon} Ln ${line}, Col ${column}`;
}

function updatePageStatus(pageCount: number): void {
  const element = document.getElementById("status-pages");
  if (!element) return;
  const icon = `<svg viewBox="0 0 12 12" fill="none" xmlns="http://www.w3.org/2000/svg">
    <rect x="1.5" y="1.5" width="9" height="9" rx="1" stroke="currentColor" stroke-width="1.1"/>
    <path d="M4 5h4M4 7h4M4 4h2" stroke="currentColor" stroke-width="1.1" stroke-linecap="round"/>
  </svg>`;
  element.innerHTML = `${icon} ${pageCount} page(s)`;
}

function updateZoomStatus(zoom: number): void {
  const element = document.getElementById("status-zoom-bar");
  if (!element) return;
  const icon = `<svg viewBox="0 0 12 12" fill="none" xmlns="http://www.w3.org/2000/svg">
    <circle cx="5.5" cy="5.5" r="3.5" stroke="currentColor" stroke-width="1.1"/>
    <path d="M5.5 4v3M4 5.5h3" stroke="currentColor" stroke-width="1.1" stroke-linecap="round"/>
    <path d="M8.5 8.5L11 11" stroke="currentColor" stroke-width="1.1" stroke-linecap="round"/>
  </svg>`;
  element.innerHTML = `${icon} ${Math.round(zoom * 100)}%`;
}

function isTransientCompileResult(result: CompileResult): boolean {
  if (result.success || result.errors.length === 0) return false;
  return result.errors.some((error) =>
    error.includes("Stale compilation request")
    || error.includes("Stale compilation result discarded")
    || error.includes("No pending content available")
    || error.includes("Lock acquisition failed for pending compilation"),
  );
}

function clearDiagnostics(): void {
  document.querySelector(".diagnostics-panel")?.remove();
}

function showDiagnostics(errors: string[]): void {
  clearDiagnostics();
  const panel = document.createElement("div");
  panel.className = "diagnostics-panel";
  for (const error of errors) {
    const item = document.createElement("div");
    item.className = "diagnostic-item error";
    item.textContent = error;
    item.addEventListener("click", () => {
      const match = error.match(/^(.*?):(\d+):/);
      const lineNumber = match ? Number.parseInt(match[2], 10) : null;
      const file = match?.[1] ?? currentFilePath;
      if (!lineNumber || !file) return;
      void openFileAtPath(file).then(() => {
        const view = editorView;
        if (!view) return;
        const line = view.state.doc.line(Math.max(1, Math.min(lineNumber, view.state.doc.lines)));
        view.dispatch({
          selection: EditorSelection.cursor(line.from),
          effects: EditorView.scrollIntoView(line.from, { y: "center" }),
        });
        view.focus();
      });
    });
    panel.appendChild(item);
  }
  document.getElementById("editor-pane")?.appendChild(panel);
}

export { closeDocument };
