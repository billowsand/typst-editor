mod commands;
mod compiler;

use compiler::CompilerState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(CompilerState::new())
        .setup(|_| {
            std::thread::spawn(|| {
                let _ = crate::compiler::world::get_or_init_fonts();
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::read_text_file,
            commands::open_file,
            commands::set_preview_entry,
            commands::clear_preview_entry_pin,
            commands::reset_session,
            commands::save_file,
            commands::open_file_dialog,
            commands::save_file_dialog,
            commands::export_pdf_dialog,
            commands::compile_current,
            commands::render_page,
            commands::page_count,
            commands::jump_from_cursor,
            commands::jump_from_offset,
            commands::jump_from_click,
            commands::get_project_root,
            commands::get_preview_entry,
            commands::get_document_outline,
            commands::resolve_project_root,
            commands::list_dir,
            commands::list_system_fonts,
            commands::create_file,
            commands::create_directory,
            commands::delete_path,
            commands::rename_path,
            commands::reveal_in_explorer,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
