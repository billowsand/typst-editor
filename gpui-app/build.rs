//! Bake the commit and build date into the binary for the About box,
//! diagnostics and crash reports. Falls back to "unknown" outside git.

use std::process::Command;

fn main() {
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
