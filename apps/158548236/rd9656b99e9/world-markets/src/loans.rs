//! Per-loan maturity view. `get_world_account` only exposes aggregated
//! lend/borrow quantities; this module lists individual loans with rate,
//! maturity, remaining time, and whether the lender marked the loan
//! non-extensible.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::client::{Account, Asset, CHAIN_ID, LendingPosition, PackedLoan, WorldClient};
use crate::pnl::now_unix;

/// World loan term: 10 days (240 hours).
pub(crate) const LOAN_TERM_SECS: u64 = 10 * 24 * 60 * 60;

const LEDGER_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct LoanView {
    pub(crate) side: &'static str,
    pub(crate) base_symbol: String,
    pub(crate) rate_apr: String,
    pub(crate) matures_at: String,
    pub(crate) time_remaining_seconds: String,
    pub(crate) extensible: bool,
    pub(crate) counterparty: String,
    pub(crate) quantity_raw: String,
    pub(crate) position_id: String,
    pub(crate) maturity_source: &'static str,
    pub(crate) extensible_source: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct LoansSnapshot {
    pub(crate) source: &'static str,
    pub(crate) chain_id: u64,
    pub(crate) exchange: String,
    pub(crate) block_number: String,
    pub(crate) executable: bool,
    pub(crate) account_id: u64,
    pub(crate) loan_term_seconds: String,
    pub(crate) note: &'static str,
    pub(crate) loans: Vec<LoanView>,
}

#[derive(Clone, Debug)]
pub(crate) enum LoanOriginStore {
    File(FileLoanOriginStore),
    #[allow(dead_code)]
    Memory(MemoryLoanOriginStore),
}

impl Default for LoanOriginStore {
    fn default() -> Self {
        Self::File(FileLoanOriginStore::default())
    }
}

impl LoanOriginStore {
    fn load(&self) -> Result<OriginLedger, String> {
        match self {
            Self::File(store) => store.load(),
            Self::Memory(store) => store.load(),
        }
    }

    fn save(&self, ledger: &OriginLedger) -> Result<(), String> {
        match self {
            Self::File(store) => store.save(ledger),
            Self::Memory(store) => store.save(ledger),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FileLoanOriginStore {
    dir: PathBuf,
}

impl Default for FileLoanOriginStore {
    fn default() -> Self {
        Self {
            dir: default_loan_dir(),
        }
    }
}

impl FileLoanOriginStore {
    fn path(&self) -> PathBuf {
        self.dir.join("origination.json")
    }

    fn load(&self) -> Result<OriginLedger, String> {
        match fs::read_to_string(self.path()) {
            Ok(raw) => serde_json::from_str(&raw).map_err(|e| {
                format!("[world-markets] failed to parse loan origination ledger: {e}")
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(OriginLedger::default()),
            Err(e) => Err(format!(
                "[world-markets] failed to read loan origination ledger: {e}"
            )),
        }
    }

    fn save(&self, ledger: &OriginLedger) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| {
            format!(
                "[world-markets] failed to create loan origination dir {}: {e}",
                self.dir.display()
            )
        })?;
        let path = self.path();
        let tmp = path.with_extension("json.tmp");
        let raw = serde_json::to_vec_pretty(ledger).map_err(|e| {
            format!("[world-markets] failed to serialize loan origination ledger: {e}")
        })?;
        fs::write(&tmp, &raw)
            .map_err(|e| format!("[world-markets] failed to write loan origination ledger: {e}"))?;
        fs::rename(&tmp, &path).map_err(|e| {
            format!("[world-markets] failed to publish loan origination ledger: {e}")
        })?;
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MemoryLoanOriginStore {
    inner: Arc<Mutex<OriginLedger>>,
}

impl MemoryLoanOriginStore {
    fn load(&self) -> Result<OriginLedger, String> {
        self.inner
            .lock()
            .map(|g| g.clone())
            .map_err(|_| "[world-markets] loan origination lock poisoned".to_string())
    }

    fn save(&self, ledger: &OriginLedger) -> Result<(), String> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| "[world-markets] loan origination lock poisoned".to_string())?;
        *guard = ledger.clone();
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct OriginLedger {
    version: u32,
    first_seen: BTreeMap<String, u64>,
}

fn default_loan_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("WORLD_LOAN_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("aomi/world-markets/loans");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/share/aomi/world-markets/loans");
    }
    std::env::temp_dir().join("aomi-world-markets-loans")
}

fn origin_key(account_id: u64, side: &str, token_id: u32, position_id: &str) -> String {
    format!("{account_id}:{side}:{token_id}:{position_id}")
}

fn remember_first_seen(ledger: &mut OriginLedger, key: &str, now: u64) -> u64 {
    ledger.version = LEDGER_VERSION;
    *ledger.first_seen.entry(key.to_string()).or_insert(now)
}

fn remaining_secs(matures_at: u64, now: u64) -> u64 {
    matures_at.saturating_sub(now)
}

fn view_from_packed(
    side: &'static str,
    symbol: &str,
    packed: &PackedLoan,
    first_seen: u64,
    now: u64,
) -> LoanView {
    let (started, maturity_source) = match packed.started_at_unix {
        Some(ts) => (ts, "contract"),
        None => (first_seen, "first_seen_plus_term"),
    };
    let matures_at = started.saturating_add(LOAN_TERM_SECS);
    let (extensible, extensible_source) = if packed.started_at_unix.is_some() {
        (!packed.do_not_return, "contract")
    } else {
        (true, "default")
    };
    let counterparty = if packed.counterparty_id == 0 {
        "unknown".to_string()
    } else {
        packed.counterparty_id.to_string()
    };
    LoanView {
        side,
        base_symbol: symbol.to_string(),
        rate_apr: packed.interest_rate.clone(),
        matures_at: matures_at.to_string(),
        time_remaining_seconds: remaining_secs(matures_at, now).to_string(),
        extensible,
        counterparty,
        quantity_raw: packed.quantity_raw.to_string(),
        position_id: packed.position_id.to_string(),
        maturity_source,
        extensible_source,
    }
}

fn view_from_aggregation(
    side: &'static str,
    position: &LendingPosition,
    position_id: &str,
    first_seen: u64,
    now: u64,
) -> LoanView {
    let matures_at = first_seen.saturating_add(LOAN_TERM_SECS);
    let qty = if side == "lender" {
        position.lender_quantity_raw
    } else {
        position.borrower_quantity_raw
    };
    LoanView {
        side,
        base_symbol: position.symbol.clone(),
        rate_apr: position.highest_interest_rate.clone(),
        matures_at: matures_at.to_string(),
        time_remaining_seconds: remaining_secs(matures_at, now).to_string(),
        extensible: true,
        counterparty: "unknown".to_string(),
        quantity_raw: qty.to_string(),
        position_id: position_id.to_string(),
        maturity_source: "first_seen_plus_term",
        extensible_source: "default",
    }
}

#[allow(dead_code)]
fn collect_side(
    client: &WorldClient,
    account_id: u64,
    asset: &Asset,
    side: &'static str,
    ledger: &mut OriginLedger,
    now: u64,
) -> Result<Vec<LoanView>, String> {
    let ids = match side {
        "lender" => client.lender_position_ids(account_id, asset.token_id),
        _ => client.borrower_position_ids(account_id, asset.token_id),
    };
    let Ok(ids) = ids else {
        return Ok(Vec::new());
    };
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for id in ids {
        let packed = match client.lending_position(id) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let key = origin_key(account_id, side, asset.token_id, &id.to_string());
        let first_seen = remember_first_seen(ledger, &key, now);
        out.push(view_from_packed(
            side,
            &asset.symbol,
            &packed,
            first_seen,
            now,
        ));
    }
    Ok(out)
}

pub(crate) fn snapshot(
    client: &WorldClient,
    origins: &LoanOriginStore,
    account: &Account,
    _assets: &[Asset],
) -> Result<LoansSnapshot, String> {
    let now = now_unix();
    let mut ledger = origins.load()?;
    let mut loans = Vec::new();
    // `readLenderPositions` / `readBorrowerPositions` revert on current UniFi
    // exchange bytecode. Use aggregated lend/borrow rows plus first-seen + 10-day
    // term until those iterators exist on-chain.
    for position in &account.lending_positions {
        if position.lender_quantity_raw > 0 {
            let pid = format!("agg:{}:lender", position.symbol);
            let key = origin_key(account.account_id, "lender", position.token_id, &pid);
            let first_seen = remember_first_seen(&mut ledger, &key, now);
            loans.push(view_from_aggregation(
                "lender", position, &pid, first_seen, now,
            ));
        }
        if position.borrower_quantity_raw > 0 {
            let pid = format!("agg:{}:borrower", position.symbol);
            let key = origin_key(account.account_id, "borrower", position.token_id, &pid);
            let first_seen = remember_first_seen(&mut ledger, &key, now);
            loans.push(view_from_aggregation(
                "borrower", position, &pid, first_seen, now,
            ));
        }
    }

    origins.save(&ledger)?;
    Ok(LoansSnapshot {
        source: "world-markets-contract",
        chain_id: CHAIN_ID,
        exchange: client.exchange(),
        block_number: client.block_number()?.to_string(),
        executable: false,
        account_id: account.account_id,
        loan_term_seconds: LOAN_TERM_SECS.to_string(),
        note: "World loans are a 10-day term. matures_at is unix seconds. When the contract does not expose start time, maturity is first-seen plus 10 days and extensible defaults true.",
        loans,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::U256;

    #[test]
    fn remaining_time_and_extensible_are_present() {
        let mut packed = U256::from(1_000u64);
        packed |= U256::from(42u64) << 64;
        packed |= U256::from(550u64) << 128;
        packed |= U256::from(1_704_067_200u64) << 144;
        let loan = PackedLoan::decode(9, packed);
        let view = view_from_packed("lender", "WETH", &loan, 1_704_067_200, 1_704_067_200);
        assert_eq!(view.time_remaining_seconds, LOAN_TERM_SECS.to_string());
        assert!(view.extensible);
        assert_eq!(view.maturity_source, "contract");
        assert_eq!(view.rate_apr, "0.055");
        assert_eq!(view.counterparty, "42");
    }

    #[test]
    fn aggregated_fallback_exposes_remaining_and_extensible() {
        let position = LendingPosition {
            token_id: 4,
            symbol: "WETH".to_string(),
            lender_quantity_raw: 10,
            lender_quantity: "10".to_string(),
            borrower_quantity_raw: 0,
            borrower_quantity: "0".to_string(),
            highest_interest_rate_raw: 550,
            highest_interest_rate: "0.055".to_string(),
            highest_interest_rate_percent: "5.50".to_string(),
        };
        let view = view_from_aggregation("lender", &position, "agg:WETH:lender", 100, 100);
        assert_eq!(view.time_remaining_seconds, LOAN_TERM_SECS.to_string());
        assert!(view.extensible);
        assert_eq!(view.extensible_source, "default");
    }
}
