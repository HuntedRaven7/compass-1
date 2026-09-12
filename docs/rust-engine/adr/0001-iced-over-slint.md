# ADR-0001: Iced for the UI, not Slint

**Status:** Accepted · **Date:** 2026-09-12 · Relates to: PLAN.md §0, §2

## Context

The Rust engine spec lists "slint / iced" without choosing. We need one.

## Decision

**Iced 0.14.**

## Why

- rustcast, our seed, is already a working Iced 0.14 launcher. Choosing Slint throws away the one
  part of the seed with real value and replaces a working shell with a rewrite.
- libcosmic is the largest production Iced deployment in existence — an entire desktop environment,
  including its own Wayland shell integration. That is strong evidence Iced carries a launcher, and
  a place to look when Iced fights us.
- Pure Rust, no separate markup language or compiler in the build, no licence question.

## Costs, stated plainly

Iced is weaker than Qt/QML on the things a mature desktop app needs: IME, accessibility, RTL, and
font shaping edge cases. Compass ships those today via Qt. This is the single largest quality risk
in the whole migration.

## What would change our mind

Phase 1 must exercise the real theme set and **test IME (`text-input-v3`) and screen-reader
behaviour explicitly** — not defer them to Phase 5, when the cost of switching is enormous. If Iced
cannot do IME acceptably on GNOME, revisit before Phase 5 starts. After that the decision is
effectively locked by volume of view code.
