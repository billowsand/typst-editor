import { invoke } from "@tauri-apps/api/core";
import { openFileFromTree } from "../editor/MonacoEditor";

interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
}

const INDENT = 16; // px per level

export async function initFileTree(container: HTMLDivElement): Promise<void> {
  container.innerHTML = '<div class="tree-empty">Open a file to show project tree</div>';

  // Try to get project root if a file is already open
  try {
    const root = await invoke<string | null>("get_project_root");
    if (root) {
      await loadTree(container, root, 0);
    }
  } catch {
    // no project root yet
  }
}

export async function loadTree(
  container: HTMLDivElement,
  rootPath: string,
  depth: number
): Promise<void> {
  container.innerHTML = "";

  // Header showing root name
  const header = document.createElement("div");
  header.className = "tree-header";
  header.textContent = rootPath.split(/[\\/]/).pop() || rootPath;
  header.title = rootPath;
  container.appendChild(header);

  const treeList = document.createElement("div");
  treeList.className = "tree-list";
  container.appendChild(treeList);

  const loading = document.createElement("div");
  loading.className = "tree-empty";
  loading.textContent = "Loading directory...";
  treeList.appendChild(loading);

  await loadDirectory(treeList, rootPath, depth);
}

async function loadDirectory(
  parent: HTMLElement,
  dirPath: string,
  depth: number
): Promise<void> {
  let entries: FileEntry[];
  try {
    entries = await invoke<FileEntry[]>("list_dir", { path: dirPath });
  } catch {
    parent.innerHTML = "";
    const err = document.createElement("div");
    err.className = "tree-error";
    err.textContent = "Failed to load directory";
    err.style.paddingLeft = `${depth * INDENT + 8}px`;
    parent.appendChild(err);
    return;
  }

  parent.innerHTML = "";

  for (const entry of entries) {
    const item = document.createElement("div");
    item.className = `tree-item ${entry.is_dir ? "dir" : "file"}`;
    item.style.paddingLeft = `${depth * INDENT + 8}px`;
    item.title = entry.path;

    const icon = document.createElement("span");
    icon.className = "tree-icon";

    const label = document.createElement("span");
    label.className = "tree-label";
    label.textContent = entry.name;

    item.appendChild(icon);
    item.appendChild(label);

    if (entry.is_dir) {
      // Directory: toggle expand/collapse
      let expanded = false;
      let childContainer: HTMLDivElement | null = null;

      item.addEventListener("click", async (e) => {
        e.stopPropagation();
        if (expanded && childContainer) {
          // Collapse
          childContainer.remove();
          childContainer = null;
          expanded = false;
          item.classList.remove("expanded");
        } else {
          // Expand
          expanded = true;
          item.classList.add("expanded");
          childContainer = document.createElement("div");
          childContainer.className = "tree-children";
          item.after(childContainer);
          await loadDirectory(childContainer, entry.path, depth + 1);
        }
      });
    } else {
      // File: open in editor
      item.addEventListener("click", async (e) => {
        e.stopPropagation();
        // Highlight active file
        parent.querySelectorAll(".tree-item.active").forEach((el) =>
          el.classList.remove("active")
        );
        item.classList.add("active");
        await openFileFromTree(entry.path);
      });
    }

    parent.appendChild(item);
  }
}
