# Typst Editor Code Index

本文件用于快速定位“功能 -> 代码”。

## 项目概览

- 前端：`frontend/`
  - 技术栈：Vite + TypeScript + Tauri API
  - 编辑器：Monaco Editor
- 后端：`src-tauri/`
  - 技术栈：Rust + Tauri + Typst
  - 职责：Tauri 命令封装、对话框、文件系统操作
- 编译核心：`crates/typst-core/`
  - 纯 Rust，无 Tauri 依赖，被 `src-tauri` 和 `gpui-app` 共用
  - 职责：编译器状态与缓存、Typst World、SVG/位图渲染、跳转映射、大纲、补全/悬停
- GPUI 原型前端：`gpui-app/`
  - 技术栈：Rust + gpui-kit（GPUI）
  - 编辑器：gpui-kit 代码编辑器 + 自定义 Typst 高亮；预览为 `typst-render` 位图

## 启动与构建

- 前端开发：`cd frontend && pnpm dev`
- 前端构建：`cd frontend && pnpm build`
- Tauri 开发：`cd src-tauri && cargo tauri dev`
- Tauri 构建：`cd src-tauri && cargo tauri build`

- GPUI 运行：`cd gpui-app && cargo run -- <文件或目录>`
- GPUI 打包（测试 + Release + 冒烟 + ZIP）：`pwsh gpui-app/scripts/package-windows.ps1`
- typst-core 测试：`cd crates/typst-core && cargo test`

Tauri 前端命令配置在 [src-tauri/tauri.conf.json](/mnt/f/typst-editor/src-tauri/tauri.conf.json)。

## 前端入口索引

- 应用启动
  - [frontend/src/main.ts](/mnt/f/typst-editor/frontend/src/main.ts)
  - 入口函数：`initApp`
- 应用壳、工具栏、状态栏、窗口控制、主题/字体设置
  - [frontend/src/App.ts](/mnt/f/typst-editor/frontend/src/App.ts)
  - 关键函数：`createApp`、`initApp`、`applyTheme`、`applyFont`
- 编辑器桥接层
  - [frontend/src/editor/editorApi.ts](/mnt/f/typst-editor/frontend/src/editor/editorApi.ts)
  - 作用：统一暴露编辑器 API，屏蔽底层实现
- 编辑器主实现
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - 覆盖：文档管理、保存、编译调度、光标/选区同步、预览联动、标签页、诊断面板
- 文件树
  - [frontend/src/file-tree/FileTree.ts](/mnt/f/typst-editor/frontend/src/file-tree/FileTree.ts)
- 预览面板 DOM 工具
  - [frontend/src/preview/PreviewPanel.ts](/mnt/f/typst-editor/frontend/src/preview/PreviewPanel.ts)
- 大纲面板
  - [frontend/src/outline/OutlinePanel.ts](/mnt/f/typst-editor/frontend/src/outline/OutlinePanel.ts)
- 空工作区壳层
  - [frontend/src/workspace/WorkspaceShell.ts](/mnt/f/typst-editor/frontend/src/workspace/WorkspaceShell.ts)
- 全局样式
  - [frontend/src/styles/global.css](/mnt/f/typst-editor/frontend/src/styles/global.css)

## 后端入口索引

- Tauri 应用入口与命令注册
  - [src-tauri/src/lib.rs](/mnt/f/typst-editor/src-tauri/src/lib.rs)
  - 关键点：`run`
- Tauri 命令实现
  - [src-tauri/src/commands.rs](/mnt/f/typst-editor/src-tauri/src/commands.rs)
- 编译器状态与缓存
  - [crates/typst-core/src/compiler.rs](/mnt/f/typst-editor/crates/typst-core/src/compiler.rs)
- Typst World、项目根发现、字体缓存
  - [crates/typst-core/src/world.rs](/mnt/f/typst-editor/crates/typst-core/src/world.rs)

## 编译核心（typst-core）索引

- 模块入口与导出
  - [crates/typst-core/src/lib.rs](/mnt/f/typst-editor/crates/typst-core/src/lib.rs)
- 编译器状态、缓存（入口/根/覆盖内容/依赖文件版本组成的 `DocKey`）、强制编译、页缓存、SVG/位图渲染、结构化诊断
  - [crates/typst-core/src/compiler.rs](/mnt/f/typst-editor/crates/typst-core/src/compiler.rs)
  - `CompilerState`、`compile_file()`（编译时不持状态锁）、`CompileOptions`、`dependencies_changed()`、`export_pdf()`（拒绝导出非当前入口文档）、`render_page_rgba()`、`DiagnosticInfo`
- Typst World、项目根发现、字体缓存、包下载（`@preview` 首次使用时从 Typst Universe 下载，之后离线可用）
  - [crates/typst-core/src/world.rs](/mnt/f/typst-editor/crates/typst-core/src/world.rs)
