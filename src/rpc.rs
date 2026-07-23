use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Free, no-API-key-required public endpoints, verified reachable and
/// jsonParsed-getBlock-capable at write time. Both are commonly rate
/// limited under sustained load, which is exactly why there's more than
/// one — see `RpcClient::call`.
pub const ENDPOINTS: &[&str] = &[
    "https://api.mainnet-beta.solana.com",
    "https://solana.publicnode.com",
];

pub struct RpcClient {
    http: reqwest::blocking::Client,
    // Which endpoint to try first on the next call. Advances past whichever
    // endpoint most recently rate-limited us, so a bad endpoint doesn't get
    // hit again on every single call.
    next_endpoint: AtomicUsize,
}

impl RpcClient {
    pub fn new() -> Result<Self, String> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| format!("failed to build http client: {e}"))?;
        Ok(Self {
            http,
            next_endpoint: AtomicUsize::new(0),
        })
    }

    /// True for transport-level failures (network error, rate limit, junk
    /// response) where trying a different endpoint might succeed. False for
    /// well-formed JSON-RPC errors (e.g. "slot skipped") — that's a real
    /// answer about chain state, not an endpoint problem, so callers should
    /// see it rather than have it silently retried elsewhere.
    fn is_retryable(err: &str) -> bool {
        err.starts_with("transport:")
    }

    fn call_once(&self, endpoint: &str, method: &str, params: &Value) -> Result<Value, String> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        let resp = self
            .http
            .post(endpoint)
            .json(&body)
            .send()
            .map_err(|e| format!("transport: request failed: {e}"))?;

        if resp.status() == 429 {
            return Err("transport: rate limited".to_string());
        }

        let value: Value = resp
            .json()
            .map_err(|e| format!("transport: bad json response: {e}"))?;

        if let Some(err) = value.get("error") {
            return Err(format!("rpc error: {err}"));
        }

        value
            .get("result")
            .cloned()
            .ok_or_else(|| "missing result field".to_string())
    }

    /// Tries each configured endpoint in turn on transport-level failures
    /// (rate limit, network error, junk response), so hitting a rate limit
    /// on one public endpoint doesn't stall the whole poll loop — it just
    /// moves on to the next and keeps going.
    pub fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let start = self.next_endpoint.load(Ordering::Relaxed) % ENDPOINTS.len();
        let mut last_err = "no endpoints configured".to_string();

        for offset in 0..ENDPOINTS.len() {
            let idx = (start + offset) % ENDPOINTS.len();
            match self.call_once(ENDPOINTS[idx], method, &params) {
                Ok(value) => {
                    self.next_endpoint.store(idx, Ordering::Relaxed);
                    return Ok(value);
                }
                Err(e) if Self::is_retryable(&e) => {
                    last_err = e;
                    self.next_endpoint
                        .store((idx + 1) % ENDPOINTS.len(), Ordering::Relaxed);
                }
                Err(e) => return Err(e),
            }
        }

        Err(format!("all endpoints failed: {last_err}"))
    }
}

pub fn get_slot(client: &RpcClient) -> Result<u64, String> {
    let result = client.call("getSlot", json!([]))?;
    result
        .as_u64()
        .ok_or_else(|| "getSlot: result not a number".to_string())
}

pub fn get_block(client: &RpcClient, slot: u64) -> Result<Option<Value>, String> {
    let params = json!([
        slot,
        {
            "encoding": "jsonParsed",
            "transactionDetails": "full",
            "maxSupportedTransactionVersion": 0,
            "rewards": false
        }
    ]);

    match client.call("getBlock", params) {
        Ok(value) => Ok(Some(value)),
        // Skipped slots and slots not yet available both surface as RPC
        // errors here; neither should stop the poll loop.
        Err(e) if e.contains("skipped") || e.contains("not available") => Ok(None),
        Err(e) => Err(e),
    }
}
