//! Persisted negative-carry regime state.
//!
//! This plugin owns the struct and the read/write. The host runtime owns the
//! daily cadence that invokes `check_negative_carry` and the push when `fired`
//! flips. The plugin cannot schedule.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::client::{Account, WorldClient};
use crate::pnl::now_unix;
use crate::rates::{
    RatesSnapshot, daily_carry_from_annual, parse_rate, snapshot as rates_snapshot,
};
use crate::reporting::{CarryState, Figure};

const LEDGER_VERSION: u32 = 1;
const DEFAULT_WINDOW_DAYS: u32 = 3;

#[derive(Clone, Debug)]
pub(crate) enum CarryLedger {
    File(FileCarryLedger),
    #[allow(dead_code)]
    Memory(MemoryCarryLedger),
}

impl Default for CarryLedger {
    fn default() -> Self {
        Self::File(FileCarryLedger::default())
    }
}

impl CarryLedger {
    fn load(&self) -> Result<CarryBook, String> {
        match self {
            Self::File(store) => store.load(),
            Self::Memory(store) => store.load(),
        }
    }

    fn save(&self, book: &CarryBook) -> Result<(), String> {
        match self {
            Self::File(store) => store.save(book),
            Self::Memory(store) => store.save(book),
        }
    }

    #[cfg(test)]
    fn memory() -> Self {
        Self::Memory(MemoryCarryLedger::default())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FileCarryLedger {
    dir: PathBuf,
}

impl Default for FileCarryLedger {
    fn default() -> Self {
        Self {
            dir: default_carry_dir(),
        }
    }
}

impl FileCarryLedger {
    fn path(&self) -> PathBuf {
        self.dir.join("carry.json")
    }

    fn load(&self) -> Result<CarryBook, String> {
        match fs::read_to_string(self.path()) {
            Ok(raw) => serde_json::from_str(&raw)
                .map_err(|e| format!("[world-markets] failed to parse carry ledger: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CarryBook::default()),
            Err(e) => Err(format!("[world-markets] failed to read carry ledger: {e}")),
        }
    }

    fn save(&self, book: &CarryBook) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| {
            format!(
                "[world-markets] failed to create carry ledger dir {}: {e}",
                self.dir.display()
            )
        })?;
        let path = self.path();
        let tmp = path.with_extension("json.tmp");
        let raw = serde_json::to_vec_pretty(book)
            .map_err(|e| format!("[world-markets] failed to serialize carry ledger: {e}"))?;
        fs::write(&tmp, &raw)
            .map_err(|e| format!("[world-markets] failed to write carry ledger: {e}"))?;
        fs::rename(&tmp, &path)
            .map_err(|e| format!("[world-markets] failed to publish carry ledger: {e}"))?;
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MemoryCarryLedger {
    inner: Arc<Mutex<CarryBook>>,
}

impl MemoryCarryLedger {
    fn load(&self) -> Result<CarryBook, String> {
        self.inner
            .lock()
            .map(|g| g.clone())
            .map_err(|_| "[world-markets] carry ledger lock poisoned".to_string())
    }

    fn save(&self, book: &CarryBook) -> Result<(), String> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| "[world-markets] carry ledger lock poisoned".to_string())?;
        *guard = book.clone();
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CarryBook {
    version: u32,
    positions: BTreeMap<String, CarryRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CarryRecord {
    position_id: String,
    entry_timestamp: u64,
    negative_carry_window_days: u32,
    days_negative: u32,
    avg_daily_carry: String,
    fired: bool,
    last_check_utc_day: Option<u64>,
}

fn default_carry_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("WORLD_CARRY_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("aomi/world-markets/carry");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/share/aomi/world-markets/carry");
    }
    std::env::temp_dir().join("aomi-world-markets-carry")
}

fn default_window() -> u32 {
    std::env::var("WORLD_NEGATIVE_CARRY_WINDOW_DAYS")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_WINDOW_DAYS)
}

fn utc_day(ts: u64) -> u64 {
    ts / 86_400
}

pub(crate) fn symbol_from_position_id(position_id: &str) -> String {
    position_id
        .rsplit([':', '/', '-'])
        .next()
        .unwrap_or(position_id)
        .to_string()
}

fn store_key(account_id: Option<u64>, position_id: &str) -> String {
    match account_id {
        Some(id) => format!("{id}:{position_id}"),
        None => format!("anon:{position_id}"),
    }
}

fn to_state(record: &CarryRecord) -> CarryState {
    CarryState {
        position_id: record.position_id.clone(),
        entry_timestamp: record.entry_timestamp.to_string(),
        negative_carry_window_days: record.negative_carry_window_days,
        days_negative: record.days_negative,
        trigger_days: record.negative_carry_window_days,
        avg_daily_carry: Figure::decimal(
            parse_rate(&record.avg_daily_carry).unwrap_or(Decimal::ZERO),
            "",
            true,
        ),
        fired: record.fired,
        plan_executed: record.fired,
        cadence_owner: "runtime",
    }
}

fn apply_observation(record: &mut CarryRecord, daily_carry: Option<Decimal>, now: u64) {
    let day = utc_day(now);
    if let Some(carry) = daily_carry {
        record.avg_daily_carry = carry.normalize().to_string();
        if record.last_check_utc_day == Some(day) {
            return;
        }
        record.last_check_utc_day = Some(day);
        if carry < Decimal::ZERO {
            record.days_negative = record.days_negative.saturating_add(1);
        } else {
            record.days_negative = 0;
        }
    }
    if record.days_negative >= record.negative_carry_window_days {
        record.fired = true;
    }
}

fn live_daily_carry(client: &WorldClient, position_id: &str) -> Result<Option<Decimal>, String> {
    let symbol = symbol_from_position_id(position_id);
    let snap = rates_snapshot(client, Some(&[symbol]))?;
    let Some(row) = snap.rates.first() else {
        return Ok(None);
    };
    let Some(spread) = row.basis_spread_apr.as_deref() else {
        return Ok(None);
    };
    Ok(Some(daily_carry_from_annual(parse_rate(spread)?)))
}

pub(crate) fn check(
    client: &WorldClient,
    ledger: &CarryLedger,
    position_id: &str,
    account_id: Option<u64>,
) -> Result<CarryState, String> {
    let daily = live_daily_carry(client, position_id)?;
    persist_observation(ledger, position_id, account_id, daily, now_unix())
}

pub(crate) fn check_open_perps(
    ledger: &CarryLedger,
    account: &Account,
    rates: &RatesSnapshot,
) -> Result<Vec<CarryState>, String> {
    let mut out = Vec::new();
    let now = now_unix();
    for perp in &account.perpetual_positions {
        let id = format!("perp:{}", perp.symbol);
        let daily = rates
            .rates
            .iter()
            .find(|row| row.base_symbol.eq_ignore_ascii_case(&perp.symbol))
            .and_then(|row| row.basis_spread_apr.as_deref())
            .map(parse_rate)
            .transpose()?
            .map(daily_carry_from_annual);
        out.push(persist_observation(
            ledger,
            &id,
            Some(account.account_id),
            daily,
            now,
        )?);
    }
    Ok(out)
}

fn persist_observation(
    ledger: &CarryLedger,
    position_id: &str,
    account_id: Option<u64>,
    daily: Option<Decimal>,
    now: u64,
) -> Result<CarryState, String> {
    let key = store_key(account_id, position_id);
    let mut book = ledger.load()?;
    book.version = LEDGER_VERSION;
    let record = book.positions.entry(key).or_insert_with(|| CarryRecord {
        position_id: position_id.to_string(),
        entry_timestamp: now,
        negative_carry_window_days: default_window(),
        days_negative: 0,
        avg_daily_carry: "0".to_string(),
        fired: false,
        last_check_utc_day: None,
    });
    apply_observation(record, daily, now);
    let state = to_state(record);
    ledger.save(&book)?;
    Ok(state)
}

/// Advance persisted state with an already-computed daily carry (tests / host).
#[cfg(test)]
pub(crate) fn observe_daily(
    ledger: &CarryLedger,
    position_id: &str,
    account_id: Option<u64>,
    daily_carry: Decimal,
    now: u64,
) -> Result<CarryState, String> {
    persist_observation(ledger, position_id, account_id, Some(daily_carry), now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_increments_across_days_and_fired_latches() {
        let ledger = CarryLedger::memory();
        let start = 1_704_067_200;
        let neg = parse_rate("-0.0002").unwrap();
        let s1 = observe_daily(&ledger, "perp:WETH", Some(1), neg, start).unwrap();
        assert_eq!(s1.days_negative, 1);
        assert!(!s1.fired);
        let s2 = observe_daily(&ledger, "perp:WETH", Some(1), neg, start + 86_400).unwrap();
        assert_eq!(s2.days_negative, 2);
        let s3 = observe_daily(&ledger, "perp:WETH", Some(1), neg, start + 2 * 86_400).unwrap();
        assert_eq!(s3.days_negative, 3);
        assert!(s3.fired);
        let s4 = observe_daily(
            &ledger,
            "perp:WETH",
            Some(1),
            parse_rate("0.001").unwrap(),
            start + 3 * 86_400,
        )
        .unwrap();
        assert_eq!(s4.days_negative, 0);
        assert!(s4.fired, "fired must latch and never reset");
        assert!(s4.plan_executed);
    }

    #[test]
    fn same_utc_day_does_not_double_count() {
        let ledger = CarryLedger::memory();
        let start = 1_704_067_200;
        let neg = parse_rate("-0.0002").unwrap();
        observe_daily(&ledger, "perp:WETH", None, neg, start).unwrap();
        let again = observe_daily(&ledger, "perp:WETH", None, neg, start + 60).unwrap();
        assert_eq!(again.days_negative, 1);
    }

    #[test]
    fn parses_asset_from_position_id() {
        assert_eq!(symbol_from_position_id("perp:WETH"), "WETH");
        assert_eq!(symbol_from_position_id("WETH"), "WETH");
    }
}
