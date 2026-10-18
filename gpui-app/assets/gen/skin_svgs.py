"""Write the skeuomorphic SVG skin used by the GPUI front end.

Full-colour images are drawn with an intrinsic size of 3x their display size,
because gpui rasterizes colour SVG images at their intrinsic size.
Monochrome glyphs (g_*.svg) are tinted by gpui and may stay at 24px.
"""
import os
import sys

OUT = sys.argv[1]
os.makedirs(OUT, exist_ok=True)

FILES = {}

FILES["paperclip.svg"] = """<svg xmlns="http://www.w3.org/2000/svg" width="108" height="54" viewBox="0 0 36 18">
  <defs>
    <linearGradient id="m" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#ffffff"/><stop offset=".35" stop-color="#d9d9d6"/>
      <stop offset=".55" stop-color="#9c9b97"/><stop offset=".8" stop-color="#e8e7e3"/>
      <stop offset="1" stop-color="#8d8b86"/>
    </linearGradient>
  </defs>
  <path d="M3.5 4.6h24.3a4.6 4.6 0 0 1 0 9.2H8.6a3 3 0 0 1 0-6h17.6" fill="none" stroke="#5a4632"
        stroke-opacity=".22" stroke-width="2.4" stroke-linecap="round" transform="translate(.6 1.4)"/>
  <path d="M3.5 4.6h24.3a4.6 4.6 0 0 1 0 9.2H8.6a3 3 0 0 1 0-6h17.6" fill="none" stroke="#6f6d68"
        stroke-width="2.6" stroke-linecap="round"/>
  <path d="M3.5 4.6h24.3a4.6 4.6 0 0 1 0 9.2H8.6a3 3 0 0 1 0-6h17.6" fill="none" stroke="url(#m)"
        stroke-width="1.9" stroke-linecap="round"/>
  <path d="M4 4.2h23.6a4.4 4.4 0 0 1 4.2 3.1" fill="none" stroke="#ffffff" stroke-opacity=".9"
        stroke-width=".45" stroke-linecap="round"/>
</svg>"""

# The same clip turned upright, for clipping the top edge of a page.
FILES["paperclip_v.svg"] = FILES["paperclip.svg"].replace(
    'width="108" height="54" viewBox="0 0 36 18">',
    'width="54" height="108" viewBox="0 0 18 36"><g transform="translate(18 0) rotate(90)">',
).replace("</svg>", "</g></svg>")

# Silk bookmark ribbon hanging from the top edge of the active card.
FILES["ribbon.svg"] = """<svg xmlns="http://www.w3.org/2000/svg" width="36" height="78" viewBox="0 0 12 26">
  <defs><linearGradient id="r" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#9e2b1e"/>
    <stop offset=".45" stop-color="#d0533d"/><stop offset="1" stop-color="#8a2317"/></linearGradient></defs>
  <path d="M2.4 0h8v25l-4-3.4-4 3.4z" fill="#3a1a10" opacity=".25" transform="translate(.5 .6)"/>
  <path d="M2 0h8v25l-4-3.4L2 25z" fill="url(#r)"/>
  <path d="M3.2 0v21.5M8.8 0v21.5" stroke="#f3a08d" stroke-width=".35" stroke-dasharray="1 .8" opacity=".7"/>
</svg>"""

PIN = "M10.6 1.8l5.6 5.6-1.4.6-2.3 2.3.4 3.2-1.3 1.3-3.4-3.4-4.7 4.7-.9-.9 4.7-4.7L3.9 7.1l1.3-1.3 3.2.4 2.3-2.3z"
STAR = "M9 1.6l2.2 4.6 5 .7-3.6 3.5.9 5L9 13l-4.5 2.4.9-5L1.8 6.9l5-.7z"


def letterpress(path):
    """Grey glyph pressed into paper: white highlight 0.7 below."""
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="54" height="54" viewBox="0 0 18 18">
  <path transform="translate(0 .7)" d="{path}" fill="#ffffff" opacity=".95"/>
  <path d="{path}" fill="#dbd3c8" stroke="#c8beb1" stroke-width=".5" stroke-linejoin="round"/>
