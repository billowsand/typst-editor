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

// ── Cache correctness (regressions from the product review) ─────────

fn outline_titles(state: &CompilerState) -> Vec<String> {
    ide::document_outline(state)
        .into_iter()
        .map(|i| i.title)
        .collect()
}

fn compile(state: &CompilerState, path: &std::path::Path) -> typst_core::CompileResult {
    let text = std::fs::read_to_string(path).unwrap();
    let path = path.display().to_string();
    let _guard = state.compile_lock();
    state.update_file_and_compile(&path, &text)
}

#[test]
fn switching_back_to_an_unchanged_entry_shows_that_entry() {
    let dir = tempdir::Dir::new();
    let a = dir.path.join("a.typ");
    let b = dir.path.join("b.typ");
    std::fs::write(&a, "= Document A\n").unwrap();
    std::fs::write(&b, "= Document B\n").unwrap();
    let state = CompilerState::new();

    assert!(compile(&state, &a).success);
    let pdf_a = state.export_pdf().unwrap();
    assert!(compile(&state, &b).success);
    assert_eq!(outline_titles(&state), ["Document B"]);
    let pdf_b = state.export_pdf().unwrap();

    // Back to A, unchanged: the cache belongs to B and must not be reused.
    let result = compile(&state, &a);
    assert!(result.success);
    assert_eq!(
        result.entry_path.as_deref(),
        Some(a.display().to_string().as_str())
    );
    assert_eq!(outline_titles(&state), ["Document A"]);
    let pdf = state.export_pdf().unwrap();
    assert_ne!(pdf, pdf_b, "exported B while A is the entry");
    assert_eq!(pdf.len(), pdf_a.len());
}

#[test]
fn external_edits_to_dependencies_are_picked_up() {
    let dir = tempdir::Dir::new();
    let main = dir.path.join("main.typ");
    let chapter = dir.path.join("chapter.typ");
    std::fs::write(&main, "#include \"chapter.typ\"\n").unwrap();
    std::fs::write(&chapter, "= Before external edit\n").unwrap();
    let state = CompilerState::new();
    assert!(compile(&state, &main).success);
    assert_eq!(outline_titles(&state), ["Before external edit"]);
    assert!(!state.dependencies_changed());

    // Another program rewrites the chapter (different length guarantees a
    // different stamp even on coarse-mtime file systems).
    std::fs::write(&chapter, "= After the external edit happened\n").unwrap();
    assert!(state.dependencies_changed());
    assert!(compile(&state, &main).success);
    assert_eq!(outline_titles(&state), ["After the external edit happened"]);
    assert!(!state.dependencies_changed());
}

#[test]
fn forced_compile_bypasses_the_cache() {
    let dir = tempdir::Dir::new();
    let main = dir.path.join("main.typ");
    std::fs::write(&main, "= Title\n").unwrap();
    let state = CompilerState::new();
    let text = std::fs::read_to_string(&main).unwrap();
    let path = main.display().to_string();
    let first = state.update_file_and_compile(&path, &text);
    let cached = state.update_file_and_compile(&path, &text);
    assert_eq!(
        cached.revision, first.revision,
        "unchanged input reuses the cache"
    );
    let forced = state.compile_file(
        &path,
        &text,
        typst_core::compiler::CompileOptions { force: true },
    );
    assert!(forced.revision > first.revision);
}

#[test]
fn unsaved_edits_reach_a_freshly_created_world() {
    let dir = tempdir::Dir::new();
    let main = dir.path.join("main.typ");
    let other = dir.path.join("other.typ");
    let chapter = dir.path.join("chapter.typ");
    std::fs::write(&main, "#include \"chapter.typ\"\n").unwrap();
    std::fs::write(&other, "= Other\n").unwrap();
    std::fs::write(&chapter, "= On disk\n").unwrap();
    let state = CompilerState::new();
    assert!(compile(&state, &main).success);

    // Edit the chapter in the editor without saving.
    let main_s = main.display().to_string();
    let chapter_s = chapter.display().to_string();
    assert!(
        state
            .update_file_and_compile(&chapter_s, "= Unsaved edit\n")
            .success
    );
    // Compile an unrelated entry (new world), then come back to main (new
    // world again): the unsaved chapter text must still be used.
    assert!(compile(&state, &other).success);
    let main_text = std::fs::read_to_string(&main).unwrap();
    assert!(state.update_file_and_compile(&main_s, &main_text).success);
    assert_eq!(outline_titles(&state), ["Unsaved edit"]);
}

