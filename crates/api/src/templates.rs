//! askama template structs — the bridge between Rust data and HTML files.
//!
//! askama works like serde-in-reverse for HTML: you write a normal Rust struct,
//! tag it with `#[template(path = "...")]` pointing at a file under the templates
//! directory, and `#[derive(Template)]` generates a `.render()` method at COMPILE
//! time. The struct's fields become the variables the template can reference. A
//! typo in `{{ agent.nmae }}` fails the build, not the page load.
//!
//! Design choice: we keep the *templates* dumb and do all formatting HERE, in
//! Rust. Each field is either a ready-to-print `String`/number or a `bool` for an
//! `{% if %}`. That means the `.html` files only ever use `{{ field }}`,
//! `{% for %}`, and `{% if %}` — no template-language filters to get wrong — and
//! all the logic stays in code you can unit-test.
//!
//! The template *files* live in `frontend/` (see `askama.toml`), keeping HTML out
//! of the Rust source tree.

use askama::Template;

/// Data for the explorer landing page (`/`): a leaderboard of scored agents.
#[derive(Template)]
#[template(path = "explorer.html")]
pub struct ExplorerPage {
    /// Rows already sorted by final score descending.
    pub agents: Vec<AgentRow>,
}

/// One line in the leaderboard table. All display-ready.
pub struct AgentRow {
    pub agent_id: i64,
    pub domain: String,
    pub chain: String,
    /// Final trust score as a whole-number percentage (e.g. 73).
    pub final_pct: i64,
    /// Whether the endpoint was alive at last probe — drives a status dot.
    pub is_alive: bool,
}

/// Data for one agent's detail page (`/agent/{id}`): the full score breakdown.
#[derive(Template)]
#[template(path = "agent.html")]
pub struct AgentDetailPage {
    pub agent_id: i64,
    pub domain: String,
    pub chain: String,
    // Every sub-score pre-formatted as a whole-number percentage, used both as
    // the label text and as the CSS bar width.
    pub final_pct: i64,
    pub payment_pct: i64,
    pub liveness_pct: i64,
    pub age_pct: i64,
    pub reputation_pct: i64,
    pub sybil_pct: i64,
    /// Show the Sybil warning box only when the agent is actually in a cluster.
    pub in_cluster: bool,
    pub cluster_size: i64,
    pub suspicion_pct: i64,
}

/// Data for the methodology write-up (`/methodology`). The weights are formatted
/// from the LIVE `scoring::ScoreWeights::default()` in the handler, so this page
/// can never drift out of sync with the actual scoring code.
#[derive(Template)]
#[template(path = "methodology.html")]
pub struct MethodologyPage {
    pub payment_w: String,
    pub liveness_w: String,
    pub age_w: String,
    pub reputation_w: String,
}
