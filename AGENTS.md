# Typst Editor Code Index

本文件用于快速定位“功能 -> 代码”。项目自 2026-10-19 起为**纯 Rust**：旧的 Tauri 版（`frontend/` + `src-tauri/`）已删除，不再维护。

## 项目概览

- 编译核心：`crates/typst-core/`
  - 纯 Rust，无 UI 框架依赖；唯一使用方是 `gpui-app`
  - 职责：编译器状态与缓存、Typst World、SVG/位图渲染、跳转映射、大纲、补全/悬停、目录与字体枚举
- 桌面应用：`gpui-app/`
  - 技术栈：Rust + gpui-kit（GPUI）
  - 编辑器：gpui-kit 代码编辑器 + 自定义 Typst 高亮；预览为 `typst-render` 位图

## 启动与构建

- 运行：`cd gpui-app && cargo run -- <文件或目录>`
- 测试：`cd gpui-app && cargo test`；`cd crates/typst-core && cargo test`
- 打包（测试 + Release + 冒烟 + ZIP + 安装程序）：`pwsh gpui-app/scripts/package-windows.ps1`
- 端到端编译测试：`cargo test --manifest-path crates/typst-core/Cargo.toml -- --ignored`

## 模块索引

### 入口与基础设施（`gpui-app/src/`）

- 启动、日志初始化、窗口、设置加载、会话恢复、崩溃草稿提示、安装器运行标记
  - `main.rs`：`main()`、`mark_running_for_installer()`
- 根状态与文稿安全逻辑（每文档编辑器、打开/切换项目、保存与冲突、外部修改轮询、恢复日志、编译调度、导出、项目查找替换、文件管理、动作与快捷键）
  - `app.rs`：`TypstEditorApp`、`bind_keys()`、`open_path()`、`switch_project()`、`perform()`、`check_disk()`、`flush_journal()`、`run_compile()`、`compile_abandoned()`、`apply_compile_result()`、`export_pdf()`、`run_project_search()`、`rename_path()`、`TreeNode`、`OpenDocument`/`TreeNewFile`/`TreeRename`/`TreeDelete` 等 action
  - 常量：`COMPILE_DEBOUNCE`、`CURSOR_DEBOUNCE`、`JOURNAL_DEBOUNCE`、`DISK_POLL`、`IGNORED_DIRS`、`TEXT_EXTS`、`SEARCH_LIMIT`
- 主界面视图（菜单、标题栏、侧栏卡片/文件树/大纲、标签、稿纸页眉与提示条、预览、状态栏、`Render`）
  - `app/views.rs`：`render_menus()`、`render_titlebar()`、`render_search()`、`render_nav_switch()`、`render_sidebar()`、`render_card()`、`render_drafts()`、`render_files()`、`render_outline_card()`、`render_tabs()`、`render_sheet_header()`、`render_banners()`、`render_ruled_paper()`、`render_notepad()`、`render_preview()`、`render_status_bar()`
- 对话框与面板（未保存、保存冲突、草稿恢复、重命名/新建、移到回收站、问题列表、项目查找）
  - `app/dialogs.rs`：`render_unsaved_dialog()`、`render_conflict_dialog()`、`render_recover_dialog()`、`render_name_dialog()`、`render_delete_dialog()`、`render_problems()`、`render_search_panel()`
- 设置界面（字体、字号、模式、侧栏、星标、最近项目）
  - `app/preferences.rs`：`make_preferences()`、`render_settings()`
- 会话状态模型（缓冲区、标签顺序、磁盘版本、改名、代次），可单测
  - `workspace.rs`：`Workspace`、`Buffer`、`normalize()`、`open()`、`set_current()`、`mark_saved()`、`changed_on_disk()`、`rename()`、`paths_under()`、`close()`
- 原子保存、磁盘版本、UTF-8 读取
  - `files.rs`：`read_text()`、`atomic_write()`、`DiskVersion`
- 数据目录与崩溃恢复日志
  - `recovery.rs`：`data_dir()`、`Journal`、`Draft`、`DraftWrite`
- 设置与上次会话（JSON，向前/向后兼容）
  - `settings.rs`：`Settings`、`Session`、`path()`、`load_from()`、`save_to()`
- 本地日志、崩溃报告、诊断信息；版本/提交号来自 `build.rs`
  - `logging.rs`：`init()`、`info()`/`warn()`/`error()`、`diagnostics()`、`log_dir()`
