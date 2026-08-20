//! CDP Bazaar — `GET /platform/v2/x402/discovery/resources`.
//!
//! The facilitator's own discovery index, unauthenticated, and the largest
//! single catalog in the ecosystem: 15,155 resources when this parser was
//! written against it (2026-08-20).
//!
//! The shape below is TRANSCRIBED FROM A CAPTURED RESPONSE, not from the
//! documentation, because the two differ in ways that matter:
//!
//! * the API reference documents `accepts[].amount`, and the live body also
//!   carries `currency` and `recipient` as aliases of `asset` and `payTo`;
//! * the reference's examples name networks `base`, and live entries name
//!   CAIP-2 ids — `eip155:8453` and eight or more others, since the Bazaar
//!   is not a Base-only catalog (see [`crate::network`], and the near miss
//!   it records);
//! * `x402Version` is 2 on the live index, where most written examples show
//!   1.
//!
//! `tests/fixtures/bazaar-resources.json` is the captured body this parser
//! is tested against, and refreshing it is how this parser stays honest —
//! the same rule the web repo's fixtures follow. A parser tested only
//! against what the docs say would pass forever while measuring nothing.

use crate::catalog::Listing;

/// The catalog name recorded on every listing and seller row from here.
pub const NAME: &str = "bazaar";

/// Parse a Bazaar discovery body into listings.
///
/// One listing per (resource, distinct payTo): a resource offering two
/// payment schemes to the same payee is one seller's one resource, while a
/// resource naming two different payees genuinely belongs to two sellers by
/// §10.1's unit — and this is where that becomes two rows rather than a
/// judgement call later.
///
/// Entries this parser cannot read at all are counted, never dropped: the
/// return carries how many items were seen and how many yielded nothing, so
/// a catalog whose shape changes under us is visible as a number rather than
/// as a quietly smaller population.
pub fn parse(body: &str) -> Result<Parsed, ParseError> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| ParseError::NotJson)?;
    let items = value
        .get("items")
        .and_then(|i| i.as_array())
        .ok_or(ParseError::NoItems)?;

    let mut listings = Vec::new();
    let mut unreadable = 0usize;
    for item in items {
        let Some(resource) = item.get("resource").and_then(|r| r.as_str()) else {
            unreadable += 1;
            continue;
        };
        let accepts = item.get("accepts").and_then(|a| a.as_array());
        // (payee, network) rather than payee alone: the network decides
        // which encoding the address is read with, and one resource can be
        // sold on more than one chain.
        let mut payees: Vec<(&str, &str)> = accepts
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|e| {
                        // `payTo` is the documented field; `recipient` is the
                        // alias the live body also carries. Reading both
                        // means an entry that drops one still names a payee.
                        let pay_to = e
                            .get("payTo")
                            .or_else(|| e.get("recipient"))
                            .and_then(|p| p.as_str())?;
                        let network = e.get("network").and_then(|n| n.as_str()).unwrap_or("");
                        Some((pay_to, network))
                    })
                    .collect()
            })
            .unwrap_or_default();
        payees.sort_unstable();
        payees.dedup();

        if payees.is_empty() {
            // A listing with no payee names no seller. It is not an error —
            // the catalog may simply carry a free or unpriced entry — but it
            // is counted, because "the Bazaar lists N resources and M of
            // them name nobody to pay" is a finding.
            unreadable += 1;
            continue;
        }
        for (pay_to, network) in payees {
            listings.push(Listing {
                catalog: NAME.to_string(),
                pay_to: pay_to.to_string(),
                resource: resource.to_string(),
                network: network.to_string(),
            });
        }
    }

    Ok(Parsed {
        listings,
        items_seen: items.len(),
        items_unreadable: unreadable,
        total: value
            .get("pagination")
            .and_then(|p| p.get("total"))
            .and_then(|t| t.as_u64()),
    })
}

/// What one Bazaar page yielded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub listings: Vec<Listing>,
    pub items_seen: usize,
    /// Items that named no resource or no payee. Counted so a shape change
    /// upstream shows up as a number rather than as a smaller population.
    pub items_unreadable: usize,
    /// The catalog's own claim about how many resources it has — the
    /// denominator this census reports the catalog as having, beside the one
    /// it could actually read.
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    NotJson,
    /// No `items` array. The shape changed, and that is a fact about the
    /// catalog worth recording as `error` rather than as an empty page.
    NoItems,
}

#[cfg(test)]
mod tests {
    use super::*;

    // The tests that read the captured Bazaar body live in
    // `tests/bazaar_population.rs`, not here: this crate's `src/` may not
    // reach the filesystem, not even at compile time to embed a fixture,
    // and CI enforces that for every crate claiming purity. (It caught this
    // exact file when the fixture was embedded here, which is the job
    // working.) The tests below need no fixture.

    #[test]
    fn a_resource_naming_two_payees_becomes_two_listings() {
        // §10.1's unit, applied at the source: two payees behind one URL are
        // two sellers, and this is where that stops being a judgement call.
        let body = r#"{"items":[{"resource":"https://a.example/x","accepts":[
            {"payTo":"0xaaa","scheme":"exact"},{"payTo":"0xbbb","scheme":"exact"}]}]}"#;
        let p = parse(body).unwrap();
        assert_eq!(p.listings.len(), 2);
    }

    #[test]
    fn one_payee_offering_two_schemes_is_still_one_listing() {
        // The live shape: `exact` and `batch-settlement` for the same payee.
        // Two ways to pay one seller is not two sellers.
        let body = r#"{"items":[{"resource":"https://a.example/x","accepts":[
            {"payTo":"0xaaa","scheme":"exact"},{"payTo":"0xaaa","scheme":"batch-settlement"}]}]}"#;
        let p = parse(body).unwrap();
        assert_eq!(p.listings.len(), 1);
    }

    #[test]
    fn the_recipient_alias_is_read_when_pay_to_is_absent() {
        let body = r#"{"items":[{"resource":"https://a.example/x","accepts":[
            {"recipient":"0xaaa","scheme":"exact"}]}]}"#;
        assert_eq!(parse(body).unwrap().listings[0].pay_to, "0xaaa");
    }

    #[test]
    fn items_that_name_nobody_are_counted_not_dropped() {
        let body = r#"{"items":[
            {"resource":"https://a.example/x","accepts":[{"payTo":"0xaaa"}]},
            {"resource":"https://b.example/x","accepts":[]},
            {"description":"no resource at all"}]}"#;
        let p = parse(body).unwrap();
        assert_eq!(p.listings.len(), 1);
        assert_eq!(p.items_seen, 3);
        assert_eq!(p.items_unreadable, 2, "the loss is a number, not a silence");
    }

    #[test]
    fn a_shape_change_upstream_is_an_error_not_an_empty_page() {
        // "The catalog served us something we do not understand" and "the
        // catalog has no resources" must never be the same row.
        assert_eq!(parse("<html/>"), Err(ParseError::NotJson));
        assert_eq!(parse(r#"{"data":[]}"#), Err(ParseError::NoItems));
        assert_eq!(parse(r#"{"items":[]}"#).unwrap().listings.len(), 0);
    }
}
