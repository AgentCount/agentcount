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

    // Re-run triggers. Watching only `.git/HEAD` is NOT enough, and getting
    // this wrong stamps runs with a commit that did not produce them:
    // committing on a branch does not change HEAD's CONTENTS (it stays
    // `ref: refs/heads/<branch>`) — it rewrites the ref file HEAD points at.
    // So cargo saw no change, skipped this script, and baked a stale SHA into
    // the binary. A run whose `checker_commit` names the wrong code is worse
    // than one with no stamp at all, because it looks trustworthy.
    println!("cargo:rerun-if-changed=.git/HEAD");
    if let Ok(head) = std::fs::read_to_string(".git/HEAD") {
        if let Some(reference) = head.trim().strip_prefix("ref: ") {
            println!("cargo:rerun-if-changed=.git/{reference}");
        }
    }
    // Catches staged changes, so the `-dirty` suffix cannot go stale either.
    println!("cargo:rerun-if-changed=.git/index");
}
