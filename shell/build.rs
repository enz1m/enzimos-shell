//! Builds sandboxed native helpers and publishes their paths to the desktop binaries.
use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=../tools/enzui-plugin-worker.c");
    println!("cargo:rerun-if-changed=../tools/enzui-password-hash.c");
    println!("cargo:rerun-if-env-changed=CC");
    let linux = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux");
    let native = env::var("HOST").ok() == env::var("TARGET").ok();
    if linux && native {
        let directory = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
        let output = directory.join("enzui-password-hash");
        let temporary = directory.join("enzui-password-hash.tmp");
        let status = Command::new(env::var_os("CC").unwrap_or_else(|| "cc".into()))
            .args([
                "-std=c11",
                "-O2",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-D_FORTIFY_SOURCE=3",
                "-fstack-protector-strong",
                "-Wl,-z,relro,-z,now",
                "../tools/enzui-password-hash.c",
                "-lcrypt",
                "-o",
            ])
            .arg(&temporary)
            .status()
            .expect("Cannot run password helper compiler");
        assert!(status.success(), "Password helper compilation failed");
        std::fs::rename(&temporary, &output).expect("Cannot publish password helper");
        println!("cargo:rustc-env=ENZUI_PASSWORD_HASHER={}", output.display());
    } else {
        println!("cargo:rustc-env=ENZUI_PASSWORD_HASHER=");
    }
    let flags = Command::new("pkg-config")
        .args(["--cflags", "--libs", "libseccomp"])
        .output()
        .ok()
        .filter(|o| o.status.success());
    if !linux || !native || flags.is_none() {
        println!("cargo:rustc-env=ENZUI_PLUGIN_WORKER=");
        println!(
            "cargo:warning=Plugin execution unavailable: native Linux and libseccomp are required"
        );
        return;
    }
    let directory = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    let output = directory.join("enzui-plugin-worker");
    let temporary = directory.join("enzui-plugin-worker.tmp");
    let flags = String::from_utf8(flags.expect("pkg-config succeeded above").stdout)
        .expect("pkg-config flags must be UTF-8");
    let compiler = env::var_os("CC").unwrap_or_else(|| "cc".into());
    let status = Command::new(compiler)
        .args([
            "-std=c11",
            "-O2",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-D_FORTIFY_SOURCE=3",
            "-fstack-protector-strong",
            "-Wl,-z,relro,-z,now",
            "../tools/enzui-plugin-worker.c",
            "-o",
        ])
        .arg(&temporary)
        .args(flags.split_whitespace())
        .status()
        .expect("Cannot run plugin worker compiler");
    assert!(status.success(), "Plugin worker compilation failed");
    std::fs::rename(&temporary, &output).expect("Cannot publish plugin worker");
    println!("cargo:rustc-env=ENZUI_PLUGIN_WORKER={}", output.display());
}
