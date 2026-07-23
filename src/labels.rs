//! Small, best-effort, hand-curated lookup tables — not comprehensive.
//! See README "Known limitations".
//!
//! IMPORTANT: the program IDs below were filled in from memory, not looked
//! up live against an on-chain or third-party registry at write time.
//! Verify each one against a source you trust (e.g. the program's own docs,
//! or its labeled entry on Solscan/Solana Explorer) before relying on the
//! label it produces — a wrong ID here just means a swap silently shows up
//! as "Wallet transfer" (label lookup misses), not a false positive, but
//! it's still worth double-checking. Add more entries here as you identify
//! them.

const KNOWN_PROGRAMS: &[(&str, &str)] = &[
    ("11111111111111111111111111111111", "System Program"),
    (
        "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4",
        "Jupiter Aggregator",
    ),
    (
        "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8",
        "Raydium AMM",
    ),
    ("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P", "pump.fun"),
    (
        "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc",
        "Orca Whirlpool",
    ),
];

// Wallet/treasury addresses deliberately left empty: attributing a specific
// address to a named real-world entity (an exchange, a foundation) without a
// verifiable source risks asserting something false. Add entries here only
// once you've confirmed them against a source you trust (Solscan's labeled
// accounts, an exchange's published deposit addresses, etc).
const KNOWN_ADDRESSES: &[(&str, &str)] = &[];

fn lookup(table: &'static [(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    table
        .iter()
        .find(|(id, _)| *id == key)
        .map(|(_, name)| *name)
}

pub fn label_program(id: &str) -> Option<&'static str> {
    lookup(KNOWN_PROGRAMS, id)
}

pub fn label_address(addr: &str) -> Option<&'static str> {
    lookup(KNOWN_ADDRESSES, addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_program_resolves() {
        assert_eq!(
            label_program("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"),
            Some("pump.fun")
        );
    }

    #[test]
    fn unknown_program_is_none() {
        assert_eq!(
            label_program("SomeRandomProgramIdThatIsNotKnown11111111"),
            None
        );
    }

    #[test]
    fn unknown_address_is_none() {
        assert_eq!(
            label_address("SomeRandomWalletThatIsNotKnown1111111111"),
            None
        );
    }
}
