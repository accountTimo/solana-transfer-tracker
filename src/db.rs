use rusqlite::Connection;
use serde::Serialize;

use crate::transfer::Transfer;

pub fn init_db(conn: &Connection) -> rusqlite::Result<()> {
    // WAL mode allows the dashboard to read the database concurrently
    // while the tracker keeps writing, without lock contention.
    conn.pragma_update(None, "journal_mode", "WAL")?;

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
    conn.execute(
        "CREATE TABLE IF NOT EXISTS watchlist (
            address  TEXT PRIMARY KEY,
            label    TEXT,
            added_at INTEGER
        )",
        [],
    )?;

    migrate_transfers_table(conn)?;
    Ok(())
}

/// `transfers.db` may already contain rows from before these columns
/// existed; add them in place instead of dropping data.
fn migrate_transfers_table(conn: &Connection) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(transfers)")?;
    let existing: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<_>>()?;

    let wanted: &[(&str, &str)] = &[
        ("is_inner", "INTEGER NOT NULL DEFAULT 0"),
        ("program_label", "TEXT"),
        ("source_label", "TEXT"),
        ("destination_label", "TEXT"),
    ];

    for (column, def) in wanted {
        if !existing.iter().any(|c| c == column) {
            conn.execute(
                &format!("ALTER TABLE transfers ADD COLUMN {column} {def}"),
                [],
            )?;
        }
    }
    Ok(())
}

pub fn load_last_slot(conn: &Connection) -> Option<u64> {
    conn.query_row("SELECT last_slot FROM progress WHERE id = 0", [], |row| {
        row.get::<_, i64>(0)
    })
    .ok()
    .map(|v| v as u64)
}

pub fn save_last_slot(conn: &Connection, slot: u64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO progress (id, last_slot) VALUES (0, ?1)
         ON CONFLICT(id) DO UPDATE SET last_slot = excluded.last_slot",
        [slot as i64],
    )?;
    Ok(())
}

pub fn store_transfer(conn: &Connection, t: &Transfer) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO transfers
         (signature, slot, block_time, source, destination, lamports,
          is_inner, program_label, source_label, destination_label)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            t.signature,
            t.slot as i64,
            t.block_time,
            t.source,
            t.destination,
            t.lamports as i64,
            t.is_inner,
            t.program_label,
            t.source_label,
            t.destination_label,
        ],
    )?;
    Ok(())
}

#[derive(Serialize)]
pub struct Stats {
    pub total_count: u64,
    pub total_sol: f64,
    pub count_24h: u64,
    pub sol_24h: f64,
}

#[derive(Serialize)]
pub struct HourBucket {
    pub hour: String,
    pub count: u64,
    pub sol: f64,
}

#[derive(Serialize)]
pub struct WatchlistEntry {
    pub address: String,
    pub label: Option<String>,
}

const TRANSFER_COLUMNS: &str = "signature, slot, block_time, source, destination, lamports,
     is_inner, program_label, source_label, destination_label";

fn row_to_transfer(row: &rusqlite::Row) -> rusqlite::Result<Transfer> {
    Ok(Transfer {
        signature: row.get(0)?,
        slot: row.get::<_, i64>(1)? as u64,
        block_time: row.get(2)?,
        source: row.get(3)?,
        destination: row.get(4)?,
        lamports: row.get::<_, i64>(5)? as u64,
        is_inner: row.get(6)?,
        program_label: row.get(7)?,
        source_label: row.get(8)?,
        destination_label: row.get(9)?,
    })
}

pub fn recent_transfers(conn: &Connection, limit: u32) -> rusqlite::Result<Vec<Transfer>> {
    let sql = format!("SELECT {TRANSFER_COLUMNS} FROM transfers ORDER BY slot DESC LIMIT ?1");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([limit], row_to_transfer)?;
    rows.collect()
}

pub fn top_transfers(conn: &Connection, limit: u32) -> rusqlite::Result<Vec<Transfer>> {
    let sql = format!("SELECT {TRANSFER_COLUMNS} FROM transfers ORDER BY lamports DESC LIMIT ?1");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([limit], row_to_transfer)?;
    rows.collect()
}

pub fn search_transfers(
    conn: &Connection,
    address: &str,
    limit: u32,
) -> rusqlite::Result<Vec<Transfer>> {
    let sql = format!(
        "SELECT {TRANSFER_COLUMNS} FROM transfers
         WHERE source = ?1 OR destination = ?1
         ORDER BY slot DESC LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![address, limit], row_to_transfer)?;
    rows.collect()
}

