// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod assets;
mod files;
mod highlight;
mod logging;
mod lsp;
#[cfg(target_os = "macos")]
mod macos_menu;
mod preview;
mod recovery;
mod settings;
mod skin;
mod theme;
mod workspace;

use std::path::PathBuf;

use gpui_kit::*;

use crate::app::TypstEditorApp;
use crate::settings::Settings;

fn main() {
    logging::init();
    #[cfg(windows)]
    mark_running_for_installer();

    // Font discovery is slow; warm the shared cache before the first compile.
    std::thread::spawn(|| {
        let _ = typst_core::get_or_init_fonts();
    });

    let initial_path = std::env::args_os().nth(1).map(PathBuf::from);
    let settings = Settings::load_from(&Settings::path());

    gpui_kit::application()
        .with_assets(assets::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::install(cx);
            app::bind_keys(cx);
            cx.activate(true);
            #[cfg(target_os = "macos")]
            macos_menu::install(cx);

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1440.), px(900.)), cx)),
                titlebar: Some(TitlebarOptions {
                    title: Some("字笺 · Typst".into()),
                    #[cfg(target_os = "macos")]
                    // Vertically center the traffic lights in the 40px bar.
                    traffic_light_position: Some(point(px(10.), px(12.))),
                    ..gpui_kit::component::TitleBar::title_bar_options()
                }),
                window_min_size: Some(size(px(960.), px(640.))),
                ..gpui_kit::component::TitleBar::window_options()
            };

            let opened = gpui_kit::open_window(options, cx, move |window, cx| {
                cx.new(|cx| {
                    let mut app = TypstEditorApp::new(settings, window, cx);
                    match initial_path {
                        Some(path) => app.open_path(path, window, cx),
                        None => app.restore_session(window, cx),
                    }
                    // Drafts from a crash take priority over everything.
                    app.offer_recovery(window, cx);
                    app
                })
            });
            // Release builds have no console: leave a trace in the log.
            if let Err(err) = opened {
                logging::error(&format!("failed to open window: {err}"));
                eprintln!("failed to open window: {err}");
                cx.quit();
            }
        });
}

/// Inno Setup checks this marker before upgrading or uninstalling. Keep it
/// alive until process termination; multiple editor windows may share it.
#[cfg(windows)]
fn mark_running_for_installer() {
    let name: Vec<u16> = "Local\\ZijianTypstEditorRunning\0".encode_utf16().collect();
    // SAFETY: name is NUL-terminated and lives through the call. The handle
    // deliberately remains open until Windows closes it with the process.
    let handle = unsafe {
        windows_sys::Win32::System::Threading::CreateMutexW(std::ptr::null(), 0, name.as_ptr())
    };
    if handle.is_null() {
        logging::warn(&format!(
            "installer running marker failed: {}",
            std::io::Error::last_os_error()
        ));
    }
}
