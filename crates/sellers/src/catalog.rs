//! The population: turning many catalogs' listings into one set of sellers.
//!
//! METHODOLOGY §10.2. Every catalog is partial and nobody publishes the
//! union, so the union is the thing worth building — and the only way it is
//! worth anything is if assembling it is deterministic and lossless:
//!
//! * **Deterministic** — the same listings in any order produce the same
//!   sellers in the same order, so a diff between two sweeps means the world
//!   changed. (The same reason `sweeper::delta` sorts its flips.)
//! * **Lossless** — a listing this census cannot turn into an identity is
//!   COUNTED and reported, never dropped. A population assembled by quietly
//!   discarding what did not parse is a population nobody can check.
//!
//! Which catalogs list a seller travels with the seller, because the
//! cross-reference — who is in Bazaar but nowhere else, who appears in five
//! indexes — is a finding this instrument can produce and nobody else
//! currently can.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::identity::{Network, SellerId};

/// One row as a catalog published it, before this census has judged whether
/// it names a seller at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    /// Which catalog this came from — the provenance that makes the union
    /// index worth having.
    pub catalog: String,
    pub pay_to: String,
    /// The priced URL. Its host becomes half the seller's identity, and the
    /// URL itself is kept as one of the seller's resources.
    pub resource: String,
    /// The network the catalog said this listing settles on, VERBATIM.
    ///
    /// Not part of the identity (§10.1's unit is `(payTo, host)`), but
    /// carried because it decides whether this census can read the address
    /// at all — and because dropping it caused a real mislabel. About half
    /// the Bazaar's listings settle somewhere other than Base (roughly a
    /// fifth on Solana), and assembling those under EVM rules recorded 138
    /// of 443 perfectly good listings as `malformed_address`. A listing on a
    /// network this sweep does not cover is out of scope, which is a fact
    /// about OUR coverage; calling it malformed is a false claim about
    /// somebody else's catalog.
    pub network: String,
}

/// One seller, assembled from every listing that resolved to its identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seller {
    pub id: SellerId,
    /// Sorted and deduped: a catalog listing the same seller twice is one
    /// mention of it, not two.
    pub catalogs: BTreeSet<String>,
    /// Every priced URL seen for this seller, sorted. Sellers and resources
    /// are both published; neither stands in for the other.
    pub resources: BTreeSet<String>,
}

/// A listing that could not become an identity, kept so the loss is
/// countable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejected {
    pub listing: Listing,
    /// The `IdentityError`, as the word that goes on the row.
    pub reason: String,
}

/// What one sweep's catalog pass produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Population {
    /// Sorted by identity, so two assemblies of the same listings are
    /// byte-identical.
    pub sellers: Vec<Seller>,
    pub rejected: Vec<Rejected>,
}

impl Population {
    /// How many distinct sellers — the denominator every rate in this
    /// instrument is stated over.
    pub fn len(&self) -> usize {
        self.sellers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sellers.is_empty()
    }

    /// Sellers listed by exactly one catalog, and which. The union index's
    /// first finding: how much of the economy any single catalog misses.
    pub fn exclusive_to(&self, catalog: &str) -> Vec<&Seller> {
        self.sellers
            .iter()
            .filter(|s| s.catalogs.len() == 1 && s.catalogs.contains(catalog))
            .collect()
    }
}

/// Split listings into the ones this sweep can read and the ones it cannot.
///
/// "Cannot read" means the network is not in `in_scope` — a real listing on
/// a chain this census does not sweep. It is returned separately rather than
/// rejected, because [`assemble`] would judge its address by the wrong
/// encoding and record a valid Solana payee as a malformed EVM one. Coverage
/// this census lacks is never somebody else's malformity.
pub fn partition_by_scope<'a>(
    listings: &'a [Listing],
    in_scope: &[&str],
) -> (Vec<&'a Listing>, Vec<&'a Listing>) {
    listings
        .iter()
        .partition(|l| in_scope.iter().any(|s| crate::network::same(s, &l.network)))
}