- 预览：可见页位图化、适宽/缩放、点击跳源码、光标标记、页面密度上限
  - `preview.rs`：`PreviewPanel`、`page_density()`、`set_document()`、`set_markers()`、`on_page_click()`、`apply_fit_width()`、`visible_range()`、`schedule_rasterization()`
- Typst 语法高亮（增量 `Source::edit`，只遍历可见范围）
  - `highlight.rs`：`typst_highlighter_factory()`、`TypstHighlighter`、`compute_spans()`
- 补全与悬停（进程内调用 typst-ide）
  - `lsp.rs`：`TypstLsp`
- 拟物主题与组件、内嵌素材
  - `theme.rs`：`install()`、`fonts()`、`apply_fonts()`、`syntax_color()`
  - `skin.rs`：`paper_shadow()`、`lifted_shadow()`、`embossed()`、`letterpress()`
  - `assets.rs`：`AppAssets`；素材 `gpui-app/assets/{textures,skin,gen}`，设计说明 `gpui-app/design/README.md`

### 编译核心（`crates/typst-core/src/`）

- 模块入口与导出：`lib.rs`
- 编译器状态与缓存（`DocKey` = 入口 + 项目根 + 覆盖指纹 + 依赖版本）、强制编译、页缓存、SVG/位图渲染、结构化诊断、PDF 导出
  - `compiler.rs`：`CompilerState`、`CompileOptions`、`compile_file()`（编译时不持状态锁）、`compile_lock()`、`session_id()`、`discard_overlay()`、`dependencies_changed()`、`export_pdf()`（拒绝导出非当前入口文档）、`render_page_svg()`、`render_page_rgba()`、`page_sizes()`、`CompileResult`、`DiagnosticInfo`、`OutlineItem`、`OutlinePosition`
- Typst World、项目根发现、字体缓存、`@preview` 包下载
  - `world.rs`：`TypstWorld`、`get_or_init_fonts()`、`discover_project_root()`、`discover_access_root()`、`update_source_by_path()`、`depended_paths()`
- 跳转映射、大纲、补全、悬停
  - `ide.rs`：`jump_from_click()`、`jump_from_offset()`、`jump_from_offset_live()`、`jump_from_cursor()`、`document_outline()`、`populate_outline()`、`autocomplete()`、`tooltip()`、`PositionResult`、`JumpResult`
- 目录列表、系统字体：`fs.rs`（`list_dir()`、`list_system_fonts()`）
- 端到端测试：`crates/typst-core/tests/roundtrip.rs`

### 发布

- 打包脚本 `gpui-app/scripts/package-windows.ps1`、`gpui-app/scripts/installer.iss`
- CI `.github/workflows/gpui-windows.yml`（标签 `gpui-v*` 发布 GitHub Release）
- 用户指南 `gpui-app/docs/用户指南.md`、打包说明 `gpui-app/docs/Windows打包.md`
- 第三方声明 `gpui-app/THIRD-PARTY-NOTICES.md`

## 功能到代码索引

### 1. 应用启动

- 入口、字体预热、窗口创建、命令行参数打开文件
  - `gpui-app/src/main.rs`
- 全局动作注册（`actions!` 宏）与快捷键绑定
  - `gpui-app/src/app.rs`：`bind_keys()`
- 首屏状态装配（编译器、预览、编辑器、树、面板实体）
  - `gpui-app/src/app.rs`：`TypstEditorApp::new()`

### 2. 打开文件 / 新建 / 切换项目

- 打开路径、恢复上次会话、崩溃草稿提示
  - `gpui-app/src/app.rs`：`open_path()`、`restore_session()`、`offer_recovery()`、`restore_drafts()`
- 项目扫描与卡片/目录树构建
  - `gpui-app/src/app.rs`：`refresh_tree()`、`TreeNode::to_item()`
- 新建文稿、新建项目、模板内容
  - `gpui-app/src/app.rs`：`new_document()`、`new_project()`、`template_for()`、`create_and_open()`、`prompt_open()`
- 切换项目并作废旧状态
  - `gpui-app/src/app.rs`：`switch_project()`、`gpui-app/src/workspace.rs`：`reset()`、`generation()`

### 3. 打开示例

- 欢迎页示例
  - `gpui-app/src/app.rs`：`open_demo()`

### 4. 编辑器创建与标签页

