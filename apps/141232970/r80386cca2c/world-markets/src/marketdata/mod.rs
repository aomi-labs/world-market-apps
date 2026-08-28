//! Pluggable OHLC market-data feeds, universe cache, and ticker resolution.
//!
//! The Yahoo Finance implementation needs no API key. A later vendor (Polygon,
//! etc.) implements [`MarketDataFeed`] and is selected with
//! `WORLD_MARKET_DATA_FEED`.

mod yahoo;

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub(crate) use yahoo::YahooFeed;

const DEFAULT_UNIVERSE_TTL_SECS: u64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChartRange {
    Day,
    Week,
    Month,
}

impl ChartRange {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        let token = raw.trim().trim_start_matches('/').to_ascii_lowercase();
        match token.as_str() {
            "d" | "day" | "1d" => Some(Self::Day),
            "w" | "week" | "1w" => Some(Self::Week),
            "m" | "month" | "1m" => Some(Self::Month),
            _ => None,
        }
    }

    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::Day => "d",
            Self::Week => "w",
            Self::Month => "m",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Day => "1D",
            Self::Week => "1W",
            Self::Month => "1M",
        }
    }

    pub(crate) fn yahoo_range(self) -> &'static str {
        match self {
            Self::Day => "1d",
            Self::Week => "5d",
            Self::Month => "1mo",
        }
    }

    pub(crate) fn yahoo_interval(self) -> &'static str {
        match self {
            Self::Day => "5m",
            Self::Week => "15m",
            Self::Month => "60m",
        }
    }

    pub(crate) fn bar_label(self) -> &'static str {
        match self {
            Self::Day => "5M",
            Self::Week => "15M",
            Self::Month => "4H",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Instrument {
    pub(crate) symbol: String,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) exchange: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AssetUniverse {
    pub(crate) feed: String,
    pub(crate) fetched_unix: u64,
    pub(crate) instruments: Vec<Instrument>,
}

impl AssetUniverse {
    pub(crate) fn contains(&self, symbol: &str) -> bool {
        self.instruments
            .iter()
            .any(|row| row.symbol.eq_ignore_ascii_case(symbol))
    }

    pub(crate) fn remember(&mut self, symbol: &str, name: &str, kind: &str) {
        if self.contains(symbol) {
            return;
        }
        self.instruments.push(Instrument {
            symbol: symbol.to_ascii_uppercase(),
            name: name.to_string(),
            kind: kind.to_string(),
            exchange: None,
        });
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Candle {
    pub(crate) ts: i64,
    pub(crate) open: f64,
    pub(crate) high: f64,
    pub(crate) low: f64,
    pub(crate) close: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct CandleSeries {
    pub(crate) feed_symbol: String,
    pub(crate) name: Option<String>,
    pub(crate) source: String,
    pub(crate) candles: Vec<Candle>,
}

impl CandleSeries {
    pub(crate) fn caption(&self, requested: &str, range: ChartRange) -> String {
        let (first, last) = match (self.candles.first(), self.candles.last()) {
            (Some(first), Some(last)) => (first.close, last.close),
            _ => return format!("{requested} {} — no bars.", range.label()),
        };
        let change_pct = if first.abs() > f64::EPSILON {
            (last - first) / first * 100.0
        } else {
            0.0
        };
        format!(
            "{} {} `{}` `{}`.",
            requested,
            range.label(),
            fmt_price(last),
            fmt_change_pct(change_pct)
        )
    }

    pub(crate) fn first_last(&self) -> Option<(f64, f64)> {
        Some((self.candles.first()?.close, self.candles.last()?.close))
    }
}

#[derive(Debug)]
pub(crate) enum FeedError {
    NotFound { symbol: String },
    Http(String),
    Parse(String),
}

impl std::fmt::Display for FeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { symbol } => {
                write!(f, "[world-markets] unknown market-data symbol {symbol}")
            }
            Self::Http(msg) | Self::Parse(msg) => write!(f, "[world-markets] {msg}"),
        }
    }
}

pub(crate) trait MarketDataFeed {
    fn id(&self) -> &'static str;
    fn refresh_universe(&self) -> Result<AssetUniverse, FeedError>;
    fn candles(&self, symbol: &str, range: ChartRange) -> Result<CandleSeries, FeedError>;
}

/// World CLOB tickers → Yahoo-style symbols. Applied after an exact universe hit.
const WORLD_ALIASES: &[(&str, &str)] = &[
    ("WETH", "ETH-USD"),
    ("ETH", "ETH-USD"),
    ("WBTC", "BTC-USD"),
    ("BTC.B", "BTC-USD"),
    ("BTC", "BTC-USD"),
    ("SOL", "SOL-USD"),
    ("USDT", "USDT-USD"),
    ("USDC", "USDC-USD"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedTicker {
    pub(crate) requested: String,
    pub(crate) feed_symbol: String,
}

pub(crate) fn normalize_ticker(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches(['$', '/']);
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_ascii_uppercase())
}

pub(crate) fn chart_startapp(symbol: &str, range: ChartRange) -> String {
    let safe: String = symbol
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    format!("{}_{}", safe, range.as_token())
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn parse_chart_startapp(raw: &str) -> Option<(String, ChartRange)> {
    let raw = raw.trim();
    let (sym, per) = raw.rsplit_once('_')?;
    let range = ChartRange::parse(per)?;
    let symbol = normalize_ticker(sym)?;
    Some((symbol, range))
}

/// Whole-message `{ticker} {d|w|m}`. Lone `d` is not a chart.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn parse_chart_lookup(message: &str) -> Option<(String, ChartRange)> {
    let mut parts: Vec<&str> = message.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let period_raw = parts.pop()?;
    let range = ChartRange::parse(period_raw)?;
    let ticker = normalize_ticker(&parts.join(" "))?;
    Some((ticker, range))
}

pub(crate) fn resolve_ticker(requested: &str, universe: &AssetUniverse) -> ResolvedTicker {
    if universe.contains(requested) {
        return ResolvedTicker {
            requested: requested.to_string(),
            feed_symbol: requested.to_string(),
        };
    }
    let feed_symbol = world_alias(requested)
        .map(str::to_string)
        .unwrap_or_else(|| requested.to_string());
    ResolvedTicker {
        requested: requested.to_string(),
        feed_symbol,
    }
}

pub(crate) fn world_alias(symbol: &str) -> Option<&'static str> {
    WORLD_ALIASES
        .iter()
        .find(|(from, _)| from.eq_ignore_ascii_case(symbol))
        .map(|(_, to)| *to)
}

pub(crate) fn feed_from_env() -> Result<Box<dyn MarketDataFeed>, String> {
    let raw = std::env::var("WORLD_MARKET_DATA_FEED").unwrap_or_default();
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "yahoo" => Ok(Box::new(YahooFeed::new())),
        other => Err(format!(
            "[world-markets] unknown market-data feed {other:?}; supported: yahoo"
        )),
    }
}

