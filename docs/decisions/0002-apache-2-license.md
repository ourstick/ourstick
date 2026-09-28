# ADR 0002: Apache-2.0

**Status:** accepted — 2026-09-28

## Decision
Public repository under Apache-2.0. Commercial parts (accounts, billing, quota,
hosting) live in a private repository that depends on the public core.

## Why
- Permissive: the private code can use the core without any self-exemption.
- No CLA needed for contributors.
- Patent grant, which MIT lacks.
- Compatible with iroh (MIT/Apache-2.0).

## Trade-off
Anyone may build a competing hosted service on the core. Accepted: at that scale
the moat is brand, users and infrastructure, not code.
