//! askama template structs — the bridge between Rust data and HTML files.
//!
//! askama works like serde-in-reverse for HTML: you write a normal Rust struct,
//! tag it with `#[template(path = "...")]` pointing at a file under the
//! templates directory, and askama generates a `.render()` method at COMPILE
//! time. The struct's fields become the variables the template can reference. A
//! typo in `{{ agent.nmae }}` fails the build, not the page load.
//!
//! The template *files* live in `frontend/` (see `askama.toml` at the repo root,
//! which points askama there). Keeping HTML out of Rust source keeps designers
//! and compilers both happy.
//!
//! Rust concept spotlight: **derive macros generating methods.** You've seen
//! `#[derive(Serialize)]` add serialization. `#[derive(Template)]` adds a
//! `render()` method whose body is your HTML with the variables filled in — more
//! evidence that in Rust, a lot of "framework magic" is just code generated from
//! an attribute you can read.

// use askama::Template;

/// Data for the explorer landing page (`/`): a leaderboard of scored agents.
///
/// Once askama is wired in this becomes:
///     #[derive(Template)]
///     #[template(path = "explorer.html")]
///     pub struct ExplorerPage { pub agents: Vec<AgentRow> }
pub struct ExplorerPage {
    /// The rows to show, already sorted by final score descending.
    pub agents: Vec<AgentRow>,
}

/// One line in the leaderboard table.
pub struct AgentRow {
    pub agent_id: u64,
    pub domain: String,
    pub chain: String,
    /// Final trust score in [0, 1], formatted in the template as a percentage.
    pub final_score: f64,
    /// Whether the endpoint was alive at last probe — a little status dot.
    pub is_alive: bool,
}

/// Data for one agent's detail page (`/agent/{id}`): the full score breakdown
/// plus enrichment, so a visitor can see *why* an agent scored as it did.
///     #[derive(Template)]
///     #[template(path = "agent.html")]
pub struct AgentDetailPage {
    pub agent_id: u64,
    pub domain: String,
    pub chain: String,
    /// The full sub-score breakdown from the `scoring` crate. Reusing the
    /// library's own type here means the page and the API can never disagree
    /// about what the score is.
    pub score: scoring::TrustScore,
    /// Cluster membership, if any — shown so the Sybil penalty is transparent.
    pub cluster_size: u32,
    pub suspicion: f64,
}

/// Data for the static-ish methodology write-up (`/methodology`). Mostly prose
/// in the template; we pass the current default weights so the page always
/// reflects the real configuration rather than hard-coded numbers that drift.
///     #[derive(Template)]
///     #[template(path = "methodology.html")]
pub struct MethodologyPage {
    pub weights: scoring::ScoreWeights,
}
