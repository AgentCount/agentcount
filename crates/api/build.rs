//! Stamp the git commit into the binary at build time.
//!
//! A result that cannot name the code that produced it is not reproducible,
//! and asking a human to type the SHA is asking for a wrong one.
//!
//! This is the same script `crates/sweeper` runs, for the same reason and by
//! deliberate duplication rather than a shared build-dependency crate: the two
//! binaries are stamped independently, and a spot check answered by an API
//! built from commit X must say X even when the sweeper that produced the
//! census rows beside it was built from something else. Sharing the stamp
//! would quietly make one of those two claims false.
//!
//! The API needs this only because of `routes::spot_check`: every other
//! endpoint reads rows the sweeper already stamped, so until 2026-08-03 there
//! was no answer this process itself was the author of.

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

    // A check answered from a dirty tree is not reproducible; say so in the
    // stamp rather than pretending the commit describes what ran.
    let stamp = if dirty {
        format!("{commit}-dirty")
    } else {
        commit
    };
    println!("cargo:rustc-env=CHECKER_COMMIT={stamp}");

    // Re-run triggers. Watching only `.git/HEAD` is NOT enough, and getting
    // this wrong stamps answers with a commit that did not produce them:
    // committing on a branch does not change HEAD's CONTENTS (it stays
    // `ref: refs/heads/<branch>`) — it rewrites the ref file HEAD points at.
    // So cargo saw no change, skipped this script, and baked a stale SHA into
    // the binary. A result whose `checker_commit` names the wrong code is
    // worse than one with no stamp at all, because it looks trustworthy.
    println!("cargo:rerun-if-changed=.git/HEAD");
    if let Ok(head) = std::fs::read_to_string(".git/HEAD")
        && let Some(reference) = head.trim().strip_prefix("ref: ")
    {
        println!("cargo:rerun-if-changed=.git/{reference}");
    }
    // Catches staged changes, so the `-dirty` suffix cannot go stale either.
    println!("cargo:rerun-if-changed=.git/index");
}
