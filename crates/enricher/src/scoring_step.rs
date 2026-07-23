//! The batch scoring step: assemble each agent's view and run the pure scorer.
//!
//! The `scoring` crate is a pure function — it has no idea Postgres exists. This
//! module is the *bridge*: it reads rows from the database, packs them into a
//! [`scoring::AgentView`], calls [`scoring::score`], and writes the result to the
//! `scores` table for the API to serve. All the I/O lives here; all the maths
//! lives over in `scoring`. That clean split is the whole reason scoring is its
//! own crate.
//!
//! Rust concept spotlight: **assembling data across a boundary.** We load two
//! flat result sets (per-agent aggregates and the raw feedback edges), then build
//! the richer nested `AgentView` in memory with plain `HashMap`s and iterators —
//! no ORM magic, just data transformation you can read top to bottom.

use std::collections::{HashMap, HashSet};

use anyhow::Result;

use crate::flags::AgentKey;
use crate::store::Db;

/// Assume feedback scores are on a 0..=5 scale; normalise to the `[0, 1]` the
/// scorer expects. The real divisor depends on the ERC-8004 reputation schema —
/// change this one constant when you know it.
const FEEDBACK_SCALE_MAX: f64 = 5.0;

/// Score every agent and persist the results. Returns how many were scored.
pub async fn score_all(db: &Db) -> Result<usize> {
    // Two flat loads from Postgres:
    let inputs = db.load_score_inputs().await?; // one aggregate row per agent
    let feedback = db.load_feedback_rows().await?; // every feedback edge

    // Index 1: directed feedback edges, for reciprocity checks. `(chain, from, to)`.
    let directed: HashSet<(String, i64, i64)> = feedback
        .iter()
        .map(|f| (f.chain.clone(), f.from_agent_id, f.to_agent_id))
        .collect();

    // Index 2: incoming feedback per agent — `(chain, to) -> [(from, score)]`.
    let mut incoming: HashMap<(String, i64), Vec<(i64, i16)>> = HashMap::new();
    for f in &feedback {
        incoming
            .entry((f.chain.clone(), f.to_agent_id))
            .or_default()
            .push((f.from_agent_id, f.score));
    }

    // Index 3: each agent's suspicion, used as a proxy for how much its opinion
    // of others should count (a suspicious attester's praise is worth less).
    let suspicion: HashMap<(String, i64), f64> = inputs
        .iter()
        .map(|r| ((r.chain.clone(), r.agent_id), r.suspicion))
        .collect();

    let mut scored = Vec::with_capacity(inputs.len());

    for row in &inputs {
        // Build the feedback edges pointing at this agent.
        let edges = incoming
            .get(&(row.chain.clone(), row.agent_id))
            .map(|list| {
                list.iter()
                    .map(|(from, score)| scoring::FeedbackEdge {
                        from_agent_id: *from as u64,
                        raw_value: (*score as f64 / FEEDBACK_SCALE_MAX).clamp(0.0, 1.0),
                        // Attester weight proxy: 1.0 for a clean attester, down to
                        // 0.0 for a fully-suspicious one. A real system would use
                        // the attester's own computed trust (EigenTrust).
                        attester_weight: 1.0
                            - suspicion
                                .get(&(row.chain.clone(), *from))
                                .copied()
                                .unwrap_or(0.0),
                        // Reciprocal iff this agent also rated the attester.
                        is_reciprocal: directed.contains(&(
                            row.chain.clone(),
                            row.agent_id,
                            *from,
                        )),
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Assemble the view the scorer expects. Every `as u64/u32` is an explicit
        // narrowing from Postgres's signed integers — safe here because counts and
        // ids are non-negative.
        let view = scoring::AgentView {
            agent_id: row.agent_id as u64,
            chain: row.chain.clone(),
            distinct_counterparties: row.distinct_counterparties as u64,
            total_payment_value: row.total_payment_value,
            probe_count: row.probe_count as u32,
            probe_successes: row.probe_successes as u32,
            first_seen: row.first_seen,
            last_activity: row.last_activity,
            active_days: row.active_days as u32,
            incoming_feedback: edges,
            cluster: scoring::ClusterInfo {
                cluster_id: None,
                cluster_size: row.cluster_size as u32,
                suspicion: row.suspicion,
            },
        };

        // The pure call. `?` bubbles a scoring error up as an anyhow error.
        let score = scoring::score(&view)?;
        scored.push((
            AgentKey {
                chain: row.chain.clone(),
                agent_id: row.agent_id,
            },
            score,
        ));
    }

    let n = scored.len();
    db.write_scores(&scored).await?;
    Ok(n)
}
