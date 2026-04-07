interface WorkspaceShellOptions {
  splitContainer: HTMLDivElement;
  sidebarPane: HTMLDivElement;
  treeGutter: HTMLDivElement;
  editorPane: HTMLDivElement;
  previewGutter: HTMLDivElement;
  previewPane: HTMLDivElement;
  onOpen: () => void | Promise<void>;
}

let shell: WorkspaceShellOptions | null = null;
let emptyStateEl: HTMLDivElement | null = null;

export function initWorkspaceShell(options: WorkspaceShellOptions): void {
  shell = options;
  emptyStateEl = document.createElement("div");
  emptyStateEl.className = "workspace-empty-state";
  emptyStateEl.innerHTML = `
    <div class="workspace-empty-icon" aria-hidden="true">
      <svg viewBox="0 0 64 64" fill="none">
        <path d="M14 18.5A4.5 4.5 0 0 1 18.5 14h13.2a4.5 4.5 0 0 1 3.2 1.3l2.8 2.8a4.5 4.5 0 0 0 3.2 1.4h4.6A4.5 4.5 0 0 1 50 24v21.5A4.5 4.5 0 0 1 45.5 50h-27A4.5 4.5 0 0 1 14 45.5z" stroke="currentColor" stroke-width="2.4" stroke-linejoin="round"/>
        <path d="M20 30.5h24" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"/>
        <path d="M32 24v13" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"/>
      </svg>
    </div>
    <div class="workspace-empty-title">Open a Typst file</div>
    <div class="workspace-empty-copy">No file is loaded. Open a project file to start editing and previewing.</div>
    <button type="button" class="workspace-empty-button">Open File</button>
  `;

  const button = emptyStateEl.querySelector<HTMLButtonElement>(".workspace-empty-button");
  button?.addEventListener("click", () => {
    void shell?.onOpen();
  });

  shell.splitContainer.appendChild(emptyStateEl);
  showEmptyWorkspace();
}

export function showEmptyWorkspace(): void {
  if (!shell || !emptyStateEl) return;

  shell.sidebarPane.style.display = "none";
  shell.treeGutter.style.display = "none";
  shell.editorPane.style.display = "none";
  shell.previewGutter.style.display = "none";
  shell.previewPane.style.display = "none";
  emptyStateEl.style.display = "flex";
}

export function showWorkspaceLayout(): void {
  if (!shell || !emptyStateEl) return;

  shell.sidebarPane.style.display = "";
  shell.treeGutter.style.display = "";
  shell.editorPane.style.display = "";
  shell.previewGutter.style.display = "";
  shell.previewPane.style.display = "";
  emptyStateEl.style.display = "none";
}
