//! Yahoo Finance chart API + NASDAQ Trader symbol directories (no API key).

use std::time::Duration;

use reqwest::blocking::Client;
use serde_json::Value;

use super::{
    AssetUniverse, Candle, CandleSeries, ChartRange, FeedError, Instrument, MarketDataFeed,
    now_unix,
};

const NASDAQ_LISTED: &str = "https://www.nasdaqtrader.com/dynamic/SymDir/nasdaqlisted.txt";
const OTHER_LISTED: &str = "https://www.nasdaqtrader.com/dynamic/SymDir/otherlisted.txt";
const CHART_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

#[derive(Clone)]
pub(crate) struct YahooFeed {
    http: Client,
}

impl YahooFeed {
    pub(crate) fn new() -> Self {
        let http = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { http }
    }

    fn get_text(&self, url: &str) -> Result<String, FeedError> {
        let resp = self
            .http
            .get(url)
            .header("Accept", "text/plain, application/json;q=0.9, */*;q=0.8")
            .send()
            .map_err(|e| FeedError::Http(format!("yahoo GET {url}: {e}")))?;
        let status = resp.status();
        let body = resp
            .text()
            .map_err(|e| FeedError::Http(format!("yahoo read {url}: {e}")))?;
        if !status.is_success() {
            return Err(FeedError::Http(format!("yahoo GET {url}: HTTP {status}")));
        }
        Ok(body)
    }

    fn get_json(&self, url: &str) -> Result<Value, FeedError> {
        let body = self.get_text(url)?;
        serde_json::from_str(&body).map_err(|e| FeedError::Parse(format!("yahoo JSON {url}: {e}")))
    }
}

impl Default for YahooFeed {
    fn default() -> Self {
        Self::new()
    }
}

impl MarketDataFeed for YahooFeed {
    fn id(&self) -> &'static str {
        "yahoo"
    }

    fn refresh_universe(&self) -> Result<AssetUniverse, FeedError> {
        let nasdaq = self.get_text(NASDAQ_LISTED)?;
        let other = self.get_text(OTHER_LISTED)?;
        Ok(parse_listed_universe(&nasdaq, &other, now_unix()))
    }

    fn candles(&self, symbol: &str, range: ChartRange) -> Result<CandleSeries, FeedError> {
        let encoded = urlencoding_lite(symbol);
        let url = format!(
            "{CHART_URL}/{encoded}?range={}&interval={}",
            range.yahoo_range(),
            range.yahoo_interval()
        );
        let value = self.get_json(&url)?;
        parse_yahoo_chart(&value, symbol)
    }
}