- 每文档一个编辑器（独立撤销、光标、选区、滚动）
  - `gpui-app/src/app.rs`：`make_editor()`、`editor_buffers()`、`text_of()`
- 标签渲染与切换
  - `gpui-app/src/app/views.rs`：`render_tabs()`
- 会话模型
  - `gpui-app/src/workspace.rs`：`open()`、`set_current()`、`close()`、`tab_index()`

### 5. 保存、脏状态、另存为

- 保存当前 / 全部 / 另存为
  - `gpui-app/src/app.rs`：`perform()`、`save_as()`、`save_as_to()`、`save_with_conflict_check()`
- 原子写与磁盘版本比对
  - `gpui-app/src/files.rs`：`atomic_write()`、`DiskVersion::of()`
- 脏状态
  - `gpui-app/src/workspace.rs`：`Buffer::is_dirty()`、`dirty_paths()`、`mark_saved()`

### 6. 关闭与未保存确认

- 关闭窗口前的决策链
  - `gpui-app/src/app.rs`：`on_close_requested()`、`SaveOutcome::continuation()`
- 未保存 / 冲突对话框
  - `gpui-app/src/app/dialogs.rs`：`render_unsaved_dialog()`、`render_conflict_dialog()`
- 放弃保存时把编译器覆盖层换回磁盘内容
  - `gpui-app/src/app.rs`：`revert_overlay()`
  - `crates/typst-core/src/compiler.rs`：`discard_overlay()`

### 7. 编译与缓存

- 编译调度（防抖 120ms）、带代次的异步编译
  - `gpui-app/src/app.rs`：`compile_active()`、`run_compile()`、`compile_abandoned()`
- 应用编译结果、刷新预览、诊断
  - `gpui-app/src/app.rs`：`apply_compile_result()`、`show_diagnostics()`、`resolve_problems()`
- 强制编译 / 依赖变化检测
  - `crates/typst-core/src/compiler.rs`：`CompileOptions`、`dependencies_changed()`、`update_file_and_compile_with_id()`
- 覆盖层（未保存内容）注入
  - `crates/typst-core/src/compiler.rs`：`set_editor_overlays()`、`gpui-app/src/app.rs`：`on_text_changed()`

### 8. 预览渲染与缩放

- 可见页位图化、页面密度上限、渲染调度
  - `gpui-app/src/preview.rs`：`schedule_rasterization()`、`render_page()`、`page_density()`、`visible_range()`
- 适宽与缩放
  - `gpui-app/src/preview.rs`：`apply_fit_width()`、`set_zoom()`、`fit_width()`
- 单页 RGBA 渲染 / 页尺寸
  - `crates/typst-core/src/compiler.rs`：`render_page_rgba()`、`page_sizes()`
- 预览面板装配
  - `gpui-app/src/app/views.rs`：`render_preview()`

### 9. 编辑器与预览双向定位

- 光标 → 预览标记（防抖 60ms）
  - `gpui-app/src/app.rs`：`schedule_cursor_sync()`、`gpui-app/src/preview.rs`：`set_markers()`
- 预览点击 → 源码定位
  - `gpui-app/src/preview.rs`：`on_page_click()`
- 位置映射实现
  - `crates/typst-core/src/ide.rs`：`jump_from_click()`、`jump_from_offset()`、`jump_from_offset_live()`、`jump_from_cursor()`
- 跳转到范围（大纲、问题面板）
  - `gpui-app/src/app.rs`：`select_range()`

### 10. 大纲与问题面板

- 大纲数据与点击定位
  - `gpui-app/src/app/views.rs`：`render_outline_card()`、`gpui-app/src/app.rs`：`select_range()`
- 大纲构建
  - `crates/typst-core/src/ide.rs`：`populate_outline()`、`document_outline()`
- 问题面板（文件:行:列，点击定位）
  - `gpui-app/src/app/dialogs.rs`：`render_problems()`、`gpui-app/src/app.rs`：`resolve_problems()`
- 诊断格式化
  - `crates/typst-core/src/compiler.rs`：`DiagnosticInfo`

### 11. 文件树与文件系统操作

- 目录树渲染
  - `gpui-app/src/app/views.rs`：`render_files()`
- 新建 / 重命名 / 删除 / 在资源管理器中显示
  - `gpui-app/src/app.rs`：`begin_name_dialog()`、`rename_path()`、`gpui-app/src/app/dialogs.rs`：`render_name_dialog()`、`render_delete_dialog()`
