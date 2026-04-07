export interface PreviewPagePosition {
  page: number;
  x: number;
  y: number;
}

export interface PreviewPanePoint {
  left: number;
  top: number;
  pageWidth: number;
}

const PREVIEW_CONTENT_SELECTOR = ".preview-content";
const PREVIEW_PAGE_SELECTOR = ".preview-page";

// Preview panel initialization
export function initPreview(container: HTMLDivElement): void {
  const existing = container.querySelector(PREVIEW_CONTENT_SELECTOR);
  if (existing) return;

  const content = document.createElement("div");
  content.className = "preview-content";
  container.appendChild(content);

  content.innerHTML = `
    <div style="display:flex;align-items:center;justify-content:center;height:100%;color:#888;font-size:14px;">
      Start typing to see a live preview...
    </div>
  `;
}

export function getPreviewPane(): HTMLDivElement | null {
  return document.getElementById("preview-pane") as HTMLDivElement | null;
}

export function getPreviewContent(): HTMLDivElement | null {
  return getPreviewPane()?.querySelector<HTMLDivElement>(PREVIEW_CONTENT_SELECTOR) ?? null;
}

export function ensurePreviewContent(): HTMLDivElement {
  const pane = getPreviewPane();
  if (!pane) throw new Error("Preview pane not found");

  let content = pane.querySelector<HTMLDivElement>(PREVIEW_CONTENT_SELECTOR);
  if (!content) {
    content = document.createElement("div");
    content.className = "preview-content";
    pane.appendChild(content);
  }

  return content;
}

export function ensurePreviewPageNode(index: number): HTMLDivElement {
  const content = ensurePreviewContent();
  let node = content.querySelector<HTMLDivElement>(`[data-page-index="${index}"]`);
  if (!node) {
    node = document.createElement("div");
    node.className = "preview-page";
    node.dataset.pageIndex = String(index);
    content.appendChild(node);
  }
  return node;
}

export function trimPreviewPages(pageCount: number): void {
  getPreviewContent()
    ?.querySelectorAll<HTMLDivElement>(PREVIEW_PAGE_SELECTOR)
    .forEach((node) => {
      if (Number(node.dataset.pageIndex ?? -1) >= pageCount) node.remove();
    });
}

export function resetPreviewContent(): void {
  ensurePreviewContent().replaceChildren();
}

export function replacePreviewPageSvg(
  pageIndex: number,
  svg: string,
  onClick: (event: MouseEvent, svgEl: SVGSVGElement, pageIndex: number) => void,
): boolean {
  const node = ensurePreviewPageNode(pageIndex);
  const wrapper = document.createElement("div");
  wrapper.innerHTML = svg;
  const svgEl = wrapper.firstElementChild as SVGSVGElement | null;
  if (!svgEl) return false;

  svgEl.addEventListener("click", (event) => onClick(event, svgEl, pageIndex));
  node.replaceChildren(svgEl);
  return true;
}

export function getPreviewPageSvg(page: number): SVGSVGElement | null {
  return getPreviewContent()
    ?.querySelector(`.preview-page[data-page-index="${page - 1}"] svg`) as SVGSVGElement | null;
}

export function clientPointToPagePoint(
  svgEl: SVGSVGElement,
  clientX: number,
  clientY: number,
): Pick<PreviewPagePosition, "x" | "y"> {
  const rect = svgEl.getBoundingClientRect();
  const { width, height } = getSvgNaturalSize(svgEl);
  return {
    x: (clientX - rect.left) * (width / rect.width),
    y: (clientY - rect.top) * (height / rect.height),
  };
}

export function pagePointToPanePoint(pos: PreviewPagePosition): PreviewPanePoint | null {
  const pane = getPreviewPane();
  const svgEl = getPreviewPageSvg(pos.page);
  if (!pane || !svgEl) return null;

  const paneRect = pane.getBoundingClientRect();
  const rect = svgEl.getBoundingClientRect();
  const { width, height } = getSvgNaturalSize(svgEl);
  const scaleX = rect.width / width;
  const scaleY = rect.height / height;

  return {
    left: rect.left - paneRect.left + pane.scrollLeft + pos.x * scaleX,
    top: rect.top - paneRect.top + pane.scrollTop + pos.y * scaleY,
    pageWidth: rect.width,
  };
}

export function scrollPreviewToPosition(
  pos: PreviewPagePosition,
  behavior: ScrollBehavior = "smooth",
): void {
  const pane = getPreviewPane();
  const panePoint = pagePointToPanePoint(pos);
  if (!pane || !panePoint) return;

  pane.scrollTo({
    top: Math.max(0, panePoint.top - pane.clientHeight / 2),
    behavior,
  });
}

function getSvgNaturalSize(svgEl: SVGSVGElement): { width: number; height: number } {
  const viewBox = svgEl.viewBox.baseVal;
  if (viewBox && viewBox.width > 0 && viewBox.height > 0) {
    return { width: viewBox.width, height: viewBox.height };
  }

  const width = Number.parseFloat(svgEl.getAttribute("width") || "0");
  const height = Number.parseFloat(svgEl.getAttribute("height") || "0");

  return {
    width: width || 1,
    height: height || 1,
  };
}