/// Percent-encode a ticker without pulling in an extra crate.
fn urlencoding_lite(symbol: &str) -> String {
    let mut out = String::with_capacity(symbol.len());
    for ch in symbol.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' => out.push(ch),
            _ => {
                for byte in ch.to_string().as_bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

pub(crate) fn parse_listed_universe(nasdaq: &str, other: &str, fetched_unix: u64) -> AssetUniverse {
    let mut instruments = Vec::new();
    parse_nasdaq_listed(nasdaq, &mut instruments);
    parse_other_listed(other, &mut instruments);
    instruments.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    instruments.dedup_by(|a, b| a.symbol == b.symbol);
    AssetUniverse {
        feed: "yahoo".to_string(),
        fetched_unix,
        instruments,
    }
}

fn parse_nasdaq_listed(body: &str, out: &mut Vec<Instrument>) {
    for line in body.lines().skip(1) {
        if line.starts_with("File Creation") || line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('|').collect();
        if cols.len() < 7 {
            continue;
        }
        if cols[3].eq_ignore_ascii_case("Y") {
            continue;
        }
        let symbol = cols[0].trim();
        if symbol.is_empty() {
            continue;
        }
        let kind = if cols[6].eq_ignore_ascii_case("Y") {
            "etf"
        } else {
            "equity"
        };
        out.push(Instrument {
            symbol: symbol.to_ascii_uppercase(),
            name: cols[1].trim().to_string(),
            kind: kind.to_string(),
            exchange: Some(cols[2].trim().to_string()),
        });
    }
}

fn parse_other_listed(body: &str, out: &mut Vec<Instrument>) {
    for line in body.lines().skip(1) {
        if line.starts_with("File Creation") || line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('|').collect();
        if cols.len() < 7 {
            continue;
        }
        if cols[6].eq_ignore_ascii_case("Y") {
            continue;
        }
        let symbol = cols[0].trim();
        if symbol.is_empty() {
            continue;
        }
        let kind = if cols[4].eq_ignore_ascii_case("Y") {
            "etf"
        } else {
            "equity"
        };
        out.push(Instrument {
            symbol: symbol.to_ascii_uppercase(),
            name: cols[1].trim().to_string(),
            kind: kind.to_string(),
            exchange: Some(cols[2].trim().to_string()),
        });
    }
}

pub(crate) fn parse_yahoo_chart(value: &Value, requested: &str) -> Result<CandleSeries, FeedError> {
    if let Some(err) = value.pointer("/chart/error")
        && !err.is_null()
    {
        let description = err
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("yahoo chart error");
        if description.to_ascii_lowercase().contains("not found")
            || err
                .get("code")
                .and_then(Value::as_str)
                .is_some_and(|c| c.eq_ignore_ascii_case("Not Found"))
        {
            return Err(FeedError::NotFound {
                symbol: requested.to_string(),
            });
        }
        return Err(FeedError::Parse(description.to_string()));
    }
    let result = value
        .pointer("/chart/result/0")
        .ok_or_else(|| FeedError::NotFound {
            symbol: requested.to_string(),
        })?;
    let meta_symbol = result
        .pointer("/meta/symbol")
        .and_then(Value::as_str)
        .unwrap_or(requested);
    let name = result
        .pointer("/meta/shortName")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let timestamps = result
        .get("timestamp")
        .and_then(Value::as_array)
        .ok_or_else(|| FeedError::Parse("yahoo chart missing timestamp".into()))?;
    let quote = result
        .pointer("/indicators/quote/0")
        .ok_or_else(|| FeedError::Parse("yahoo chart missing quote".into()))?;
    let opens = quote.get("open").and_then(Value::as_array);
    let highs = quote.get("high").and_then(Value::as_array);
    let lows = quote.get("low").and_then(Value::as_array);
    let closes = quote.get("close").and_then(Value::as_array);
    let mut candles = Vec::new();
    for (i, ts_val) in timestamps.iter().enumerate() {
        let ts = ts_val
            .as_i64()
            .or_else(|| ts_val.as_u64().map(|n| n as i64));
        let Some(ts) = ts else { continue };
        let (Some(open), Some(high), Some(low), Some(close)) = (
            num_at(opens, i),
            num_at(highs, i),
            num_at(lows, i),
            num_at(closes, i),
        ) else {
            continue;
        };
        if !open.is_finite() || !high.is_finite() || !low.is_finite() || !close.is_finite() {
            continue;
        }
        candles.push(Candle {
            ts,
            open,
            high,
            low,
            close,
        });
    }
    if candles.is_empty() {
        return Err(FeedError::NotFound {
            symbol: requested.to_string(),
        });
    }
    Ok(CandleSeries {
        feed_symbol: meta_symbol.to_ascii_uppercase(),
        name,
        source: "yahoo-finance".to_string(),
        candles,
    })
}

fn num_at(arr: Option<&Vec<Value>>, i: usize) -> Option<f64> {
    arr.and_then(|rows| rows.get(i)).and_then(|v| {
        v.as_f64()
            .or_else(|| v.as_i64().map(|n| n as f64))
            .or_else(|| v.as_u64().map(|n| n as f64))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_nasdaq_and_other_listed() {
        let nasdaq = "\
Symbol|Security Name|Market Category|Test Issue|Financial Status|Round Lot Size|ETF|NextShares
AAPL|Apple Inc. - Common Stock|Q|N|N|100|N|N
ZZZZ|Fake Test Issue|G|Y|N|100|N|N
QQQ|Invesco QQQ Trust|G|N|N|100|Y|N
File Creation Time: 0824202604:01
";
        let other = "\
ACT Symbol|Security Name|Exchange|CQS Symbol|ETF|Round Lot Size|Test Issue|NASDAQ Symbol
BRK.B|Berkshire Hathaway Inc. Class B|N|BRK.B|N|100|N|BRK.B
SPY|SPDR S&P 500|P|SPY|Y|100|N|SPY
File Creation Time: 0824202604:01
";
        let universe = parse_listed_universe(nasdaq, other, 42);
        let symbols: Vec<&str> = universe
            .instruments
            .iter()
            .map(|i| i.symbol.as_str())
            .collect();
        assert_eq!(symbols, vec!["AAPL", "BRK.B", "QQQ", "SPY"]);
        assert_eq!(universe.instruments[2].kind, "etf");
        assert!(!universe.contains("ZZZZ"));
    }

    #[test]
    fn parse_yahoo_chart_fixture() {
        let raw = include_str!("../../tests/fixtures/yahoo_chart_aapl.json");
        let value: Value = serde_json::from_str(raw).unwrap();
        let series = parse_yahoo_chart(&value, "AAPL").unwrap();
        assert_eq!(series.feed_symbol, "AAPL");
        assert_eq!(series.candles.len(), 5);
        assert!((series.candles[0].open - 100.0).abs() < 1e-9);
        assert!((series.candles[4].close - 104.0).abs() < 1e-9);
    }

    #[test]
    fn parse_yahoo_chart_not_found() {
        let value = serde_json::json!({
            "chart": {
                "result": null,
                "error": { "code": "Not Found", "description": "No data found, symbol may be delisted" }
            }
        });
        match parse_yahoo_chart(&value, "NOPE") {
            Err(FeedError::NotFound { symbol }) => assert_eq!(symbol, "NOPE"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    #[ignore = "requires live Yahoo Finance"]
    fn yahoo_chart_live_aapl_and_btc() {
        let feed = YahooFeed::new();
        let aapl = feed.candles("AAPL", ChartRange::Day).unwrap();
        assert!(
            aapl.candles.len() >= 10,
            "AAPL day chart too short: {}",
            aapl.candles.len()
        );
        let btc = feed.candles("BTC-USD", ChartRange::Week).unwrap();
        assert!(
            btc.candles.len() >= 10,
            "BTC-USD week chart too short: {}",
            btc.candles.len()
        );
    }
}