</svg>"""


FILES["pin.svg"] = letterpress(PIN)
FILES["star.svg"] = letterpress(STAR)

FILES["pin_on.svg"] = f"""<svg xmlns="http://www.w3.org/2000/svg" width="54" height="54" viewBox="0 0 18 18">
  <defs>
    <radialGradient id="h" cx=".35" cy=".3" r=".8"><stop offset="0" stop-color="#f08a72"/>
      <stop offset=".55" stop-color="#c0392b"/><stop offset="1" stop-color="#7c1d14"/></radialGradient>
    <linearGradient id="n" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#ecebe7"/>
      <stop offset="1" stop-color="#8f8e8a"/></linearGradient>
  </defs>
  <ellipse cx="5.4" cy="15.6" rx="3.2" ry="1.1" fill="#4a3320" opacity=".25"/>
  <path d="M8.4 9.6L3.2 14.8" stroke="url(#n)" stroke-width="1.3" stroke-linecap="round"/>
  <path d="M10.6 1.8l5.6 5.6-1.4.6-2.3 2.3.4 3.2-1.3 1.3L4.1 7.1l1.3-1.3 3.2.4 2.3-2.3z"
        fill="url(#h)" stroke="#6e1a12" stroke-width=".45"/>
  <path d="M11 3.1l1.9 1.9" stroke="#ffffff" stroke-opacity=".7" stroke-width=".8" stroke-linecap="round"/>
</svg>"""

FILES["star_on.svg"] = f"""<svg xmlns="http://www.w3.org/2000/svg" width="54" height="54" viewBox="0 0 18 18">
  <defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffe7a1"/>
    <stop offset=".5" stop-color="#f2b938"/><stop offset="1" stop-color="#c98a14"/></linearGradient></defs>
  <path transform="translate(.3 .9)" d="{STAR}" fill="#5a3d10" opacity=".25"/>
  <path d="{STAR}" fill="url(#g)" stroke="#a96f0c" stroke-width=".55" stroke-linejoin="round"/>
  <path d="M9 3.3l1.5 3.1" stroke="#fff6d8" stroke-width=".8" stroke-linecap="round" opacity=".9"/>
</svg>"""

FILES["notebook.svg"] = """<svg xmlns="http://www.w3.org/2000/svg" width="72" height="72" viewBox="0 0 24 24">
  <defs>
    <linearGradient id="c" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#c08d58"/>
      <stop offset="1" stop-color="#7a5230"/></linearGradient>
    <linearGradient id="p" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#fffaf0"/>
      <stop offset="1" stop-color="#e9dcc4"/></linearGradient>
  </defs>
  <rect x="4.6" y="2.6" width="15.6" height="19.4" rx="1.6" fill="#2a1c10" opacity=".35" transform="translate(.5 .7)"/>
  <rect x="6" y="3.2" width="14.4" height="18.2" rx="1.2" fill="url(#p)"/>
  <rect x="3.6" y="2.4" width="14.4" height="19.2" rx="1.4" fill="url(#c)" stroke="#5c3b20" stroke-width=".4"/>
  <path d="M4.6 3.3h12.4" stroke="#e7c398" stroke-width=".4" opacity=".8"/>
  <rect x="3.6" y="2.4" width="2.4" height="19.2" rx="1" fill="#5c3b20" opacity=".5"/>
  <rect x="15.2" y="2.4" width="1.3" height="19.2" fill="#3b2614" opacity=".85"/>
  <rect x="8" y="8.6" width="5.6" height="5.2" rx=".5" fill="#f3e3c4" opacity=".92"/>
  <path d="M9 10.2h3.6M9 11.4h3.6M9 12.6h2.4" stroke="#9c7650" stroke-width=".4"/>
