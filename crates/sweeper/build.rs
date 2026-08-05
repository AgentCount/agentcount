//! Stamp the git commit into the binary at build time.
//!
//! A result that cannot name the code that produced it is not reproducible,
//! and asking a human to type the SHA is asking for a wrong one.

fn main() {
    // An explicit stamp wins, and exists because the ONLY place this build
    // runs unattended has no git at all.
    //
    // `.dockerignore` excludes `.git` — correctly, since a repository in an
    // image layer is bulk nobody needs — and Cloud Build uploads the source as
    // a tarball regardless. So `git rev-parse` below fails there and falls back
    // to "unknown", which is exactly what happened: every scheduled run of the
    // 2026-08 census recorded `checker_commit: unknown`, and a result that
    // cannot name the code that produced it is not reproducible. That is the
    // one property this project sells.
    //
    // The value comes from `--build-arg CHECKER_COMMIT=$(git rev-parse HEAD)`,
    // which `scripts/deploy-weekly-sweep.sh` supplies from the tree it is
    // deploying. Trusted without verification on purpose: nothing inside the
    // container CAN verify it, and a stamp that lies is a problem with whoever
    // passed it, not something this build script can defend against.
    if let Ok(explicit) = std::env::var("CHECKER_COMMIT_OVERRIDE")
        && !explicit.trim().is_empty()
    {
        println!("cargo:rustc-env=CHECKER_COMMIT={}", explicit.trim());
        println!("cargo:rerun-if-env-changed=CHECKER_COMMIT_OVERRIDE");
        return;
    }
    println!("cargo:rerun-if-env-changed=CHECKER_COMMIT_OVERRIDE");

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
    let stamp = if dirty {
        format!("{commit}-dirty")
    } else {
        commit
    };
    println!("cargo:rustc-env=CHECKER_COMMIT={stamp}");

    // Re-run triggers. Watching only `.git/HEAD` is NOT enough, and getting
    // this wrong stamps runs with a commit that did not produce them:
    // committing on a branch does not change HEAD's CONTENTS (it stays
    // `ref: refs/heads/<branch>`) — it rewrites the ref file HEAD points at.
    // So cargo saw no change, skipped this script, and baked a stale SHA into
    // the binary. A run whose `checker_commit` names the wrong code is worse
    // than one with no stamp at all, because it looks trustworthy.
    println!("cargo:rerun-if-changed=.git/HEAD");
    if let Ok(head) = std::fs::read_to_string(".git/HEAD")
        && let Some(reference) = head.trim().strip_prefix("ref: ")
    {
        println!("cargo:rerun-if-changed=.git/{reference}");
    }
    // Catches staged changes, so the `-dirty` suffix cannot go stale either.
    println!("cargo:rerun-if-changed=.git/index");
}
