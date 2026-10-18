// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod assets;
mod highlight;
mod lsp;
mod preview;
mod skin;
mod theme;

use std::path::PathBuf;

use gpui_kit::*;

use crate::app::TypstEditorApp;

fn main() {
    // Font discovery is slow; warm the shared cache before the first compile.
    std::thread::spawn(|| {
        let _ = typst_core::get_or_init_fonts();
    });

    let initial_path = std::env::args_os().nth(1).map(PathBuf::from);

    gpui_kit::application()
        .with_assets(assets::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::install(cx);
            app::bind_keys(cx);
            cx.activate(true);

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1440.), px(900.)), cx)),
                titlebar: Some(TitlebarOptions {
                    title: Some("字笺 · Typst".into()),
                    ..gpui_kit::component::TitleBar::title_bar_options()
                }),
                window_min_size: Some(size(px(960.), px(640.))),
                ..gpui_kit::component::TitleBar::window_options()
            };

            gpui_kit::open_window(options, cx, move |window, cx| {
                cx.new(|cx| {
                    let mut app = TypstEditorApp::new(window, cx);
                    if let Some(path) = initial_path {
                        app.open_path(path, window, cx);
                    }
                    app
                })
            })
            .expect("failed to open window");
        });
}
