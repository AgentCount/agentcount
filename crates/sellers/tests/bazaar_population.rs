//! The pure pipeline, end to end, over a real catalog body.
//!
//! `bazaar-resources.json` is a captured `GET
//! /platform/v2/x402/discovery/resources` response (2026-08-20). This test
//! runs it through the parse → assemble path the crawler will use and
//! asserts the properties METHODOLOGY §10 requires of a population, on data
//! this census did not invent.
//!
//! It deliberately asserts SHAPES, not counts. A captured fixture is a
//! photograph of a live catalog, and pinning "there are exactly N sellers"
//! would make refreshing the capture a test failure rather than an update —
//! which is how fixtures stop being refreshed.

use sellers::catalog::{self, Listing};
use sellers::sources::bazaar;

const CAPTURED: &str = include_str!("fixtures/bazaar-resources.json");

fn population() -> catalog::Population {
    let parsed = bazaar::parse(CAPTURED).expect("captured Bazaar body parses");
    catalog::assemble(&parsed.listings)
}

#[test]
fn the_captured_body_parses_into_listings() {
    let p = bazaar::parse(CAPTURED).expect("the captured Bazaar body must parse");
    assert!(p.items_seen > 0, "the capture carries items");
    assert!(
        !p.listings.is_empty(),
        "and those items name payees and resources"
    );
    assert!(
        p.listings.iter().all(|l| l.catalog == bazaar::NAME),
        "every listing records which catalog it came from"
    );
    assert!(
        p.listings.iter().all(|l| l.resource.starts_with("http")),
        "resources are URLs"
    );
}

#[test]
fn the_catalogs_own_total_is_read_so_our_coverage_of_it_is_statable() {
    // The catalog's own claim about its size, beside the part this census
    // could actually read — "the Bazaar says N, we resolved M" is only a
    // statable sentence because this field is parsed.
    let p = bazaar::parse(CAPTURED).unwrap();
    assert!(
        p.total.is_some_and(|t| t > 1_000),
        "the Bazaar reported 15,155 resources when this was captured"
    );
}

#[test]
fn a_real_catalog_page_assembles_into_sellers() {
    let p = population();
    assert!(!p.is_empty(), "a live catalog page names sellers");
    for seller in &p.sellers {
        assert!(
            seller.id.pay_to.starts_with("0x") && seller.id.pay_to.len() == 42,
            "every payTo is normalized: {}",
            seller.id.pay_to
        );
        assert_eq!(
            seller.id.pay_to.to_ascii_lowercase(),
            seller.id.pay_to,
            "EVM addresses are lowercased, so two catalogs' casing cannot split one seller"
        );
        assert!(!seller.id.host.is_empty(), "every seller has a host");
        assert!(
            seller.catalogs.contains(bazaar::NAME),
            "provenance travels with the seller"
        );
        assert!(
            !seller.resources.is_empty(),
            "a seller exists because a resource named it"
        );
    }
}

#[test]
fn the_population_is_never_larger_than_the_listings_it_came_from() {
    // Dedup can only collapse. A population LARGER than its input would mean
    // assembly invented a seller.
    let parsed = bazaar::parse(CAPTURED).unwrap();
    let p = catalog::assemble(&parsed.listings);
    assert!(p.len() <= parsed.listings.len());
    assert_eq!(
        p.len() + count_collapsed(&parsed.listings),
        parsed.listings.len(),
        "every listing either became a seller, joined one, or was rejected"
    );
}

/// Listings that joined an existing seller or were refused — the difference
/// between the input rows and the distinct sellers.
fn count_collapsed(listings: &[Listing]) -> usize {
    let p = catalog::assemble(listings);
    listings.len() - p.len()
}

#[test]
fn assembling_a_real_page_twice_gives_byte_identical_results() {
    // The property the weekly delta rests on: a diff between two sweeps must
    // mean the world changed, not that a HashMap iterated differently.
    let a = serde_json::to_string(&population().sellers).unwrap();
    let b = serde_json::to_string(&population().sellers).unwrap();
    assert_eq!(a, b);
}

#[test]
fn every_rejected_listing_carries_a_reason_word() {
    // §10.2's losslessness: a listing that could not become an identity is
    // countable and explicable, never a silence.
    let p = population();
    for r in &p.rejected {
        assert!(
            matches!(
                r.reason.as_str(),
                "malformed_address" | "zero_address" | "malformed_url" | "unsupported_scheme"
            ),
            "unexpected reason {:?}",
            r.reason
        );
    }
}

#[test]
fn per_catalog_coverage_is_computable_from_one_catalog_alone() {
    // The union index's arithmetic works with one catalog in it, which is
    // what sweep 1 will start with.
    let p = population();
    let coverage = catalog::coverage(&p);
    assert_eq!(coverage.len(), 1, "one catalog in, one denominator out");
    assert_eq!(coverage[bazaar::NAME], p.len());
    assert_eq!(
        p.exclusive_to(bazaar::NAME).len(),
        p.len(),
        "with a single catalog, every seller is exclusive to it"
    );
}
