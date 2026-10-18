# Windows 安装包构建

在 Windows x64 上安装 Rust MSVC 工具链、Visual Studio C++ Build Tools、Windows SDK 和 Inno Setup 6.7+。

```powershell
pwsh gpui-app/scripts/package-windows.ps1
# 编译器不在默认位置时：
pwsh gpui-app/scripts/package-windows.ps1 -IsccPath 'C:\tools\Inno Setup 6\ISCC.exe'
```

脚本检查未提交改动，运行 typst-core 与 GPUI 测试，构建 Release 程序，收集示例、用户指南和许可证，启动验证后生成 `dist/` 下的便携 ZIP、`-Setup.exe` 及各自 SHA-256 校验文件。`-AllowDirty` 仅用于本地试包，`-SkipTests` 跳过已单独验证的测试，`-SkipInstaller` 只输出 ZIP。CI 使用同一脚本。

安装包采用固定 AppId，默认安装到 `%LOCALAPPDATA%\Programs\Zijian`，支持覆盖升级、开始菜单、可选桌面快捷方式、可选 `.typ` 打开方式和 Windows 应用列表卸载。不会强制抢占 `.typ` 的默认打开程序，卸载不会清除用户设置、恢复草稿或文稿。

应用及安装包图标复用 `assets/skin/notebook.svg`，ICO 内含 16–256 px 共九档尺寸。修改 SVG 后，安装 Node.js 和 sharp，再运行 `node gpui-app/assets/gen/app-icon.cjs` 重新生成并提交图标。`build.rs` 将图标和 Cargo 包版本写入 Windows PE 资源。

安装向导的中文翻译固定保存在 `scripts/languages/`，不依赖构建机的额外语言包。

当前构建不含代码签名证书。正式对外分发时可在构建流程中接入组织持有的 Authenticode 证书；不要把证书私钥提交到仓库。