/// Assemble a population from every catalog's listings.
///
/// `network` is the ENCODING to read addresses with, not a filter: callers
/// pass only listings they have already scoped (see [`partition_by_scope`]),
/// because assembling a Solana listing under EVM rules mislabels it.
pub fn assemble(listings: &[Listing], network: Network) -> Population {
    let mut by_id: BTreeMap<SellerId, Seller> = BTreeMap::new();
    let mut rejected = Vec::new();

    for listing in listings {
        match SellerId::new(&listing.pay_to, &listing.resource, network) {
            Ok(id) => {
                let entry = by_id.entry(id.clone()).or_insert_with(|| Seller {
                    id,
                    catalogs: BTreeSet::new(),
                    resources: BTreeSet::new(),
                });
                entry.catalogs.insert(listing.catalog.clone());
                entry.resources.insert(listing.resource.clone());
            }
            Err(e) => rejected.push(Rejected {
                listing: listing.clone(),
                reason: e.to_string(),
            }),
        }
    }

    Population {
        sellers: by_id.into_values().collect(),
        rejected,
    }
}

/// How many sellers each catalog lists, sorted by catalog name — the
/// per-catalog denominators, so "Bazaar lists N of the M sellers that exist"
/// is a countable claim.
pub fn coverage(population: &Population) -> BTreeMap<String, usize> {
    let mut out: BTreeMap<String, usize> = BTreeMap::new();
    for seller in &population.sellers {
        for catalog in &seller.catalogs {
            *out.entry(catalog.clone()).or_insert(0) += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(catalog: &str, pay_to: &str, resource: &str) -> Listing {
        listing_on(catalog, pay_to, resource, crate::network::BASE)
    }

    fn listing_on(catalog: &str, pay_to: &str, resource: &str, network: &str) -> Listing {
        Listing {
            catalog: catalog.into(),
            pay_to: pay_to.into(),
            resource: resource.into(),
            network: network.into(),
        }
    }

    const A: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn two_catalogs_listing_one_seller_produce_one_seller_with_both_names() {
        // The union index, in miniature: the whole point of enumerating from
        // every catalog is that this collapses to one row carrying both.
        let p = assemble(
            &[
                listing("bazaar", A, "https://api.example.com/weather"),
                listing("x402scan", A, "https://api.example.com/weather"),
            ],
            Network::Evm,
        );
        assert_eq!(p.len(), 1);
        assert_eq!(
            p.sellers[0].catalogs.iter().cloned().collect::<Vec<_>>(),
            ["bazaar", "x402scan"]
        );
    }

    #[test]
    fn one_catalog_listing_a_seller_twice_is_one_mention() {
        let p = assemble(
            &[
                listing("bazaar", A, "https://api.example.com/weather"),
                listing("bazaar", A, "https://api.example.com/weather"),
            ],
            Network::Evm,
        );
        assert_eq!(p.len(), 1);
        assert_eq!(p.sellers[0].catalogs.len(), 1);
        assert_eq!(p.sellers[0].resources.len(), 1);
    }

    #[test]
    fn one_sellers_several_resources_all_hang_off_the_one_identity() {
        let p = assemble(
            &[
                listing("bazaar", A, "https://api.example.com/weather"),
                listing("bazaar", A, "https://api.example.com/tides"),
            ],
            Network::Evm,
        );
        assert_eq!(p.len(), 1, "same payTo, same host — one seller");
        assert_eq!(p.sellers[0].resources.len(), 2);
    }

    #[test]
    fn the_same_pay_to_on_two_hosts_stays_two_sellers_through_assembly() {
        let p = assemble(
            &[
                listing("bazaar", A, "https://one.example.com/x"),
                listing("bazaar", A, "https://two.example.com/x"),
            ],
            Network::Evm,
        );
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn assembly_is_deterministic_regardless_of_the_order_listings_arrive() {
        let forward = [
            listing("bazaar", B, "https://z.example.com/x"),
            listing("atlas", A, "https://a.example.com/x"),
            listing("x402scan", A, "https://a.example.com/x"),
        ];
        let mut backward = forward.clone();
        backward.reverse();
        assert_eq!(
            assemble(&forward, Network::Evm),
            assemble(&backward, Network::Evm),
            "a diff between two sweeps must mean the world changed"
        );
    }

    #[test]
    fn a_listing_that_cannot_become_an_identity_is_counted_never_dropped() {
        let p = assemble(
            &[
                listing("bazaar", A, "https://api.example.com/x"),
                listing("bazaar", "0x0", "https://api.example.com/x"),
                listing("bazaar", A, "not a url"),
                listing(
                    "bazaar",
                    "0x0000000000000000000000000000000000000000",
                    "https://api.example.com/x",
                ),
            ],
            Network::Evm,
        );
        assert_eq!(p.len(), 1);
        assert_eq!(p.rejected.len(), 3, "{:?}", p.rejected);
        let reasons: Vec<&str> = p.rejected.iter().map(|r| r.reason.as_str()).collect();
        assert!(reasons.contains(&"malformed_address"));
        assert!(reasons.contains(&"malformed_url"));
        assert!(reasons.contains(&"zero_address"));
    }

    #[test]
    fn a_listing_on_a_chain_we_do_not_sweep_is_out_of_scope_not_malformed() {
        // THE mislabel, as a regression test. About half the Bazaar's
        // listings settle off Base, a fifth of them on Solana; read under
        // EVM rules they were recorded as `malformed_address` — 138 of 443
        // on the first three pages. They are not malformed. They are
        // somebody else's perfectly good listing on a chain this sweep does
        // not cover, and the difference is a false claim about another
        // project's data.
        let solana = "7dHbWXmci3dT8UFYWYZweBLXgycu7Y3iL6trKn1Y7ARj";
        let listings = [
            listing("bazaar", A, "https://a.example/x"),
            listing_on("bazaar", solana, "https://b.example/x", "solana"),
        ];
        let (in_scope, out_of_scope) = partition_by_scope(&listings, &[crate::network::BASE]);
        assert_eq!(in_scope.len(), 1);
        assert_eq!(out_of_scope.len(), 1);

        let scoped: Vec<Listing> = in_scope.into_iter().cloned().collect();
        let p = assemble(&scoped, Network::Evm);
        assert_eq!(p.len(), 1);
        assert!(
            p.rejected.is_empty(),
            "nothing valid was called malformed: {:?}",
            p.rejected
        );
    }

    #[test]
    fn scope_matching_uses_canonical_network_names() {
        // `base` and `eip155:8453` are one network; a catalog writing either
        // is in scope.
        let listings = [
            listing_on("bazaar", A, "https://a.example/x", "base"),
            listing_on("bazaar", B, "https://b.example/x", "eip155:8453"),
        ];
        let (in_scope, out) = partition_by_scope(&listings, &[crate::network::BASE]);
        assert_eq!(in_scope.len(), 2);
        assert!(out.is_empty());
    }

    #[test]
    fn per_catalog_coverage_counts_the_sellers_each_one_knows_about() {
        let p = assemble(
            &[
                listing("bazaar", A, "https://a.example.com/x"),
                listing("bazaar", B, "https://b.example.com/x"),
                listing("atlas", A, "https://a.example.com/x"),
            ],
            Network::Evm,
        );
        let c = coverage(&p);
        assert_eq!(c["bazaar"], 2);
        assert_eq!(c["atlas"], 1);
        // ...and the finding that needs the union to exist at all.
        assert_eq!(p.exclusive_to("bazaar").len(), 1);
        assert!(p.exclusive_to("atlas").is_empty());
    }
}
