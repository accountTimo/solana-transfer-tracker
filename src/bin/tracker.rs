use rusqlite::Connection;
use std::thread;
use std::time::Duration;

use solana_tracker::db::{init_db, load_last_slot, save_last_slot, store_transfer};
use solana_tracker::rpc::{get_block, get_slot};
use solana_tracker::transfer::{THRESHOLD_LAMPORTS, extract_transfers};

const DB_PATH: &str = "transfers.db";
const POLL_INTERVAL: Duration = Duration::from_millis(1500);

fn main() {
    let conn = Connection::open(DB_PATH).expect("failed to open sqlite db");
    init_db(&conn).expect("failed to init schema");

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("failed to build http client");

    let current_slot = match get_slot(&client) {
        Ok(slot) => slot,
        Err(e) => {
            eprintln!("failed to fetch starting slot: {e}");
            return;
        }
    };

    // Resume from where we left off, but never try to backfill more than a
    // couple thousand slots on first run — that's a lot of RPC calls against
    // a free public endpoint's rate limit.
    let mut next_slot = load_last_slot(&conn)
        .map(|s| s + 1)
        .unwrap_or(current_slot.saturating_sub(50));

    println!("Solana tracker starting at slot {next_slot} (chain tip: {current_slot})");
    println!(
        "Filtering native SOL transfers >= {} SOL",
        THRESHOLD_LAMPORTS as f64 / 1e9
    );

    loop {
        let tip = match get_slot(&client) {
            Ok(slot) => slot,
            Err(e) => {
                eprintln!("getSlot failed: {e}, retrying...");
                thread::sleep(POLL_INTERVAL);
                continue;
            }
        };

        if next_slot > tip {
            thread::sleep(POLL_INTERVAL);
            continue;
        }

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

        thread::sleep(POLL_INTERVAL);
    }
}
