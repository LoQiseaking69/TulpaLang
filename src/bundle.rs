//! `tulpa bundle`: a standalone executable that carries the whole language.
//!
//! The bundle is a copy of a Tulpa runtime binary with the program appended: the main file,
//! every module it imports (including `std/` modules), and the options to run it with. When a
//! Tulpa binary starts and finds such a payload at its own end, it runs that program instead
//! of reading the command line. This is the deployment path for programs that use features
//! the static native compiler does not cover (dicts, closures, tasks, ...), on any desktop or
//! Linux board: pass `--runtime` a Tulpa binary built for the target to cross-bundle.

use crate::ast::Stmt;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"TULPABND";

pub struct Payload {
    pub main: String,
    pub hardware: bool,
    pub files: BTreeMap<String, String>,
}

fn imports(stmts: &[Stmt], out: &mut Vec<String>) {
    for s in stmts {
        match s {
            Stmt::Import(p, _, _, _) => out.push(p.clone()),
            Stmt::Fn(d) => imports(&d.body, out),
            Stmt::Struct(sd) => sd.methods.iter().for_each(|m| imports(&m.body, out)),
            Stmt::If(_, a, b) => {
                imports(a, out);
                if let Some(b) = b {
                    imports(b, out);
                }
            }
            Stmt::While(_, _, b, _) | Stmt::Loop(b) | Stmt::For(_, _, b, _) => imports(b, out),
            Stmt::Try(a, _, b) => {
                imports(a, out);
                imports(b, out);
            }
            _ => {}
        }
    }
}

/// The main file plus everything it imports, keyed by the path it will have in the bundle.
pub fn collect(main: &Path) -> Result<Payload, String> {
    let main = std::fs::canonicalize(main).map_err(|e| format!("{}: {e}", main.display()))?;
    let root = main.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut files = BTreeMap::new();
    let mut work: Vec<(PathBuf, String)> = vec![(
        main.clone(),
        main.file_name().unwrap().to_string_lossy().into_owned(),
    )];
    let mut seen = std::collections::HashSet::new();
    while let Some((path, key)) = work.pop() {
        if !seen.insert(path.clone()) {
            continue;
        }
        let src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let prog = crate::parser::parse(&src)
            .map_err(|e| format!("{}:{}:{}: {}", path.display(), e.line, e.col, e.msg))?;
        files.insert(key, src);
        let mut found = Vec::new();
        imports(&prog, &mut found);
        let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
        for imp in found {
            let mut full = base.join(&imp);
            if !full.exists() && full.extension().is_none() {
                full.set_extension("rav");
            }
            if full.exists() {
                let canon = std::fs::canonicalize(&full).map_err(|e| e.to_string())?;
                // a library module importing its neighbour stays in the library tree
                let lib_root = crate::stdx_path::roots()
                    .into_iter()
                    .filter_map(|r| std::fs::canonicalize(r).ok())
                    .find(|r| canon.starts_with(r));
                let key = match (canon.strip_prefix(&root), lib_root) {
                    (Ok(rel), _) => rel.to_string_lossy().replace('\\', "/"),
                    (_, Some(r)) => format!("lib/{}", canon.strip_prefix(&r).unwrap().to_string_lossy().replace('\\', "/")),
                    _ => return Err(format!("import \"{imp}\" in {} is outside the program's folder; bundles need imports under {}", path.display(), root.display())),
                };
                work.push((canon, key));
            } else if let Some(found) = crate::stdx_path::resolve(&imp) {
                let rel = imp.strip_prefix("std/").unwrap_or(&imp);
                let rel = if Path::new(rel).extension().is_none() {
                    format!("{rel}.rav")
                } else {
                    rel.to_string()
                };
                work.push((
                    std::fs::canonicalize(found).map_err(|e| e.to_string())?,
                    format!("lib/{rel}"),
                ));
            } else {
                return Err(format!(
                    "import \"{imp}\" in {} was not found",
                    path.display()
                ));
            }
        }
    }
    Ok(Payload {
        main: main.file_name().unwrap().to_string_lossy().into_owned(),
        hardware: false,
        files,
    })
}

/// Writes `runtime` + payload to `out` and marks it executable.
pub fn write(runtime: &Path, out: &Path, p: &Payload) -> Result<(), String> {
    let mut bytes = std::fs::read(runtime).map_err(|e| format!("{}: {e}", runtime.display()))?;
    if bytes.ends_with(MAGIC) {
        // bundling from a bundle: keep only the runtime part
        let n = bytes.len();
        let len = u64::from_le_bytes(bytes[n - 16..n - 8].try_into().unwrap()) as usize;
        bytes.truncate(n - 16 - len);
    }
    let body =
        serde_json::json!({"main": p.main, "hardware": p.hardware, "files": p.files}).to_string();
    bytes.extend(body.as_bytes());
    bytes.extend((body.len() as u64).to_le_bytes());
    bytes.extend(MAGIC);
    std::fs::write(out, bytes).map_err(|e| format!("{}: {e}", out.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

/// The program appended to this executable, if any.
pub fn embedded() -> Option<Payload> {
    use std::io::{Read, Seek, SeekFrom};
    let exe = std::env::current_exe().ok()?;
    let mut f = std::fs::File::open(exe).ok()?;
    let size = f.metadata().ok()?.len();
    if size < 16 {
        return None;
    }
    f.seek(SeekFrom::Start(size - 16)).ok()?;
    let mut tail = [0u8; 16];
    f.read_exact(&mut tail).ok()?;
    if &tail[8..] != MAGIC {
        return None;
    }
    let len = u64::from_le_bytes(tail[..8].try_into().unwrap());
    if len > size - 16 {
        return None;
    }
    f.seek(SeekFrom::Start(size - 16 - len)).ok()?;
    let mut body = vec![0u8; len as usize];
    f.read_exact(&mut body).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&body).ok()?;
    let files = v["files"]
        .as_object()?
        .iter()
        .map(|(k, s)| (k.clone(), s.as_str().unwrap_or_default().to_string()))
        .collect();
    Some(Payload {
        main: v["main"].as_str()?.to_string(),
        hardware: v["hardware"].as_bool().unwrap_or(false),
        files,
    })
}

/// Unpacks the payload into a per-program cache folder and returns the main file's path.
/// The library modules land in `<folder>/lib`, which the caller puts on `TULPA_PATH`.
pub fn extract(p: &Payload) -> Result<(PathBuf, PathBuf), String> {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for (k, v) in &p.files {
        for b in k.bytes().chain(v.bytes()) {
            h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    let dir = std::env::temp_dir().join(format!("tulpa-bundle-{h:016x}"));
    for (k, src) in &p.files {
        if k.split('/').any(|part| part == "..") {
            return Err(format!("bundle: bad path {k}"));
        }
        let path = dir.join(k);
        if path.is_file()
            && std::fs::read_to_string(&path)
                .map(|s| s == *src)
                .unwrap_or(false)
        {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("bundle: {}: {e}", parent.display()))?;
        }
        std::fs::write(&path, src).map_err(|e| format!("bundle: {}: {e}", path.display()))?;
    }
    Ok((dir.join(&p.main), dir.join("lib")))
}
