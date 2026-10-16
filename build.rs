use std::{env, path::PathBuf, process::Command};
fn main() {
    for name in [
        "app.manifest",
        "cli.manifest",
        "app.rc",
        "cli.rc",
        "app.ico",
    ] {
        println!("cargo:rerun-if-changed={name}");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    for (binary, manifest, resource) in [
        ("memreduct-rs", "app.manifest", "app.rc"),
        ("memreduct-cli", "cli.manifest", "cli.rc"),
    ] {
        if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            // Compile resources for both icon and explicit UAC manifest. Disable the default linker fragment.
            let output =
                PathBuf::from(env::var_os("OUT_DIR").unwrap()).join(format!("{binary}.res"));
            let mut command = Command::new("rc.exe");
            command
                .current_dir(&root)
                .arg("/nologo")
                .arg("/fo")
                .arg(&output)
                .arg(resource);
            let status = command.status().expect("Windows SDK rc.exe is required");
            assert!(status.success(), "resource compilation failed: {manifest}");
            println!("cargo:rustc-link-arg-bin={binary}=/MANIFEST:NO");
            println!("cargo:rustc-link-arg-bin={binary}={}", output.display());
        } else {
            let compiler = if cfg!(windows) {
                "windres"
            } else {
                "x86_64-w64-mingw32-windres"
            };
            let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join(format!("{binary}.o"));
            let status = Command::new(compiler)
                .current_dir(&root)
                .args(["-i", resource, "-O", "coff", "-o"])
                .arg(&output)
                .status()
                .expect("GNU Windows builds require MinGW windres on PATH");
            assert!(status.success(), "windres failed");
            println!("cargo:rustc-link-arg-bin={binary}={}", output.display());
        }
    }
}