- 跳转映射、大纲、补全、悬停
  - [crates/typst-core/src/ide.rs](/mnt/f/typst-editor/crates/typst-core/src/ide.rs)
  - `jump_from_click()`、`jump_from_offset()`、`document_outline()`、`autocomplete()`、`tooltip()`
- 目录列表、系统字体
  - [crates/typst-core/src/fs.rs](/mnt/f/typst-editor/crates/typst-core/src/fs.rs)
- 端到端测试
  - [crates/typst-core/tests/roundtrip.rs](/mnt/f/typst-editor/crates/typst-core/tests/roundtrip.rs)

## GPUI 前端（gpui-app）索引

- 启动、日志初始化、设置加载、会话恢复、崩溃草稿提示
  - [gpui-app/src/main.rs](/mnt/f/typst-editor/gpui-app/src/main.rs)
- 会话与文稿安全逻辑（每文档编辑器、打开/切换项目、保存与冲突、外部修改轮询、恢复日志、编译调度、导出、项目查找替换、文件管理）
  - [gpui-app/src/app.rs](/mnt/f/typst-editor/gpui-app/src/app.rs)
  - `TypstEditorApp`、`make_editor()`、`open_document()`、`switch_project()`、`save_paths()`、`check_disk()`、`flush_journal()`、`run_compile()`、`export_pdf()`、`run_project_search()`、`rename_path()`、`delete_path()`
- 主界面视图（顶栏菜单、侧栏卡片/文件树/目录、标签、稿纸页眉与提示条、预览、状态栏、`Render` 与动作绑定）
  - [gpui-app/src/app/views.rs](/mnt/f/typst-editor/gpui-app/src/app/views.rs)
- 对话框与面板（未保存、保存冲突、草稿恢复、重命名/新建、移到回收站、关于、欢迎页、问题列表、项目查找）
  - [gpui-app/src/app/dialogs.rs](/mnt/f/typst-editor/gpui-app/src/app/dialogs.rs)
- 会话状态模型（缓冲区、标签顺序、磁盘版本、改名、代次），可单测
  - [gpui-app/src/workspace.rs](/mnt/f/typst-editor/gpui-app/src/workspace.rs)
- 原子保存、磁盘版本、UTF-8 读取
  - [gpui-app/src/files.rs](/mnt/f/typst-editor/gpui-app/src/files.rs)
- 数据目录与崩溃恢复日志
  - [gpui-app/src/recovery.rs](/mnt/f/typst-editor/gpui-app/src/recovery.rs)
- 设置与上次会话（JSON，向前/向后兼容）
  - [gpui-app/src/settings.rs](/mnt/f/typst-editor/gpui-app/src/settings.rs)
- 本地日志、崩溃报告、诊断信息；版本/提交号来自 [gpui-app/build.rs](/mnt/f/typst-editor/gpui-app/build.rs)
  - [gpui-app/src/logging.rs](/mnt/f/typst-editor/gpui-app/src/logging.rs)
- 预览：可见页位图化、适宽/缩放、点击跳源码、光标标记
  - [gpui-app/src/preview.rs](/mnt/f/typst-editor/gpui-app/src/preview.rs)
- Typst 语法高亮（增量 `Source::edit`，只遍历可见范围）
  - [gpui-app/src/highlight.rs](/mnt/f/typst-editor/gpui-app/src/highlight.rs)
- 补全与悬停（进程内调用 typst-ide）
  - [gpui-app/src/lsp.rs](/mnt/f/typst-editor/gpui-app/src/lsp.rs)
- 拟物主题与组件、内嵌素材
  - [gpui-app/src/theme.rs](/mnt/f/typst-editor/gpui-app/src/theme.rs)、[gpui-app/src/skin.rs](/mnt/f/typst-editor/gpui-app/src/skin.rs)、[gpui-app/src/assets.rs](/mnt/f/typst-editor/gpui-app/src/assets.rs)
  - 素材与生成脚本：`gpui-app/assets/{textures,skin,gen}`；设计说明：[gpui-app/design/README.md](/mnt/f/typst-editor/gpui-app/design/README.md)
- 发布：打包脚本 [gpui-app/scripts/package-windows.ps1](/mnt/f/typst-editor/gpui-app/scripts/package-windows.ps1)、CI [.github/workflows/gpui-windows.yml](/mnt/f/typst-editor/.github/workflows/gpui-windows.yml)、用户指南 [gpui-app/docs/用户指南.md](/mnt/f/typst-editor/gpui-app/docs/用户指南.md)、第三方声明 [gpui-app/THIRD-PARTY-NOTICES.md](/mnt/f/typst-editor/gpui-app/THIRD-PARTY-NOTICES.md)

## 功能到代码索引

### 1. 应用初始化

