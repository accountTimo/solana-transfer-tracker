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

1. **Ingestion** (`src/bin/tracker.rs`) — polls the public Solana RPC for
   the current chain tip (`getSlot`) and fetches each new block
   (`getBlock`) as it's produced. [src/rpc.rs](src/rpc.rs)'s `RpcClient`
   rotates across multiple free public endpoints on rate limits or
   transport errors, so one endpoint being throttled doesn't stall
   ingestion — it just moves to the next.
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

`tracker` runs indefinitely in normal (live-tail) mode, printing each
detected transfer and persisting it to `transfers.db`. `dashboard` serves
live stats, recent/top transfers, charts, search, watchlist, and alerts,
refreshing every 5s.

To pull in a chunk of history instead of only live-tailing, set
`BACKFILL_HOURS`:

```sh
BACKFILL_HOURS=24 cargo run --release --bin tracker
```

This seeds the starting slot ~24h behind the chain tip and processes
unthrottled (no fixed poll delay) until it catches up, then **exits on its
own** with a summary instead of sliding into an indefinite live tail — a
backfill run is meant to grab a bounded window and finish. Throughput
depends entirely on the free public endpoints' mood; expect on the order of
a few slots/second, so a 24h backfill can take several hours.

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

## Architecture & design decisions

A few choices worth explaining, not just stating:

**Why Rust.** A live, unbounded, occasionally-inconsistent data source
(blockchain blocks) has to be ingested, decoded, filtered, and persisted
continuously — the same shape as any real streaming pipeline. Rust gives
predictable performance without GC pauses (relevant for a process meant to
keep pace with a live chain) and compile-time safety without C++'s manual
memory management — a reasonable fit for something meant to run unattended
for hours.

**Two binaries sharing one library, not a monolith.** Ingestion and the
dashboard have different failure modes and different lifecycles — the
tracker needs to survive for hours unattended, the dashboard is a short-lived
read-only view a human is actively looking at. Splitting them means a
dashboard bug can't take down ingestion, and vice versa, while `rpc`,
`transfer`, `db`, and `labels` stay as one tested, shared implementation
instead of two copies drifting apart.

**Catch-up vs. live-tail as two speeds of the same loop**, not two separate
code paths. Early on, `tracker` paced every slot at a fixed 1.5s interval
regardless of how far behind the chain tip it was — fine for live-tailing,
but it meant catching up 24h of history would have taken over a day. The
fix wasn't a rewrite: the same loop skips the sleep while more than a few
slots behind, and resumes pacing once caught up. One `catching_up` boolean,
not two implementations to maintain.

**Multi-endpoint failover over a paid RPC provider.** A dedicated provider
(Helius, QuickNode) would remove rate-limit risk entirely, but costs money
for a portfolio-scale project. Rotating across multiple free public
endpoints on transport failures gets most of the resilience benefit — in
practice, an unthrottled 24h backfill ran for hours against two free
endpoints without a single rate-limit hit — at zero cost. That's a real
engineering trade-off (cost vs. reliability guarantees), made deliberately
rather than defaulted into.

**Where I chose not to guess.** The label system in `src/labels.rs` maps
well-known *program* IDs (Jupiter, Raydium, pump.fun) to names, but
deliberately ships with an empty *wallet address* label table. Attributing
a specific address to a named real-world entity (an exchange, a person)
without a verifiable source risks asserting something false — and when I
tried to verify a few high-volume addresses from live data against public
explorers, none of them returned a confirmed label anyway, which validated
the caution rather than making it feel overly conservative in hindsight.
The mechanism is there; entries only get added once verified against a
trusted source.

**What I'd do differently with more time/budget:** capture SPL token
transfers (not just native SOL — the bigger remaining data gap), and move
to a paid RPC provider if this ever needed to run continuously rather than
in bounded demo sessions.
