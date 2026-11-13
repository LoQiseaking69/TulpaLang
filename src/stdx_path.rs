//! Library search path for `import "std/..."` and other non-relative imports.
//!
//! Searched in order: every entry of `TULPA_PATH` (`:`-separated, `;` on Windows), then
//! `<exe>/../lib/tulpa` (installed layout), then `<exe>/../../stdlib` and the source tree's
//! `stdlib/` (development builds). A leading `std/` maps onto the library root.

use std::path::{Path, PathBuf};

pub fn roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = std::env::var_os("TULPA_PATH") {
        out.extend(std::env::split_paths(&p));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin) = exe.parent() {
            out.push(bin.join("../lib/tulpa"));
            out.push(bin.join("lib"));
            out.push(bin.join("../../stdlib"));
        }
    }
    out.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("stdlib"));
    out
}

pub fn resolve(path: &str) -> Option<PathBuf> {
    if path.starts_with('.') || Path::new(path).is_absolute() {
        return None;
    }
    let rel = path.strip_prefix("std/").unwrap_or(path);
    for root in roots() {
        let mut p = root.join(rel);
        if p.extension().is_none() {
            p.set_extension("rav");
        }
        if p.is_file() {
            return Some(p);
        }
    }
    None
}