- 目录枚举
  - `crates/typst-core/src/fs.rs`：`list_dir()`
- 文本 / 非文本判定
  - `gpui-app/src/app.rs`：`TEXT_EXTS`

### 12. 主文件（Preview Entry）

- 固定 / 取消固定预览入口
  - `crates/typst-core/src/compiler.rs`：`pin_preview_entry()`、`pinned_entry()`、`entry_for()`
- 卡片图钉与页眉“入口”标记
  - `gpui-app/src/app/views.rs`：`render_card()`、`render_sheet_header()`

### 13. 查找 / 替换 / 全文搜索

- 编辑器内查找替换（gpui-kit 自带）
  - `gpui-app/src/app.rs`：`open_search()`
- 项目全文搜索与替换面板
  - `gpui-app/src/app/dialogs.rs`：`render_search_panel()`、`gpui-app/src/app.rs`：`run_project_search()`

### 14. 补全与悬停

- 编辑器语言服务接入
  - `gpui-app/src/lsp.rs`：`TypstLsp`、`completions()`、`hover()`
- 高亮样式
  - `gpui-app/src/highlight.rs`：`TypstHighlighter::styles()`、`compute_spans()`、`gpui-app/src/theme.rs`：`syntax_color()`
- 折叠区间（当前恒空，已知限制）
  - `gpui-app/src/highlight.rs`：`fold_ranges()`

### 15. 设置与会话持久化

- 设置项与最近项目
  - `gpui-app/src/settings.rs`：`Settings`、`Session`
- 设置界面
  - `gpui-app/src/app/preferences.rs`：`make_preferences()`、`render_settings()`
- 字号 / 模式 / 侧栏 / 大纲开关
  - `gpui-app/src/app.rs`：`set_compact()`、`gpui-app/src/app/views.rs`：`render_status_bar()`

### 16. 崩溃恢复与日志

- 恢复日志写入（防抖 1.2s）
  - `gpui-app/src/app.rs`：`schedule_journal()`、`flush_journal()`、`forget_draft()`
- 日志实现
  - `gpui-app/src/recovery.rs`：`Journal`、`DraftWrite`、`gpui-app/src/logging.rs`：`init()`、`diagnostics()`
- 复制诊断信息
  - `gpui-app/src/app.rs`：`copy_diagnostics()`

### 17. 外部修改检测

- 打开文件轮询（1.5s）与处理
  - `gpui-app/src/app.rs`：`check_disk()`
- 状态模型
  - `gpui-app/src/workspace.rs`：`changed_on_disk()`、`reload()`、`keep_mine()`

### 18. 导出 PDF

- 导出入口与入口确认
  - `gpui-app/src/app.rs`：`export_pdf()`
- PDF 生成
  - `crates/typst-core/src/compiler.rs`：`export_pdf()`

### 19. 主题与素材

- 主题安装、字体选择与回退
  - `gpui-app/src/theme.rs`：`install()`、`apply_fonts()`、`fonts()`
- 拟物组件（浮雕、压印、阴影、纸面）
  - `gpui-app/src/skin.rs`
- 内嵌素材
  - `gpui-app/src/assets.rs`：`AppAssets`；生成脚本 `gpui-app/assets/gen/{textures.py,skin_svgs.py}`

## 调试建议

- 界面布局、卡片、文件树、稿纸、状态栏问题
  - `gpui-app/src/app/views.rs`、`gpui-app/src/skin.rs`、`gpui-app/src/theme.rs`
- 会话 / 标签 / 保存 / 冲突逻辑
  - `gpui-app/src/app.rs`、`gpui-app/src/workspace.rs`、`gpui-app/src/files.rs`
- 编译结果异常、缓存不生效、旧结果回写
  - `crates/typst-core/src/compiler.rs`、`gpui-app/src/app.rs`（`run_compile` / `compile_abandoned`）
- 预览空白、错位、点击不跳转
  - `gpui-app/src/preview.rs`、`crates/typst-core/src/compiler.rs`（`render_page_rgba`）
- 资源路径、项目根、`@preview` 包找不到
  - `crates/typst-core/src/world.rs`
- 补全、悬停、大纲不正确
  - `crates/typst-core/src/ide.rs`、`gpui-app/src/lsp.rs`
- 发布 / 安装 / 打包
  - `gpui-app/scripts/package-windows.ps1`、`.github/workflows/gpui-windows.yml`、`gpui-app/docs/Windows打包.md`
