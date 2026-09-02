//! Brand candlestick card (1280×720 layout, 2560×1440 PNG) and disposable chart-dir retention.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use image::RgbaImage;
use plotters::prelude::*;
use plotters::style::Color;
use plotters::style::text_anchor::{HPos, Pos, VPos};
use png::{BitDepth, ColorType, Compression, Encoder};

use crate::marketdata::{Candle, CandleSeries, ChartRange};

pub(crate) const CHART_WIDTH: u32 = 1280;
pub(crate) const CHART_HEIGHT: u32 = 720;
#[cfg(test)]
pub(crate) const CHART_SIZE_CAP_BYTES: u64 = 700_000;
const DRAW_SCALE: u32 = 2;
const DRAW_W: u32 = CHART_WIDTH * DRAW_SCALE;
const DRAW_H: u32 = CHART_HEIGHT * DRAW_SCALE;
const DEFAULT_KEEP: usize = 3;
const DEFAULT_TTL_SECS: u64 = 3600;
const MAX_BARS: usize = 96;
const MONTH_BAR_SECS: i64 = 4 * 3600;

const TITLE_FONT: &[u8] = include_bytes!("../assets/fonts/WCMPro-Regular.ttf");
const BODY_FONT: &[u8] = include_bytes!("../assets/fonts/OverusedGrotesk-Regular.ttf");
const LOGO_PNG: &[u8] = include_bytes!("../assets/brand/logomark-white.png");

const CREAM: RGBColor = RGBColor(255, 252, 245);
const UP: RGBColor = RGBColor(0x22, 0xE0, 0x8A);
const DOWN: RGBColor = RGBColor(0xFF, 0x4D, 0x4D);
const INK: RGBColor = RGBColor(0x0B, 0x0A, 0x09);
const WHITE: RGBColor = RGBColor(255, 255, 255);

const PAD_X: i32 = 52;
const PAD_TOP: i32 = 44;
const PAD_BOTTOM: i32 = 36;
const PLOT_W: i32 = 1088;
const PLOT_X0: i32 = PAD_X;
const SVG_W: i32 = 1176;
const HEADER_H: i32 = 84;
const HEADER_CHART_GAP: i32 = 12;
const CHART_ZONE_H: i32 = 498;
const FOOTER_GAP: i32 = 14;
const FOOTER_H: i32 = 32;
const FOOTER_PAD_TOP: i32 = 15;
const FOOTER_CONTENT_H: i32 = 16;
const SVG_H: i32 = 420;
const SVG_PLOT_TOP: i32 = 12;
const SVG_PLOT_BOT: i32 = 356;
const TAG_W_MIN: i32 = 82;
const TAG_RIGHT: i32 = PAD_X + 1176; // content-right; tag grows left from here
const ARROW_GAP: i32 = 10;
const ARROW_W: i32 = 22;

// Visual type (1× CSS px). Plotters ab_glyph + VPos::Center sits glyphs above
// the anchor; `text()` compensates. Boxes use these sizes, not measure().1.
const TICKER_PX: i32 = 56;
const CHIP_PX: i32 = 18;
const SUB_PX: i32 = 18;
const PRICE_PX: i32 = 110;
const DATE_PX: i32 = 18;
const HIST_PX: i32 = 16;
const AXIS_PX: f32 = 18.0;
const TIME_PX: f32 = 16.0;
const HILO_PX: f32 = 18.0;
const TAG_PX: i32 = 32;
const FOOT_PX: i32 = 18;
const BRAND_PX: i32 = 16;

const _: () = assert!(
    PAD_TOP + HEADER_H + HEADER_CHART_GAP + CHART_ZONE_H + FOOTER_GAP + FOOTER_H + PAD_BOTTOM
        == CHART_HEIGHT as i32
);

fn footer_rule_y() -> i32 {
    PAD_TOP + HEADER_H + HEADER_CHART_GAP + CHART_ZONE_H + FOOTER_GAP
}

fn footer_mid_y() -> i32 {
    footer_rule_y() + FOOTER_PAD_TOP + FOOTER_CONTENT_H / 2
}

fn chart_top() -> i32 {
    PAD_TOP + HEADER_H + HEADER_CHART_GAP
}

fn chart_bot() -> i32 {
    chart_top() + CHART_ZONE_H
}

fn svg_origin_y() -> i32 {
    chart_bot() - SVG_H
}

fn plot_top() -> i32 {
    svg_origin_y() + SVG_PLOT_TOP
}

fn plot_bot() -> i32 {
    svg_origin_y() + SVG_PLOT_BOT
}

fn time_label_y() -> i32 {
    chart_bot() - 12
}

static FONTS_OK: OnceLock<bool> = OnceLock::new();
static LOGO: OnceLock<Option<RgbaImage>> = OnceLock::new();

#[inline]
fn s(v: i32) -> i32 {
    v * DRAW_SCALE as i32
}

#[inline]
fn sf(v: f32) -> i32 {
    (v * DRAW_SCALE as f32).round() as i32
}

fn ensure_fonts() -> bool {
    *FONTS_OK.get_or_init(|| {
        let title = plotters::style::register_font("wcm-title", FontStyle::Normal, TITLE_FONT);
        let body = plotters::style::register_font("wcm-body", FontStyle::Normal, BODY_FONT);
        title.is_ok() && body.is_ok()
    })
}

