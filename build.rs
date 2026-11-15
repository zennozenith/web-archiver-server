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
    process::{Command, Stdio},
};

/// Justfile recipe that builds the frontend.
const FRONTEND_RECIPE: &str = "frontend-build";

fn main() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("should have cargo manifest"));
    let root = manifest_dir.join("static/vite/src/templates");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", root.display());
    println!("cargo:rerun-if-env-changed=WEB_ARCHIVER_BUILD_FRONTEND");

    // `just` is a development convenience, not a build prerequisite. Plain
    // `cargo build` works in environments without it (or when the frontend has
    // already been built); set `WEB_ARCHIVER_BUILD_FRONTEND=0` to skip the
    // build explicitly.
    if frontend_build_disabled() {
        println!("cargo:warning=frontend build disabled; using existing templates");
    } else if !just_available() {
        println!("cargo:warning=just not found; skipping frontend build, using existing templates");
    } else {
        run_frontend_build(&manifest_dir);
    }

    if !root.exists() {
        println!("cargo:warning={} not found, skipping", root.display());
        return;
    }

    if let Err(e) = visit(&root) {
        panic!("failed to fix templates: {e}");
    }
}

fn frontend_build_disabled() -> bool {
    matches!(
        env::var("WEB_ARCHIVER_BUILD_FRONTEND").as_deref(),
        Ok("0" | "false" | "no")
    )
}

fn just_available() -> bool {
    Command::new("just")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn run_frontend_build(manifest_dir: &Path) {
    let status = Command::new("just")
        .arg(FRONTEND_RECIPE)
        .current_dir(manifest_dir)
        .status();

    match status {
        Ok(status) if status.success() => {}
        Ok(status) => {
            println!("cargo:warning=frontend build exited with {status}; using existing templates")
        }
        Err(error) => println!(
            "cargo:warning=could not run frontend build: {error}; using existing templates"
        ),
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
