use rusqlite::Connection;
use serde_json::{json, Value};
use std::thread;
use std::time::Duration;

const RPC_URL: &str = "https://api.mainnet-beta.solana.com";
const THRESHOLD_LAMPORTS: u64 = 1_000_000_000; // 1 SOL
const DB_PATH: &str = "whales.db";
const POLL_INTERVAL: Duration = Duration::from_millis(1500);

fn rpc_call(client: &reqwest::blocking::Client, method: &str, params: Value) -> Result<Value, String> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });

    let resp = client
        .post(RPC_URL)
        .json(&body)
        .send()
        .map_err(|e| format!("request failed: {e}"))?;

    if resp.status() == 429 {
        return Err("rate limited".to_string());
    }

    let value: Value = resp.json().map_err(|e| format!("bad json response: {e}"))?;

    if let Some(err) = value.get("error") {
        return Err(format!("rpc error: {err}"));
    }

    value
        .get("result")
        .cloned()
        .ok_or_else(|| "missing result field".to_string())
}

fn get_slot(client: &reqwest::blocking::Client) -> Result<u64, String> {
    let result = rpc_call(client, "getSlot", json!([]))?;
    result.as_u64().ok_or_else(|| "getSlot: result not a number".to_string())
}

fn get_block(client: &reqwest::blocking::Client, slot: u64) -> Result<Option<Value>, String> {
    let params = json!([
        slot,
        {
            "encoding": "jsonParsed",
            "transactionDetails": "full",
            "maxSupportedTransactionVersion": 0,
            "rewards": false
        }
    ]);

    match rpc_call(client, "getBlock", params) {
        Ok(value) => Ok(Some(value)),
        // Skipped slots and slots not yet available both surface as RPC
        // errors here; neither should stop the poll loop.
        Err(e) if e.contains("skipped") || e.contains("not available") => Ok(None),
        Err(e) => Err(e),
    }
}

struct WhaleTransfer {
    signature: String,
    slot: u64,
    block_time: Option<i64>,
    source: String,
    destination: String,
    lamports: u64,
}

/// Extracts native SOL transfers above THRESHOLD_LAMPORTS from a
/// jsonParsed block. Only looks at top-level instructions; transfers
/// nested inside program CPIs (inner instructions) are out of scope
/// for this first version.
fn extract_whale_transfers(block: &Value, slot: u64) -> Vec<WhaleTransfer> {
    let mut transfers = Vec::new();
    let block_time = block.get("blockTime").and_then(|v| v.as_i64());

    let Some(transactions) = block.get("transactions").and_then(|v| v.as_array()) else {
        return transfers;
    };

    for tx in transactions {
        let Some(signature) = tx
            .pointer("/transaction/signatures/0")
            .and_then(|v| v.as_str())
        else {
            continue;
        };

        let Some(instructions) = tx
            .pointer("/transaction/message/instructions")
            .and_then(|v| v.as_array())
        else {
            continue;
        };

        for ix in instructions {
            let is_system_transfer = ix.get("program").and_then(|v| v.as_str()) == Some("system")
                && ix.pointer("/parsed/type").and_then(|v| v.as_str()) == Some("transfer");
            if !is_system_transfer {
                continue;
            }

            let Some(info) = ix.pointer("/parsed/info") else {
                continue;
            };
            let Some(lamports) = info.get("lamports").and_then(|v| v.as_u64()) else {
                continue;
            };
            if lamports < THRESHOLD_LAMPORTS {
                continue;
            }

            let source = info.get("source").and_then(|v| v.as_str()).unwrap_or("unknown");
            let destination = info.get("destination").and_then(|v| v.as_str()).unwrap_or("unknown");

            transfers.push(WhaleTransfer {
                signature: signature.to_string(),
                slot,
                block_time,
                source: source.to_string(),
                destination: destination.to_string(),
                lamports,
            });
        }
    }

    transfers
}

fn init_db(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS transfers (
            signature   TEXT PRIMARY KEY,
            slot        INTEGER NOT NULL,
            block_time  INTEGER,
            source      TEXT NOT NULL,
            destination TEXT NOT NULL,
            lamports    INTEGER NOT NULL
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS progress (
            id        INTEGER PRIMARY KEY CHECK (id = 0),
            last_slot INTEGER NOT NULL
        )",
        [],
    )?;
    Ok(())
}

fn load_last_slot(conn: &Connection) -> Option<u64> {
    conn.query_row("SELECT last_slot FROM progress WHERE id = 0", [], |row| {
        row.get::<_, i64>(0)
    })
    .ok()
    .map(|v| v as u64)
}

fn save_last_slot(conn: &Connection, slot: u64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO progress (id, last_slot) VALUES (0, ?1)
         ON CONFLICT(id) DO UPDATE SET last_slot = excluded.last_slot",
        [slot as i64],
    )?;
    Ok(())
}

fn store_transfer(conn: &Connection, t: &WhaleTransfer) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO transfers (signature, slot, block_time, source, destination, lamports)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![t.signature, t.slot as i64, t.block_time, t.source, t.destination, t.lamports as i64],
    )?;
    Ok(())
}

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

    println!("Solana whale tracker starting at slot {next_slot} (chain tip: {current_slot})");
    println!("Filtering native SOL transfers >= {} SOL", THRESHOLD_LAMPORTS as f64 / 1e9);

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
                let transfers = extract_whale_transfers(&block, next_slot);
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
