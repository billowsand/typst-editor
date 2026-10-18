//! Bake the commit and build date into the binary for the About box,
//! diagnostics and crash reports. Falls back to "unknown" outside git.

use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
    let commit = std::env::var("ZIJIAN_COMMIT").ok().or_else(|| {
        Command::new("git")
            .args(["rev-parse", "--short=7", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    });
    let dirty = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .ok()
        .is_some_and(|o| o.status.success() && !o.stdout.is_empty());
    let commit = match commit {
        Some(c) if dirty => format!("{c}-dirty"),
        Some(c) => c,
        None => "unknown".into(),
    };
    let date = std::env::var("ZIJIAN_BUILD_DATE").unwrap_or_else(|_| {
        // Days since the epoch → civil date, without a date crate.
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0) as i64;
        let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
        format!("{y:04}-{m:02}-{d:02}")
    });
    println!("cargo:rustc-env=ZIJIAN_COMMIT={commit}");
    println!("cargo:rustc-env=ZIJIAN_BUILD_DATE={date}");
    println!("cargo:rerun-if-env-changed=ZIJIAN_COMMIT");
    println!("cargo:rerun-if-env-changed=ZIJIAN_BUILD_DATE");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");
}

fn windows_resources() {
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let icon = root.join("assets/icons/zijian.ico");
    let icon_path = icon.to_string_lossy().replace('\\', "/");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let numeric = version.split('.').take(3).collect::<Vec<_>>().join(",");
    let rc = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("zijian.rc");
    std::fs::write(
        &rc,
        format!(
            r#"
#include <windows.h>
1 ICON "{icon_path}"
1 VERSIONINFO
FILEVERSION {numeric},0
PRODUCTVERSION {numeric},0
FILEOS VOS_NT_WINDOWS32
FILETYPE VFT_APP
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Zijian Contributors\0"
      VALUE "FileDescription", "Zijian - Typst Editor\0"
      VALUE "FileVersion", "{version}\0"
      VALUE "InternalName", "zijian\0"
      VALUE "OriginalFilename", "typst-editor-gpui.exe\0"
      VALUE "ProductName", "Zijian Typst\0"
      VALUE "ProductVersion", "{version}\0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#
        ),
    )
    .expect("write Windows resources");
    embed_resource::compile(&rc, embed_resource::NONE)
        .manifest_required()
        .expect("embed application icon and version information");
    println!("cargo:rerun-if-changed={}", icon.display());
}

/// Howard Hinnant's days-to-civil algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