pub(crate) fn universe_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("WORLD_UNIVERSE_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    xdg_subdir("universe")
}

pub(crate) fn universe_path(feed_id: &str) -> PathBuf {
    universe_dir().join(format!("{feed_id}.json"))
}

pub(crate) fn universe_ttl_secs() -> u64 {
    env_u64("WORLD_UNIVERSE_TTL_SECS", DEFAULT_UNIVERSE_TTL_SECS)
}

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(crate) fn is_stale(universe: &AssetUniverse, now: u64, ttl_secs: u64) -> bool {
    now.saturating_sub(universe.fetched_unix) >= ttl_secs
}

pub(crate) fn load_universe(path: &std::path::Path) -> Result<Option<AssetUniverse>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("[world-markets] read universe {}: {e}", path.display()))?;
    let parsed: AssetUniverse = serde_json::from_str(&raw)
        .map_err(|e| format!("[world-markets] parse universe {}: {e}", path.display()))?;
    Ok(Some(parsed))
}

pub(crate) fn save_universe(
    path: &std::path::Path,
    universe: &AssetUniverse,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "[world-markets] create universe dir {}: {e}",
                parent.display()
            )
        })?;
    }
    let raw = serde_json::to_string(universe)
        .map_err(|e| format!("[world-markets] serialize universe: {e}"))?;
    fs::write(path, raw)
        .map_err(|e| format!("[world-markets] write universe {}: {e}", path.display()))
}

pub(crate) fn load_or_refresh(feed: &dyn MarketDataFeed) -> Result<AssetUniverse, String> {
    let path = universe_path(feed.id());
    let ttl = universe_ttl_secs();
    let now = now_unix();
    if let Some(cached) = load_universe(&path)? {
        if !is_stale(&cached, now, ttl) {
            return Ok(cached);
        }
        match feed.refresh_universe() {
            Ok(fresh) => {
                save_universe(&path, &fresh)?;
                return Ok(fresh);
            }
            Err(_) => return Ok(cached),
        }
    }
    let fresh = feed.refresh_universe().map_err(|e| e.to_string())?;
    save_universe(&path, &fresh)?;
    Ok(fresh)
}