fn logo_image() -> Option<&'static RgbaImage> {
    LOGO.get_or_init(|| {
        image::load_from_memory(LOGO_PNG)
            .ok()
            .map(|img| img.to_rgba8())
    })
    .as_ref()
}

pub(crate) fn chart_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("WORLD_CHART_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("aomi/world-markets/charts");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/share/aomi/world-markets/charts");
    }
    std::env::temp_dir().join("aomi-world-markets/charts")
}

pub(crate) fn chart_keep() -> usize {
    std::env::var("WORLD_CHART_KEEP")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(DEFAULT_KEEP)
}

pub(crate) fn chart_ttl_secs() -> u64 {
    std::env::var("WORLD_CHART_TTL_SECS")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(DEFAULT_TTL_SECS)
}

pub(crate) fn chart_open_enabled() -> bool {
    matches!(
        std::env::var("WORLD_CHART_OPEN")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes"
    )
}

pub(crate) fn render_png(
    series: &CandleSeries,
    range: ChartRange,
    title_symbol: &str,
) -> Result<Vec<u8>, String> {
    let _ = ensure_fonts();
    if series.candles.is_empty() {
        return Err("[world-markets] no candles to chart".to_string());
    }
    let candles = prepare_bars(&series.candles, range);
    let mut buf = vec![0u8; (DRAW_W * DRAW_H * 3) as usize];
    fill_background(&mut buf);
    blit_logomark(
        &mut buf,
        s(CHART_WIDTH as i32 / 2),
        s(chart_top() + (chart_bot() - chart_top()) * 45 / 100),
        330 * DRAW_SCALE,
        0.05,
    );
    let logo_cx;
    {
        let root = BitMapBackend::with_buffer(&mut buf, (DRAW_W, DRAW_H)).into_drawing_area();
        logo_cx = draw_card(&root, &candles, range, title_symbol)?;
        root.present()
            .map_err(|e| format!("[world-markets] chart present: {e}"))?;
    }
    blit_logomark(&mut buf, logo_cx, s(footer_mid_y()), 20 * DRAW_SCALE, 0.90);
    encode_png(&buf)
}

fn prepare_bars(candles: &[Candle], range: ChartRange) -> Vec<Candle> {
    let bars = match range {
        ChartRange::Month => aggregate_secs(candles, MONTH_BAR_SECS),
        _ => candles.to_vec(),
    };
    fit_bars(&bars)
}

fn fit_bars(candles: &[Candle]) -> Vec<Candle> {
    if candles.len() <= MAX_BARS {
        return candles.to_vec();
    }
    let bucket = candles.len().div_ceil(MAX_BARS).max(1);
    candles.chunks(bucket).map(merge_bucket).collect()
}