pub fn watchlist_activity(conn: &Connection, limit: u32) -> rusqlite::Result<Vec<Transfer>> {
    let sql = format!(
        "SELECT {TRANSFER_COLUMNS} FROM transfers
         WHERE source IN (SELECT address FROM watchlist)
            OR destination IN (SELECT address FROM watchlist)
         ORDER BY slot DESC LIMIT ?1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([limit], row_to_transfer)?;
    rows.collect()
}

pub fn stats(conn: &Connection) -> rusqlite::Result<Stats> {
    let (total_count, total_lamports): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(lamports), 0) FROM transfers",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    let (count_24h, lamports_24h): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(lamports), 0) FROM transfers
         WHERE block_time >= strftime('%s', 'now', '-24 hours')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    Ok(Stats {
        total_count: total_count as u64,
        total_sol: total_lamports as f64 / 1e9,
        count_24h: count_24h as u64,
        sol_24h: lamports_24h as f64 / 1e9,
    })
}

pub fn hourly_volume(conn: &Connection, hours: u32) -> rusqlite::Result<Vec<HourBucket>> {
    let sql = format!(
        "SELECT strftime('%Y-%m-%d %H:00', block_time, 'unixepoch') AS hour,
                COUNT(*), COALESCE(SUM(lamports), 0)
         FROM transfers
         WHERE block_time >= strftime('%s', 'now', '-{hours} hours')
         GROUP BY hour
         ORDER BY hour ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let count: i64 = row.get(1)?;
        let lamports: i64 = row.get(2)?;
        Ok(HourBucket {
            hour: row.get(0)?,
            count: count as u64,
            sol: lamports as f64 / 1e9,
        })
    })?;
    rows.collect()
}

#[derive(Serialize, Clone)]
pub struct ProgramShare {
    pub label: String,
    pub count: u64,
    pub sol: f64,
}

/// Volume grouped by originating program, folded down to the top 3 plus
/// "Other" — matches the dataviz palette's series cap for all-pairs charts
/// (donut/pie), where more than 3-4 categorical slots stop being
/// distinguishable.
pub fn program_breakdown(conn: &Connection) -> rusqlite::Result<Vec<ProgramShare>> {
    let label_expr = "COALESCE(program_label, 'Wallet transfer')";
    let sql = format!(
        "SELECT {label_expr} AS label, COUNT(*), COALESCE(SUM(lamports), 0)
         FROM transfers GROUP BY label ORDER BY 3 DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<ProgramShare> = stmt
        .query_map([], |row| {
            let count: i64 = row.get(1)?;
            let lamports: i64 = row.get(2)?;
            Ok(ProgramShare {
                label: row.get(0)?,
                count: count as u64,
                sol: lamports as f64 / 1e9,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    if rows.len() <= 4 {
        return Ok(rows);
    }
    let (top, rest) = rows.split_at(3);
    let mut result: Vec<ProgramShare> = top.to_vec();
    result.push(ProgramShare {
        label: "Other".to_string(),
        count: rest.iter().map(|r| r.count).sum(),
        sol: rest.iter().map(|r| r.sol).sum(),
    });
    Ok(result)
}

#[derive(Serialize)]
pub struct SizeBucket {
    pub range: String,
    pub count: u64,
}

/// How many transfers fall in each order-of-magnitude SOL range. Fixed,
/// ordered buckets rather than a computed histogram — the same shape every
/// time makes the chart easy to read run over run.
pub fn size_distribution(conn: &Connection) -> rusqlite::Result<Vec<SizeBucket>> {
    let buckets: &[(&str, u64, Option<u64>)] = &[
        ("1-10", 1_000_000_000, Some(10_000_000_000)),
        ("10-100", 10_000_000_000, Some(100_000_000_000)),
        ("100-1,000", 100_000_000_000, Some(1_000_000_000_000)),
        ("1,000+", 1_000_000_000_000, None),
    ];

    let mut result = Vec::with_capacity(buckets.len());
    for (range, min, max) in buckets {
        let count: i64 = match max {
            Some(max) => conn.query_row(
                "SELECT COUNT(*) FROM transfers WHERE lamports >= ?1 AND lamports < ?2",
                rusqlite::params![*min as i64, *max as i64],
                |row| row.get(0),
            )?,
            None => conn.query_row(
                "SELECT COUNT(*) FROM transfers WHERE lamports >= ?1",
                [*min as i64],
                |row| row.get(0),
            )?,
        };
        result.push(SizeBucket {
            range: range.to_string(),
            count: count as u64,
        });
    }
    Ok(result)
}

#[derive(Serialize)]
pub struct AddressVolume {
    pub address: String,
    pub label: Option<String>,
    pub count: u64,
    pub sol: f64,
}

/// Ranks addresses by total SOL moved (as sender or receiver combined),
/// surfacing repeat "whales" rather than one-off transfer parties.
pub fn top_addresses(conn: &Connection, limit: u32) -> rusqlite::Result<Vec<AddressVolume>> {
    let sql = "
        SELECT address, MAX(label), SUM(cnt), SUM(lamports)
        FROM (
            SELECT source AS address, source_label AS label, COUNT(*) AS cnt, SUM(lamports) AS lamports
            FROM transfers GROUP BY source
            UNION ALL
            SELECT destination, destination_label, COUNT(*), SUM(lamports)
            FROM transfers GROUP BY destination
        )
        GROUP BY address
        ORDER BY SUM(lamports) DESC
        LIMIT ?1";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([limit], |row| {
        let count: i64 = row.get(2)?;
        let lamports: i64 = row.get(3)?;
        Ok(AddressVolume {
            address: row.get(0)?,
            label: row.get(1)?,
            count: count as u64,
            sol: lamports as f64 / 1e9,
        })
    })?;
    rows.collect()
}

#[derive(Serialize)]
pub struct HourOfDayActivity {
    pub hour: u32,
    pub count: u64,
    pub sol: f64,
}

/// Activity grouped by hour-of-day (0-23), across all captured history —
/// a "what time does this happen" view, distinct from `hourly_volume`'s
/// last-N-hours timeline.
pub fn activity_by_hour_of_day(conn: &Connection) -> rusqlite::Result<Vec<HourOfDayActivity>> {
    let sql = "
        SELECT CAST(strftime('%H', block_time, 'unixepoch') AS INTEGER) AS h,
               COUNT(*), COALESCE(SUM(lamports), 0)
        FROM transfers
        WHERE block_time IS NOT NULL
        GROUP BY h
        ORDER BY h ASC";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        let hour: i64 = row.get(0)?;
        let count: i64 = row.get(1)?;
        let lamports: i64 = row.get(2)?;
        Ok(HourOfDayActivity {
            hour: hour as u32,
            count: count as u64,
            sol: lamports as f64 / 1e9,
        })
    })?;
    rows.collect()
}

pub fn add_watchlist(
    conn: &Connection,
    address: &str,
    label: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO watchlist (address, label, added_at) VALUES (?1, ?2, strftime('%s', 'now'))
         ON CONFLICT(address) DO UPDATE SET label = excluded.label",
        rusqlite::params![address, label],
    )?;
    Ok(())
}

