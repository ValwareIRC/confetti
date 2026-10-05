//! The bar for the parser: every configuration file UnrealIRCd ships must
//! parse, tile completely, and come back byte for byte.

use confetti_lib::cst::{Doc, Kind, Node, Severity};
use std::fs;
use std::path::{Path, PathBuf};

const CORPUS: &str = r"C:\Users\Valerie\unrealircd-pr\doc\conf";

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().map(|x| x == "conf").unwrap_or(false) {
            out.push(p);
        }
    }
}

fn depth(n: &Node) -> usize {
    let mut best = 0;
    for c in &n.children {
        let d = if c.kind == Kind::Entry { 1 + depth(c) } else { depth(c) };
        best = best.max(d);
    }
    best
}

fn count_entries(n: &Node) -> usize {
    let mut t = if n.kind == Kind::Entry { 1 } else { 0 };
    for c in &n.children {
        t += count_entries(c);
    }
    t
}

#[test]
fn every_shipped_config_round_trips() {
    let root = Path::new(CORPUS);
    if !root.exists() {
        eprintln!("corpus not present at {CORPUS}, skipping");
        return;
    }
    let mut files = Vec::new();
    collect(root, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no .conf files found under {CORPUS}");

    let mut total_bytes = 0usize;
    let mut total_entries = 0usize;
    let mut deepest = (0usize, String::new());
    let mut with_errors: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for path in &files {
        let Ok(src) = fs::read_to_string(path) else { continue };
        total_bytes += src.len();
        let doc = Doc::parse(src.clone());
        let name = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        if let Err(e) = doc.verify_tiling() {
            failures.push(format!("{name}: tiling: {e}"));
        }
        if doc.render() != src {
            failures.push(format!("{name}: round-trip changed the file"));
        }

        let d = depth(&doc.root);
        if d > deepest.0 {
            deepest = (d, name.clone());
        }
        total_entries += count_entries(&doc.root);

        let errs: Vec<_> = doc
            .diagnostics
            .iter()
            .filter(|x| x.severity == Severity::Error)
            .collect();
        if !errs.is_empty() {
            with_errors.push(format!(
                "{name}: {}",
                errs.iter()
                    .map(|e| format!("line {}: {}", e.line, e.message))
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
    }

    println!(
        "\n{} files, {} bytes, {} entries; deepest nesting {} ({})",
        files.len(),
        total_bytes,
        total_entries,
        deepest.0,
        deepest.1
    );

    if !with_errors.is_empty() {
        println!("\nfiles reporting parse errors:");
        for e in &with_errors {
            println!("  {e}");
        }
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    assert!(
        with_errors.is_empty(),
        "shipped configs should parse cleanly, but:\n{}",
        with_errors.join("\n")
    );
    assert!(
        deepest.0 >= 5,
        "expected the corpus to reach 5 levels, only saw {}",
        deepest.0
    );
}
