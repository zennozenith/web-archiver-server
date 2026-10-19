// build.rs
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=frontend");
    println!("cargo:rerun-if-changed=templates");

    let status = Command::new("devenv")
        .args(["tasks", "run", "frontend"])
        // .current_dir("frontend")
        .status()
        .expect("failed to run vite build — is bun installed?");

    if !status.success() {
        panic!("tailwindcss build failed");
    }
}
