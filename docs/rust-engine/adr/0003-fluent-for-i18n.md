# ADR-0003: fluent-rs for i18n, with the Qt Linguist catalogue converted

**Status:** Accepted · **Date:** 2026-09-12 · Relates to: PLAN.md §2.1

## Context

The Rust engine spec does not mention internationalisation at all. Compass has a live Qt Linguist
catalogue in `src/server/translations/*.ts` and strict i18n rules in `AGENTS.md`. Contributors have
donated those translations.

## Decision

**`fluent-rs`**, with a build-time string extractor, plus a **one-off `.ts` → `.ftl` converter** so
the existing catalogue survives the migration.

## Why

- Fluent handles plurals, gender and per-locale grammatical variation properly, which `gettext`-style
  `%1`-substitution does not. Compass's rules already forbid concatenating translated fragments;
  Fluent makes that the natural way to write.
- Losing the catalogue would be a visible regression for every non-English user, and an insult to
  the people who wrote it. The converter is not optional scope.

## Costs

Fluent's syntax is unfamiliar to contributors who know Qt Linguist, and the tooling ecosystem is
smaller. Converted strings will need review, since `.ts` context metadata does not map cleanly onto
Fluent message ids.

## Consequences

CI checks catalogue coverage, so a new user-visible string without a key fails the build rather than
silently shipping untranslated.