fn aggregate_secs(candles: &[Candle], secs: i64) -> Vec<Candle> {
    if candles.is_empty() || secs <= 0 {
        return candles.to_vec();
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut key = candles[0].ts.div_euclid(secs);
    for (i, c) in candles.iter().enumerate().skip(1) {
        let next = c.ts.div_euclid(secs);
        if next != key {
            out.push(merge_bucket(&candles[start..i]));
            start = i;
            key = next;
        }
    }
    out.push(merge_bucket(&candles[start..]));
    out
}

fn merge_bucket(chunk: &[Candle]) -> Candle {
    Candle {
        ts: chunk[0].ts,
        open: chunk[0].open,
        high: chunk
            .iter()
            .map(|c| c.high)
            .fold(f64::NEG_INFINITY, f64::max),
        low: chunk.iter().map(|c| c.low).fold(f64::INFINITY, f64::min),
        close: chunk[chunk.len() - 1].close,
    }
}

fn fill_background(buf: &mut [u8]) {
    let w = DRAW_W;
    let h = DRAW_H;
    for y in 0..h {
        let t = y as f32 / (h.saturating_sub(1).max(1) as f32);
        let r = lerp_u8(0x0D, 0x07, t);
        let g = lerp_u8(0x0C, 0x06, t);
        let b = lerp_u8(0x0A, 0x05, t);
        for x in 0..w {
            let i = ((y * w + x) * 3) as usize;
            buf[i] = r;
            buf[i + 1] = g;
            buf[i + 2] = b;
        }
    }
    let border = cream_mix(0.09);
    for x in 0..w {
        put_px(buf, x as i32, 0, border);
        put_px(buf, x as i32, h as i32 - 1, border);
    }
    for y in 0..h {
        put_px(buf, 0, y as i32, border);
        put_px(buf, w as i32 - 1, y as i32, border);
    }
}

fn blit_logomark(buf: &mut [u8], cx: i32, cy: i32, dest_w: u32, opacity: f32) {
    let Some(logo) = logo_image() else {
        return;
    };
    let dest_h = (dest_w as u64 * u64::from(logo.height()) / u64::from(logo.width()).max(1)) as u32;
    let dest_h = dest_h.max(1);
    let resized = image::imageops::resize(
        logo,
        dest_w.max(1),
        dest_h,
        image::imageops::FilterType::Triangle,
    );
    let x0 = cx - dest_w as i32 / 2;
    let y0 = cy - dest_h as i32 / 2;
    for py in 0..dest_h {
        for px in 0..dest_w {
            let p = resized[(px, py)].0;
            let lum =
                (0.30 * f32::from(p[0]) + 0.59 * f32::from(p[1]) + 0.11 * f32::from(p[2])) / 255.0;
            let a = lum * (f32::from(p[3]) / 255.0) * opacity;
            if a < 0.003 {
                continue;
            }
            let x = x0 + px as i32;
            let y = y0 + py as i32;
            blend_px(buf, x, y, (255, 252, 245), a);
        }
    }
}

fn draw_card<DB: DrawingBackend>(
    root: &DrawingArea<DB, plotters::coord::Shift>,
    candles: &[Candle],
    range: ChartRange,
    title_symbol: &str,
) -> Result<i32, String>
where
    DB::ErrorType: std::error::Error,
{
    let n = candles.len();
    let last = candles[n - 1];
    let first = candles[0];
    let mut y_min = candles.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    let mut y_max = candles
        .iter()
        .map(|c| c.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut idx_hi = 0usize;
    let mut idx_lo = 0usize;
    for (i, c) in candles.iter().enumerate() {
        if c.high > candles[idx_hi].high {
            idx_hi = i;
        }
        if c.low < candles[idx_lo].low {
            idx_lo = i;
        }
    }
    if !y_min.is_finite() || !y_max.is_finite() || y_max <= y_min {
        y_min = 0.0;
        y_max = 1.0;
    }
    let pad = (y_max - y_min) * 0.06;
    y_min -= pad;
    y_max += pad;

    let y_of = |price: f64| -> i32 {
        let t = (y_max - price) / (y_max - y_min);
        s(plot_top()) + (t * f64::from(s(plot_bot() - plot_top()))).round() as i32
    };
    let cx = |i: usize| -> i32 {
        let step = f64::from(s(PLOT_W)) / n as f64;
        s(PLOT_X0) + (i as f64 * step + step / 2.0).round() as i32
    };

    let symbol = display_symbol(title_symbol);
    let period_up = last.close >= first.open;
    let last_color = if period_up { UP } else { DOWN };
    let muted = cream_mix(0.48);
    let muted_hi = cream_mix(0.75);
    let axis = cream_mix(0.45);
    let grid = cream_mix(0.07);
    let cream_88 = cream_mix(0.88);
    let cream_42 = cream_mix(0.42);
    let cream_50 = cream_mix(0.50);
    let cream_92 = cream_mix(0.92);

    // Header: ticker + SPOT share one centerline; sub sits under that row.
    let ticker_size = s(TICKER_PX);
    let stack_h = ticker_size + s(6) + s(SUB_PX);
    let stack_top = s(PAD_TOP) + (s(HEADER_H) - stack_h).max(0) / 2;
    let row_cy = stack_top + ticker_size / 2;
    let price_cy = s(PAD_TOP) + s(HEADER_H) / 2;
    let (sym_w, _) = measure(root, &symbol, "wcm-title", ticker_size);
    text(
        root,
        &symbol,
        (s(PAD_X), row_cy + center_nudge("wcm-title", ticker_size)),
        "wcm-title",
        ticker_size,
        CREAM,
        HPos::Left,
        VPos::Center,
    )?;
    let chip = instrument_kind(title_symbol);
    let chip_font = s(CHIP_PX);
    let (chip_tw, _) = measure(root, chip, "wcm-body", chip_font);
    let chip_w = chip_tw + s(10) * 2;
    let chip_h = s(6) + chip_font + s(5);
    let chip_x = s(PAD_X) + sym_w + s(14);
    let chip_y = row_cy - chip_h / 2 - s(2);
    stroke_rect(
        root,
        chip_x,
        chip_y,
        chip_w,
        chip_h,
        cream_mix(0.22),
        DRAW_SCALE,
    )?;
    text(
        root,
        chip,
        (chip_x + chip_w / 2, row_cy - s(2)),
        "wcm-body",
        chip_font,
        cream_mix(0.70),
        HPos::Center,
        VPos::Center,
    )?;
    let sub = format!(
        "{} · UPDATED {} UTC",
        range.bar_label(),
        format_updated(last.ts)
    );
    let sub_font = s(SUB_PX);
    let (sub_w, _) = measure(root, &sub, "wcm-body", sub_font);
    let sub_y = row_cy + ticker_size / 2 + s(6);
    text(
        root,
        &sub,
        (s(PAD_X), sub_y),
        "wcm-body",
        sub_font,
        muted,
        HPos::Left,
        VPos::Top,
    )?;
    let block_w = (sym_w + s(14) + chip_w).max(sub_w);
    let divider_x = s(PAD_X) + block_w + s(28);
    let div_h = s(68);
    line(
        root,
        (divider_x, row_cy - div_h / 2),
        (divider_x, row_cy + div_h / 2),
        cream_mix(0.10),
        DRAW_SCALE,
    )?;
    let price_label = fmt_price(last.close);
    text(
        root,
        &price_label,
        (
            divider_x + s(28),
            price_cy + center_nudge("wcm-body", s(PRICE_PX)),
        ),
        "wcm-body",
        s(PRICE_PX),
        CREAM,
        HPos::Left,
        VPos::Center,
    )?;
    text(
        root,
        &format_range_label(first.ts, last.ts),
        (s(CHART_WIDTH as i32 - PAD_X), s(PAD_TOP + 6)),
        "wcm-body",
        s(DATE_PX),
        muted_hi,
        HPos::Right,
        VPos::Top,
    )?;
    text(
        root,
        "HISTORICAL PRICE",
        (
            s(CHART_WIDTH as i32 - PAD_X),
            s(PAD_TOP + 6) + s(DATE_PX) + s(8),
        ),
        "wcm-body",
        s(HIST_PX),
        cream_mix(0.45),
        HPos::Right,
        VPos::Top,
    )?;

    let plot_x0 = s(PLOT_X0);
    let plot_x1 = s(PLOT_X0 + PLOT_W);
    let last_y = y_of(last.close);
    let axis_right = s(PAD_X + SVG_W);
    let tag_font = s(TAG_PX);
    let (tag_tw, _) = measure(root, &price_label, "wcm-body", tag_font);
    let tag_h = ((tag_font as f32) * 0.62).round() as i32 + s(4);
    let tag_w = (tag_tw + s(10) * 2).max(s(TAG_W_MIN));
    let tag_x = s(TAG_RIGHT) - tag_w;

    for v in nice_ticks(y_min, y_max, 5) {
        let py = y_of(v);
        line(root, (plot_x0, py), (plot_x1, py), grid, 1)?;
        if (py - last_y).abs() > tag_h / 2 + s(6) {
            text(
                root,
                &fmt_price(v),
                (axis_right, py),
                "wcm-body",
                sf(AXIS_PX),
                axis,
                HPos::Right,
                VPos::Center,
            )?;
        }
    }

    let cw = ((f64::from(s(PLOT_W)) / n as f64) * 0.60)
        .floor()
        .clamp(f64::from(s(3)), f64::from(s(12))) as i32;
    let wick = sf(1.4).max(1) as u32;
    let body_min = sf(1.6).max(1);
    for (i, c) in candles.iter().enumerate() {
        let color = if c.close >= c.open { UP } else { DOWN };
        let x = cx(i);
        line(root, (x, y_of(c.high)), (x, y_of(c.low)), color, wick)?;
        let top = y_of(c.open.max(c.close));
        let bot = y_of(c.open.min(c.close));
        let h = (bot - top).abs().max(body_min);
        rect(root, x - cw / 2, top, cw, h, color)?;
    }

    dashed_h(root, last_y, plot_x0, plot_x1, last_color)?;
    rect(root, tag_x, last_y - tag_h / 2, tag_w, tag_h, WHITE)?;
    text(
        root,
        &price_label,
        (
            tag_x + tag_w / 2,
            last_y + center_nudge("wcm-body", tag_font),
        ),
        "wcm-body",
        tag_font,
        INK,
        HPos::Center,
        VPos::Center,
    )?;
    direction_arrow(
        root,
        tag_x + tag_w + s(ARROW_GAP) + s(ARROW_W) / 2,
        last_y,
        period_up,
        last_color,
    )?;

    let hl_font = sf(HILO_PX);
    for (i, is_hi) in [(idx_hi, true), (idx_lo, false)] {
        let c = &candles[i];
        let px = cx(i);
        let py = if is_hi { y_of(c.high) } else { y_of(c.low) };
        root.draw(&Circle::new((px, py), sf(2.5), CREAM.filled()))
            .map_err(|e| format!("[world-markets] chart marker: {e}"))?;
        let word = if is_hi { "HIGH" } else { "LOW" };
        let price = fmt_price(if is_hi { c.high } else { c.low });
        let (ww, _) = measure(root, word, "wcm-body", hl_font);
        let (pw, _) = measure(root, &price, "wcm-body", hl_font);
        let gap = s(7);
        let total = ww + gap + pw;
        let label_cx = px.clamp(s(PAD_X + 70), s(PAD_X + 1010));
        let left = label_cx - total / 2;
        let ly = if is_hi { py - s(8) } else { py + s(26) };
        text(
            root,
            word,
            (left, ly),
            "wcm-body",
            hl_font,
            cream_50,
            HPos::Left,
            VPos::Bottom,
        )?;
        text(
            root,
            &price,
            (left + ww + gap, ly),
            "wcm-body",
            hl_font,
            cream_92,
            HPos::Left,
            VPos::Bottom,
        )?;
    }

    for k in 0..6 {
        let i = ((k as f64 + 0.5) * n as f64 / 6.0).round() as usize;
        let i = i.min(n - 1);
        text(
            root,
            &date_label(candles[i].ts, range),
            (cx(i), s(time_label_y())),
            "wcm-body",
            sf(TIME_PX),
            cream_mix(0.40),
            HPos::Center,
            VPos::Bottom,
        )?;
    }

    let footer_rule = s(footer_rule_y());
    line(
        root,
        (s(PAD_X), footer_rule),
        (axis_right, footer_rule),
        cream_mix(0.09),
        1,
    )?;
    let (hi24, lo24) = window_hl(candles, last.ts.saturating_sub(86_400));
    let footer_y = s(footer_mid_y());
    let foot_font = s(FOOT_PX);
    let hi_lab = "24H HIGH";
    let lo_lab = "24H LOW";
    let hi_val = fmt_price(hi24);
    let lo_val = fmt_price(lo24);
    let (hi_lw, _) = measure(root, hi_lab, "wcm-body", foot_font);
    let (hi_vw, _) = measure(root, &hi_val, "wcm-body", foot_font);
    let (lo_lw, _) = measure(root, lo_lab, "wcm-body", foot_font);
    let gap_in = s(8);
    let gap_grp = s(24);
    let x0 = s(PAD_X);
    text(
        root,
        hi_lab,
        (x0, footer_y),
        "wcm-body",
        foot_font,
        cream_42,
        HPos::Left,
        VPos::Center,
    )?;
    text(
        root,
        &hi_val,
        (x0 + hi_lw + gap_in, footer_y),
        "wcm-body",
        foot_font,
        cream_88,
        HPos::Left,
        VPos::Center,
    )?;
    let lo_x = x0 + hi_lw + gap_in + hi_vw + gap_grp;
    text(
        root,
        lo_lab,
        (lo_x, footer_y),
        "wcm-body",
        foot_font,
        cream_42,
        HPos::Left,
        VPos::Center,
    )?;
    text(
        root,
        &lo_val,
        (lo_x + lo_lw + gap_in, footer_y),
        "wcm-body",
        foot_font,
        cream_88,
        HPos::Left,
        VPos::Center,
    )?;
    let brand = "WORLD MARKETS";
    let (brand_w, _) = measure(root, brand, "wcm-title", s(BRAND_PX));
    text(
        root,
        brand,
        (axis_right, footer_y),
        "wcm-title",
        s(BRAND_PX),
        cream_88,
        HPos::Right,
        VPos::Center,
    )?;
    let logo_cx = axis_right - brand_w - s(10) - s(20) / 2;

    Ok(logo_cx)
}

fn center_nudge(family: &str, size: i32) -> i32 {
    let k = if family == "wcm-title" { 0.192 } else { 0.145 };
    (size as f32 * k).round() as i32
}

#[allow(clippy::too_many_arguments)]
fn text<DB: DrawingBackend>(
    root: &DrawingArea<DB, plotters::coord::Shift>,
    content: &str,
    pos: (i32, i32),
    family: &'static str,
    size: i32,
    color: RGBColor,
    h: HPos,
    v: VPos,
) -> Result<(), String>
where
    DB::ErrorType: std::error::Error,
{
    let style = (family, size).into_font().color(&color).pos(Pos::new(h, v));
    root.draw_text(content, &style, pos)
        .map_err(|e| format!("[world-markets] chart text: {e}"))
}

fn measure<DB: DrawingBackend>(
    root: &DrawingArea<DB, plotters::coord::Shift>,
    content: &str,
    family: &'static str,
    size: i32,
) -> (i32, i32)
where
    DB::ErrorType: std::error::Error,
{
    let style = (family, size).into_text_style(root);
    match root.estimate_text_size(content, &style) {
        Ok((w, h)) => (w as i32, h as i32),
        Err(_) => (approx_width(content, size), size),
    }
}

fn line<DB: DrawingBackend>(
    root: &DrawingArea<DB, plotters::coord::Shift>,
    a: (i32, i32),
    b: (i32, i32),
    color: RGBColor,
    width: u32,
) -> Result<(), String>
where
    DB::ErrorType: std::error::Error,
{
    root.draw(&PathElement::new(vec![a, b], color.stroke_width(width)))
        .map_err(|e| format!("[world-markets] chart line: {e}"))
}

fn rect<DB: DrawingBackend>(
    root: &DrawingArea<DB, plotters::coord::Shift>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: RGBColor,
) -> Result<(), String>
where
    DB::ErrorType: std::error::Error,
{
    root.draw(&Rectangle::new([(x, y), (x + w, y + h)], color.filled()))
        .map_err(|e| format!("[world-markets] chart rect: {e}"))
}

fn stroke_rect<DB: DrawingBackend>(
    root: &DrawingArea<DB, plotters::coord::Shift>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: RGBColor,
    width: u32,
) -> Result<(), String>
where
    DB::ErrorType: std::error::Error,
{
    root.draw(&Rectangle::new(
        [(x, y), (x + w, y + h)],
        color.stroke_width(width),
    ))
    .map_err(|e| format!("[world-markets] chart chip: {e}"))
}

fn direction_arrow<DB: DrawingBackend>(
    root: &DrawingArea<DB, plotters::coord::Shift>,
    cx: i32,
    cy: i32,
    up: bool,
    color: RGBColor,
) -> Result<(), String>
where
    DB::ErrorType: std::error::Error,
{
    let dx = sf(8.5);
    let dy = s(9);
    let pts = if up {
        vec![
            (cx, cy - dy),
            (cx - dx, cy + dy / 2),
            (cx + dx, cy + dy / 2),
        ]
    } else {
        vec![
            (cx, cy + dy),
            (cx - dx, cy - dy / 2),
            (cx + dx, cy - dy / 2),
        ]
    };
    root.draw(&Polygon::new(pts, color.filled()))
        .map_err(|e| format!("[world-markets] chart arrow: {e}"))
}

fn dashed_h<DB: DrawingBackend>(
    root: &DrawingArea<DB, plotters::coord::Shift>,
    y: i32,
    x0: i32,
    x1: i32,
    color: RGBColor,
) -> Result<(), String>
where
    DB::ErrorType: std::error::Error,
{
    let mut x = x0;
    while x < x1 {
        let x2 = (x + s(3)).min(x1);
        line(root, (x, y), (x2, y), color, DRAW_SCALE)?;
        x += s(8);
    }
    Ok(())
}

fn encode_png(rgb: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = Encoder::new(&mut out, DRAW_W, DRAW_H);
    encoder.set_color(ColorType::Rgb);
    encoder.set_depth(BitDepth::Eight);
    encoder.set_compression(Compression::Best);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("[world-markets] png header: {e}"))?;
    writer
        .write_image_data(rgb)
        .map_err(|e| format!("[world-markets] png data: {e}"))?;
    writer
        .finish()
        .map_err(|e| format!("[world-markets] png finish: {e}"))?;
    Ok(out)
}

pub(crate) fn write_chart(
    dir: &Path,
    series: &CandleSeries,
    range: ChartRange,
    title_symbol: &str,
) -> Result<PathBuf, String> {
    fs::create_dir_all(dir)
        .map_err(|e| format!("[world-markets] create chart dir {}: {e}", dir.display()))?;
    let png = render_png(series, range, title_symbol)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let safe = sanitize_symbol(title_symbol);
    let path = dir.join(format!("{safe}_{}_{nanos}.png", range.as_token()));
    fs::write(&path, png)
        .map_err(|e| format!("[world-markets] write chart {}: {e}", path.display()))?;
    Ok(path)
}

pub(crate) fn prune_charts(
    dir: &Path,
    keep: usize,
    ttl_secs: u64,
    now: SystemTime,
) -> Result<u64, String> {
    if !dir.exists() {
        return Ok(0);
    }
    let mut files = list_pngs(dir)?;
    let mut deleted = 0u64;
    if ttl_secs > 0 {
        let mut kept = Vec::new();
        for file in files {
            let age_ok = match now.duration_since(file.modified) {
                Ok(age) => ttl_secs == 0 || age.as_secs() < ttl_secs,
                Err(_) => true,
            };
            if age_ok {
                kept.push(file);
            } else if remove_png(&file.path)? {
                deleted += 1;
            }
        }
        files = kept;
    }
    files.sort_by(|a, b| b.modified.cmp(&a.modified));
    let drop = if keep == 0 {
        files.as_slice()
    } else if files.len() > keep {
        &files[keep..]
    } else {
        &[]
    };
    for file in drop {
        if remove_png(&file.path)? {
            deleted += 1;
        }
    }
    Ok(deleted)
}

pub(crate) fn clear_charts(dir: &Path) -> Result<(u64, PathBuf), String> {
    let mut deleted = 0u64;
    if dir.exists() {
        for file in list_pngs(dir)? {
            if remove_png(&file.path)? {
                deleted += 1;
            }
        }
    }
    Ok((deleted, dir.to_path_buf()))
}

pub(crate) fn clear_charts_tool() -> Result<serde_json::Value, String> {
    let dir = chart_dir();
    let (deleted, directory) = clear_charts(&dir)?;
    Ok(serde_json::json!({
        "ok": true,
        "deleted": deleted,
        "directory": directory.to_string_lossy(),
        "caption": format!("Cleared `{deleted}` chart image(s)."),
        "executable": false,
    }))
}

pub(crate) fn maybe_open(path: &Path) -> bool {
    if !chart_open_enabled() {
        return false;
    }
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(target_os = "linux") {
        Command::new("xdg-open")
    } else {
        return false;
    };
    cmd.arg(path).status().map(|s| s.success()).unwrap_or(false)
}

fn list_pngs(dir: &Path) -> Result<Vec<ChartFile>, String> {
    let mut out = Vec::new();
    let entries = fs::read_dir(dir)
        .map_err(|e| format!("[world-markets] read chart dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("[world-markets] chart dir entry: {e}"))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let meta = entry
            .metadata()
            .map_err(|e| format!("[world-markets] chart metadata {}: {e}", path.display()))?;
        let modified = meta.modified().unwrap_or(UNIX_EPOCH);
        out.push(ChartFile { path, modified });
    }
    Ok(out)
}

fn remove_png(path: &Path) -> Result<bool, String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("[world-markets] delete {}: {e}", path.display())),
    }
}

