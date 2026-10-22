//! Embeds the git commit in the version line (`jastrzab --version`, and the first line of every
//! log), so a pasted log snippet says which build it came from. `JASTRZAB_GIT_HASH` in the
//! environment wins (a sandboxed build, such as the Flatpak's, may have no usable git).

fn main() {
    let hash = std::env::var("JASTRZAB_GIT_HASH")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::process::Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .output()
                .ok()
                .filter(|out| out.status.success())
                .and_then(|out| String::from_utf8(out.stdout).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=JASTRZAB_GIT_HASH={hash}");
    println!("cargo:rerun-if-env-changed=JASTRZAB_GIT_HASH");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=build.rs");
}
