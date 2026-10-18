# Windows 安装包构建

在 Windows x64 上安装 Rust MSVC 工具链、Visual Studio C++ Build Tools、Windows SDK 和 Inno Setup 6.7+。

```powershell
pwsh gpui-app/scripts/package-windows.ps1
# 编译器不在默认位置时：
pwsh gpui-app/scripts/package-windows.ps1 -IsccPath 'C:\tools\Inno Setup 6\ISCC.exe'
```

脚本检查未提交改动，运行 typst-core 与 GPUI 测试，构建 Release 程序，收集示例、用户指南和许可证，启动验证后生成 `dist/` 下的便携 ZIP、`-Setup.exe` 及各自 SHA-256 校验文件。`-AllowDirty` 仅用于本地试包，`-SkipTests` 跳过已单独验证的测试，`-SkipInstaller` 只输出 ZIP。CI 使用同一脚本。

## 发布到 GitHub Release

CI（`.github/workflows/gpui-windows.yml`）在 `main`、`feat/**` 分支推送和 PR 上打包为工作流产物；推送 `gpui-v<版本>` 标签会在同一次运行中把 ZIP、`-Setup.exe` 及其 SHA-256 校验文件发布为 GitHub Release。

```powershell
# 1. 把 gpui-app/Cargo.toml 的 version 改成目标版本（同步 gpui-app/Cargo.lock）
# 2. 提交并推送
git add gpui-app/Cargo.toml gpui-app/Cargo.lock
git commit -m "chore(release): 字笺 · Typst 1.0.0"
git push origin main
# 3. 打带说明的标签并推送，Release 标题与说明取自标签消息
git tag -a gpui-v1.0.0 -m "字笺 · Typst 1.0.0：首个正式版"
git push origin gpui-v1.0.0
# 4. 等待 "GPUI Windows" 运行结束，在 Releases 页面核对产物
```

标签去掉 `gpui-v` 前缀后必须与 `gpui-app/Cargo.toml` 的 `version` 完全一致，否则发布任务会在上传前失败，避免安装包与程序内嵌版本号不符。发布前请确认构建产物来自干净的 `main`。安装包当前不含代码签名，Windows SmartScreen 可能提示未知发布者。

安装包采用固定 AppId，默认安装到 `%LOCALAPPDATA%\Programs\Zijian`，支持覆盖升级、开始菜单、可选桌面快捷方式、可选 `.typ` 打开方式和 Windows 应用列表卸载。不会强制抢占 `.typ` 的默认打开程序，卸载不会清除用户设置、恢复草稿或文稿。

应用及安装包图标复用 `assets/skin/notebook.svg`，ICO 内含 16–256 px 共九档尺寸。修改 SVG 后，安装 Node.js 和 sharp，再运行 `node gpui-app/assets/gen/app-icon.cjs` 重新生成并提交图标。`build.rs` 将图标和 Cargo 包版本写入 Windows PE 资源。

安装向导的中文翻译固定保存在 `scripts/languages/`，不依赖构建机的额外语言包。

程序启动时创建 `Local\ZijianTypstEditorRunning` 命名互斥体，安装器与卸载器据此要求先关闭正在运行的编辑器，避免删除仍被使用的程序和示例。

当前构建不含代码签名证书。正式对外分发时可在构建流程中接入组织持有的 Authenticode 证书；不要把证书私钥提交到仓库。
