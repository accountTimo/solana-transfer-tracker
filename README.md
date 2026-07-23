# solana-whale-tracker

A Rust pipeline that polls the Solana blockchain for large ("whale") native
SOL transfers and stores them in a local SQLite database, in real time.

## Why

This is where on-chain data and data engineering meet: continuously
ingesting a live, unbounded, occasionally-inconsistent data source
(blockchain blocks), decoding it into structured records, filtering it, and
persisting it — the same shape of problem as any real-world streaming data
pipeline, just with a blockchain as the source instead of Kafka or a message
queue.

## How it works

1. **Ingestion** — polls the public Solana RPC endpoint for the current
   chain tip (`getSlot`) and fetches each new block (`getBlock`) as it's
   produced.
2. **Decoding** — requests blocks with `jsonParsed` encoding, so the Solana
   RPC node decodes native System Program transfer instructions for us
   (no manual instruction-data parsing needed for this first version).
3. **Filtering** — keeps only native SOL transfers at or above
   `THRESHOLD_LAMPORTS` (currently 1 SOL) in [src/main.rs](src/main.rs),
   discarding the enormous volume of dust/bot/vote traffic.
4. **Storage** — writes matching transfers to a local SQLite database
   (`whales.db`), deduplicated by transaction signature. Progress (last
   processed slot) is persisted too, so the tracker resumes where it left
   off after a restart instead of re-scanning from the chain tip.

Skipped slots (Solana doesn't produce a block for every slot) and transient
RPC errors are handled without crashing the poll loop.

## Running

```sh
cargo run --release
```

Runs indefinitely, printing each detected whale transfer and persisting it
to `whales.db` in the project directory.

## Scope / what's not handled (yet)

- Only **top-level** instructions are inspected — transfers nested inside
  program CPIs (inner instructions), e.g. a swap that moves SOL as a side
  effect, are not currently captured.
- Only **native SOL** transfers are tracked, not SPL token transfers.
- Uses the public, rate-limited Solana RPC endpoint — fine for a
  portfolio-scale demo, but a dedicated RPC provider (Helius, QuickNode)
  would be needed for sustained higher-volume ingestion.
