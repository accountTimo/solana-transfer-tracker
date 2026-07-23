use rusqlite::Connection;
use std::thread;
use std::time::Duration;

use solana_tracker::db::{init_db, load_last_slot, save_last_slot, stats, store_transfer};
use solana_tracker::rpc::{RpcClient, get_block, get_slot};
use solana_tracker::transfer::{THRESHOLD_LAMPORTS, extract_transfers};

const DB_PATH: &str = "transfers.db";
const POLL_INTERVAL: Duration = Duration::from_millis(1500);
// Mainnet produces roughly one slot every ~0.4s; used only to translate a
// `BACKFILL_HOURS` request into a starting slot, not for pacing.
const APPROX_SLOTS_PER_HOUR: u64 = 9_000;
// While catching up (more than this many slots behind the tip), skip the
// poll-interval sleep entirely and fetch as fast as the RPC allows — the
// fixed 1.5s pace is only needed once we're live-tailing near the tip, to
// avoid hammering the free public endpoint for no reason.
const CATCH_UP_GAP: u64 = 5;

fn main() {
    let conn = Connection::open(DB_PATH).expect("failed to open sqlite db");
    init_db(&conn).expect("failed to init schema");

    let client = RpcClient::new().expect("failed to build http client");

    let current_slot = match get_slot(&client) {
        Ok(slot) => slot,
        Err(e) => {
            eprintln!("failed to fetch starting slot: {e}");
            return;
        }
    };

    // BACKFILL_HOURS=24 seeds the start slot ~24h before the chain tip and
    // ignores saved progress — for pulling in a chunk of history in one run
    // instead of resuming the live tail. Without it: resume from last_slot,
    // or default to a small 50-slot backfill on a fresh DB.
    let backfill_hours: Option<u64> = std::env::var("BACKFILL_HOURS")
        .ok()
        .and_then(|v| v.parse().ok());

    let mut next_slot = match backfill_hours {
        Some(hours) => current_slot.saturating_sub(hours * APPROX_SLOTS_PER_HOUR),
        None => load_last_slot(&conn)
            .map(|s| s + 1)
            .unwrap_or(current_slot.saturating_sub(50)),
    };

    // In backfill mode, stop once we reach the tip observed at startup
    // instead of sliding into an indefinite live tail — a backfill run is
    // meant to grab a bounded chunk of history and finish, not run forever.
    let backfill_target = backfill_hours.map(|_| current_slot);
    let mut processed = 0u64;

    println!("Solana tracker starting at slot {next_slot} (chain tip: {current_slot})");
    println!(
        "Filtering native SOL transfers >= {} SOL",
        THRESHOLD_LAMPORTS as f64 / 1e9
    );
    if let Some(hours) = backfill_hours {
        println!(
            "Backfill mode: ~{hours}h of history (up to slot {current_slot}), running unthrottled, exits when caught up"
        );
    }

    loop {
        let tip = match get_slot(&client) {
            Ok(slot) => slot,
            Err(e) => {
                eprintln!("getSlot failed: {e}, retrying...");
                thread::sleep(POLL_INTERVAL);
                continue;
            }
        };

        if let Some(target) = backfill_target
            && next_slot > target
        {
            let total = stats(&conn).map(|s| s.total_count).unwrap_or(0);
            println!(
                "Backfill complete: scanned {processed} slots up to {target}, {total} transfers in the database"
            );
            return;
        }

        if next_slot > tip {
            thread::sleep(POLL_INTERVAL);
            continue;
        }

        let catching_up = tip - next_slot > CATCH_UP_GAP;

        match get_block(&client, next_slot) {
            Ok(Some(block)) => {
                let transfers = extract_transfers(&block, next_slot);
                for t in &transfers {
                    println!(
                        "[slot {}] {} SOL  {} -> {}  ({})",
                        t.slot,
                        t.lamports as f64 / 1e9,
                        t.source,
                        t.destination,
                        t.signature
                    );
                    if let Err(e) = store_transfer(&conn, t) {
                        eprintln!("failed to store transfer {}: {e}", t.signature);
                    }
                }
            }
            Ok(None) => {
                // Slot was skipped or not produced; nothing to process.
            }
            Err(e) => {
                eprintln!("getBlock({next_slot}) failed: {e}, retrying after backoff");
                thread::sleep(POLL_INTERVAL * 2);
                continue;
            }
        }

        if let Err(e) = save_last_slot(&conn, next_slot) {
            eprintln!("failed to persist progress: {e}");
        }
        next_slot += 1;
        processed += 1;

        if !catching_up {
            thread::sleep(POLL_INTERVAL);
        }
    }
}
