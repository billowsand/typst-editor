<#
.SYNOPSIS
  Build, test and package 字笺 · Typst (GPUI) as a portable Windows ZIP.

.DESCRIPTION
  Reproducible from a clean checkout:
    1. refuses a dirty working tree (unless -AllowDirty),
    2. runs the typst-core and gpui-app tests (unless -SkipTests),
    3. builds the release binary with the commit and date baked in,
    4. stages exe + examples + docs + licenses + build-info.json,
    5. zips it and writes a SHA-256 checksum,
    6. smoke-tests the staged exe: it must start and write its log.

  Output: dist\Zijian-<version>-<commit>-windows-x64(.zip|.zip.sha256)

  If the Windows SDK's fxc.exe cannot be found automatically, set
  GPUI_FXC_PATH to it before running.

.EXAMPLE
  pwsh gpui-app/scripts/package-windows.ps1
#>
param(
    [switch]$AllowDirty,
    [switch]$SkipTests,
    [string]$OutDir
)

$ErrorActionPreference = 'Stop'
$app = Split-Path -Parent $PSScriptRoot           # gpui-app
$repo = Split-Path -Parent $app                    # repository root
if (-not $OutDir) { $OutDir = Join-Path $repo 'dist' }

function Step($text) { Write-Host "==> $text" -ForegroundColor Cyan }
function Run($exe, [string[]]$arguments) {
    & $exe @arguments
    if ($LASTEXITCODE -ne 0) { throw "$exe $($arguments -join ' ') failed ($LASTEXITCODE)" }
}

Push-Location $repo
try {
    # ── Version and provenance ──────────────────────────────
    $dirty = (git status --porcelain --untracked-files=no) -ne $null
    if ($dirty -and -not $AllowDirty) {
        throw 'Working tree has uncommitted changes. Commit them, or pass -AllowDirty for a test build.'
    }
    $commit = (git rev-parse --short=7 HEAD).Trim()
    if ($dirty) { $commit = "$commit-dirty" }
    $version = (Select-String -Path (Join-Path $app 'Cargo.toml') -Pattern '^version\s*=\s*"(.+)"' |
        Select-Object -First 1).Matches[0].Groups[1].Value
    $date = (Get-Date).ToString('yyyy-MM-dd')
    $env:ZIJIAN_COMMIT = $commit
    $env:ZIJIAN_BUILD_DATE = $date
    $name = "Zijian-$version-$commit-windows-x64"
    Step "Packaging $name"

    # ── Tests ───────────────────────────────────────────────
    if (-not $SkipTests) {
        Step 'Testing typst-core'
        Run cargo @('test', '--manifest-path', 'crates/typst-core/Cargo.toml', '--release',
                    '--target-dir', (Join-Path $app 'target'))
        Step 'Testing gpui-app'
        Run cargo @('test', '--manifest-path', 'gpui-app/Cargo.toml')
    }

    # ── Release build ───────────────────────────────────────
    Step 'Building release binary'
    Run cargo @('build', '--manifest-path', 'gpui-app/Cargo.toml', '--release', '--locked')
    $exe = Join-Path $app 'target\release\typst-editor-gpui.exe'

    # ── Stage ───────────────────────────────────────────────
    $stage = Join-Path $OutDir $name
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force -Path $stage, "$stage\docs", "$stage\licenses" | Out-Null
    Copy-Item $exe $stage
    Copy-Item -Recurse (Join-Path $app 'examples') "$stage\examples"
    Copy-Item (Join-Path $app 'docs\用户指南.md') "$stage\docs\"
    Copy-Item (Join-Path $app 'THIRD-PARTY-NOTICES.md') $stage

    # Original license files of bundled components, located via cargo.
    $meta = cargo metadata --format-version 1 --manifest-path gpui-app/Cargo.toml | ConvertFrom-Json
    foreach ($crate in 'typst-assets', 'typst', 'gpui-pre', 'gpui-kit', 'gpui-kit-assets') {
        $pkg = $meta.packages | Where-Object name -eq $crate | Select-Object -First 1
        if (-not $pkg) { continue }
        $dir = Split-Path $pkg.manifest_path
        foreach ($file in Get-ChildItem $dir -File | Where-Object Name -match '^(LICENSE|NOTICE|COPYING)') {
            Copy-Item $file.FullName "$stage\licenses\$crate-$($file.BaseName).txt"
        }
    }

    @"
字笺 · Typst — Windows 便携版 $version（$commit，$date）

1. 解压到可写文件夹，双击 typst-editor-gpui.exe 启动，无需安装。
2. 欢迎页可新建项目、打开文件夹或打开排版示例（examples 文件夹）。
3. 使用说明见 docs\用户指南.md；第三方声明见 THIRD-PARTY-NOTICES.md 与 licenses\。

设置、崩溃恢复草稿与日志保存在 %LOCALAPPDATA%\Zijian（可用环境变量 ZIJIAN_DATA_DIR 改到别处）。
更新：退出后用新版本文件夹替换旧文件夹，设置会被沿用。
卸载：删除程序文件夹；如需同时清除设置与日志，删除 %LOCALAPPDATA%\Zijian。
"@ | Set-Content -Encoding utf8 "$stage\使用说明.txt"

    $exeHash = (Get-FileHash -Algorithm SHA256 "$stage\typst-editor-gpui.exe").Hash
    [ordered]@{
        name        = '字笺 · Typst GPUI'
        version     = $version
        commit      = $commit
        sourceCommit = (git rev-parse HEAD).Trim()
        buildDate   = $date
        platform    = 'windows-x64'
        profile     = 'release'
        rustc       = (rustc --version).Trim()
        exeSha256   = $exeHash
    } | ConvertTo-Json | Set-Content -Encoding utf8 "$stage\build-info.json"

    # ── Smoke test: the staged exe starts and logs ──────────
    Step 'Smoke test'
    $data = Join-Path ([IO.Path]::GetTempPath()) "zijian-smoke-$([guid]::NewGuid())"
    New-Item -ItemType Directory -Force -Path $data | Out-Null
    $env:ZIJIAN_DATA_DIR = $data
    $proc = Start-Process -FilePath "$stage\typst-editor-gpui.exe" -ArgumentList "`"$stage\examples`"" -PassThru
    Start-Sleep -Seconds 8
    $alive = -not $proc.HasExited
    $log = Join-Path $data 'logs\zijian.log'
    $started = (Test-Path $log) -and (Select-String -Path $log -Pattern 'open project' -Quiet)
    if ($alive) { Stop-Process -Id $proc.Id -Force }
    Remove-Item Env:\ZIJIAN_DATA_DIR
    if (-not ($alive -and $started)) {
        throw "Smoke test failed (alive=$alive, project opened=$started). Log: $log"
    }
    Remove-Item -Recurse -Force $data

    # ── Zip and checksum ────────────────────────────────────
    Step 'Compressing'
    $zip = Join-Path $OutDir "$name.zip"
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Compress-Archive -Path $stage -DestinationPath $zip
    $zipHash = (Get-FileHash -Algorithm SHA256 $zip).Hash
    "$zipHash  $name.zip" | Set-Content -Encoding ascii "$zip.sha256"

    Step "Done: $zip"
    Write-Host "SHA-256 $zipHash"
}
finally {
    Pop-Location
}
