// Preview panel initialization
export function initPreview(container: HTMLDivElement): void {
  const existing = container.querySelector(".preview-content");
  if (existing) return;

  const content = document.createElement("div");
  content.className = "preview-content";
  container.appendChild(content);

  // Placeholder
  content.innerHTML = `
    <div style="display:flex;align-items:center;justify-content:center;height:100%;color:#888;font-size:14px;">
      Start typing to see a live preview...
    </div>
  `;
}
