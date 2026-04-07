import { invoke } from "@tauri-apps/api/core";

type MonacoEditorModule = typeof import("./MonacoEditor");

let editorModulePromise: Promise<MonacoEditorModule> | null = null;
let editorContainer: HTMLDivElement | null = null;
let editorInitPromise: Promise<void> | null = null;
let pendingTheme: "light" | "dark" | null = null;
let pendingFont: { family: string; size: number } | null = null;
const pendingFileChangeCallbacks: Array<(path: string | null) => void> = [];

async function loadEditorModule(): Promise<MonacoEditorModule> {
  if (!editorModulePromise) {
    editorModulePromise = (async () => {
      await import("../monaco-env");
      return import("./MonacoEditor");
    })();
  }
  return editorModulePromise;
}

export function registerEditorContainer(container: HTMLDivElement): void {
  editorContainer = container;
}

export async function ensureEditorReady(): Promise<MonacoEditorModule> {
  if (!editorContainer) {
    throw new Error("Editor container is not registered");
  }

  const mod = await loadEditorModule();
  if (!editorInitPromise) {
    editorInitPromise = (async () => {
      await mod.initEditor(editorContainer!);

      if (pendingTheme) {
        mod.setMonacoTheme(pendingTheme);
      }
      if (pendingFont) {
        mod.setEditorFont(pendingFont.family, pendingFont.size);
      }
      for (const cb of pendingFileChangeCallbacks) {
        mod.onEditorFileChange(cb);
      }
    })();
  }

  await editorInitPromise;
  return mod;
}

export function onEditorFileChange(cb: (path: string | null) => void): void {
  pendingFileChangeCallbacks.push(cb);
  if (editorInitPromise) {
    void editorInitPromise.then(async () => {
      const mod = await loadEditorModule();
      mod.onEditorFileChange(cb);
    });
  }
}

export function setMonacoTheme(theme: "light" | "dark"): void {
  pendingTheme = theme;
  if (editorInitPromise) {
    void editorInitPromise.then(async () => {
      const mod = await loadEditorModule();
      mod.setMonacoTheme(theme);
    });
  }
}

export function setEditorFont(family: string, size: number): void {
  pendingFont = { family, size };
  if (editorInitPromise) {
    void editorInitPromise.then(async () => {
      const mod = await loadEditorModule();
      mod.setEditorFont(family, size);
    });
  }
}

export async function openFileAtPath(path: string): Promise<void> {
  const mod = await ensureEditorReady();
  await mod.openFileAtPath(path);
}

export async function openFileFromTree(path: string): Promise<void> {
  const mod = await ensureEditorReady();
  await mod.openFileFromTree(path);
}

export async function jumpToFileOffset(
  path: string,
  offset: number,
  refreshPreview = false,
): Promise<void> {
  const mod = await ensureEditorReady();
  await mod.jumpToFileOffset(path, offset, refreshPreview);
}

export async function revealSourceOffsetInPreview(
  path: string,
  offset: number,
  behavior: ScrollBehavior = "auto",
): Promise<void> {
  const mod = await ensureEditorReady();
  await mod.revealSourceOffsetInPreview(path, offset, behavior);
}

export async function runEditorAction(actionId: string): Promise<void> {
  const mod = await ensureEditorReady();
  mod.getEditor()?.getAction(actionId)?.run();
}

export async function closeDocument(path: string): Promise<boolean> {
  if (!editorInitPromise) return true;
  const mod = await ensureEditorReady();
  return mod.closeDocument(path);
}

export async function closeDocumentsUnderPath(dirPath: string): Promise<boolean> {
  if (!editorInitPromise) return true;
  const mod = await ensureEditorReady();
  return mod.closeDocumentsUnderPath(dirPath);
}

export async function pickFilePath(): Promise<string | null> {
  return invoke<string | null>("open_file_dialog");
}

export async function hasUnsavedDocumentsAsync(): Promise<boolean> {
  if (!editorInitPromise) return false;
  const mod = await ensureEditorReady();
  return mod.hasUnsavedDocuments();
}