pub fn remove_watchlist(conn: &Connection, address: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM watchlist WHERE address = ?1", [address])?;
    Ok(())
}

pub fn list_watchlist(conn: &Connection) -> rusqlite::Result<Vec<WatchlistEntry>> {
    let mut stmt = conn.prepare("SELECT address, label FROM watchlist ORDER BY added_at DESC")?;
    let rows = stmt.query_map([], |row| {
        Ok(WatchlistEntry {
            address: row.get(0)?,
            label: row.get(1)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_transfer(signature: &str, lamports: u64, source: &str, destination: &str) -> Transfer {
        Transfer {
            signature: signature.to_string(),
            slot: 1,
            block_time: Some(1_700_000_000),
            source: source.to_string(),
            destination: destination.to_string(),
            lamports,
            is_inner: false,
            program_label: None,
            source_label: None,
            destination_label: None,
        }
    }

    #[test]
    fn init_db_is_idempotent_and_migrates_missing_columns() {
        let conn = Connection::open_in_memory().unwrap();
        // Simulate an old-schema DB: create the table without the new columns.
        conn.execute(
            "CREATE TABLE transfers (
                signature TEXT PRIMARY KEY, slot INTEGER NOT NULL, block_time INTEGER,
                source TEXT NOT NULL, destination TEXT NOT NULL, lamports INTEGER NOT NULL
            )",
            [],
        )
        .unwrap();

        init_db(&conn).unwrap();
        init_db(&conn).unwrap(); // must not error the second time

        // New columns should now exist and be usable.
        store_transfer(&conn, &test_transfer("sig1", 2_000_000_000, "A", "B")).unwrap();
        let rows = recent_transfers(&conn, 10).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn store_transfer_dedupes_by_signature() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        store_transfer(&conn, &test_transfer("dup", 1_000_000_000, "A", "B")).unwrap();
        store_transfer(&conn, &test_transfer("dup", 1_000_000_000, "A", "B")).unwrap();

        let (count,): (i64,) = conn
            .query_row("SELECT COUNT(*) FROM transfers", [], |r| Ok((r.get(0)?,)))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn stats_computes_totals() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        store_transfer(&conn, &test_transfer("s1", 1_000_000_000, "A", "B")).unwrap();
        store_transfer(&conn, &test_transfer("s2", 2_000_000_000, "C", "D")).unwrap();

        let s = stats(&conn).unwrap();
        assert_eq!(s.total_count, 2);
        assert!((s.total_sol - 3.0).abs() < 1e-9);
    }

    #[test]
    fn top_transfers_orders_by_lamports_desc() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        store_transfer(&conn, &test_transfer("small", 1_000_000_000, "A", "B")).unwrap();
        store_transfer(&conn, &test_transfer("big", 5_000_000_000, "C", "D")).unwrap();

        let top = top_transfers(&conn, 10).unwrap();
        assert_eq!(top[0].signature, "big");
    }

    #[test]
    fn recent_transfers_orders_by_slot_desc() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let mut early = test_transfer("early", 1_000_000_000, "A", "B");
        early.slot = 1;
        let mut later = test_transfer("later", 1_000_000_000, "A", "B");
        later.slot = 2;
        store_transfer(&conn, &early).unwrap();
        store_transfer(&conn, &later).unwrap();

        let recent = recent_transfers(&conn, 10).unwrap();
        assert_eq!(recent[0].signature, "later");
    }

    #[test]
    fn search_transfers_matches_source_or_destination() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        store_transfer(&conn, &test_transfer("s1", 1_000_000_000, "A", "B")).unwrap();
        store_transfer(&conn, &test_transfer("s2", 1_000_000_000, "C", "A")).unwrap();
        store_transfer(&conn, &test_transfer("s3", 1_000_000_000, "X", "Y")).unwrap();

        let results = search_transfers(&conn, "A", 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn watchlist_add_list_remove() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        add_watchlist(&conn, "addr1", Some("My wallet")).unwrap();
        let list = list_watchlist(&conn).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].label.as_deref(), Some("My wallet"));

        remove_watchlist(&conn, "addr1").unwrap();
        assert!(list_watchlist(&conn).unwrap().is_empty());
    }

    #[test]
    fn program_breakdown_groups_and_folds_to_other() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let mut wallet = test_transfer("s1", 1_000_000_000, "A", "B");
        wallet.program_label = None;
        store_transfer(&conn, &wallet).unwrap();

        for (i, program) in ["Jupiter", "Raydium", "pump.fun", "Orca", "Solend"]
            .iter()
            .enumerate()
        {
            let mut t = test_transfer(&format!("p{i}"), 1_000_000_000, "A", "B");
            t.program_label = Some(program.to_string());
            store_transfer(&conn, &t).unwrap();
        }

        let breakdown = program_breakdown(&conn).unwrap();
        // 6 distinct labels in (Wallet transfer + 5 programs) fold to top 3 + Other.
        assert_eq!(breakdown.len(), 4);
        assert!(breakdown.iter().any(|b| b.label == "Other"));
    }

    #[test]
    fn size_distribution_buckets_by_order_of_magnitude() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        store_transfer(&conn, &test_transfer("small", 2_000_000_000, "A", "B")).unwrap(); // 2 SOL
        store_transfer(&conn, &test_transfer("mid", 50_000_000_000, "A", "B")).unwrap(); // 50 SOL
        store_transfer(&conn, &test_transfer("big", 2_000_000_000_000, "A", "B")).unwrap(); // 2000 SOL

        let buckets = size_distribution(&conn).unwrap();
        let get = |range: &str| buckets.iter().find(|b| b.range == range).unwrap().count;
        assert_eq!(get("1-10"), 1);
        assert_eq!(get("10-100"), 1);
        assert_eq!(get("1,000+"), 1);
    }

    #[test]
    fn top_addresses_ranks_by_combined_volume() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        store_transfer(&conn, &test_transfer("s1", 1_000_000_000, "whale", "B")).unwrap();
        store_transfer(&conn, &test_transfer("s2", 4_000_000_000, "C", "whale")).unwrap();
        store_transfer(&conn, &test_transfer("s3", 1_000_000_000, "X", "Y")).unwrap();

        let top = top_addresses(&conn, 5).unwrap();
        assert_eq!(top[0].address, "whale");
        assert!((top[0].sol - 5.0).abs() < 1e-9);
        assert_eq!(top[0].count, 2);
    }

    #[test]
    fn activity_by_hour_of_day_groups_correctly() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let mut t = test_transfer("s1", 1_000_000_000, "A", "B");
        t.block_time = Some(1_700_000_000); // fixed timestamp, deterministic hour-of-day
        store_transfer(&conn, &t).unwrap();

        let activity = activity_by_hour_of_day(&conn).unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].count, 1);
    }

    #[test]
    fn watchlist_activity_filters_by_watched_addresses() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        store_transfer(
            &conn,
            &test_transfer("watched", 1_000_000_000, "whale", "elsewhere"),
        )
        .unwrap();
        store_transfer(&conn, &test_transfer("unwatched", 1_000_000_000, "A", "B")).unwrap();

        add_watchlist(&conn, "whale", None).unwrap();
        let activity = watchlist_activity(&conn, 10).unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].signature, "watched");
    }
}
