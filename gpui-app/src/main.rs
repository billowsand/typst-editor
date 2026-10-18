// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod highlight;
mod lsp;
mod preview;

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
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            // `.SystemUIFont` falls back to a serif face for Latin text on
            // Windows; YaHei UI covers both Latin and CJK.
            #[cfg(target_os = "windows")]
            {
                gpui_kit::component::Theme::global_mut(cx).font_family = "Microsoft YaHei UI".into();
            }
            app::bind_keys(cx);
            cx.activate(true);

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1440.), px(900.)), cx)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Typst Editor (GPUI)".into()),
                    ..Default::default()
                }),
                ..Default::default()
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