fn sanitize_symbol(symbol: &str) -> String {
    symbol
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn display_symbol(symbol: &str) -> String {
    if let Some((base, quote)) = symbol.split_once('-') {
        format!("{base} / {quote}")
    } else {
        symbol.to_string()
    }
}

fn instrument_kind(symbol: &str) -> &'static str {
    let upper = symbol.to_ascii_uppercase();
    if upper.contains("PERP") {
        "PERP"
    } else if upper.contains("LOAN") {
        "LOAN"
    } else {
        "SPOT"
    }
}

fn approx_width(text: &str, size: i32) -> i32 {
    let n = text.chars().count() as i32;
    (n * size * 70) / 100
}

fn fmt_price(value: f64) -> String {
    let raw = if value.abs() >= 1000.0 {
        format!("{value:.1}")
    } else if value.abs() >= 10.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.4}")
    };
    format!("${}", add_commas(&raw))
}

fn add_commas(raw: &str) -> String {
    let (sign, rest) = raw.strip_prefix('-').map(|r| ("-", r)).unwrap_or(("", raw));
    let (int, frac) = rest.split_once('.').unwrap_or((rest, ""));
    let mut grouped = String::new();
    for (i, ch) in int.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let int: String = grouped.chars().rev().collect();
    if frac.is_empty() {
        format!("{sign}{int}")
    } else {
        format!("{sign}{int}.{frac}")
    }
}

