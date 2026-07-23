use serde_json::{Value, json};

pub const RPC_URL: &str = "https://api.mainnet-beta.solana.com";

pub fn rpc_call(
    client: &reqwest::blocking::Client,
    method: &str,
    params: Value,
) -> Result<Value, String> {
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

pub fn get_slot(client: &reqwest::blocking::Client) -> Result<u64, String> {
    let result = rpc_call(client, "getSlot", json!([]))?;
    result
        .as_u64()
        .ok_or_else(|| "getSlot: result not a number".to_string())
}

pub fn get_block(client: &reqwest::blocking::Client, slot: u64) -> Result<Option<Value>, String> {
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
