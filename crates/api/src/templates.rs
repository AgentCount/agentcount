//! askama template structs — the bridge between Rust data and HTML files.
//!
//! Same design rule as ever: templates stay dumb — every field is
//! display-ready, all formatting happens in Rust. The `.html` files only use
//! `{{ }}`, `{% for %}`, and `{% if %}`, and all the logic stays in code you
//! can unit-test. A typo in a template variable fails the BUILD, not the page.

use askama::Template;

/// Data for the explorer landing page (`/`): a directory of agents.
#[derive(Template)]
#[template(path = "explorer.html")]
pub struct ExplorerPage {
    /// Ordering is EXPLICIT and shown to the reader ("newest first") — there
    /// is deliberately no ranking; ranking is a judgment.
    pub agents: Vec<AgentRow>,
}

/// One line in the directory table. All display-ready.
pub struct AgentRow {
    pub agent_id: i64,
    pub chain: String,
    pub domain: String,
    /// Whether the endpoint answered at the last observation — drives a status dot.
    pub is_alive: bool,
    /// e.g. "2026-05-03"
    pub registered: String,
    pub flag_count: i64,
}

/// Data for one agent's detail page (`/agent/{chain}/{id}`): facts + flags.
#[derive(Template)]
#[template(path = "agent.html")]
pub struct AgentDetailPage {
    pub agent_id: i64,
    pub chain: String,
    pub domain: String,
    pub address: String,
    pub facts: Vec<FactRow>,
    pub flags: Vec<FlagRow>,
}

/// One fact, pre-rendered: a label, a value sentence, and its evidence.
pub struct FactRow {
    pub label: String,
    pub value: String,
    pub evidence: String,
}

/// One flag, pre-rendered with its evidence summary.
pub struct FlagRow {
    pub label: String,
    pub detail: String,
    pub raised: String,
}

/// Data for the methodology write-up (`/methodology`). Static prose — the
/// facts and flags it describes are defined in the `facts` crate and the
/// enricher's flag producer.
#[derive(Template)]
#[template(path = "methodology.html")]
pub struct MethodologyPage {}