fn cream_mix(alpha: f32) -> RGBColor {
    RGBColor(
        lerp_u8(0x0D, 255, alpha),
        lerp_u8(0x0C, 252, alpha),
        lerp_u8(0x0A, 245, alpha),
    )
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (f32::from(a) + (f32::from(b) - f32::from(a)) * t.clamp(0.0, 1.0)).round() as u8
}

fn put_px(buf: &mut [u8], x: i32, y: i32, color: RGBColor) {
    if x < 0 || y < 0 || x >= DRAW_W as i32 || y >= DRAW_H as i32 {
        return;
    }
    let i = ((y as u32 * DRAW_W + x as u32) * 3) as usize;
    buf[i] = color.0;
    buf[i + 1] = color.1;
    buf[i + 2] = color.2;
}

fn blend_px(buf: &mut [u8], x: i32, y: i32, fg: (u8, u8, u8), a: f32) {
    if x < 0 || y < 0 || x >= DRAW_W as i32 || y >= DRAW_H as i32 {
        return;
    }
    let i = ((y as u32 * DRAW_W + x as u32) * 3) as usize;
    buf[i] = (f32::from(buf[i]) * (1.0 - a) + f32::from(fg.0) * a).round() as u8;
    buf[i + 1] = (f32::from(buf[i + 1]) * (1.0 - a) + f32::from(fg.1) * a).round() as u8;
    buf[i + 2] = (f32::from(buf[i + 2]) * (1.0 - a) + f32::from(fg.2) * a).round() as u8;
}

