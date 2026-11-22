//! Build script: the version shown in the app and, on Windows, the resources
//! of the executable (icon and version information).

use std::path::Path;
use std::process::Command;

fn main() {
    let version = version();
    println!("cargo:rustc-env=RUSTQL_VERSION={version}");
    #[cfg(windows)]
    windows_resources(&version);
}

/// The version from Cargo.toml, plus the commit for builds that are not a
/// release, so a development build is never mistaken for one:
/// `0.9.0+g1a2b3c4`.
///
/// It is the plain version when `RUSTQL_RELEASE` is set (the release
/// workflow, the RPM and Flatpak builds), outside a git checkout (a source
/// tarball) and on the commit tagged with that version.
fn version() -> String {
    let base = env!("CARGO_PKG_VERSION").to_string();
    println!("cargo:rerun-if-env-changed=RUSTQL_RELEASE");
    if std::env::var_os("RUSTQL_RELEASE").is_some() {
        return base;
    }
    let Some(hash) = git(&["rev-parse", "--short=7", "HEAD"]) else {
        return base;
    };
    // Rerun whenever HEAD moves (commit, checkout, reset…)
    if let Some(git_dir) = git(&["rev-parse", "--git-dir"]) {
        for file in ["HEAD", "logs/HEAD"] {
            let path = Path::new(&git_dir).join(file);
            if path.exists() {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
    let tag = git(&["describe", "--tags", "--exact-match", "HEAD"]);
    if tag.as_deref() == Some(format!("v{base}").as_str()) {
        return base;
    }
    format!("{base}+g{hash}")
}

/// Output of a git command, if git is installed and it succeeds.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    (output.status.success() && !text.trim().is_empty()).then(|| text.trim().to_string())
}

/// Icon and version information of rustql.exe (Properties → Details).
#[cfg(windows)]
fn windows_resources(version: &str) {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=assets/icon/icon.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon/icon.ico")
        .set("ProductName", "RustQL")
        .set("FileDescription", "RustQL")
        .set("ProductVersion", version)
        .set("LegalCopyright", "Copyright © 2026 Alberto López");
    res.compile().expect("Windows resources");
}