- 前端启动
  - [frontend/src/main.ts](/mnt/f/typst-editor/frontend/src/main.ts)
  - `initApp()`
- 页面结构与各面板容器创建
  - [frontend/src/App.ts](/mnt/f/typst-editor/frontend/src/App.ts)
  - `createApp()`
- 初始化预览、文件树、大纲、工作区壳层
  - [frontend/src/App.ts](/mnt/f/typst-editor/frontend/src/App.ts)
  - `initApp()`

### 2. 编辑器初始化与对外 API

- 编辑器容器注册、延迟加载、统一 API
  - [frontend/src/editor/editorApi.ts](/mnt/f/typst-editor/frontend/src/editor/editorApi.ts)
  - `registerEditorContainer()`、`ensureEditorReady()`
- Monaco 初始化
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `initEditor()`
- 编辑器主题与字体
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `setEditorTheme()`、`setEditorFont()`

### 3. 打开文件 / 切换标签页

- 工具栏打开文件
  - [frontend/src/App.ts](/mnt/f/typst-editor/frontend/src/App.ts)
  - `bindToolbarButtons()` 内触发 `pickFilePath()` / `openFileAtPath()`
- 编辑器内打开文件
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `openFile()`、`openFileAtPath()`
- 切换文档与恢复视图
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `switchToDocument()`
- 标签页渲染与右键菜单
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `renderTabs()`、`showTabContextMenu()`
- 后端读取文本文件
  - [src-tauri/src/commands.rs](/mnt/f/typst-editor/src-tauri/src/commands.rs)
  - `read_text_file`

### 4. 保存、脏状态、关闭文件

- 脏状态计算
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `updateActiveDocumentState()`
- 保存当前文件
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `saveCurrentFile()`
- 关闭标签页 / 批量关闭目录下文件
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `closeDocument()`、`closeDocumentsUnderPath()`
- 后端保存文件
  - [src-tauri/src/commands.rs](/mnt/f/typst-editor/src-tauri/src/commands.rs)
  - `save_file`

### 5. 编译与预览刷新

- 编译请求入队，只保留最新意图
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `enqueueCompile()`、`drainCompileQueue()`、`runCompileJob()`
- 应用编译结果、刷新页面、更新大纲和状态栏
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `applyCompileResult()`
- 打开文档后触发首编译
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `compileOpenDocument()`
- 后端编译主命令
  - [src-tauri/src/commands.rs](/mnt/f/typst-editor/src-tauri/src/commands.rs)
  - `open_file`、`set_preview_entry`、`compile_current`
- 编译器状态与复用逻辑
  - [crates/typst-core/src/compiler.rs](/mnt/f/typst-editor/crates/typst-core/src/compiler.rs)
  - `CompilerState::update_file_and_compile_with_id()`

### 6. 预览页面渲染

- 预览容器与页面节点
  - [frontend/src/preview/PreviewPanel.ts](/mnt/f/typst-editor/frontend/src/preview/PreviewPanel.ts)
  - `initPreview()`、`ensurePreviewPageNode()`、`replacePreviewPageSvg()`
- 前端批量刷新 SVG 页面
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `renderAllPages()`、`renderSinglePage()`
- 后端单页 SVG 渲染
  - [src-tauri/src/commands.rs](/mnt/f/typst-editor/src-tauri/src/commands.rs)
  - `render_page`
- 编译器缓存 SVG / 页哈希
  - [crates/typst-core/src/compiler.rs](/mnt/f/typst-editor/crates/typst-core/src/compiler.rs)
  - `render_page_svg()`

### 7. 编辑器与预览双向定位

- 光标 -> 预览
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `syncCursorToPreview()`
- 预览悬停 -> 源码高亮
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `handlePreviewHover()`、`applySourceHoverDecoration()`
- 预览点击 -> 源码定位
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `handlePreviewClick()`、`jumpToFileOffset()`
- 后端位置映射（Tauri 命令是薄封装，实现在 typst-core）
  - [src-tauri/src/commands.rs](/mnt/f/typst-editor/src-tauri/src/commands.rs)
  - `jump_from_cursor`、`jump_from_click`
  - [crates/typst-core/src/ide.rs](/mnt/f/typst-editor/crates/typst-core/src/ide.rs)
  - `jump_from_cursor()`、`jump_from_offset()`、`jump_from_click()`

### 8. 新增内容高亮、选择高亮、hover 高亮

- 未保存变更区间计算
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `computeUnsavedChangedTextRanges()`
- 未保存内容在预览中的高亮
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `renderUnsavedInsertHighlights()`
- 区间高亮绘制
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `highlightRangeInPreview()`、`highlightCursorInPreview()`、`highlightHoverInPreview()`
- 样式
  - [frontend/src/styles/global.css](/mnt/f/typst-editor/frontend/src/styles/global.css)
  - `.highlight-overlay*`、`.editor-source-hover`

