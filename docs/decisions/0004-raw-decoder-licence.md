# 0004 — RAW Decoder Licence

- **Status:** Accepted
- **Date:** 2026-10-02
- **Decided by:** Operator

## Context

ADR 0001 selects `rawler` for RAW decode. `rawler` uses the LGPL-2.1 licence. The project rule permits only MIT, Apache-2.0, BSD, zlib, and ISC licences without operator approval. Without a RAW library, Expose can read only DNG, not the proprietary RAW formats of most cameras.

## Decision

- The operator approves `rawler` (LGPL-2.1) as the RAW backend of `expose-raw`.
- This approval is for `rawler` and its own dependencies only. All other dependencies must still use a permissive licence, or need a new approval.
- `rawler` is behind the default-on cargo feature `rawler` in `expose-raw`, under a `RawDecoder` trait. No other crate may depend on `rawler` directly.

## Consequences

- Rust links `rawler` statically. To obey the LGPL, a distributed build must let users relink Expose with a modified `rawler` (for example: supply the object files, or put `rawler` in a separate dynamic library). Do this before the first public release.
- `rawler` is pure Rust, so the web version stays possible.
- A build without the `rawler` feature can use only the permissive backends.
