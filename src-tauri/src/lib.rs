mod commands;
mod compiler;

use compiler::CompilerState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(CompilerState::new())
        .invoke_handler(tauri::generate_handler![
            commands::read_text_file,
            commands::open_file,
            commands::save_file,
            commands::open_file_dialog,
            commands::save_file_dialog,
            commands::export_pdf_dialog,
            commands::compile_current,
            commands::render_page,
            commands::page_count,
            commands::jump_from_cursor,
            commands::jump_from_click,
            commands::get_project_root,
            commands::resolve_project_root,
            commands::list_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
