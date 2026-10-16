use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=app.rc");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
            root.join("app.manifest").display()
        );
    } else {
        let compiler = if cfg!(windows) {
            "windres"
        } else {
            "x86_64-w64-mingw32-windres"
        };
        let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("app-resource.o");
        let result = Command::new(compiler)
            .current_dir(&root)
            .args(["-i", "app.rc", "-O", "coff", "-o"])
            .arg(&output)
            .status()
            .expect("GNU Windows builds require MinGW windres on PATH");
        assert!(result.success(), "windres failed");
        println!("cargo:rustc-link-arg-bins={}", output.display());
    }
}
