# ADR 0001: Rust core on iroh

**Status:** accepted — 2026-09-28

## Decision
Core library in Rust, networking via iroh (QUIC, hole punching, relay fallback,
content-addressed blobs). UI via Tauri 2 with a TypeScript frontend.

## Why
- iroh is Rust-native; any other language would go through less mature bindings.
- Tauri 2 targets desktop and mobile from one codebase.
- The compiler catches errors early, which fits an AI-assisted workflow.

## Alternatives considered
- Flutter + Rust (flutter_rust_bridge): adds Dart for no gain.
- Node/Electron + Holepunch/Pear: JS-native networking, but no clean mobile story.
