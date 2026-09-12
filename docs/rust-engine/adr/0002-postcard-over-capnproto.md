# ADR-0002: postcard framing, not Cap'n Proto

**Status:** Accepted · **Date:** 2026-09-12 · Relates to: PLAN.md §2.1, §6 Phase 2

## Context

The spec asks for Cap'n Proto zero-copy on the core socket, and JSON-RPC on the worker socket.
Compass already has a working framed protocol plus an in-tree generator (`figura`).

## Decision

**`serde` + length-prefixed `postcard` frames** on the core socket. **JSON-RPC 2.0 stays** on the
extension-worker socket, because `@raycast/api` compatibility requires it.

## Why

- The spec's own IPC SLA is a 0.5 ms round-trip. That is not a demanding target for a local Unix
  socket with a compact binary encoding, and we have not measured a need for zero-copy.
- A schema compiler is a real cost: a build-time dependency, a second source of truth, and an extra
  thing every contributor must install. `serde` derives are already in the graph.
- Two wire formats in one process is a maintenance surface we would be adopting on speculation.

## What would change our mind

Phase 4 benchmarks. If the round-trip misses 0.5 ms p99, or profiling shows deserialisation
dominating a hot path, adopt Cap'n Proto for that path. The framing layer is deliberately separable
so the encoding can be swapped without touching transport or message definitions.

This is a "not yet", not a "no".
