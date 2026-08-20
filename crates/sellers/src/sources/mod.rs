//! Catalog adapters — one per named catalog in METHODOLOGY §10.2.
//!
//! Each adapter turns one catalog's response body into
//! [`crate::catalog::Listing`]s and nothing else: no fetching (that belongs
//! to the crawler binary), no judgement, no dedup. Dedup across catalogs is
//! [`crate::catalog::assemble`]'s job precisely so that adding a catalog
//! cannot change how the union is computed.
//!
//! **Every adapter is written against a CAPTURED response body**, kept in
//! `tests/fixtures/`, not against the catalog's documentation. The two
//! differ: the Bazaar's reference says networks are named `base` and every
//! live entry says `eip155:8453`, which is the difference between measuring
//! the ecosystem and measuring nothing (see [`crate::network`]). Refreshing
//! a fixture is how its adapter stays honest.
//!
//! The catalog list is part of the method — adding one here changes the
//! population and is a methodology-changelog event, not a code change.

pub mod bazaar;
