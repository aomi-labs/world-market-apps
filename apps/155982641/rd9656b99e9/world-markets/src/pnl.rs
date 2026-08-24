//! Account- and position-level PnL.
//!
//! Open perpetual PnL is computed from live contract state: signed quantity ×
//! (mark − entry), minus unpaid funding (`owed_nom`). That needs no store.
//!
//! Closed-position and realized (true-up / partial-close) PnL disappears from
//! the exchange once the position is gone or the entry is reset, so this module
//! keeps a small local ledger. Persistence lives in an app-local JSON file
//! (`WORLD_PNL_DIR`, else `$XDG_DATA_HOME/aomi/world-markets/pnl`, else
//! `~/.local/share/aomi/world-markets/pnl`) until Aomi offers a host store.
//! The ledger is keyed by World account id and records each position's lifetime
//! only — not arbitrary calendar windows.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::client::{Account, PerpetualPosition, WorldClient};
use crate::reporting::Figure;

const QUOTE: &str = "USDT";
const MAX_CLOSED: usize = 50;
const LEDGER_VERSION: u32 = 1;
const WINDOW: &str = "position_lifetime";
const COVERAGE: &str = "perpetual_positions";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct PersistenceInfo {
    pub(crate) store: &'static str,
    pub(crate) path: String,
    pub(crate) note: String,
    pub(crate) closed_retained: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct PositionPnl {
    pub(crate) position_id: String,
    pub(crate) product: String,
    pub(crate) symbol: String,
    pub(crate) side: String,
    pub(crate) status: &'static str,
    pub(crate) quantity: Figure,
    pub(crate) entry_price: Figure,
    pub(crate) mark_or_exit_price: Figure,
    pub(crate) opened_at_unix: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) closed_at_unix: Option<String>,
    pub(crate) unrealized: Figure,
    pub(crate) realized: Figure,
    pub(crate) funding: Figure,
    pub(crate) total: Figure,
    pub(crate) window: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct AccountPnl {
    pub(crate) unrealized: Figure,
    pub(crate) realized: Figure,
    pub(crate) funding: Figure,
    pub(crate) total: Figure,
    pub(crate) open_positions: u32,
    pub(crate) closed_positions_tracked: u32,
    pub(crate) window: &'static str,
    pub(crate) coverage: &'static str,
    pub(crate) baseline: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct PnlReport {
    pub(crate) quote: String,
    pub(crate) persistence: PersistenceInfo,
    pub(crate) account: AccountPnl,
    pub(crate) positions: Vec<PositionPnl>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct AccountLedger {
    version: u32,
    account_id: u64,
    open: BTreeMap<String, OpenRecord>,
    closed: Vec<ClosedRecord>,
}

impl AccountLedger {
    fn new(account_id: u64) -> Self {
        Self {
            version: LEDGER_VERSION,
            account_id,
            open: BTreeMap::new(),
            closed: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct OpenRecord {
    position_id: String,
    token_id: u32,
    symbol: String,
    side: String,
    quantity: String,
    entry_price: String,
    owed_nom: String,
    funding_start_time: u64,
    first_seen_unix: u64,
    accumulated_realized: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ClosedRecord {
    position_id: String,
    token_id: u32,
    symbol: String,
    side: String,
    quantity: String,
    entry_price: String,
    exit_price: String,
    opened_at_unix: u64,
    closed_at_unix: u64,
    realized: String,
    funding: String,
    total: String,
}

#[derive(Clone, Debug)]
pub(crate) enum PnlLedger {
    File(FilePnlLedger),
    #[allow(dead_code)]
    Memory(MemoryPnlLedger),
}

impl Default for PnlLedger {
    fn default() -> Self {
        Self::File(FilePnlLedger::default())
    }
}

impl PnlLedger {
    fn load(&self, account_id: u64) -> Result<AccountLedger, String> {
        match self {
            Self::File(store) => store.load(account_id),
            Self::Memory(store) => store.load(account_id),
        }
    }

    fn save(&self, ledger: &AccountLedger) -> Result<(), String> {
        match self {
            Self::File(store) => store.save(ledger),
            Self::Memory(store) => store.save(ledger),
        }
    }

    pub(crate) fn describe(&self) -> PersistenceInfo {
        match self {
            Self::File(store) => store.describe(),
            Self::Memory(store) => store.describe(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FilePnlLedger {
    dir: PathBuf,
}

impl Default for FilePnlLedger {
    fn default() -> Self {
        Self {
            dir: default_pnl_dir(),
        }
    }
}

impl FilePnlLedger {
    fn load(&self, account_id: u64) -> Result<AccountLedger, String> {
        let path = self.path_for(account_id);
        match fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).map_err(|e| {
                format!(
                    "[world-markets] failed to parse PnL ledger {}: {e}",
                    path.display()
                )
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(AccountLedger::new(account_id))
            }
            Err(e) => Err(format!(
                "[world-markets] failed to read PnL ledger {}: {e}",
                path.display()
            )),
        }
    }

    fn save(&self, ledger: &AccountLedger) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| {
            format!(
                "[world-markets] failed to create PnL ledger dir {}: {e}",
                self.dir.display()
            )
        })?;
        let path = self.path_for(ledger.account_id);
        let tmp = path.with_extension("json.tmp");
        let raw = serde_json::to_vec_pretty(ledger)
            .map_err(|e| format!("[world-markets] failed to serialize PnL ledger: {e}"))?;
        fs::write(&tmp, raw).map_err(|e| {
            format!(
                "[world-markets] failed to write PnL ledger {}: {e}",
                tmp.display()
            )
        })?;
        fs::rename(&tmp, &path).map_err(|e| {
            format!(
                "[world-markets] failed to publish PnL ledger {}: {e}",
                path.display()
            )
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    fn describe(&self) -> PersistenceInfo {
        PersistenceInfo {
            store: "local_file",
            path: self.dir.display().to_string(),
            note: persistence_note(),
            closed_retained: MAX_CLOSED as u32,
        }
    }

    fn path_for(&self, account_id: u64) -> PathBuf {
        self.dir.join(format!("{account_id}.json"))
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MemoryPnlLedger {
    inner: Arc<Mutex<BTreeMap<u64, AccountLedger>>>,
}

impl MemoryPnlLedger {
    fn load(&self, account_id: u64) -> Result<AccountLedger, String> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| "[world-markets] PnL ledger lock poisoned".to_string())?;
        Ok(guard
            .get(&account_id)
            .cloned()
            .unwrap_or_else(|| AccountLedger::new(account_id)))
    }

    fn save(&self, ledger: &AccountLedger) -> Result<(), String> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| "[world-markets] PnL ledger lock poisoned".to_string())?;
        guard.insert(ledger.account_id, ledger.clone());
        Ok(())
    }

    fn describe(&self) -> PersistenceInfo {
        PersistenceInfo {
            store: "memory",
            path: "memory".to_string(),
            note: persistence_note(),
            closed_retained: MAX_CLOSED as u32,
        }
    }
}

fn persistence_note() -> String {
    "Temporary app-local ledger until Aomi host persistence is agreed. Realized PnL is captured when this app observes a close or true-up. Windows are position lifetime, not calendar ranges.".to_string()
}

pub(crate) fn default_pnl_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("WORLD_PNL_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("aomi/world-markets/pnl");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/share/aomi/world-markets/pnl");
    }
    std::env::temp_dir().join("aomi-world-markets-pnl")
}

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Live price PnL for a signed perp quantity: `qty × (mark − entry)`.
pub(crate) fn price_pnl(
    signed_qty: Decimal,
    entry: Decimal,
    mark: Decimal,
) -> Result<Decimal, String> {
    let delta = mark
        .checked_sub(entry)
        .ok_or_else(|| "[world-markets] mark−entry overflow".to_string())?;
    signed_qty
        .checked_mul(delta)
        .ok_or_else(|| "[world-markets] price PnL overflow".to_string())
}

/// Unpaid funding is a liability when `owed_nom` is positive.
pub(crate) fn funding_pnl(owed_nom: Decimal) -> Decimal {
    -owed_nom
}

pub(crate) fn report(
    client: &WorldClient,
    ledger: &PnlLedger,
    account: &Account,
    position_filter: Option<&str>,
) -> Result<PnlReport, String> {
    let mut marks = BTreeMap::new();
    for position in &account.perpetual_positions {
        ensure_mark(client, &mut marks, position.token_id)?;
    }
    let mut state = ledger.load(account.account_id)?;
    for record in state.open.values() {
        ensure_mark(client, &mut marks, record.token_id)?;
    }
    let snapshot = sync_ledger(&mut state, account, &marks, now_unix())?;
    ledger.save(&state)?;
    Ok(build_report(
        ledger.describe(),
        &state,
        &snapshot,
        position_filter,
    ))
}

struct LiveSnapshot {
    positions: BTreeMap<String, LiveOpen>,
}

struct LiveOpen {
    symbol: String,
    side: String,
    quantity: Decimal,
    entry: Decimal,
    mark: Decimal,
    owed_nom: Decimal,
    funding_start_time: u64,
    first_seen_unix: u64,
    accumulated_realized: Decimal,
}

fn position_id(symbol: &str) -> String {
    format!("perp:{symbol}")
}

fn sync_ledger(
    ledger: &mut AccountLedger,
    account: &Account,
    marks: &BTreeMap<u32, String>,
    now: u64,
) -> Result<LiveSnapshot, String> {
    ledger.account_id = account.account_id;
    ledger.version = LEDGER_VERSION;

    let live: BTreeMap<String, &PerpetualPosition> = account
        .perpetual_positions
        .iter()
        .map(|p| (position_id(&p.symbol), p))
        .collect();

    let previous_ids: Vec<String> = ledger.open.keys().cloned().collect();
    for id in previous_ids {
        if live.contains_key(&id) {
            continue;
        }
        let record = ledger.open.remove(&id).expect("open record exists");
        let mark = mark_for(marks, record.token_id)?;
        close_position(ledger, record, &mark, now)?;
    }

    let mut snapshot = LiveSnapshot {
        positions: BTreeMap::new(),
    };

    for (id, position) in &live {
        let mark = mark_for(marks, position.token_id)?;
        let qty = dec(&position.quantity, "quantity")?;
        let entry = dec(&position.entry_price, "entry_price")?;
        let owed = dec(&position.owed_nom, "owed_nom")?;
        let (first_seen, accumulated) = match ledger.open.get(id) {
            Some(prev) => {
                let prev_qty = dec(&prev.quantity, "quantity")?;
                let prev_entry = dec(&prev.entry_price, "entry_price")?;
                let prev_owed = dec(&prev.owed_nom, "owed_nom")?;
                let prev_realized = dec(&prev.accumulated_realized, "accumulated_realized")?;
                let increment = realized_increment(
                    Snap {
                        quantity: prev_qty,
                        entry: prev_entry,
                        owed_nom: prev_owed,
                        side: prev.side.as_str(),
                    },
                    Snap {
                        quantity: qty,
                        entry,
                        owed_nom: owed,
                        side: position.side.as_str(),
                    },
                    &mark,
                )?;
                (
                    prev.first_seen_unix,
                    add(prev_realized, increment, "accumulated_realized")?,
                )
            }
            None => {
                let opened = if position.funding_start_time == 0 {
                    now
                } else {
                    position.funding_start_time.min(now)
                };
                (opened, Decimal::ZERO)
            }
        };

        ledger.open.insert(
            id.clone(),
            OpenRecord {
                position_id: id.clone(),
                token_id: position.token_id,
                symbol: position.symbol.clone(),
                side: position.side.clone(),
                quantity: position.quantity.clone(),
                entry_price: position.entry_price.clone(),
                owed_nom: position.owed_nom.clone(),
                funding_start_time: position.funding_start_time,
                first_seen_unix: first_seen,
                accumulated_realized: accumulated.normalize().to_string(),
            },
        );
        snapshot.positions.insert(
            id.clone(),
            LiveOpen {
                symbol: position.symbol.clone(),
                side: position.side.clone(),
                quantity: qty,
                entry,
                mark: dec(&mark, "mark")?,
                owed_nom: owed,
                funding_start_time: position.funding_start_time,
                first_seen_unix: first_seen,
                accumulated_realized: accumulated,
            },
        );
    }

    Ok(snapshot)
}

fn realized_increment(prev: Snap<'_>, next: Snap<'_>, mark: &str) -> Result<Decimal, String> {
    let flipped = (!prev.quantity.is_zero() && !next.quantity.is_zero())
        && (prev.quantity.is_sign_positive() != next.quantity.is_sign_positive()
            || prev.side != next.side);

    if flipped {
        let mark_d = dec(mark, "mark")?;
        let price = price_pnl(prev.quantity, prev.entry, mark_d)?;
        return add(price, funding_pnl(prev.owed_nom), "close_on_flip");
    }

    if next.entry != prev.entry {
        let price = price_pnl(prev.quantity, prev.entry, next.entry)?;
        return add(price, funding_pnl(prev.owed_nom), "true_up");
    }

    let closed_qty = prev.quantity - next.quantity;
    if closed_qty.abs() > Decimal::ZERO && closed_qty.abs() <= prev.quantity.abs() {
        let mark_d = dec(mark, "mark")?;
        return price_pnl(closed_qty, prev.entry, mark_d);
    }

    Ok(Decimal::ZERO)
}

struct Snap<'a> {
    quantity: Decimal,
    entry: Decimal,
    owed_nom: Decimal,
    side: &'a str,
}

fn ensure_mark(
    client: &WorldClient,
    marks: &mut BTreeMap<u32, String>,
    token_id: u32,
) -> Result<(), String> {
    if let Entry::Vacant(slot) = marks.entry(token_id) {
        let (_raw, mark) = client.mark_price(token_id)?;
        slot.insert(mark);
    }
    Ok(())
}

fn close_position(
    ledger: &mut AccountLedger,
    record: OpenRecord,
    exit_price: &str,
    now: u64,
) -> Result<(), String> {
    let qty = dec(&record.quantity, "quantity")?;
    let entry = dec(&record.entry_price, "entry_price")?;
    let owed = dec(&record.owed_nom, "owed_nom")?;
    let prior = dec(&record.accumulated_realized, "accumulated_realized")?;
    let exit = dec(exit_price, "exit_price")?;
    let remaining = add(
        price_pnl(qty, entry, exit)?,
        funding_pnl(owed),
        "close_remaining",
    )?;
    let total = add(prior, remaining, "close_total")?;
    let opened_at = if record.funding_start_time == 0 {
        record.first_seen_unix
    } else {
        record.funding_start_time.min(record.first_seen_unix)
    };
    ledger.closed.insert(
        0,
        ClosedRecord {
            position_id: record.position_id,
            token_id: record.token_id,
            symbol: record.symbol,
            side: record.side,
            quantity: record.quantity,
            entry_price: record.entry_price,
            exit_price: exit.normalize().to_string(),
            opened_at_unix: opened_at,
            closed_at_unix: now,
            realized: total.normalize().to_string(),
            funding: funding_pnl(owed).normalize().to_string(),
            total: total.normalize().to_string(),
        },
    );
    ledger.closed.truncate(MAX_CLOSED);
    Ok(())
}

fn build_report(
    persistence: PersistenceInfo,
    ledger: &AccountLedger,
    snapshot: &LiveSnapshot,
    position_filter: Option<&str>,
) -> PnlReport {
    let mut unrealized_sum = Decimal::ZERO;
    let mut realized_sum = Decimal::ZERO;
    let mut funding_sum = Decimal::ZERO;
    let mut realized_is_estimate = false;

    for live in snapshot.positions.values() {
        let price = price_pnl(live.quantity, live.entry, live.mark).unwrap_or(Decimal::ZERO);
        let funding = funding_pnl(live.owed_nom);
        unrealized_sum += price + funding;
        funding_sum += funding;
        realized_sum += live.accumulated_realized;
        if live.accumulated_realized != Decimal::ZERO {
            realized_is_estimate = true;
        }
    }
    for closed in &ledger.closed {
        if let Ok(total) = dec(&closed.total, "total") {
            realized_sum += total;
            realized_is_estimate = true;
        }
        if let Ok(funding) = dec(&closed.funding, "funding") {
            funding_sum += funding;
        }
    }

    let mut positions = Vec::new();
    for live in snapshot.positions.values() {
        let id = position_id(&live.symbol);
        if !matches_filter(&id, &live.symbol, position_filter) {
            continue;
        }
        let price = price_pnl(live.quantity, live.entry, live.mark).unwrap_or(Decimal::ZERO);
        let funding = funding_pnl(live.owed_nom);
        let unrealized = price + funding;
        let total = live.accumulated_realized + unrealized;
        let opened_at = if live.funding_start_time == 0 {
            live.first_seen_unix
        } else {
            live.funding_start_time.min(live.first_seen_unix)
        };
        positions.push(PositionPnl {
            position_id: id,
            product: "perp".to_string(),
            symbol: live.symbol.clone(),
            side: live.side.clone(),
            status: "open",
            quantity: Figure::exact(live.quantity.abs(), live.symbol.clone()),
            entry_price: Figure::exact(live.entry, QUOTE),
            mark_or_exit_price: Figure::exact(live.mark, QUOTE),
            opened_at_unix: opened_at.to_string(),
            closed_at_unix: None,
            unrealized: Figure::exact(unrealized, QUOTE),
            realized: Figure::decimal(
                live.accumulated_realized,
                QUOTE,
                live.accumulated_realized != Decimal::ZERO,
            ),
            funding: Figure::exact(funding, QUOTE),
            total: Figure::decimal(total, QUOTE, live.accumulated_realized != Decimal::ZERO),
            window: WINDOW,
        });
    }

    for closed in &ledger.closed {
        if !matches_filter(&closed.position_id, &closed.symbol, position_filter) {
            continue;
        }
        let total = dec(&closed.total, "total").unwrap_or(Decimal::ZERO);
        let qty = dec(&closed.quantity, "quantity").unwrap_or(Decimal::ZERO);
        let entry = dec(&closed.entry_price, "entry").unwrap_or(Decimal::ZERO);
        let exit = dec(&closed.exit_price, "exit").unwrap_or(Decimal::ZERO);
        let funding = dec(&closed.funding, "funding").unwrap_or(Decimal::ZERO);
        positions.push(PositionPnl {
            position_id: closed.position_id.clone(),
            product: "perp".to_string(),
            symbol: closed.symbol.clone(),
            side: closed.side.clone(),
            status: "closed",
            quantity: Figure::decimal(qty.abs(), closed.symbol.clone(), true),
            entry_price: Figure::decimal(entry, QUOTE, true),
            mark_or_exit_price: Figure::decimal(exit, QUOTE, true),
            opened_at_unix: closed.opened_at_unix.to_string(),
            closed_at_unix: Some(closed.closed_at_unix.to_string()),
            unrealized: Figure::exact(Decimal::ZERO, QUOTE),
            realized: Figure::decimal(total, QUOTE, true),
            funding: Figure::decimal(funding, QUOTE, true),
            total: Figure::decimal(total, QUOTE, true),
            window: WINDOW,
        });
    }

    let account_total = unrealized_sum + realized_sum;
    PnlReport {
        quote: QUOTE.to_string(),
        persistence,
        account: AccountPnl {
            unrealized: Figure::exact(unrealized_sum, QUOTE),
            realized: Figure::decimal(realized_sum, QUOTE, realized_is_estimate),
            funding: Figure::decimal(funding_sum, QUOTE, realized_is_estimate),
            total: Figure::decimal(account_total, QUOTE, realized_is_estimate),
            open_positions: snapshot.positions.len() as u32,
            closed_positions_tracked: ledger.closed.len() as u32,
            window: WINDOW,
            coverage: COVERAGE,
            baseline: "Mark versus the contract entry on the current aggregated perp, minus unpaid funding (owed_nom). Realized is captured on observed true-ups and closes. Not a calendar window.".to_string(),
        },
        positions,
    }
}

fn matches_filter(position_id: &str, symbol: &str, filter: Option<&str>) -> bool {
    let Some(filter) = filter.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    position_id.eq_ignore_ascii_case(filter)
        || symbol.eq_ignore_ascii_case(filter)
        || format!("perp:{symbol}").eq_ignore_ascii_case(filter)
}

fn mark_for(marks: &BTreeMap<u32, String>, token_id: u32) -> Result<String, String> {
    marks
        .get(&token_id)
        .cloned()
        .ok_or_else(|| format!("[world-markets] missing mark price for token {token_id}"))
}

fn dec(value: &str, field: &str) -> Result<Decimal, String> {
    Decimal::from_str(value).map_err(|e| format!("[world-markets] {field} is not a decimal: {e}"))
}

fn add(a: Decimal, b: Decimal, field: &str) -> Result<Decimal, String> {
    a.checked_add(b)
        .ok_or_else(|| format!("[world-markets] {field} overflow"))
}

#[cfg(test)]
fn fixture_position(
    symbol: &str,
    token_id: u32,
    qty: &str,
    side: &str,
    entry: &str,
    owed: &str,
) -> PerpetualPosition {
    PerpetualPosition {
        token_id,
        symbol: symbol.to_string(),
        quantity_raw: 0,
        quantity: qty.to_string(),
        side: side.to_string(),
        entry_price_raw: 0,
        entry_price: entry.to_string(),
        funding_start_time: 1_704_067_200,
        owed_nom_raw: "0".to_string(),
        owed_nom: owed.to_string(),
        owed_base_raw: "0".to_string(),
    }
}

#[cfg(test)]
fn fixture_account(id: u64, positions: Vec<PerpetualPosition>) -> Account {
    Account {
        account_id: id,
        owner: "0x0000000000000000000000000000000000000001".to_string(),
        risk_adjusted_portfolio_value_raw: 0,
        risk_adjusted_portfolio_value: "0".to_string(),
        eligible_for_liquidation: false,
        balances: Vec::new(),
        lending_positions: Vec::new(),
        perpetual_positions: positions,
        debt_token_ids: Vec::new(),
        non_debt_token_ids: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    // World docs true-up example: long 2 ETH, entry 3000, mark 3010, funding 0.602.
    #[test]
    fn long_unrealized_matches_world_true_up_example() {
        let price = price_pnl(d("2"), d("3000"), d("3010")).unwrap();
        assert_eq!(price, d("20"));
        let total = add(price, funding_pnl(d("0.602")), "total").unwrap();
        assert_eq!(total, d("19.398"));
    }

    #[test]
    fn short_unrealized_is_negative_when_mark_rises() {
        let price = price_pnl(d("-0.274"), d("77267.42"), d("77707.85")).unwrap();
        assert!(price.is_sign_negative());
        assert_eq!(price.round_dp(2), d("-120.68"));
    }

    #[test]
    fn sync_opens_then_closes_into_ledger() {
        let ledger = PnlLedger::Memory(MemoryPnlLedger::default());
        let mut state = ledger.load(7).unwrap();
        let mut marks = BTreeMap::new();
        marks.insert(4, "3010".to_string());
        let account = fixture_account(
            7,
            vec![fixture_position("WETH", 4, "2", "long", "3000", "0.602")],
        );
        let snap = sync_ledger(&mut state, &account, &marks, 1_800_000_000).unwrap();
        ledger.save(&state).unwrap();
        assert_eq!(snap.positions.len(), 1);
        assert!(state.closed.is_empty());

        let flat = fixture_account(7, vec![]);
        let mut state = ledger.load(7).unwrap();
        sync_ledger(&mut state, &flat, &marks, 1_800_000_100).unwrap();
        ledger.save(&state).unwrap();
        assert!(state.open.is_empty());
        assert_eq!(state.closed.len(), 1);
        assert_eq!(state.closed[0].symbol, "WETH");
        assert_eq!(state.closed[0].total, "19.398");
    }

    #[test]
    fn true_up_moves_unrealized_into_realized() {
        let mut state = AccountLedger::new(1);
        let mut marks = BTreeMap::new();
        marks.insert(4, "3010".to_string());
        let open = fixture_account(
            1,
            vec![fixture_position("WETH", 4, "2", "long", "3000", "0.602")],
        );
        sync_ledger(&mut state, &open, &marks, 10).unwrap();

        marks.insert(4, "3010".to_string());
        let trued = fixture_account(
            1,
            vec![fixture_position("WETH", 4, "2", "long", "3010", "0")],
        );
        let snap = sync_ledger(&mut state, &trued, &marks, 20).unwrap();
        let live = snap.positions.get("perp:WETH").unwrap();
        assert_eq!(live.accumulated_realized, d("19.398"));
        let price = price_pnl(live.quantity, live.entry, live.mark).unwrap();
        assert_eq!(price, Decimal::ZERO);
    }

    #[test]
    fn partial_close_realizes_the_closed_slice() {
        let mut state = AccountLedger::new(1);
        let mut marks = BTreeMap::new();
        marks.insert(4, "3010".to_string());
        let open = fixture_account(
            1,
            vec![fixture_position("WETH", 4, "2", "long", "3000", "0")],
        );
        sync_ledger(&mut state, &open, &marks, 10).unwrap();
        let reduced = fixture_account(
            1,
            vec![fixture_position("WETH", 4, "1", "long", "3000", "0")],
        );
        let snap = sync_ledger(&mut state, &reduced, &marks, 20).unwrap();
        let live = snap.positions.get("perp:WETH").unwrap();
        assert_eq!(live.accumulated_realized, d("10"));
        assert_eq!(
            price_pnl(live.quantity, live.entry, live.mark).unwrap(),
            d("10")
        );
    }

    #[test]
    fn file_ledger_round_trips() {
        let dir = std::env::temp_dir().join(format!("world-pnl-test-{}", now_unix()));
        let store = PnlLedger::File(FilePnlLedger { dir: dir.clone() });
        let mut state = AccountLedger::new(42);
        state.open.insert(
            "perp:WETH".to_string(),
            OpenRecord {
                position_id: "perp:WETH".to_string(),
                token_id: 4,
                symbol: "WETH".to_string(),
                side: "long".to_string(),
                quantity: "1".to_string(),
                entry_price: "2000".to_string(),
                owed_nom: "0".to_string(),
                funding_start_time: 1,
                first_seen_unix: 1,
                accumulated_realized: "0".to_string(),
            },
        );
        store.save(&state).unwrap();
        let loaded = store.load(42).unwrap();
        assert_eq!(loaded.open["perp:WETH"].quantity, "1");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn filter_matches_symbol_or_position_id() {
        assert!(matches_filter("perp:WETH", "WETH", Some("WETH")));
        assert!(matches_filter("perp:WETH", "WETH", Some("perp:WETH")));
        assert!(!matches_filter("perp:WETH", "WETH", Some("WBTC")));
        assert!(matches_filter("perp:WETH", "WETH", None));
    }

    #[test]
    fn build_report_sums_account_and_lists_positions() {
        let mut state = AccountLedger::new(3);
        let mut marks = BTreeMap::new();
        marks.insert(4, "3010".to_string());
        let account = fixture_account(
            3,
            vec![fixture_position("WETH", 4, "2", "long", "3000", "0.602")],
        );
        let snap = sync_ledger(&mut state, &account, &marks, 50).unwrap();
        let report = build_report(
            PersistenceInfo {
                store: "memory",
                path: "memory".to_string(),
                note: persistence_note(),
                closed_retained: MAX_CLOSED as u32,
            },
            &state,
            &snap,
            None,
        );
        assert_eq!(report.account.unrealized.value, "19.398");
        assert_eq!(report.account.total.value, "19.398");
        assert_eq!(report.positions.len(), 1);
        assert_eq!(report.positions[0].position_id, "perp:WETH");
        assert_eq!(report.positions[0].status, "open");
        assert!(!report.account.unrealized.is_estimate);
    }
}