</svg>"""

for name, (hi, mid, lo) in {
    "green": ("#b8f48c", "#4fae2c", "#1f6b10"),
    "amber": ("#ffe08a", "#e2a52a", "#8c5a0c"),
    "red": ("#ff9a86", "#d4402c", "#7a170c"),
}.items():
    FILES[f"led_{name}.svg"] = f"""<svg xmlns="http://www.w3.org/2000/svg" width="36" height="36" viewBox="0 0 12 12">
  <defs><radialGradient id="l" cx=".38" cy=".32" r=".75"><stop offset="0" stop-color="{hi}"/>
    <stop offset=".6" stop-color="{mid}"/><stop offset="1" stop-color="{lo}"/></radialGradient></defs>
  <circle cx="6" cy="6" r="5.4" fill="#2b2016" opacity=".55"/>
  <circle cx="6" cy="6" r="4.4" fill="url(#l)"/>
  <ellipse cx="4.9" cy="4.3" rx="1.7" ry="1.1" fill="#ffffff" opacity=".7"/>
</svg>"""

GLYPHS = {
    "compose": '<path d="M4 5.5A1.5 1.5 0 0 1 5.5 4H13v2.2H6.2v11.6h11.6V11H20v7.5a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 18.5z"/><path d="M18.6 2.6l2.8 2.8-8.6 8.6-3.6.8.8-3.6z"/>',
    "folder": '<path d="M2.5 6A1.5 1.5 0 0 1 4 4.5h5.2l2 2H20A1.5 1.5 0 0 1 21.5 8v1H6.4a1.8 1.8 0 0 0-1.7 1.2L2.5 17z"/><path d="M5.6 10.5h17.2l-2.6 8a1.5 1.5 0 0 1-1.4 1H3.4z"/>',
    "disk": '<path fill-rule="evenodd" d="M4.5 3h12.2L21 7.3V19.5A1.5 1.5 0 0 1 19.5 21h-15A1.5 1.5 0 0 1 3 19.5v-15A1.5 1.5 0 0 1 4.5 3zM7 5v4.5h8.5V5zM6.5 13v6h11v-6zM12.5 5.8h2v3h-2z"/>',
    "compile": '<path d="M12 3.5a8.5 8.5 0 0 1 7.6 4.7l1.4-1.4v5.2h-5.2l2.1-2.1A6.3 6.3 0 0 0 5.8 11H3.6A8.5 8.5 0 0 1 12 3.5z"/><path d="M12 20.5a8.5 8.5 0 0 1-7.6-4.7L3 17.2V12h5.2l-2.1 2.1A6.3 6.3 0 0 0 18.2 13h2.2a8.5 8.5 0 0 1-8.4 7.5z"/>',
    "plane": '<path d="M21.6 2.6L2.4 10.4l6.4 2.6 9-6.6-6.8 7.8v6.2l3.4-3.9 3.9 1.6z"/>',
    "sidebar": '<path fill-rule="evenodd" d="M4.5 4h15A1.5 1.5 0 0 1 21 5.5v13a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5v-13A1.5 1.5 0 0 1 4.5 4zM9.5 6.2v11.6h9.3V6.2zM5 7h3v1.6H5zm0 3h3v1.6H5zm0 3h3v1.6H5z"/>',
    "triangle": '<path d="M5 8.5h14l-7 8z"/>',
    "search": '<path fill-rule="evenodd" d="M10.5 3a7.5 7.5 0 0 1 6 12l4.6 4.6-1.6 1.6-4.6-4.6A7.5 7.5 0 1 1 10.5 3zm0 2.2a5.3 5.3 0 1 0 0 10.6 5.3 5.3 0 0 0 0-10.6z"/>',
    "min": '<path d="M5 11h14v2H5z"/>',
    "max": '<path fill-rule="evenodd" d="M5 5h14v14H5zm2 3v9h10V8z"/>',
    "close": '<path d="M6.4 5l5.6 5.6L17.6 5 19 6.4 13.4 12l5.6 5.6-1.4 1.4-5.6-5.6L6.4 19 5 17.6 10.6 12 5 6.4z"/>',
    "plus": '<path d="M11 5h2v6h6v2h-6v6h-2v-6H5v-2h6z"/>',
    "outline": '<path d="M3 5h3v2.4H3zm5 0h13v2.4H8zm-2 5.8h3v2.4H6zm5 0h10v2.4H11zm-5 5.8h3V19H6zm5 0h10V19H11z"/>',
    # File tree: one glyph per kind of file, plus disclosure chevrons.
    "doc": '<path fill-rule="evenodd" d="M6 2.5h8.4L19 7.1V20a1.5 1.5 0 0 1-1.5 1.5h-11.5A1.5 1.5 0 0 1 4.5 20V4A1.5 1.5 0 0 1 6 2.5zm7.6 1.6V8h3.9zM7.5 11h9v1.6h-9zm0 3.4h9V16h-9zm0 3.4h6v1.6h-6z"/>',
    "file": '<path fill-rule="evenodd" d="M6 2.5h8.4L19 7.1V20a1.5 1.5 0 0 1-1.5 1.5h-11.5A1.5 1.5 0 0 1 4.5 20V4A1.5 1.5 0 0 1 6 2.5zm7.6 1.6V8h3.9z"/>',
    "image": '<path fill-rule="evenodd" d="M4.5 3.5h15A1.5 1.5 0 0 1 21 5v14a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 19V5a1.5 1.5 0 0 1 1.5-1.5zm.7 2.2v10.6l4.3-4.6 3.2 3.4 2.2-2.2 3.9 4.1V5.7zM15.5 7a2 2 0 1 1 0 4 2 2 0 0 1 0-4z"/>',
    "book": '<path d="M3 4.5c3.3-.9 6.1-.4 8.2 1.3V20c-2.2-1.6-4.9-2-8.2-1.2zM21 4.5c-3.3-.9-6.1-.4-8.2 1.3V20c2.2-1.6 4.9-2 8.2-1.2z"/>',
    "braces": '<path d="M9 3H7.8C6.2 3 5.4 3.9 5.4 5.4v3.3c0 1.2-.7 2-1.9 2.2v2.2c1.2.2 1.9 1 1.9 2.2v3.3C5.4 20.1 6.2 21 7.8 21H9v-2.2H8.2c-.4 0-.6-.2-.6-.7v-3.5c0-1.4-.6-2.3-1.6-2.6 1-.3 1.6-1.2 1.6-2.6V5.9c0-.5.2-.7.6-.7H9zm6 0h1.2c1.6 0 2.4.9 2.4 2.4v3.3c0 1.2.7 2 1.9 2.2v2.2c-1.2.2-1.9 1-1.9 2.2v3.3c0 1.5-.8 2.4-2.4 2.4H15v-2.2h.8c.4 0 .6-.2.6-.7v-3.5c0-1.4.6-2.3 1.6-2.6-1-.3-1.6-1.2-1.6-2.6V5.9c0-.5-.2-.7-.6-.7H15z"/>',
    "folder_open": '<path d="M2.5 6A1.5 1.5 0 0 1 4 4.5h5.2l2 2H18A1.5 1.5 0 0 1 19.5 8v1.5H7.1a2 2 0 0 0-1.9 1.4L2.5 18z"/><path d="M7 11h15l-2.9 8a1.5 1.5 0 0 1-1.4 1H3.6z"/>',
    "chevron_down": '<path d="M5.6 8.4L12 14.8l6.4-6.4 1.6 1.6-8 8-8-8z"/>',
    "chevron_up": '<path d="M5.6 15.6L12 9.2l6.4 6.4 1.6-1.6-8-8-8 8z"/>',
    "files": '<path d="M7 2.5h8.2L20 7.3V17a1.5 1.5 0 0 1-1.5 1.5H7A1.5 1.5 0 0 1 5.5 17V4A1.5 1.5 0 0 1 7 2.5zm7.5 1.3V8h4.2z"/><path d="M3.5 6.5v13A1.5 1.5 0 0 0 5 21h10.5v-1.6H5.1V6.5z"/>',
}
for name, body in GLYPHS.items():
    FILES[f"g_{name}.svg"] = (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">{body}</svg>'
    )

for name, svg in FILES.items():
    with open(os.path.join(OUT, name), "w", encoding="utf-8") as f:
        f.write(svg + "\n")
print(len(FILES), "files")
