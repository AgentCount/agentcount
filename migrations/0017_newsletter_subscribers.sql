-- ─────────────────────────────────────────────────────────────────────────────
-- Migration 0017 — the newsletter list, kept here rather than at a provider.
-- ─────────────────────────────────────────────────────────────────────────────
--
-- The decision this encodes: find out whether anyone wants the reports before
-- paying for and configuring a sending platform. That is sound, and it has one
-- consequence that has to be designed for rather than discovered later —
-- **these addresses are not confirmed.**
--
-- Nothing here sends mail, so nothing here can run a double opt-in. Anyone can
-- type anyone else's address into the form. That is fine for counting demand
-- and NOT fine for sending to, so `confirmed_at` exists from the start and is
-- NULL for every row this endpoint writes. Whoever eventually sends the first
-- report has to confirm first, and the column is there so that step cannot be
-- skipped by accident. Importing an unconfirmed list into a sending platform
-- is also how people lose the platform account.
--
-- ## Personal data, stated plainly
--
-- An email address is personal data, and the operator is an EU company. Three
-- consequences are built into the shape of this table rather than left to
-- policy:
--
--   * **Consent is the basis, and it is per-address.** A row exists because
--     somebody typed their address into a form that said what it was for.
--   * **No IP address, no user agent, no referrer.** They would be a second
--     category of personal data collected for no stated purpose, and "we might
--     want it later" is not a purpose. Rate limiting uses the requesting IP in
--     memory and never writes it.
--   * **Deletion is a DELETE.** `unsubscribed_at` marks someone who asked to
--     stop hearing from us and whose address is kept only so a later import
--     cannot silently re-add them. Someone who asks to be forgotten gets the
--     row removed, and the table is small enough that this needs no tooling
--     beyond one statement.
CREATE TABLE IF NOT EXISTS newsletter_subscribers (
    -- Lowercased before insert. Not `citext`: that needs an extension, and one
    -- `lower()` at the boundary is less machinery than a type nobody expects.
    -- The address a person typed is not preserved verbatim — `Filip@…` and
    -- `filip@…` are one subscriber, and treating them as two would mail
    -- somebody twice.
    email           text        PRIMARY KEY,
    subscribed_at   timestamptz NOT NULL DEFAULT now(),
    -- NULL until a confirmation link is actually clicked. Nothing writes this
    -- yet, because nothing sends mail yet. See the note above.
    confirmed_at    timestamptz,
    unsubscribed_at timestamptz,
    -- Which page the form was on. The only non-address column, and it is about
    -- the SITE, not the person: it answers "does the report page convert
    -- better than the homepage", which is the question that decides whether
    -- this experiment is worth continuing.
    source          text
);

-- "Everyone who should receive the next report" is the only query this table
-- has, and it is a small minority of rows once the list grows.
CREATE INDEX IF NOT EXISTS newsletter_subscribers_sendable_idx
    ON newsletter_subscribers (subscribed_at)
    WHERE unsubscribed_at IS NULL;

COMMENT ON COLUMN newsletter_subscribers.confirmed_at IS
    'NULL means the address was typed into a form and never confirmed by its owner. Do not send to a NULL row.';