fn nice_ticks(min: f64, max: f64, n: usize) -> Vec<f64> {
    let span = (max - min).abs().max(1e-9);
    let raw = span / n as f64;
    let mag = 10.0_f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0]
        .into_iter()
        .map(|s| s * mag)
        .find(|s| span / s <= n as f64 + 1.0)
        .unwrap_or(mag * 10.0);
    let mut v = (min / step).ceil() * step;
    let mut out = Vec::new();
    while v <= max + step * 0.001 {
        out.push(v);
        v += step;
        if out.len() > 12 {
            break;
        }
    }
    out
}

fn window_hl(candles: &[Candle], since: i64) -> (f64, f64) {
    let mut hi = f64::NEG_INFINITY;
    let mut lo = f64::INFINITY;
    for c in candles {
        if c.ts >= since {
            hi = hi.max(c.high);
            lo = lo.min(c.low);
        }
    }
    if !hi.is_finite() {
        for c in candles {
            hi = hi.max(c.high);
            lo = lo.min(c.low);
        }
    }
    (hi, lo)
}

const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

fn split_ts(ts: i64) -> (i32, u32, u32, u32, u32) {
    let secs = ts.max(0) as u64;
    let days = (secs / 86400) as i64;
    let of_day = secs % 86400;
    let hour = (of_day / 3600) as u32;
    let min = ((of_day % 3600) / 60) as u32;
    let (y, m, d) = civil_from_unix_days(days);
    (y, m, d, hour, min)
}