#[test]
fn export_refuses_a_document_from_another_entry() {
    let dir = tempdir::Dir::new();
    let good = dir.path.join("good.typ");
    let broken = dir.path.join("broken.typ");
    std::fs::write(&good, "= Good\n").unwrap();
    std::fs::write(&broken, "#undefined-call()\n").unwrap();
    let state = CompilerState::new();
    assert!(compile(&state, &good).success);
    assert!(!compile(&state, &broken).success);
    assert!(
        state.export_pdf().is_err(),
        "exported good.typ while broken.typ is the entry"
    );
    assert_eq!(
        state.page_count(),
        0,
        "another entry's pages must not stay on screen"
    );
}

/// Needs network access; run with `cargo test -- --ignored`.
#[test]
#[ignore]
fn preview_packages_are_downloaded_on_first_use() {
    let dir = tempdir::Dir::new();
    let main = dir.path.join("main.typ");
    std::fs::write(
        &main,
        "#import \"@preview/example:0.1.0\": add\n= Sum #add(1, 2)\n",
    )
    .unwrap();
    let state = CompilerState::new();
    let result = compile(&state, &main);
    assert!(result.success, "{:?}", result.errors);
}

#[test]
fn cached_compiles_keep_their_warnings() {
    let dir = tempdir::Dir::new();
    let main = dir.path.join("main.typ");
    // An unknown font name is a warning, not an error.
    std::fs::write(&main, "#set text(font: \"No Such Font 123\")\n= Title\n").unwrap();
    let state = CompilerState::new();
    let first = compile(&state, &main);
    assert!(first.success);
    assert!(!first.diagnostics.is_empty(), "expected a font warning");
    let cached = compile(&state, &main);
    assert_eq!(
        cached.revision, first.revision,
        "second compile is a cache hit"
    );
    assert_eq!(cached.diagnostics.len(), first.diagnostics.len());
}

#[test]
fn discarded_overlays_take_effect_without_compiling() {
    let dir = tempdir::Dir::new();
    let main = dir.path.join("main.typ");
    let chapter = dir.path.join("chapter.typ");
    std::fs::write(&main, "#include \"chapter.typ\"\n").unwrap();
    std::fs::write(&chapter, "= On disk\n").unwrap();
    let state = CompilerState::new();
    assert!(compile(&state, &main).success);
    let chapter_s = chapter.display().to_string();
    assert!(
        state
            .update_file_and_compile(&chapter_s, "= Unsaved\n")
            .success
    );
    assert_eq!(outline_titles(&state), ["Unsaved"]);

    state.discard_overlay(&chapter_s, "= On disk\n");
    assert!(compile(&state, &main).success);
    assert_eq!(outline_titles(&state), ["On disk"]);
}

#[test]
fn live_jumps_refuse_text_the_compiler_has_not_seen() {
    let dir = tempdir::Dir::new();
    let main = dir.path.join("main.typ");
    std::fs::write(&main, "= Title\nHello world.\n").unwrap();
    let state = CompilerState::new();
    assert!(compile(&state, &main).success);
    let path = main.display().to_string();
    let text = std::fs::read_to_string(&main).unwrap();
    let at = text.find("Hello").unwrap();
    assert!(!ide::jump_from_offset_live(&state, &path, &text, at).is_empty());
    let edited = format!("{text}more typing");
    assert!(ide::jump_from_offset_live(&state, &path, &edited, at).is_empty());
}

#[test]
fn session_ids_change_on_reset() {
    let state = CompilerState::new();
    let before = state.session_id();
    state.reset_session();
    assert_ne!(state.session_id(), before);
}
