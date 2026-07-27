//! Assemble the published facts for one agent: SQL aggregates in, evidence-
//! carrying `facts::Fact` values out. This is the ONLY place API queries meet
//! the pure facts crate — pages and JSON routes both call `assemble` so the
//! site can never disagree with the API.

use chrono::{Duration, Utc};
use serde::Serialize;
use sqlx::PgPool;

/// The probe window facts are computed over.
const LIVENESS_WINDOW_DAYS: i64 = 30;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AgentSummary {
    pub chain: String,
    pub agent_id: i64,
    pub domain: String,
    pub address: String,
    pub registered_at: chrono::DateTime<chrono::Utc>,
    pub endpoint_alive: bool,
    pub flag_count: i64,
}

/// One flag as published: the raw evidence, plus the words we say about it.
///
/// No `sqlx::FromRow` here any more — `display` is derived, not selected, so
/// the row is read into a private struct below and mapped into this one.
#[derive(Debug, Serialize)]
pub struct FlagView {
    pub kind: String,
    pub evidence: serde_json::Value,
    pub raised_at: chrono::DateTime<chrono::Utc>,
    pub display: facts::FlagDisplay,
}

#[derive(Debug, Serialize)]
pub struct AgentFacts {
    pub summary: AgentSummary,
    pub facts: Vec<facts::PublishedFact>,
    pub flags: Vec<FlagView>,
}

pub async fn assemble(
    pool: &PgPool,
    chain: &str,
    agent_id: i64,
) -> Result<Option<AgentFacts>, sqlx::Error> {
    // Identity + registration. Chain is ALWAYS part of the lookup — agent #7
    // on Base and agent #7 on Ethereum are different agents.
    #[derive(sqlx::FromRow)]
    struct AgentRow {
        chain: String,
        agent_id: i64,
        domain: String,
        address: String,
        registered_at: chrono::DateTime<chrono::Utc>,
        registered_tx: String,
        endpoint_alive: bool,
    }
    let Some(agent) = sqlx::query_as::<_, AgentRow>(
        "SELECT a.chain, a.agent_id, a.domain, a.address_norm AS address, \
                a.registered_at, a.registered_tx, \
                COALESCE(e.endpoint_healthy, false) AS endpoint_alive \
         FROM agents a \
         LEFT JOIN agent_enrichment e ON e.chain = a.chain AND e.agent_id = a.agent_id \
         WHERE a.chain = $1 AND a.agent_id = $2",
    )
    .bind(chain)
    .bind(agent_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };

    let now = Utc::now();
    let window_from = now - Duration::days(LIVENESS_WINDOW_DAYS);

    #[derive(sqlx::FromRow)]
    struct Probes {
        probes: i64,
        alive: i64,
        payment_required: i64,
    }
    let p = sqlx::query_as::<_, Probes>(
        "SELECT count(*) AS probes, \
                count(*) FILTER (WHERE outcome IN ('healthy','payment_required')) AS alive, \
                count(*) FILTER (WHERE outcome = 'payment_required') AS payment_required \
         FROM probe_history WHERE chain = $1 AND agent_id = $2 AND probed_at >= $3",
    )
    .bind(chain)
    .bind(agent_id)
    .bind(window_from)
    .fetch_one(pool)
    .await?;

    #[derive(sqlx::FromRow)]
    struct Snaps {
        total: i64,
        last_ok_at: Option<chrono::DateTime<chrono::Utc>>,
        last_ok_snapshot_id: Option<i64>,
        last_attempt_at: Option<chrono::DateTime<chrono::Utc>>,
    }
    let s = sqlx::query_as::<_, Snaps>(
        "SELECT count(*) AS total, \
                max(fetched_at) FILTER (WHERE body IS NOT NULL) AS last_ok_at, \
                (SELECT id FROM metadata_snapshots \
                 WHERE chain = $1 AND agent_id = $2 AND body IS NOT NULL \
                 ORDER BY fetched_at DESC LIMIT 1) AS last_ok_snapshot_id, \
                max(fetched_at) AS last_attempt_at \
         FROM metadata_snapshots WHERE chain = $1 AND agent_id = $2",
    )
    .bind(chain)
    .bind(agent_id)
    .fetch_one(pool)
    .await?;

    #[derive(sqlx::FromRow)]
    struct Attest {
        total: i64,
    }
    let a = sqlx::query_as::<_, Attest>(
        "SELECT count(*) AS total FROM feedback WHERE chain = $1 AND to_agent_id = $2",
    )
    .bind(chain)
    .bind(agent_id)
    .fetch_one(pool)
    .await?;

    #[derive(sqlx::FromRow)]
    struct Vals {
        registry_available: bool,
        passed: i64,
        failed: i64,
    }
    let v = sqlx::query_as::<_, Vals>(
        "SELECT COALESCE((SELECT validation_registry IS NOT NULL FROM chains WHERE chain = $1), false) AS registry_available, \
                count(*) FILTER (WHERE passed) AS passed, \
                count(*) FILTER (WHERE NOT passed) AS failed \
         FROM validations WHERE chain = $1 AND subject_id = $2",
    )
    .bind(chain)
    .bind(agent_id)
    .fetch_one(pool)
    .await?;

    #[derive(sqlx::FromRow)]
    struct FlagRowDb {
        kind: String,
        evidence: serde_json::Value,
        raised_at: chrono::DateTime<chrono::Utc>,
    }
    let flags: Vec<FlagView> = sqlx::query_as::<_, FlagRowDb>(
        "SELECT kind, evidence, raised_at FROM flags \
         WHERE chain = $1 AND agent_id = $2 ORDER BY raised_at DESC",
    )
    .bind(chain)
    .bind(agent_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| FlagView {
        display: facts::describe_flag(&r.kind, &r.evidence),
        kind: r.kind,
        evidence: r.evidence,
        raised_at: r.raised_at,
    })
    .collect();

    // SQL aggregates → pure derivations. The facts crate owns the phrasing.
    let probe_stats = facts::ProbeStats {
        from: window_from,
        to: now,
        probes: p.probes,
        alive: p.alive,
        payment_required: p.payment_required,
    };
    let mut fact_list = vec![
        facts::registered_since(&facts::Registration {
            chain: agent.chain.clone(),
            registered_at: agent.registered_at,
            tx_hash: agent.registered_tx.clone(),
        }),
        facts::endpoint_liveness(&probe_stats),
        facts::metadata_status(
            &facts::SnapshotStats {
                total: s.total,
                last_ok_at: s.last_ok_at,
                last_ok_snapshot_id: s.last_ok_snapshot_id,
                last_attempt_at: s.last_attempt_at,
            },
            now,
        ),
        facts::attestations(
            &facts::AttestationStats { total: a.total },
            &agent.chain,
            now,
        ),
        facts::validations(
            &facts::ValidationStats {
                registry_available: v.registry_available,
                passed: v.passed,
                failed: v.failed,
            },
            &agent.chain,
            now,
        ),
    ];
    if let Some(payable) = facts::payable_endpoint(&probe_stats) {
        fact_list.push(payable);
    }

    Ok(Some(AgentFacts {
        summary: AgentSummary {
            chain: agent.chain,
            agent_id: agent.agent_id,
            domain: agent.domain,
            address: agent.address,
            registered_at: agent.registered_at,
            endpoint_alive: agent.endpoint_alive,
            flag_count: flags.len() as i64,
        },
        facts: fact_list
            .into_iter()
            .map(facts::PublishedFact::new)
            .collect(),
        flags,
    }))
}

