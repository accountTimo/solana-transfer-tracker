use serde::Serialize;
use serde_json::Value;

use crate::labels;

pub const THRESHOLD_LAMPORTS: u64 = 1_000_000_000; // 1 SOL

#[derive(Serialize)]
pub struct Transfer {
    pub signature: String,
    pub slot: u64,
    pub block_time: Option<i64>,
    pub source: String,
    pub destination: String,
    pub lamports: u64,
    pub is_inner: bool,
    pub program_label: Option<String>,
    pub source_label: Option<String>,
    pub destination_label: Option<String>,
}

/// Parses a single jsonParsed instruction as a native System Program
/// transfer above THRESHOLD_LAMPORTS, if it is one.
fn parse_system_transfer(ix: &Value) -> Option<(String, String, u64)> {
    let is_system_transfer = ix.get("program").and_then(|v| v.as_str()) == Some("system")
        && ix.pointer("/parsed/type").and_then(|v| v.as_str()) == Some("transfer");
    if !is_system_transfer {
        return None;
    }

    let info = ix.pointer("/parsed/info")?;
    let lamports = info.get("lamports").and_then(|v| v.as_u64())?;
    if lamports < THRESHOLD_LAMPORTS {
        return None;
    }

    let source = info
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let destination = info
        .get("destination")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    Some((source, destination, lamports))
}

fn build_transfer(
    signature: &str,
    slot: u64,
    block_time: Option<i64>,
    (source, destination, lamports): (String, String, u64),
    is_inner: bool,
    program_label: Option<&str>,
) -> Transfer {
    Transfer {
        signature: signature.to_string(),
        slot,
        block_time,
        source_label: labels::label_address(&source).map(str::to_string),
        destination_label: labels::label_address(&destination).map(str::to_string),
        source,
        destination,
        lamports,
        is_inner,
        program_label: program_label.map(str::to_string),
    }
}

/// Extracts native SOL transfers above THRESHOLD_LAMPORTS from a
/// jsonParsed block — both top-level instructions and transfers nested
/// inside program CPIs (inner instructions), e.g. SOL moved as a side
/// effect of a DEX swap.
pub fn extract_transfers(block: &Value, slot: u64) -> Vec<Transfer> {
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

        let top_level_instructions = tx
            .pointer("/transaction/message/instructions")
            .and_then(|v| v.as_array());

        if let Some(instructions) = top_level_instructions {
            for ix in instructions {
                if let Some(parsed) = parse_system_transfer(ix) {
                    // A top-level System Program transfer is a plain
                    // wallet-to-wallet transfer; it has no triggering
                    // program worth labeling.
                    transfers.push(build_transfer(
                        signature, slot, block_time, parsed, false, None,
                    ));
                }
            }
        }

        let Some(inner_groups) = tx
            .pointer("/meta/innerInstructions")
            .and_then(|v| v.as_array())
        else {
            continue;
        };

        for group in inner_groups {
            let Some(inner_instructions) = group.get("instructions").and_then(|v| v.as_array())
            else {
                continue;
            };

            // `index` points at the top-level instruction that triggered
            // this group of inner instructions via CPI — that's the
            // program we attribute the resulting transfer to.
            let triggering_program_id = group
                .get("index")
                .and_then(|v| v.as_u64())
                .and_then(|idx| top_level_instructions.and_then(|ixs| ixs.get(idx as usize)))
                .and_then(|ix| ix.get("programId"))
                .and_then(|v| v.as_str());
            let program_label = triggering_program_id.and_then(labels::label_program);

            for ix in inner_instructions {
                if let Some(parsed) = parse_system_transfer(ix) {
                    transfers.push(build_transfer(
                        signature,
                        slot,
                        block_time,
                        parsed,
                        true,
                        program_label,
                    ));
                }
            }
        }
    }

    transfers
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn system_transfer_ix(source: &str, destination: &str, lamports: u64) -> Value {
        json!({
            "program": "system",
            "programId": "11111111111111111111111111111111",
            "parsed": {
                "type": "transfer",
                "info": { "source": source, "destination": destination, "lamports": lamports }
            }
        })
    }

    fn block_with(signature: &str, top_level: Vec<Value>, inner_groups: Vec<Value>) -> Value {
        json!({
            "blockTime": 1_700_000_000,
            "transactions": [{
                "transaction": {
                    "signatures": [signature],
                    "message": { "instructions": top_level }
                },
                "meta": { "innerInstructions": inner_groups }
            }]
        })
    }

    #[test]
    fn top_level_transfer_above_threshold_is_captured() {
        let block = block_with(
            "sig1",
            vec![system_transfer_ix("A", "B", 2_000_000_000)],
            vec![],
        );
        let transfers = extract_transfers(&block, 1);
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].lamports, 2_000_000_000);
        assert!(!transfers[0].is_inner);
        assert_eq!(transfers[0].program_label, None);
    }

    #[test]
    fn transfer_below_threshold_is_ignored() {
        let block = block_with(
            "sig2",
            vec![system_transfer_ix("A", "B", 500_000_000)],
            vec![],
        );
        assert!(extract_transfers(&block, 1).is_empty());
    }

    #[test]
    fn non_system_instruction_is_ignored() {
        let ix = json!({ "program": "spl-token", "parsed": { "type": "transfer", "info": {} } });
        let block = block_with("sig3", vec![ix], vec![]);
        assert!(extract_transfers(&block, 1).is_empty());
    }

    #[test]
    fn inner_instruction_transfer_is_captured_and_labeled() {
        let top_level = vec![json!({
            "program": "unknown",
            "programId": "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P",
            "parsed": { "type": "swap", "info": {} }
        })];
        let inner_groups = vec![json!({
            "index": 0,
            "instructions": [system_transfer_ix("A", "B", 3_000_000_000)]
        })];
        let block = block_with("sig4", top_level, inner_groups);
        let transfers = extract_transfers(&block, 1);
        assert_eq!(transfers.len(), 1);
        assert!(transfers[0].is_inner);
        assert_eq!(transfers[0].program_label.as_deref(), Some("pump.fun"));
    }

    #[test]
    fn inner_instruction_with_unknown_program_has_no_label() {
        let top_level = vec![json!({
            "program": "unknown",
            "programId": "SomeUnknownProgramId111111111111111111111",
            "parsed": { "type": "swap", "info": {} }
        })];
        let inner_groups = vec![json!({
            "index": 0,
            "instructions": [system_transfer_ix("A", "B", 3_000_000_000)]
        })];
        let block = block_with("sig5", top_level, inner_groups);
        let transfers = extract_transfers(&block, 1);
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].program_label, None);
    }
}
