# solana-tracker

A Rust pipeline that polls the Solana blockchain for native SOL transfers
above a configurable threshold and stores them in a local SQLite database,
in real time.

## Why

This is where on-chain data and data engineering meet: continuously
ingesting a live, unbounded, occasionally-inconsistent data source
(blockchain blocks), decoding it into structured records, filtering it, and
persisting it — the same shape of problem as any real-world streaming data
pipeline, just with a blockchain as the source instead of Kafka or a message
queue.

## How it works

The project is split into a shared library (`src/lib.rs`, modules `rpc`,
`transfer`, `db`, `labels`) and two binaries built on top of it:

1. **Ingestion** (`src/bin/tracker.rs`) — polls the public Solana RPC
   endpoint for the current chain tip (`getSlot`) and fetches each new
   block (`getBlock`) as it's produced.
2. **Decoding** ([src/transfer.rs](src/transfer.rs)) — requests blocks with
   `jsonParsed` encoding, so the Solana RPC node decodes native System
   Program transfer instructions for us (no manual instruction-data parsing
   needed). Both **top-level** transfers (plain wallet-to-wallet) and
   **inner-instruction transfers** (SOL moved via CPI, e.g. as a side effect
   of a DEX swap) are captured; inner-instruction transfers are attributed
   to the program that triggered them (`is_inner`, `program_label` fields).
3. **Filtering** — keeps only native SOL transfers at or above
   `THRESHOLD_LAMPORTS` (currently 1 SOL), discarding the enormous volume of
   dust/bot/vote traffic.
4. **Labeling** ([src/labels.rs](src/labels.rs)) — a small, hand-curated,
   best-effort lookup table maps well-known program IDs (Jupiter, Raydium,
   pump.fun, Orca) to human-readable names. Wallet/treasury address labels
   are deliberately left empty by default (see "Known limitations" below);
   the mechanism exists, add entries once verified against a trusted source.
5. **Storage** ([src/db.rs](src/db.rs)) — writes matching transfers to a
   local SQLite database (`transfers.db`) in WAL mode, deduplicated by
   transaction signature. Progress (last processed slot) is persisted too,
   so the tracker resumes where it left off after a restart instead of
   re-scanning from the chain tip. WAL mode also lets the dashboard read the
   database concurrently while the tracker keeps writing. Schema migrations
   are applied automatically and idempotently on startup, so existing
   databases from earlier versions keep working.
6. **Dashboard** (`src/bin/dashboard.rs`) — a small `axum` web server that
   reads `transfers.db` and serves a JSON API plus a static HTML/JS
   frontend: live stats, an hourly volume chart, recent/top transfer tables
   with program and address labels, address search, a watchlist with its
   own activity feed, and in-page alert banners for transfers above a
   configurable threshold.

Skipped slots (Solana doesn't produce a block for every slot) and transient
RPC errors are handled without crashing the poll loop.

## Testing

```sh
cargo test
```

Unit tests cover the core logic: transfer extraction and filtering
(top-level and inner-instruction, threshold, program labeling) in
`src/transfer.rs`, database operations (schema migration, dedup, stats,
search, watchlist) in `src/db.rs` against an in-memory SQLite database, and
label lookups in `src/labels.rs`. The RPC client and the HTTP layer itself
are not unit tested — they're thin glue around network calls, verified
instead by running the binaries against live mainnet (see "Running").

`cargo clippy --all-targets` and `cargo fmt --check` are clean.

## Running

Ingestion and dashboard are separate binaries — run them in two terminals:

```sh
cargo run --release --bin tracker      # polls mainnet, fills transfers.db
cargo run --release --bin dashboard    # serves http://127.0.0.1:3000
```

`tracker` runs indefinitely, printing each detected transfer and persisting
it to `transfers.db` in the project directory. `dashboard` serves live stats,
recent/top transfers, an hourly volume chart, search, watchlist, and alerts,
refreshing every 5s.

## Scope / what's not handled (yet)

- Only **native SOL** transfers are tracked, not SPL token transfers.
- The address/program label list in `src/labels.rs` is small and
  hand-curated, not comprehensive, and the program IDs in it were filled in
  from memory rather than verified live — double-check before relying on a
  label. Wallet/treasury addresses are intentionally left unlabeled by
  default rather than risk asserting a wrong real-world attribution.
- Uses the public, rate-limited Solana RPC endpoint — fine for a
  portfolio-scale demo, but a dedicated RPC provider (Helius, QuickNode)
  would be needed for sustained higher-volume ingestion.
- Alerts are in-page only (banner + row highlight on refresh), not browser
  desktop notifications — they only fire while the dashboard tab is open.
- The RPC client and HTTP layer are not unit tested (see "Testing").
