//! macOS 原生菜单栏与 Dock 菜单。
//!
//! 顶栏的窗口内菜单只服务于 Windows / Linux；macOS 把它们整体迁入系统
//! 菜单栏。菜单项的快捷键由 keymap 绑定自动生成（`secondary-*` 在 mac
//! 上即 Cmd）。勾选态（目录卡 / 紧凑模式）与"最近项目"会变化，相关状态
//! 变更后调用 [`rebuild`] 重建菜单。

use std::path::Path;

use gpui_kit::*;
use gpui_kit::component::input as edit;

use crate::app::{
    About, CloseTab, CopyDiagnostics, ExportPdf, Find, FindReplace, FitWidth, FontBigger,
    FontReset, FontSmaller, HideApp, HideOthers, MinimizeWindow, NewDocument, NewProject,
    OpenDemo, OpenFile, OpenFolder, OpenGuide, OpenLogFolder, OpenRecent, OpenSettings,
    ProjectSearch, Quit, Recompile, Rescan, RevealProject, Save, SaveAll, SaveAs, ShowProblems,
    ToggleCompact, ToggleFullScreen, ToggleOutline, ToggleSidebar, TypstEditorApp, UnhideOthers,
    ZoomIn, ZoomOut, ZoomReset, ZoomWindow,
};

/// Checked items and dynamic entries that require a menu rebuild.
pub(crate) struct MenuState {
    recent: Vec<String>,
    outline_open: bool,
    compact: bool,
}

/// Install the menu bar and the Dock menu before the first window opens.
pub(crate) fn install(cx: &mut App) {
    cx.set_menus(build(None));
    cx.set_dock_menu(vec![
        MenuItem::action("新建文稿…", NewDocument),
        MenuItem::action("打开文件…", OpenFile),
        MenuItem::action("打开文件夹…", OpenFolder),
    ]);
}

/// Rebuild the menu bar from the app's current state.
pub(crate) fn rebuild(app: &TypstEditorApp, cx: &App) {
    cx.set_menus(build(Some(MenuState {
        recent: app.recent_projects(),
        outline_open: app.outline_open,
        compact: app.compact,
    })));
}

fn build(state: Option<MenuState>) -> Vec<Menu> {
    let state = state.unwrap_or(MenuState {
        recent: vec![],
        outline_open: false,
        compact: false,
    });
    vec![
        app_menu(),
        file_menu(&state),
        edit_menu(),
        view_menu(&state),
        doc_menu(),
        // gpui recognizes the exact name "Window" and hands the menu to
        // AppKit, which appends the list of open windows.
        window_menu(),
        help_menu(),
    ]
}

fn app_menu() -> Menu {
    Menu::new("字笺").items(vec![
        MenuItem::action("关于字笺", About),
        MenuItem::separator(),
        MenuItem::action("设置…", OpenSettings),
        MenuItem::separator(),
        MenuItem::os_submenu("服务", SystemMenuType::Services),
        MenuItem::separator(),
        MenuItem::action("隐藏字笺", HideApp),
        MenuItem::action("隐藏其他", HideOthers),
        MenuItem::action("显示全部", UnhideOthers),
        MenuItem::separator(),
        MenuItem::action("退出字笺", Quit),
    ])
}

fn file_menu(state: &MenuState) -> Menu {
    let mut items = vec![
        MenuItem::action("新建文稿…", NewDocument),
        MenuItem::action("新建项目…", NewProject),
        MenuItem::separator(),
        MenuItem::action("打开文件…", OpenFile),
        MenuItem::action("打开文件夹…", OpenFolder),
    ];
    if !state.recent.is_empty() {
        items.push(MenuItem::separator());
        items.push(MenuItem::submenu(Menu::new("最近项目").items(
            state.recent.iter().map(|root| {
                let label = Path::new(root)
                    .file_name()
                    .map(|n| format!("{}    {root}", n.to_string_lossy()))
                    .unwrap_or_else(|| root.clone());
                MenuItem::action(label, OpenRecent { root: root.clone() })
            }),
        )));
    }
    items.extend([
        MenuItem::separator(),
        MenuItem::action("保存", Save),
        MenuItem::action("另存为…", SaveAs),
        MenuItem::action("全部保存", SaveAll),
        MenuItem::action("关闭文稿", CloseTab),
        MenuItem::separator(),
        MenuItem::action("导出 PDF…", ExportPdf),
    ]);
    Menu::new("文件").items(items)
}

fn edit_menu() -> Menu {
    Menu::new("编辑").items(vec![
        MenuItem::action("撤销", edit::Undo),
        MenuItem::action("重做", edit::Redo),
        MenuItem::separator(),
        MenuItem::os_action("剪切", edit::Cut, OsAction::Cut),
        MenuItem::os_action("拷贝", edit::Copy, OsAction::Copy),
        MenuItem::os_action("粘贴", edit::Paste, OsAction::Paste),
        MenuItem::os_action("全选", edit::SelectAll, OsAction::SelectAll),
        MenuItem::separator(),
        MenuItem::action("查找…", Find),
        MenuItem::action("替换…", FindReplace),
        MenuItem::action("在项目中查找与替换…", ProjectSearch),
    ])
}

fn view_menu(state: &MenuState) -> Menu {
    Menu::new("视图").items(vec![
        MenuItem::action("显示 / 隐藏文稿栏", ToggleSidebar),
        MenuItem::action("目录卡", ToggleOutline).checked(state.outline_open),
        MenuItem::action("问题列表", ShowProblems),
        MenuItem::separator(),
        MenuItem::action("紧凑代码模式（行号）", ToggleCompact).checked(state.compact),
        MenuItem::action("增大字号", FontBigger),
        MenuItem::action("减小字号", FontSmaller),
        MenuItem::action("默认字号", FontReset),
        MenuItem::separator(),
        MenuItem::action("预览适合宽度", FitWidth),
        MenuItem::action("预览实际大小", ZoomReset),
        MenuItem::action("放大预览", ZoomIn),
        MenuItem::action("缩小预览", ZoomOut),
    ])
}

fn doc_menu() -> Menu {
    Menu::new("文档").items(vec![
        MenuItem::action("强制重新编译", Recompile),
        MenuItem::action("重新扫描项目", Rescan),
        MenuItem::action("在访达中显示项目", RevealProject),
        MenuItem::separator(),
        MenuItem::action("打开排版示例", OpenDemo),
    ])
}

fn window_menu() -> Menu {
    Menu::new("Window").items(vec![
        MenuItem::action("最小化", MinimizeWindow),
        MenuItem::action("缩放", ZoomWindow),
        MenuItem::action("进入全屏", ToggleFullScreen),
    ])
}

fn help_menu() -> Menu {
    Menu::new("帮助").items(vec![
        MenuItem::action("用户指南", OpenGuide),
        MenuItem::action("复制诊断信息", CopyDiagnostics),
        MenuItem::action("打开日志文件夹", OpenLogFolder),
        MenuItem::separator(),
        MenuItem::action("关于字笺", About),
    ])
}