### 9. 文件树与文件系统操作

- 文件树初始化和刷新
  - [frontend/src/file-tree/FileTree.ts](/mnt/f/typst-editor/frontend/src/file-tree/FileTree.ts)
  - `initFileTree()`、`loadTree()`、`loadDirectory()`
- 文件树点击、展开目录、右键菜单
  - [frontend/src/file-tree/FileTree.ts](/mnt/f/typst-editor/frontend/src/file-tree/FileTree.ts)
  - `appendTreeItem()`、`toggleDir()`、`showContextMenu()`
- 新建、重命名、删除
  - [frontend/src/file-tree/FileTree.ts](/mnt/f/typst-editor/frontend/src/file-tree/FileTree.ts)
  - `startNewItem()`、`startRename()`、`confirmDelete()`
- 文件树中的主文件标记
  - [frontend/src/file-tree/FileTree.ts](/mnt/f/typst-editor/frontend/src/file-tree/FileTree.ts)
  - `setTreePreviewEntryPath()`、`setAsPreviewEntry()`
- 后端文件系统命令
  - [src-tauri/src/commands.rs](/mnt/f/typst-editor/src-tauri/src/commands.rs)
  - `list_dir`、`create_file`、`create_directory`、`delete_path`、`rename_path`、`reveal_in_explorer`

### 10. 主文件（Preview Entry）与大纲

- 设置当前文件为主文件
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `setCurrentDocumentAsPreviewEntry()`
- 取消主文件固定
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `clearPreviewEntryPin()`
- 大纲面板初始化与渲染
  - [frontend/src/outline/OutlinePanel.ts](/mnt/f/typst-editor/frontend/src/outline/OutlinePanel.ts)
  - `initOutlinePanel()`、`setOutlineItems()`、`updateOutlineActiveItem()`
- 后端大纲数据
  - [src-tauri/src/commands.rs](/mnt/f/typst-editor/src-tauri/src/commands.rs)
  - `get_document_outline`
- 编译器构建大纲
  - [crates/typst-core/src/ide.rs](/mnt/f/typst-editor/crates/typst-core/src/ide.rs)
  - `populate_outline()`、`build_document_outline()`

### 11. 工作区空态与布局显示

- 空工作区初始化
  - [frontend/src/workspace/WorkspaceShell.ts](/mnt/f/typst-editor/frontend/src/workspace/WorkspaceShell.ts)
  - `initWorkspaceShell()`
- 切换空态 / 正常布局
  - [frontend/src/workspace/WorkspaceShell.ts](/mnt/f/typst-editor/frontend/src/workspace/WorkspaceShell.ts)
  - `showEmptyWorkspace()`、`showWorkspaceLayout()`

### 12. 诊断与状态栏

- 前端状态栏更新
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `updateStatus()`、`updateStatusFile()`、`updateCursorStatus()`、`updatePageStatus()`、`updateZoomStatus()`
- 前端诊断面板
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - `showDiagnostics()`、`clearDiagnostics()`
- 后端错误格式化
  - [crates/typst-core/src/compiler.rs](/mnt/f/typst-editor/crates/typst-core/src/compiler.rs)
  - `format_diagnostic()`

### 13. 项目根、字体与 Typst World

- 项目根发现
  - [crates/typst-core/src/world.rs](/mnt/f/typst-editor/crates/typst-core/src/world.rs)
  - `discover_project_root()`
- 字体缓存与系统字体枚举
  - [crates/typst-core/src/world.rs](/mnt/f/typst-editor/crates/typst-core/src/world.rs)
  - `get_or_init_fonts()`
  - [src-tauri/src/commands.rs](/mnt/f/typst-editor/src-tauri/src/commands.rs)
  - `list_system_fonts`
- Typst 文件解析上下文
  - [crates/typst-core/src/world.rs](/mnt/f/typst-editor/crates/typst-core/src/world.rs)
  - `TypstWorld`

## 调试建议

- 编辑器行为问题，先看
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
- 前端事件但不确定编辑器是否已初始化，先看
  - [frontend/src/editor/editorApi.ts](/mnt/f/typst-editor/frontend/src/editor/editorApi.ts)
- 预览映射错位，先看
  - [frontend/src/editor/MonacoEditor.ts](/mnt/f/typst-editor/frontend/src/editor/MonacoEditor.ts)
  - [src-tauri/src/commands.rs](/mnt/f/typst-editor/src-tauri/src/commands.rs)
- 编译结果异常、缓存不生效，先看
  - [crates/typst-core/src/compiler.rs](/mnt/f/typst-editor/crates/typst-core/src/compiler.rs)
- 资源路径、项目根、导入文件找不到，先看
  - [crates/typst-core/src/world.rs](/mnt/f/typst-editor/crates/typst-core/src/world.rs)

