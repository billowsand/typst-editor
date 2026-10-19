#!/usr/bin/env bash
#
# Build, test and package 字笺 · Typst (GPUI) as a macOS .app inside a DMG.
#
# Usage:
#   gpui-app/scripts/package-macos.sh [--allow-dirty] [--skip-tests] [--out-dir DIR]
#
# The resulting app is ad-hoc signed. It is not notarized: when copying the
# DMG to another Mac, open the app once with right-click → Open.
set -euo pipefail

app="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repo="$(cd "$app/.." && pwd)"
out_dir="$repo/dist"
allow_dirty=0
skip_tests=0

usage() {
  sed -n '2,10p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

while (($#)); do
  case "$1" in
    --allow-dirty) allow_dirty=1 ;;
    --skip-tests) skip_tests=1 ;;
    --out-dir)
      [[ $# -ge 2 ]] || { echo "--out-dir needs a value" >&2; exit 2; }
      out_dir="$2"
      shift
      ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

step() { printf '\n==> %s\n' "$*"; }
fail() { printf 'error: %s\n' "$*" >&2; exit 1; }

[[ "$(uname -s)" == Darwin ]] || fail "run this script on macOS"
command -v iconutil >/dev/null || fail "iconutil not found"
command -v hdiutil >/dev/null || fail "hdiutil not found"
command -v sips >/dev/null || fail "sips not found"

cd "$repo"

# ── Provenance ────────────────────────────────────────────────
dirty=0
if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
  dirty=1
fi
if ((dirty && !allow_dirty)); then
  fail "working tree has uncommitted changes. Commit them, or pass --allow-dirty for a test build."
fi

raw_commit="$(git rev-parse --short=7 HEAD)"
commit="$raw_commit"
((dirty)) && commit="$commit-dirty"
version="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\(.*\)"/\1/p' "$app/Cargo.toml" | head -1)"
[[ -n "$version" ]] || fail "cannot read version from gpui-app/Cargo.toml"
date="$(TZ=Asia/Shanghai date +%F)"
arch="$(uname -m)"
name="Zijian-$version-$commit-macos-$arch"
# build.rs appends -dirty itself when the tree is dirty; give it the raw hash
# so the baked commit is not marked dirty twice.
export ZIJIAN_COMMIT="$raw_commit"
export ZIJIAN_BUILD_DATE="$date"

step "Packaging $name"

# ── Tests ─────────────────────────────────────────────────────
if ((!skip_tests)); then
  step "Testing typst-core"
  cargo test --manifest-path crates/typst-core/Cargo.toml --release \
    --target-dir "$app/target"
  step "Testing gpui-app"
  cargo test --manifest-path gpui-app/Cargo.toml --release
fi

# ── Release build ─────────────────────────────────────────────
step "Building release binary"
cargo build --manifest-path gpui-app/Cargo.toml --release --locked
bin="$app/target/release/typst-editor-gpui"
[[ -x "$bin" ]] || fail "missing release binary: $bin"

# ── Stage .app ────────────────────────────────────────────────
step "Staging app bundle"
rm -rf "$out_dir/$name" "$out_dir/$name.dmg" "$out_dir/$name.dmg.sha256"
mkdir -p "$out_dir"
stage="$out_dir/$name"
bundle="$stage/字笺 · Typst.app"
contents="$bundle/Contents"
mkdir -p "$contents/MacOS" "$contents/Resources/docs"
cp "$bin" "$contents/MacOS/typst-editor-gpui"
chmod +x "$contents/MacOS/typst-editor-gpui"
cp -R "$app/examples" "$contents/MacOS/examples"
cp "$app/docs/用户指南.md" "$contents/Resources/docs/"
cp "$app/THIRD-PARTY-NOTICES.md" "$contents/Resources/"
printf 'APPL????' > "$contents/PkgInfo"

# ── App icon ──────────────────────────────────────────────────
icon_src="$app/assets/icons/zijian-1024.png"
[[ -f "$icon_src" ]] || icon_src="$app/assets/icons/zijian.png"
iconset="$app/target/zijian.iconset"
rm -rf "$iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$icon_src" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$icon_src" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$contents/Resources/zijian.icns"

# ── Info.plist ────────────────────────────────────────────────
cat > "$contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>zh_CN</string>
  <key>CFBundleDisplayName</key><string>字笺 · Typst</string>
  <key>CFBundleName</key><string>字笺 Typst</string>
  <key>CFBundleExecutable</key><string>typst-editor-gpui</string>
  <key>CFBundleIdentifier</key><string>app.zijian.typst</string>
  <key>CFBundleIconFile</key><string>zijian.icns</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <key>NSHumanReadableCopyright</key><string>字笺 · Typst Contributors</string>
</dict>
</plist>
PLIST

plutil -lint "$contents/Info.plist" >/dev/null || fail "Info.plist is invalid"

cat > "$stage/使用说明.txt" <<TXT
字笺 · Typst — macOS 版 ${version}（${commit}，${date}，${arch}）

1. 把“字笺 · Typst.app”拖到“应用程序”文件夹。
2. 首次启动若被 Gatekeeper 拦截，请在 Finder 中右键应用 →“打开”。
3. 欢迎页可新建项目、打开文件夹或打开排版示例。
4. 使用说明见应用包 Contents/Resources/docs/用户指南.md，
   第三方声明见 Contents/Resources/THIRD-PARTY-NOTICES.md。

设置、崩溃恢复草稿与日志保存在 ~/Library/Application Support/Zijian
（可用环境变量 ZIJIAN_DATA_DIR 改到别处）。

本构建仅做了 ad-hoc 签名，未经 Apple 公证。
TXT

cat > "$stage/build-info.json" <<JSON
{
  "name": "字笺 · Typst GPUI",
  "version": "$version",
  "commit": "$commit",
  "buildDate": "$date",
  "platform": "macos-$arch",
  "profile": "release",
  "rustc": "$(rustc --version | tr -d '\n')",
  "appSha256": "$(shasum -a 256 "$contents/MacOS/typst-editor-gpui" | cut -d' ' -f1)"
}
JSON

# ── Sign and smoke test ───────────────────────────────────────
step "Ad-hoc signing"
xattr -cr "$bundle"
codesign --force --deep --sign - "$bundle"
codesign --verify --deep --strict "$bundle"

step "Smoke test"
smoke_dir="$(mktemp -d "${TMPDIR:-/tmp}/zijian-smoke.XXXXXX")"
log_dir="$smoke_dir/logs"
ZIJIAN_DATA_DIR="$smoke_dir" "$contents/MacOS/typst-editor-gpui" \
  "$contents/MacOS/examples" >/dev/null 2>&1 &
pid=$!
ok=0
for _ in $(seq 1 20); do
  sleep 0.5
  if ! kill -0 "$pid" 2>/dev/null; then break; fi
  if [[ -f "$log_dir/zijian.log" ]] && grep -q 'open project' "$log_dir/zijian.log"; then
    ok=1
    break
  fi
done
alive=0
kill -0 "$pid" 2>/dev/null && alive=1
kill "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
if ((!ok || !alive)); then
  echo "--- log ---" >&2
  cat "$log_dir/zijian.log" >&2 2>/dev/null || true
  rm -rf "$smoke_dir"
  fail "smoke test failed (alive=$alive, project opened=$ok)"
fi
rm -rf "$smoke_dir"

# ── DMG ───────────────────────────────────────────────────────
step "Building DMG"
dmg_root="$stage/dmg"
mkdir -p "$dmg_root"
mv "$bundle" "$dmg_root/"
cp "$stage/使用说明.txt" "$dmg_root/"
ln -s /Applications "$dmg_root/Applications"

dmg="$out_dir/$name.dmg"
hdiutil create -quiet -volname "Zijian Typst $version" \
  -srcfolder "$dmg_root" -ov -format UDZO "$dmg"
rm -rf "$dmg_root"
shasum -a 256 "$dmg" | sed "s#  $out_dir/#  #" > "$dmg.sha256"

step "Done: $dmg"
cat "$dmg.sha256"
