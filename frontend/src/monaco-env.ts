/**
 * Monaco Editor web-worker configuration.
 *
 * This module MUST be imported before any other module that transitively
 * imports 'monaco-editor', because Monaco reads `self.MonacoEnvironment`
 * the moment its ESM chunk is first evaluated.  Vite's static-import
 * evaluation order guarantees that a direct import listed first in main.ts
 * is executed before deeper dependency graphs.
 *
 * We only need the base editor worker – our custom 'typst' language uses
 * the Rust backend for all intelligence, so none of the language-specific
 * workers (json, css, ts …) are required.
 */

// Vite resolves `?worker` at build time; the `/* @vite-ignore */` comment
// suppresses the TypeScript "cannot find module" error for the query-suffix
// import, which is a Vite-specific feature not covered by the TS compiler.
// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
// eslint-disable-next-line import/no-unresolved
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";

(self as unknown as Record<string, unknown>).MonacoEnvironment = {
  getWorker(_moduleId: string, _label: string): Worker {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    return new (EditorWorker as any)();
  },
};
