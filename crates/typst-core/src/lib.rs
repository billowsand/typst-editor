//! Typst compiler core used by the GPUI editor.
//!
//! - [`compiler`]: compiler state, overlays, incremental compile and page cache.
//! - [`world`]: `typst::World` implementation, project root discovery, fonts.
//! - [`ide`]: editor <-> preview jumps and document outline.
//! - [`fs`]: small file system helpers used by the file tree.

pub mod compiler;
pub mod fs;
pub mod ide;
pub mod world;

pub use compiler::{CompileResult, CompilerState, DiagnosticInfo, OutlineItem, OutlinePosition};
pub use world::{discover_project_root, get_or_init_fonts, TypstWorld};
