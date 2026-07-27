//! Stamp the git commit into the binary at build time.
//!
//! A result that cannot name the code that produced it is not reproducible,
//! and asking a human to type the SHA is asking for a wrong one.

fn main() {
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);

    // A sweep from a dirty tree is not reproducible; say so in the stamp
    // rather than pretending the commit describes what ran.
    let stamp = if dirty { format!("{commit}-dirty") } else { commit };
    println!("cargo:rustc-env=CHECKER_COMMIT={stamp}");
    println!("cargo:rerun-if-changed=.git/HEAD");
}