fn format_updated(ts: i64) -> String {
    let (y, m, d, hour, min) = split_ts(ts);
    format!(
        "{} {} {y} {hour:02}:{min:02}",
        d,
        MONTHS[(m.saturating_sub(1) as usize).min(11)]
    )
}

fn format_range_label(start: i64, end: i64) -> String {
    let (_ys, ms, ds, _, _) = split_ts(start);
    let (ye, me, de, _, _) = split_ts(end);
    format!(
        "{} {} — {} {} {ye}",
        ds,
        MONTHS[(ms.saturating_sub(1) as usize).min(11)],
        de,
        MONTHS[(me.saturating_sub(1) as usize).min(11)]
    )
}

fn date_label(ts: i64, range: ChartRange) -> String {
    let (_y, m, d, hour, min) = split_ts(ts);
    match range {
        ChartRange::Day => format!("{hour:02}:{min:02}"),
        ChartRange::Week | ChartRange::Month => {
            format!(
                "{} {} {hour:02}:00",
                d,
                MONTHS[(m.saturating_sub(1) as usize).min(11)]
            )
        }
    }
}

/// Unix epoch day number → UTC month/day (Howard Hinnant civil_from_days).
fn civil_from_unix_days(unix_days: i64) -> (i32, u32, u32) {
    let z = unix_days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

struct ChartFile {
    path: PathBuf,
    modified: SystemTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marketdata::Candle;
    use std::fs::File;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    fn fixture_series() -> CandleSeries {
        let mut candles = Vec::new();
        let mut price = 100.0;
        for i in 0..56 {
            let open: f64 = price;
            let close: f64 = price + if i % 3 == 0 { 1.2 } else { -0.8 };
            let high = open.max(close) + 0.4;
            let low = open.min(close) - 0.3;
            candles.push(Candle {
                ts: 1_700_000_000 + i * 300,
                open,
                high,
                low,
                close,
            });
            price = close;
        }
        CandleSeries {
            feed_symbol: "AAPL".into(),
            name: Some("Apple".into()),
            source: "fixture".into(),
            candles,
        }
    }

    fn unique_dir() -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "aomi-chart-test-{}-{nanos}-{n}",
            std::process::id()
        ))
    }

    #[test]
    fn instrument_chip_defaults_to_spot() {
        assert_eq!(instrument_kind("AAPL"), "SPOT");
        assert_eq!(instrument_kind("BTC-USD"), "SPOT");
        assert_eq!(instrument_kind("ETH-PERP"), "PERP");
        assert_eq!(instrument_kind("USDC-LOAN"), "LOAN");
        assert_eq!(fmt_price(310.34), "$310.34");
        assert_eq!(fmt_price(61400.2), "$61,400.2");
    }

    #[test]
    fn brand_fonts_register() {
        assert!(
            ensure_fonts(),
            "WCM Pro / Overused Grotesk failed to register"
        );
    }

    #[test]
    fn month_bars_aggregate_to_4h() {
        let base = 1_700_000_000 / MONTH_BAR_SECS * MONTH_BAR_SECS;
        let mut candles = Vec::new();
        for i in 0..8 {
            candles.push(Candle {
                ts: base + i * 3600,
                open: 10.0 + i as f64,
                high: 11.0 + i as f64,
                low: 9.0 + i as f64,
                close: 10.5 + i as f64,
            });
        }
        let bars = prepare_bars(&candles, ChartRange::Month);
        assert_eq!(
            bars.len(),
            2,
            "8 hourly bars should collapse to two 4H candles"
        );
        assert_eq!(bars[0].open, 10.0);
        assert_eq!(bars[0].close, 13.5);
        assert_eq!(bars[1].open, 14.0);
    }

    #[test]
    fn fit_bars_caps_series() {
        let candles: Vec<Candle> = (0..400)
            .map(|i| Candle {
                ts: i,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
            })
            .collect();
        let fit = fit_bars(&candles);
        assert!(fit.len() <= MAX_BARS, "fit {}", fit.len());
        assert!(fit.len() >= MAX_BARS / 2);
    }

    #[test]
    fn renders_compact_png() {
        let png = render_png(&fixture_series(), ChartRange::Day, "AAPL").unwrap();
        assert!(
            png.starts_with(&[0x89, b'P', b'N', b'G']),
            "missing PNG magic"
        );
        assert_eq!(
            &png[16..24],
            &[0, 0, 0x0A, 0, 0, 0, 0x05, 0xA0],
            "expected 2560×1440"
        );
        assert!(
            (png.len() as u64) < CHART_SIZE_CAP_BYTES,
            "chart too large: {} bytes",
            png.len()
        );
        assert!(png.len() > 32, "chart too small to be a real image");
    }

    #[test]
    fn prune_keeps_n_newest() {
        let dir = unique_dir();
        fs::create_dir_all(&dir).unwrap();
        let now = SystemTime::now();
        for i in 0..5 {
            let path = dir.join(format!("c{i}.png"));
            fs::write(&path, b"\x89PNG").unwrap();
            let mtime = now - Duration::from_secs(10 + i as u64);
            let file = File::options().write(true).open(&path).unwrap();
            let _ = file.set_modified(mtime);
        }
        let deleted = prune_charts(&dir, 3, 3600, now).unwrap();
        assert_eq!(deleted, 2);
        let left = fs::read_dir(&dir).unwrap().count();
        assert_eq!(left, 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_ttl_deletes_old() {
        let dir = unique_dir();
        fs::create_dir_all(&dir).unwrap();
        let now = SystemTime::now();
        let fresh = dir.join("fresh.png");
        let stale = dir.join("stale.png");
        fs::write(&fresh, b"\x89PNG").unwrap();
        fs::write(&stale, b"\x89PNG").unwrap();
        File::options()
            .write(true)
            .open(&fresh)
            .unwrap()
            .set_modified(now)
            .unwrap();
        File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(now - Duration::from_secs(7200))
            .unwrap();
        let deleted = prune_charts(&dir, 10, 3600, now).unwrap();
        assert_eq!(deleted, 1);
        assert!(fresh.exists());
        assert!(!stale.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keep_zero_deletes_all() {
        let dir = unique_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.png"), b"\x89PNG").unwrap();
        fs::write(dir.join("b.png"), b"\x89PNG").unwrap();
        fs::write(dir.join("notes.txt"), b"leave me").unwrap();
        let deleted = prune_charts(&dir, 0, 3600, SystemTime::now()).unwrap();
        assert_eq!(deleted, 2);
        assert!(dir.join("notes.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_wipes_pngs_only() {
        let dir = unique_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.png"), b"\x89PNG").unwrap();
        fs::write(dir.join("b.png"), b"\x89PNG").unwrap();
        fs::write(dir.join("keep.json"), b"{}").unwrap();
        let (deleted, path) = clear_charts(&dir).unwrap();
        assert_eq!(deleted, 2);
        assert_eq!(path, dir);
        assert!(dir.join("keep.json").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_then_prune_respects_keep() {
        let dir = unique_dir();
        let series = fixture_series();
        for _ in 0..4 {
            write_chart(&dir, &series, ChartRange::Day, "AAPL").unwrap();
        }
        prune_charts(&dir, 2, 3600, SystemTime::now()).unwrap();
        let pngs = fs::read_dir(&dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .ok()
                    .and_then(|e| e.path().extension().map(|x| x == "png"))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(pngs, 2);
        let _ = fs::remove_dir_all(&dir);
    }
}
