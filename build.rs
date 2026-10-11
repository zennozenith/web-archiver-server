// build.rs
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=frontend/src");
    println!("cargo:rerun-if-changed=templates");
    println!("cargo:rerun-if-changed=frontend/vite.config.js");

    let status = Command::new("bun")
        .args(["run", "build"])
        .current_dir("frontend")
        .status()
        .expect("failed to run vite build — is bun installed?");

    if !status.success() {
        panic!("vite build failed");
    }
}