pub(crate) fn force_refresh(feed: &dyn MarketDataFeed) -> Result<AssetUniverse, String> {
    let fresh = feed.refresh_universe().map_err(|e| e.to_string())?;
    save_universe(&universe_path(feed.id()), &fresh)?;
    Ok(fresh)
}

pub(crate) fn remember_on_demand(
    universe: &mut AssetUniverse,
    series: &CandleSeries,
) -> Result<(), String> {
    universe.remember(
        &series.feed_symbol,
        series.name.as_deref().unwrap_or(""),
        "on_demand",
    );
    save_universe(&universe_path(&universe.feed), universe)
}

pub(crate) fn render_chart_tool(ticker: &str, period: &str) -> Result<serde_json::Value, String> {
    let range = ChartRange::parse(period)
        .ok_or_else(|| format!("[world-markets] period must be d, w, or m (got {period:?})"))?;
    let requested =
        normalize_ticker(ticker).ok_or_else(|| "[world-markets] ticker is empty".to_string())?;
    let feed = feed_from_env()?;
    let mut universe = load_or_refresh(feed.as_ref())?;
    let resolved = resolve_ticker(&requested, &universe);
    let series = match feed.candles(&resolved.feed_symbol, range) {
        Ok(series) => series,
        Err(FeedError::NotFound { symbol }) => {
            return Ok(serde_json::json!({
                "ok": false,
                "source": feed.id(),
                "symbol": requested,
                "feed_symbol": symbol,
                "period": range.as_token(),
                "image_status": "none",
                "image": serde_json::Value::Null,
                "caption": format!("No chart for `{requested}` — not in the market-data universe."),
                "executable": false,
            }));
        }
        Err(err) => return Err(err.to_string()),
    };
    if !universe.contains(&series.feed_symbol) {
        let _ = remember_on_demand(&mut universe, &series);
    }
    let caption = series.caption(&requested, range);
    let (first, last) = series.first_last().unwrap_or((0.0, 0.0));
    let change_pct = if first.abs() > f64::EPSILON {
        (last - first) / first * 100.0
    } else {
        0.0
    };
    let dir = crate::chart::chart_dir();
    let path = crate::chart::write_chart(&dir, &series, range, &requested)?;
    let opened = crate::chart::maybe_open(&path);
    let pruned = crate::chart::prune_charts(
        &dir,
        crate::chart::chart_keep(),
        crate::chart::chart_ttl_secs(),
        std::time::SystemTime::now(),
    )?;
    let persisted = path.exists();
    Ok(serde_json::json!({
        "ok": true,
        "source": series.source,
        "symbol": requested,
        "feed_symbol": series.feed_symbol,
        "period": range.as_token(),
        "period_label": range.label(),
        "image_status": if persisted { "ready" } else { "ephemeral" },
        "image": "candlestick",
        "image_path": path.to_string_lossy(),
        "opened": opened,
        "pruned": pruned,
        "last": fmt_price(last),
        "change_pct": format!("{change_pct:.2}"),
        "caption": caption,
        "photo_action": "viewer",
        "controls": [{ "label": "Open chart", "action": "mini_app.chart" }],
        "mini_app": {
            "kind": "chart",
            "path": format!(
                "/chart?symbol={}&period={}",
                requested,
                range.as_token()
            ),
            "startapp": chart_startapp(&requested, range),
            "symbol": requested,
            "period": range.as_token(),
        },
        "executable": false,
    }))
}

pub(crate) fn refresh_universe_tool() -> Result<serde_json::Value, String> {
    let feed = feed_from_env()?;
    let universe = force_refresh(feed.as_ref())?;
    Ok(serde_json::json!({
        "ok": true,
        "source": universe.feed,
        "fetched_unix": universe.fetched_unix,
        "count": universe.instruments.len(),
        "path": universe_path(&universe.feed).to_string_lossy(),
        "caption": format!("Market universe `{}` · `{}` symbols.", universe.feed, universe.instruments.len()),
        "executable": false,
    }))
}

fn xdg_subdir(leaf: &str) -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("aomi/world-markets").join(leaf);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".local/share/aomi/world-markets")
            .join(leaf);
    }
    std::env::temp_dir().join("aomi-world-markets").join(leaf)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(default)
}