/// The orderings the directory offers. An enum, not a free string, because
/// the variant names are what get interpolated into SQL — user input can
/// never reach the query text.
///
/// Every ordering ends in `a.agent_id DESC` so it is TOTAL. Without that
/// tiebreaker two agents sharing a `registered_at` can swap places between
/// two paged queries, which shows one of them twice and hides the other.
#[derive(Debug, Clone, Copy)]
pub enum Sort {
    Registered,
    Alive,
}

impl Sort {
    pub fn from_param(s: Option<&str>) -> Self {
        match s {
            Some("alive") => Sort::Alive,
            _ => Sort::Registered,
        }
    }

    pub fn order_by(&self) -> &'static str {
        match self {
            Sort::Registered => "a.registered_at DESC, a.agent_id DESC",
            Sort::Alive => "endpoint_alive DESC, a.registered_at DESC, a.agent_id DESC",
        }
    }
}

/// What to list. Built by the JSON route from query params and by the HTML
/// page from constants — one query serves both.
#[derive(Debug)]
pub struct ListFilter {
    pub chain: Option<String>,
    pub limit: i64,
    pub offset: i64,
    pub sort: Sort,
}

/// Where this page sits in the whole result set. `total` is what lets a UI
/// render "page 3 of 15" instead of guessing whether a next page exists.
#[derive(Debug, Serialize)]
pub struct PageMeta {
    pub limit: i64,
    pub offset: i64,
    pub total: i64,
}

#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub page: PageMeta,
}

/// The agent directory, paginated. The ONLY agent-list query in the codebase:
/// the JSON route and the HTML explorer both call it, so they cannot drift.
pub async fn list_agents(
    pool: &PgPool,
    filter: &ListFilter,
) -> Result<Page<AgentSummary>, sqlx::Error> {
    // `order` is interpolated, but only from `Sort`'s fixed arms above.
    let sql = format!(
        "SELECT a.chain, a.agent_id, a.domain, a.address_norm AS address, a.registered_at, \
                COALESCE(e.endpoint_healthy, false) AS endpoint_alive, \
                COALESCE(fl.n, 0) AS flag_count \
         FROM agents a \
         LEFT JOIN agent_enrichment e ON e.chain = a.chain AND e.agent_id = a.agent_id \
         LEFT JOIN (SELECT chain, agent_id, count(*) AS n FROM flags GROUP BY chain, agent_id) fl \
                ON fl.chain = a.chain AND fl.agent_id = a.agent_id \
         WHERE ($2::text IS NULL OR a.chain = $2) \
         ORDER BY {} \
         LIMIT $1 OFFSET $3",
        filter.sort.order_by()
    );
    let items = sqlx::query_as::<_, AgentSummary>(&sql)
        .bind(filter.limit)
        .bind(&filter.chain)
        .bind(filter.offset)
        .fetch_all(pool)
        .await?;

    // A separate count, deliberately — `count(*) OVER ()` would ride along on
    // the rows, and so would vanish on an empty page. An offset past the end
    // is exactly when a UI most needs the total.
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agents a WHERE ($1::text IS NULL OR a.chain = $1)",
    )
    .bind(&filter.chain)
    .fetch_one(pool)
    .await?;

    Ok(Page {
        items,
        page: PageMeta {
            limit: filter.limit,
            offset: filter.offset,
            total,
        },
    })
}
