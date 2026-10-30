#![allow(
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    clippy::restriction,
    clippy::cargo
)]

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("shuld have cargo manifest"));
    let root = manifest_dir.join("static/vite/src/templates");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", root.display());

    let status = Command::new("devenv")
        .args(["tasks", "run", "frontend"])
        // .current_dir("frontend")
        .status()
        .expect("failed to run vite build — is bun installed?");

    if !status.success() {
        panic!("vite build failed");
    }

    if !root.exists() {
        println!("cargo:warning={} not found, skipping", root.display());
        return;
    }

    if let Err(e) = visit(&root) {
        panic!("failed to fix templates: {e}");
    }
}

fn visit(dir: &Path) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            visit(&path)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("html") {
            fix_template(&path)?;
        }
    }
    Ok(())
}

fn is_extends(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("{% extends") || t.starts_with("{%- extends")
}

fn fix_template(path: &Path) -> io::Result<()> {
    let src = fs::read_to_string(path)?;
    // split_inclusive keeps the original line endings (\n or \r\n)
    let lines: Vec<&str> = src.split_inclusive('\n').collect();

    let Some(idx) = lines.iter().position(|l| is_extends(l)) else {
        return Ok(()); // no extends line, leave file alone
    };
    if idx == 0 {
        return Ok(()); // already first, nothing to do (keeps build idempotent)
    }

    let mut extends_line = lines[idx].to_string();
    if !extends_line.ends_with('\n') {
        extends_line.push('\n');
    }

    let mut out = extends_line;
    for (i, line) in lines.iter().enumerate() {
        if i != idx {
            out.push_str(line);
        }
    }

    fs::write(path, out)?;
    println!("cargo:warning=moved extends to top in {}", path.display());
    Ok(())
}
