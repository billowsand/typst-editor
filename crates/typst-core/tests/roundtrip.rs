//! End-to-end check of the paths both front ends rely on: compile an entry
//! that includes another file, rasterize a page, and map editor offsets to
//! preview positions and back.

use std::path::PathBuf;

use typst_core::{ide, CompilerState};

fn sample_project() -> (tempdir::Dir, PathBuf, PathBuf) {
    let dir = tempdir::Dir::new();
    let main = dir.path.join("main.typ");
    let chapter = dir.path.join("chapter.typ");
    std::fs::write(
        &main,
        "#set page(width: 200pt, height: 200pt, margin: 20pt)\n= Title\nHello world.\n#include \"chapter.typ\"\n",
    )
    .unwrap();
    std::fs::write(&chapter, "Included text.\n").unwrap();
    (dir, main, chapter)
}

#[test]
fn compile_render_and_jump() {
    let (_dir, main, chapter) = sample_project();
    let main_str = main.display().to_string();
    let main_text = std::fs::read_to_string(&main).unwrap();

    let state = CompilerState::new();
    state.open(&main_str, &main_text).unwrap();
    let mut result = state.update_file_and_compile(&main_str, &main_text);
    ide::populate_outline(&state, &mut result);
    assert!(result.success, "compile failed: {:?}", result.errors);
    assert_eq!(result.page_count, 1);
    assert_eq!(result.outline.len(), 1);
    assert_eq!(result.outline[0].title, "Title");

    // Page geometry and raster output.
    let sizes = state.page_sizes();
    assert_eq!(sizes, vec![(200.0, 200.0)]);
    let (w, h, rgba) = state.render_page_rgba(0, 2.0).unwrap();
    assert_eq!((w, h), (400, 400));
    assert_eq!(rgba.len(), (w * h * 4) as usize);

    // Editor offset inside "Hello" -> a position on page 1.
    let hello = main_text.find("Hello").unwrap() + 2;
    let positions = ide::jump_from_offset(&state, &main_str, hello);
    let pos = positions.first().expect("no preview position for cursor");
    assert_eq!(pos.page, 1);

    // Clicking slightly above that baseline lands back in main.typ near "Hello".
    let jump = ide::jump_from_click(&state, pos.page, pos.x + 1.0, pos.y - 3.0)
        .expect("click did not resolve");
    assert_eq!(jump.file.as_deref().map(PathBuf::from), Some(main.clone()));
    let offset = jump.offset.unwrap();
    assert!(
        (hello.saturating_sub(3)..hello + 4).contains(&offset),
        "offset {offset} not near {hello}"
    );

    // Editing the included file recompiles through its entry (route table).
    let chapter_str = chapter.display().to_string();
    let edited = "Changed included text.\n";
    let result = state.update_file_and_compile(&chapter_str, edited);
    assert!(result.success, "compile failed: {:?}", result.errors);
    assert_eq!(result.entry_path.as_deref(), Some(main_str.as_str()));

    // Route lookups never fall back to compiling a file on its own.
    assert_eq!(state.entry_for(&chapter_str), Some(main.clone()));
    let stray = main.with_file_name("refs.bib").display().to_string();
    assert_eq!(state.entry_for(&stray), None);
    state.pin_preview_entry(Some(&chapter_str)).unwrap();
    assert_eq!(state.pinned_entry(), Some(chapter.clone()));
    assert_eq!(state.entry_for(&stray), Some(chapter.clone()));
}

#[test]
fn diagnostics_carry_ranges() {
    let (_dir, main, _) = sample_project();
    let main_str = main.display().to_string();
    let text = "#let x = \n= Broken\n#undefined-fn()\n";
    let state = CompilerState::new();
    state.open(&main_str, text).unwrap();
    let result = state.update_file_and_compile(&main_str, text);
    assert!(!result.success);
    let diag = result
        .diagnostics
        .iter()
        .find(|d| d.is_error)
        .expect("no structured diagnostic");
    assert_eq!(PathBuf::from(&diag.file), main);
    assert!(diag.start <= diag.end && diag.end <= text.len());
}

#[test]
fn completion_and_tooltip() {
    let (_dir, main, _) = sample_project();
    let main_str = main.display().to_string();
    let text = std::fs::read_to_string(&main).unwrap();
    let state = CompilerState::new();
    state.open(&main_str, &text).unwrap();
    assert!(state.update_file_and_compile(&main_str, &text).success);

    // Unsaved edit: typing `#hea` should offer `heading`.
    let live = format!("{text}#hea");
    let (from, items) =
        ide::autocomplete(&state, &main_str, &live, live.len(), false).expect("no completions");
    assert_eq!(from, live.len() - 3);
    assert!(items.iter().any(|c| c.label == "heading"));

    // Hovering `page` in `#set page(...)` gives docs.
    let hover_at = text.find("page").unwrap() + 1;
    assert!(ide::tooltip(&state, &main_str, &text, hover_at).is_some());
}

/// Minimal self-cleaning temp dir (avoids a dev-dependency).
mod tempdir {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    pub struct Dir {
        pub path: PathBuf,
    }

    impl Dir {
        pub fn new() -> Self {
            static N: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "typst-core-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
