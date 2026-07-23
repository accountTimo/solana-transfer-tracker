# Project status (for picking this back up later)

## Context

Portfolio project #3, after:
1. `snake-rl` (ML/RL project, on GitHub)
2. `http-server-rs` (hand-rolled HTTP/1.1 server in Rust, on GitHub:
   https://github.com/accountTimo/http-server-rs)

This project (`solana-tracker`, formerly named `solana-whale-tracker` — renamed
because 1 SOL is not remotely "whale" size) was chosen because the user wants
to move toward **data engineering** and is a "crypto/data freak". It's the
intersection of on-chain data and data engineering: ingest a live,
unbounded blockchain data source, decode it, filter it, persist it — the
same shape as any real streaming data pipeline.

Chain chosen: **Solana** (not Ethereum, not Monero — Monero was ruled out
because its privacy tech, i.e. ring signatures/stealth addresses/RingCT,
makes this kind of transaction-level analytics impossible by design).

## What's built (working, tested against live mainnet)

- Rust binary that polls the public Solana RPC (`api.mainnet-beta.solana.com`)
- Fetches new blocks as they're produced (`getSlot` + `getBlock` with
  `jsonParsed` encoding, so Solana decodes native transfers for us)
- Filters native SOL transfers >= 1 SOL (`THRESHOLD_LAMPORTS` in
  `src/main.rs` — was 0.4, then discussed 1, settled on 1 SOL to reduce
  data volume)
- Stores matches in local SQLite (`transfers.db`), deduped by tx signature,
  with resumable progress (last processed slot) so restarts don't
  re-scan from the chain tip
- Handles skipped slots and transient RPC errors without crashing
- Verified live: caught 58 real transfers in ~15s on mainnet, correctly
  bounded between 1.0 and 16.85 SOL

Files: `src/main.rs` (everything), `README.md` (written, English),
`.gitignore` (ignores `/target` and `*.db`). Git repo initialized, first
commit staged but **not yet committed** — user commits manually by choice.

## Known limitations (documented in README, intentional scope cuts)

- Only top-level instructions inspected — transfers nested inside program
  CPIs (e.g. SOL moved as a side effect of a swap) are NOT captured yet
- Only native SOL transfers, not SPL token transfers
- Uses the free public RPC endpoint (rate-limited) — fine for portfolio
  demo scale, not for sustained high-volume ingestion

## Sizing context (for reference, don't re-derive from scratch)

- User's machine: 16 cores, 30GB RAM, 1.6TB free disk — none of this is a
  real constraint for this project (network-IO bound, not compute-bound)
- User originally floated a 3TB cap, then clarified they meant 3GB
- Solana produces ~100-110M non-vote tx/day chain-wide (2026 figures);
  after filtering to >=1 SOL native transfers only, actual local storage
  is expected to be a small fraction of that — order of tens of MB to
  low GB per week is a reasonable expectation, but this was never
  precisely measured over a long run — validate before relying on it

## Sensible next steps (not yet started)

1. A simple query/report command (e.g. "top 10 largest transfers today",
   or a daily summary) — currently the only way to inspect data is raw
   SQL against `transfers.db`
2. Capture inner-instruction transfers (SOL moved via CPI, e.g. as part
   of a DEX swap) — currently missed entirely
3. Optionally: label/tag transfers by originating program (e.g. detect
   pump.fun-related activity) to distinguish "memecoin" traffic from
   plain wallet-to-wallet transfers — discussed as a nice-to-have, not
   started
4. Decide whether to keep using the free public RPC or switch to a
   provider with an API key (Helius/QuickNode) if rate limits become a
   problem during longer runs
