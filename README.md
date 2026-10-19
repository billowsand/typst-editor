# 字笺 · Typst

本机运行的 [Typst](https://typst.app/) 编辑器：文稿栏（卡片 / 完整目录树）、稿纸编辑器、实时预览，全部由 Rust 进程内完成，无 WebView、无 Node 依赖。文稿只保存在你的电脑上。

```
crates/typst-core/   编译核心：编译器状态与缓存、Typst World、渲染、跳转映射、大纲、补全/悬停
gpui-app/            桌面应用：Rust + gpui-kit（GPUI）
```

## 特性

- **增量编译**：输入后约 0.1 秒重编译；缓存键包含入口、项目根、未保存覆盖内容和依赖文件版本，切换文档不会串文档。
- **实时预览**：页面按可见范围位图化（像素密度有上限），适宽 / 实际大小切换；点击预览跳回源码，光标位置有标记。
- **文稿安全**：原子保存（临时文件 → 写入确认 → 替换），磁盘被外部修改时先询问再决定，绝不静默覆盖；保存失败保留修改并常驻提示。
- **崩溃恢复**：停止输入约 1 秒写恢复草稿，异常退出后下次启动询问恢复。
- **IDE**：补全、悬停提示、文档大纲、问题列表（文件:行:列，点击定位）、当前文稿内查找替换、项目全文查找替换。
- **项目查找与替换**：跨文件搜索，`全部替换` 直接作用于编辑器缓冲（可撤销）后再保存。
- **文件管理**：完整目录树，右键新建文稿 / 文件夹、重命名（打开的文稿随之改名）、移到回收站、在资源管理器中显示。
- **外部修改感知**：已打开文稿与被引用的依赖文件被外部程序修改时自动重新编译，脏缓冲则交给用户决定。
- **模板包**：`#import "@preview/…"` 首次使用从 Typst Universe 下载到本机缓存，之后离线可用。
- **会话与设置持久化**：上次项目、标签页、字号、模式、侧栏、星标、最近项目写入 `settings.json`。
- **导出版式**：导出 PDF 前用当前文本重新确认编译，拒绝导出非当前入口的文档。

## 环境要求

- Windows x64
- Rust stable（MSVC 工具链）
- Visual Studio C++ Build Tools 与 Windows SDK（GPUI 依赖）
- 可选：Inno Setup 6.7+（只影响生成安装程序）

## 从源码运行

```powershell
# 打开示例项目
cd gpui-app
cargo run -- examples

# 打开指定项目或文稿
cargo run -- C:\path\to\project
cargo run -- C:\path\to\main.typ
```

命令行参数可选；不带参数启动会恢复上次的项目和标签页。

## 测试

```powershell
cd crates/typst-core && cargo test        # 编译、渲染、跳转、缓存与文稿安全
cd gpui-app && cargo test                 # 会话、文件、恢复日志、高亮、设置、素材

# 端到端编译测试（需要联网下载 @preview 包）
cargo test --manifest-path crates/typst-core/Cargo.toml -- --ignored
```

## 打包

```powershell
pwsh gpui-app/scripts/package-windows.ps1
```

脚本检查未提交改动，运行两个 crate 的测试，构建 Release 程序，收集示例、用户指南与许可证，启动验证后生成 `dist/` 下的便携 ZIP、`-Setup.exe` 及各自 SHA-256。开关：`-AllowDirty`（本地试包）、`-SkipTests`、`SkipInstaller`（只出 ZIP）、`-IsccPath`（指定 ISCC.exe）、`-OutDir`。详见 [gpui-app/docs/Windows打包.md](gpui-app/docs/Windows打包.md)。

CI（`.github/workflows/gpui-windows.yml`）使用同一脚本；推送 `gpui-v<版本>` 标签会同时发布 GitHub Release，标签版本号必须与 `gpui-app/Cargo.toml` 的 `version` 一致。

## 文档

| 文档 | 内容 |
|---|---|
| [gpui-app/docs/用户指南.md](gpui-app/docs/用户指南.md) | 安装、界面、保存与恢复、快捷键、常见问题、数据位置 |
| [gpui-app/docs/Windows打包.md](gpui-app/docs/Windows打包.md) | 打包与发布流程 |
| [gpui-app/design/README.md](gpui-app/design/README.md) | 拟物主题设计说明、素材生成 |
| [gpui-app/THIRD-PARTY-NOTICES.md](gpui-app/THIRD-PARTY-NOTICES.md) | 第三方组件与许可证声明 |
| [AGENTS.md](AGENTS.md) | 功能 → 代码索引 |

## 数据位置

Windows 上设置、会话、恢复草稿与日志都在 `%LOCALAPPDATA%\Zijian`；用环境变量 `ZIJIAN_DATA_DIR` 可改到别处。日志只记录事件、路径和错误，不记录文稿内容。唯一的网络访问是首次下载 `@preview` 模板包。

## 许可

本仓库暂未附开源许可证文件。依赖组件的许可证汇总见 [gpui-app/THIRD-PARTY-NOTICES.md](gpui-app/THIRD-PARTY-NOTICES.md)，随发布包在 `licenses/` 目录中分发。