fn fmt_price(price: f64) -> String {
    if price.abs() >= 1.0 {
        format!("{price:.2}")
    } else {
        format!("{price:.4}")
    }
}

fn fmt_change_pct(pct: f64) -> String {
    if pct > 0.0 {
        format!("+{pct:.2}%")
    } else {
        format!("{pct:.2}%")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_chart_lookup_two_tokens() {
        assert_eq!(
            parse_chart_lookup("AAPL d"),
            Some(("AAPL".into(), ChartRange::Day))
        );
        assert_eq!(
            parse_chart_lookup("$btc-usd w"),
            Some(("BTC-USD".into(), ChartRange::Week))
        );
        assert_eq!(
            parse_chart_lookup("/WETH m"),
            Some(("WETH".into(), ChartRange::Month))
        );
        assert_eq!(
            parse_chart_lookup("AAPL day"),
            Some(("AAPL".into(), ChartRange::Day))
        );
    }

    #[test]
    fn parse_chart_lookup_rejects_lone_period() {
        assert_eq!(parse_chart_lookup("d"), None);
        assert_eq!(parse_chart_lookup("/d"), None);
        assert_eq!(parse_chart_lookup("AAPL"), None);
        assert_eq!(parse_chart_lookup("clear charts"), None);
    }

    #[test]
    fn chart_startapp_round_trips() {
        assert_eq!(chart_startapp("AAPL", ChartRange::Day), "AAPL_d");
        assert_eq!(
            parse_chart_startapp("BTC-USD_w"),
            Some(("BTC-USD".into(), ChartRange::Week))
        );
        assert_eq!(parse_chart_startapp("AAPL"), None);
        assert_eq!(parse_chart_startapp("_d"), None);
    }

    #[test]
    fn chart_range_tokens() {
        assert_eq!(ChartRange::parse("d"), Some(ChartRange::Day));
        assert_eq!(ChartRange::parse("1w"), Some(ChartRange::Week));
        assert_eq!(ChartRange::parse("month"), Some(ChartRange::Month));
        assert_eq!(ChartRange::parse("x"), None);
        assert_eq!(ChartRange::Day.as_token(), "d");
        assert_eq!(ChartRange::Week.label(), "1W");
    }

    #[test]
    fn world_aliases_after_exact_match() {
        let universe = AssetUniverse {
            feed: "yahoo".into(),
            fetched_unix: 1,
            instruments: vec![Instrument {
                symbol: "AAPL".into(),
                name: "Apple".into(),
                kind: "equity".into(),
                exchange: None,
            }],
        };
        assert_eq!(
            resolve_ticker("AAPL", &universe).feed_symbol,
            "AAPL".to_string()
        );
        assert_eq!(
            resolve_ticker("WETH", &universe).feed_symbol,
            "ETH-USD".to_string()
        );
        assert_eq!(
            resolve_ticker("BTC.b", &universe).feed_symbol,
            "BTC-USD".to_string()
        );
    }

    #[test]
    fn universe_round_trip_and_stale_flag() {
        let dir = std::env::temp_dir().join(format!(
            "aomi-universe-test-{}-{}",
            std::process::id(),
            now_unix()
        ));
        let path = dir.join("yahoo.json");
        let universe = AssetUniverse {
            feed: "yahoo".into(),
            fetched_unix: 100,
            instruments: vec![Instrument {
                symbol: "MSFT".into(),
                name: "Microsoft".into(),
                kind: "equity".into(),
                exchange: Some("Q".into()),
            }],
        };
        save_universe(&path, &universe).unwrap();
        let loaded = load_universe(&path).unwrap().unwrap();
        assert_eq!(loaded, universe);
        assert!(!is_stale(&universe, 100 + 10, 86_400));
        assert!(is_stale(&universe, 100 + 86_400, 86_400));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn caption_uses_first_last_close() {
        let series = CandleSeries {
            feed_symbol: "AAPL".into(),
            name: Some("Apple".into()),
            source: "yahoo".into(),
            candles: vec![
                Candle {
                    ts: 1,
                    open: 100.0,
                    high: 101.0,
                    low: 99.0,
                    close: 100.0,
                },
                Candle {
                    ts: 2,
                    open: 100.0,
                    high: 102.0,
                    low: 100.0,
                    close: 101.24,
                },
            ],
        };
        assert_eq!(
            series.caption("AAPL", ChartRange::Day),
            "AAPL 1D `101.24` `+1.24%`."
        );
    }
}
