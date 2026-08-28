/* global Telegram, LightweightCharts, COPY, fillCopy */
const tg = window.Telegram && window.Telegram.WebApp;
const reduceMotion =
  window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
const C = COPY;
/* copy.js already exports global `fill`; use fillCopy to avoid a duplicate binding. */

if (tg) {
  try {
    tg.ready();
    if (typeof tg.expand === "function" && tg.isExpanded) {
      if (typeof tg.disableVerticalSwipes === "function") tg.disableVerticalSwipes();
    }
    if (typeof tg.onEvent === "function") {
      tg.onEvent("viewportChanged", () => {
        if (tg.isExpanded && typeof tg.disableVerticalSwipes === "function") {
          try {
            tg.disableVerticalSwipes();
          } catch (_) {
            /* ignore */
          }
        }
        const was = state.compact;
        state.compact = !tg.isExpanded;
        if (was !== state.compact && state.view === "main") paint();
      });
    }
  } catch (_) {
    /* WebView without a live Telegram host */
  }
}

const app = document.getElementById("app");
let sessionToken = "";
let chartHandle = null;
let candleSeries = null;
let pollTimer = null;
let ageTimer = null;
let ageTimerMs = 250;
let toastTimer = null;
let burstTimer = null;
let suppressClickUntil = 0;
const flushedIds = new Set();
const DISMISS_KEY = "aomi.ledger.dismissed";

const state = {
  view: "main",
  tab: "ledger",
  sheet: null,
  detent: "half",
  openSwipe: "",
  search: "",
  searchOpen: false,
  products: [],
  productId: "",
  earlierOpen: false,
  compose: null,
  sent: null,
  blocked: null,
  toast: null,
  compact: true,
  riskOpen: false,
  flags: {
    primary_view: "ledger",
    jobline_negative: false,
    family: "blue",
    voice_home: true,
    voice_mode: "hold",
    live_words: true,
  },
  portfolio: null,
  ledger: [],
  summary: { holding: 0, needs_you: 0, last_check_at: null },
  ledgerStatus: "loading",
  pending: {},
  optimistic: [],
  fillUx: {},
  dismissed: new Set(),
  voice: {
    phase: "idle",
    heldMs: 0,
    transcript: "",
    transcriptRaw: "",
    nudge: "",
    micDenied: false,
    typePulse: false,
  },
  nearMatch: null,
};

function applyFlags(flags) {
  state.flags = {
    primary_view: "ledger",
    jobline_negative: false,
    family: "blue",
    voice_home: true,
    voice_mode: "hold",
    live_words: true,
    ...(flags || {}),
  };
}

function voiceHomeOn() {
  return state.flags.voice_home !== false;
}

function voiceMode() {
  return state.flags.voice_mode === "tap" ? "tap" : "hold";
}

function liveWordsOn() {
  return state.flags.live_words !== false;
}

function showLiveWords() {
  if (!liveWordsOn()) return false;
  if (state.voice.phase === "listening" || voiceFinalizing) return true;
  return (
    state.voice.phase === "sending" &&
    Boolean(state.voice.transcriptRaw || state.voice.transcript)
  );
}

function isVoiceHome() {
  return voiceHomeOn() && state.view === "main" && state.tab === "ledger" && !state.compact;
}

function showBottomBar() {
  if (state.view !== "main") return true;
  if (state.tab === "portfolio") return true;
  if (state.compact) return true;
  if (!voiceHomeOn()) return true;
  return false;
}

function haptic(kind, arg) {
  const h = tg && tg.HapticFeedback;
  if (!h) return;
  try {
    if (kind === "impact") h.impactOccurred(arg || "light");
    else if (kind === "notify") h.notificationOccurred(arg);
    else if (kind === "select") h.selectionChanged();
  } catch (_) {
    /* no haptics */
  }
}

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function usd(raw) {
  if (raw == null || raw === "") return "—";
  const negative = String(raw).charAt(0) === "-";
  const body = negative ? String(raw).slice(1) : String(raw);
  const parts = body.split(".");
  const grouped = parts[0].replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const frac = parts[1];
  const shown = !frac || frac === "00" ? grouped : grouped + "." + frac;
  return (negative ? "−" : "") + "$" + shown;
}

function newId() {
  if (crypto && crypto.randomUUID) return crypto.randomUUID();
  return "c-" + Date.now() + "-" + Math.random().toString(16).slice(2, 8);
}

function nowSecs() {
  return Math.floor(Date.now() / 1000);
}

function fmtDate(unix) {
  if (!unix) return "—";
  const d = new Date(Number(unix) * 1000);
  const months = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
  return months[d.getUTCMonth()] + " " + d.getUTCDate();
}

function relCheck(at) {
  if (!at) return null;
  return Math.max(0, nowSecs() - Number(at));
}

function previewState() {
  if (location.hostname !== "127.0.0.1" && location.hostname !== "localhost") return null;
  return new URLSearchParams(location.search).get("preview");
}

function startParam() {
  return (
    (tg && tg.initDataUnsafe && tg.initDataUnsafe.start_param) ||
    new URLSearchParams(location.search).get("startapp") ||
    ""
  );
}

function parseChartStart(raw) {
  const m = String(raw || "").trim().match(/^(.+)_([dwm])$/i);
  if (!m) return null;
  return { symbol: m[1], period: m[2].toLowerCase() };
}

function instructionStart(raw) {
  const s = String(raw || "").trim();
  if (!s) return null;
  if (parseChartStart(s)) return null;
  if (s.startsWith("i_")) return s.slice(2);
  return s;
}

function remainingSecs(row) {
  return remainingDisplaySecs(row, Date.now());
}

function queueView(row, nowMs) {
  if (!isQueueRow(row)) return null;
  const now = nowMs != null ? nowMs : Date.now();
  const ux = touchFillUx(row, state.fillUx[row.instruction_id], now, { reduceMotion });
  state.fillUx[row.instruction_id] = ux;
  const presented = ux.waitUntil != null ? { ...row, wait_until_ms: ux.waitUntil } : row;
  return presentQueue(presented, ux, now, { reduceMotion });
}

function adoptFillUx(fromId, toId) {
  if (!fromId || !toId || fromId === toId) return;
  if (state.fillUx[toId] && state.fillUx[toId].waitUntil != null) return;
  if (!state.fillUx[fromId]) return;
  state.fillUx[toId] = { ...state.fillUx[fromId] };
  delete state.fillUx[fromId];
}

function queuedSentenceBetter(current, next) {
  const a = String(current || "").trim();
  const b = String(next || "").trim();
  if (!b) return a;
  if (!a) return b;
  const placeholder =
    typeof window.isPlaceholderTranscript === "function"
      ? window.isPlaceholderTranscript
      : () => false;
  if (placeholder(b) && !placeholder(a)) return a;
  if (placeholder(a) && !placeholder(b)) return b;
  if (b === C.draftRow.hearing && a !== C.draftRow.hearing) return a;
  if (a === C.draftRow.hearing && b !== C.draftRow.hearing) return b;
  return b.length >= a.length ? b : a;
}

function dropOptimistic(correlation_id, instruction_id) {
  const ids = new Set(
    [correlation_id, instruction_id].filter((v) => v != null && String(v)).map(String),
  );
  state.optimistic = state.optimistic.filter(
    (r) => !ids.has(String(r.instruction_id)) && !ids.has(String(r.correlation_id)),
  );
}

function landQueuedTask(text, correlation_id, instruction_id, extra = {}) {
  const id = instruction_id || correlation_id;
  const existing = state.optimistic.find(
    (r) =>
      r.instruction_id === id ||
      (correlation_id && r.correlation_id === correlation_id),
  );
  const sentence = String(text || "").trim();
  if (existing) {
    const next = queuedSentenceBetter(existing.sentence, sentence);
    if (next) existing.sentence = next;
    if (instruction_id && existing.instruction_id !== instruction_id) {
      adoptFillUx(existing.instruction_id, instruction_id);
      existing.instruction_id = instruction_id;
    }
    existing.updated_at = nowSecs();
    tunePoll();
    return existing;
  }
  const now = nowSecs();
  const row = {
    instruction_id: id,
    sentence: sentence || C.draftRow.hearing,
    status: "pending_execute",
    kind: extra.kind || "trade",
    fire_kind: "act",
    correlation_id,
    created_at: now,
    updated_at: now,
    execute_at: now + 3,
    delay_secs: 3,
    queued_local: true,
    voice_landed_at: extra.voice ? Date.now() : undefined,
    ...extra.row,
  };
  state.optimistic = [row].concat(
    state.optimistic.filter((r) => r.instruction_id !== id && r.correlation_id !== correlation_id),
  );
  tunePoll();
  return row;
}

function loadDismissed() {
  try {
    const raw = JSON.parse(localStorage.getItem(DISMISS_KEY) || "[]");
    state.dismissed = new Set((Array.isArray(raw) ? raw : []).map(String));
  } catch (_) {
    state.dismissed = new Set();
  }
}

function rememberDismissed(id) {
  if (!id) return;
  state.dismissed.add(String(id));
  try {
    localStorage.setItem(DISMISS_KEY, JSON.stringify([...state.dismissed].slice(-400)));
  } catch (_) {
    /* quota */
  }
}

loadDismissed();

function clientProgress(row) {
  if (!row) return null;
  const view = queueView(row);
  if (view) return view.showMeter ? view.fillPct : null;
  return row.progress_pct != null ? Number(row.progress_pct) : null;
}

function heldCount() {
  return instructions().filter((row) => {
    const view = queueView(row);
    if (view && view.zone === "queued") return true;
    return [
      "with_aomi",
      "watching",
      "triggered",
      "awaiting_confirm",
      "pending_execute",
      "executing",
      "paused",
    ].includes(row.status);
  }).length;
}

function instructions() {
  const seen = new Set();
  const out = [];
  for (const row of state.optimistic.concat(state.ledger)) {
    if (seen.has(row.instruction_id)) continue;
    if (state.dismissed && state.dismissed.has(String(row.instruction_id))) continue;
    seen.add(row.instruction_id);
    out.push(row);
  }
  return out;
}

function zoneOf(row) {
  const view = queueView(row);
  if (view && view.zone === "queued") return "queued";
  if (row.status === "awaiting_confirm" || row.status === "triggered") {
    return "needs";
  }
  if (row.status === "pending_execute" || row.status === "executing" || row.status === "with_aomi") {
    return "queued";
  }
  if (row.status === "misheard") return "queued";
  if (row.status === "watching" || row.status === "paused") return "watch";
  if (row.status === "cant") return "cant";
  const today = new Date().toISOString().slice(0, 10);
  const changed = new Date((row.status_changed_at || row.updated_at || 0) * 1000)
    .toISOString()
    .slice(0, 10);
  if (row.status === "done" && changed === today) return "done";
  return "earlier";
}

function glyph(row) {
  const view = queueView(row);
  if (view && view.phase === "wait") {
    return { g: view.remainingDisplay == null ? "·" : String(view.remainingDisplay), cls: "count" };
  }
  if (view && (view.phase === "fill" || view.phase === "sliced")) {
    return { g: "", spin: true };
  }
  if (row.status === "misheard") return { g: "·", cls: "faint" };
  if (row.voice_draft) return { g: "›", cls: "warn" };
  if (row.status === "awaiting_confirm" || row.status === "triggered") return { g: "!", cls: "" };
  if (row.status === "with_aomi") return { g: "›", cls: "" };
  if (row.status === "executing") return { g: "", spin: true };
  if (row.status === "pending_execute") {
    const n = remainingSecs(row);
    return { g: n == null ? "·" : String(n), cls: "count" };
  }
  if (row.status === "paused") return { g: "❚❚", cls: "faint" };
  if (row.status === "done") return { g: "✓", cls: "pos" };
  if (row.status === "cant") return { g: "·", cls: "faint" };
  if (row.status === "expired") return { g: "·", cls: "faint" };
  if (row.fire_kind === "act") return { g: "⏱", cls: "" };
  return { g: "◎", cls: "" };
}

function chipClass(status) {
  if (status === "watching") return "pos";
  if (status === "paused") return "mute";
  if (status === "done" || status === "expired" || status === "misheard") return "faint";
  if (status === "cant") return "cant";
  if (status === "blocked") return "neg";
  return "";
}

function cancellable(row) {
  if (!row || state.pending[row.instruction_id] === "cancel") return false;
  return (
    row.status === "watching" ||
    row.status === "paused" ||
    row.status === "with_aomi" ||
    row.status === "awaiting_confirm" ||
    row.status === "triggered" ||
    row.status === "pending_execute" ||
    row.status === "executing"
  );
}

function taskIdOf(row) {
  return row.task_id || row.instruction_id;
}

async function cancelInPlace(row) {
  const id = taskIdOf(row);
  const message = fillCopy(C.drafts.cancel, { id });
  haptic("impact", "light");
  state.pending[row.instruction_id] = "cancel";
  delete state.fillUx[row.instruction_id];
  state.ledger = state.ledger.filter((r) => r.instruction_id !== row.instruction_id);
  state.optimistic = state.optimistic.filter((r) => r.instruction_id !== row.instruction_id);
  if (state.insId === row.instruction_id) {
    state.sheet = null;
    state.insId = null;
  }
  showToast(C.toasts.cancelSent);
  const preview = previewState();
  if (preview && preview !== "dev") return;
  try {
    const initData = (tg && tg.initData) || (preview === "dev" ? "dev" : "");
    await ensureSession(initData);
    await api("/api/v1/mini-app/compose", {
      method: "POST",
      body: {
        kind: "cancel",
        instruction_id: row.instruction_id,
        message,
      },
    });
    refreshLedger();
  } catch (_) {
    showToast(C.toasts.cancelFailed);
  }
}

function archivable(row) {
  return row && (row.status === "done" || row.status === "cant");
}

async function archiveInPlace(rowOrId) {
  const row =
    typeof rowOrId === "string"
      ? instructions().find((r) => r.instruction_id === rowOrId)
      : rowOrId;
  if (!row || !archivable(row)) return;
  haptic("impact", "medium");
  rememberDismissed(row.instruction_id);
  state.ledger = state.ledger.filter((r) => r.instruction_id !== row.instruction_id);
  state.optimistic = state.optimistic.filter((r) => r.instruction_id !== row.instruction_id);
  if (state.openSwipe === row.instruction_id) state.openSwipe = "";
  if (state.insId === row.instruction_id) {
    state.sheet = null;
    state.insId = null;
  }
  showToast(C.toasts.archived);
  paint();
  const preview = previewState();
  if (preview && preview !== "dev") return;
  try {
    const initData = (tg && tg.initData) || (preview === "dev" ? "dev" : "");
    await ensureSession(initData);
    await api("/api/v1/mini-app/compose", {
      method: "POST",
      body: {
        kind: "archive",
        instruction_id: row.instruction_id,
        message: "",
      },
    });
    refreshLedger();
  } catch (_) {
    showToast(C.toasts.archiveFailed);
  }
}

function subLine(row) {
  if (state.pending[row.instruction_id] === "pause") return C.sub.pendingPause;
  if (state.pending[row.instruction_id] === "resume") return C.sub.pendingResume;
  if (row.status === "misheard") return "";
  const view = queueView(row);
  if (view && view.phase === "wait") {
    return fillCopy(C.sub.pendingExecute, { n: view.remainingDisplay == null ? "—" : view.remainingDisplay });
  }
  if (view && view.phase === "fill") return C.sub.completing;
  if (row.voice_draft) return C.draftRow.sub;
  if (row.status === "with_aomi") return C.sub.withAomi;
  if (row.status === "paused") return C.sub.paused;
  if (row.status === "awaiting_confirm" || row.status === "triggered") {
    return row.trigger_value
      ? fillCopy(C.sub.needsYouAt, { value: row.trigger_value })
      : C.sub.needsYou;
  }
  if (row.status === "pending_execute") {
    const n = remainingSecs(row);
    return fillCopy(C.sub.pendingExecute, { n: n == null ? "—" : n });
  }
  if (row.status === "executing") {
    const type = String(row.order_type || (row.params && row.params.order_type) || "")
      .toUpperCase();
    const labeled = type === "TWAP" || type === "DCA";
    return fillCopy(labeled ? C.sub.executing : C.sub.executingMarket, {
      type: labeled ? type : "filling",
      i: row.slice_i || "—",
      n: row.slice_n || "—",
      price: row.avg_price || "—",
    });
  }
  if (row.status === "done" && row.receipt) return row.receipt;
  if (row.status === "cant") {
    if (row.sub_line) return row.sub_line;
    if (row.repeat_count > 1) return fillCopy(C.cant.sublineRepeat, { n: row.repeat_count === 2 ? "twice" : row.repeat_count + " times" });
    return C.cant.subline;
  }
  if (row.status === "expired") return fillCopy(C.sub.expired, { date: fmtDate(row.expires_at) });
  if (row.status === "watching") {
    const n = relCheck(row.check_stats && row.check_stats.last_check_at);
    const stale = n != null && n > 120;
    if (stale) return fillCopy(C.sub.watchingStale, { n: Math.round(n / 60) });
    const dist =
      row.distance && row.distance.mark
        ? fillCopy(C.sub.watchingDist, { mark: usd(row.distance.mark).replace("$", "$"), pct: row.distance.pct })
        : "";
    const body =
      n != null && n >= 60
        ? fillCopy(C.sub.watchingSlow, { n: Math.round(n / 60), date: fmtDate(row.expires_at) })
        : fillCopy(C.sub.watching, { n: n == null ? "—" : n, date: fmtDate(row.expires_at) });
    return dist + body;
  }
  return "";
}

function heartbeatText() {
  if (state.ledgerStatus === "loading") return { text: C.heartbeat.loading, dot: "well" };
  if (state.ledgerStatus === "error") return { text: C.heartbeat.error, dot: "neg" };
  if (state.ledgerStatus === "stale") return { text: C.heartbeat.stale, dot: "warn" };
  const held = state.summary.holding || heldCount();
  const needs = state.summary.needs_you || 0;
  if (!held) return { text: C.heartbeat.empty, dot: "accent" };
  const n = relCheck(state.summary.last_check_at);
  if (needs === 1) return { text: fillCopy(C.heartbeat.holdingNeeds1, { held }), dot: "accent" };
  if (needs > 1) return { text: fillCopy(C.heartbeat.holdingNeedsN, { held, n: needs }), dot: "accent" };
  return {
    text: fillCopy(C.heartbeat.holdingOk, { held, n: n == null ? "—" : n }),
    dot: "accent",
  };
}

function backIconHtml() {
  return `<svg class="back-icon" width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 18l-6-6 6-6"/></svg>`;
}

function hasInAppBack() {
  return Boolean(
    state.searchOpen ||
      state.sheet ||
      (state.view && state.view !== "main") ||
      state.tab === "portfolio",
  );
}

function headerHtml(mode) {
  const showSearch =
    mode === "root" && (state.searchOpen || state.tab === "portfolio" || !voiceHomeOn());
  const search = showSearch ? searchBarHtml() : "";
  const sub = voiceHomeOn() ? C.header.subtitle : C.header.subtitleLedger;
  const hdrCls = "header" + (showSearch ? " with-search" : "");
  const showBack = mode !== "root" || hasInAppBack();
  const backHidden = showBack ? "" : ' style="visibility:hidden" aria-hidden="true" tabindex="-1"';
  return `<header class="${hdrCls}">
    <button type="button" class="header-btn" id="backBtn" aria-label="Back"${backHidden}>${backIconHtml()}</button>
    <div class="header-main">
      <h1 class="header-title">${escapeHtml(C.header.title)}</h1>
      <p class="header-sub">${escapeHtml(sub)}</p>
    </div>
    <button type="button" class="header-btn" id="moreBtn" aria-label="More">⋯</button>
    ${search}
  </header>`;
}

function productKindLabel(product) {
  if (product === "perp") return C.search.perp;
  if (product === "lend") return C.search.lending;
  return C.search.spot;
}

function filterProducts(q) {
  const all = state.products || [];
  const needle = String(q || "").trim().toLowerCase();
  if (!needle) return all.slice();
  return all.filter((row) => {
    const hay = (row.keywords || row.symbol || "").toLowerCase();
    return hay.split(/\s+/).some((tok) => tok.includes(needle));
  });
}

function searchBarHtml() {
  const q = state.search.trim();
  const filtered = filterProducts(q);
  const count = state.searchOpen
    ? q
      ? fillCopy(C.search.matches, { n: filtered.length })
      : fillCopy(C.search.products, { n: (state.products || []).length })
    : "";
  const clear = state.searchOpen
    ? `<button type="button" class="search-x" id="searchClear" aria-label="Close search">✕</button>`
    : "";
  return `<div class="search header-search">
    <span>⌕</span>
    <input id="search" placeholder="${escapeHtml(C.search.placeholder)}" value="${escapeHtml(state.search)}" autocomplete="off" />
    ${count ? `<span class="n">${escapeHtml(count)}</span>` : ""}
    ${clear}
  </div>`;
}

function searchMenuHtml() {
  if (!state.searchOpen) return "";
  const q = state.search.trim();
  const filtered = filterProducts(q);
  if (!state.products.length && !q) {
    return `<div class="search-menu"><p class="edge">${escapeHtml(C.search.loading)}</p></div>`;
  }
  if (q && !filtered.length) {
    return `<div class="search-menu"><p class="edge">${escapeHtml(fillCopy(C.search.noMatch, { q: state.search }))}</p></div>`;
  }
  const groups = [
    ["spot", C.search.spot],
    ["perp", C.search.perp],
    ["lend", C.search.lending],
  ];
  const body = groups
    .map(([key, label]) => {
      const rows = filtered.filter((row) => row.product === key);
      if (!rows.length) return "";
      return `<div class="sec-h">${escapeHtml(label)}</div>${rows.map(productRowHtml).join("")}`;
    })
    .join("");
  return `<div class="search-menu">${body}</div>`;
}

function productRowHtml(row) {
  const held = heldPosition(row);
  const sub = row.product === "lend"
    ? productKindLabel(row.product)
    : fillCopy(C.search.quote, { base: row.symbol, quote: row.quote_symbol || "USDT" });
  const mark = row.mark_price
    ? fillCopy(C.search.mark, { price: usd(row.mark_price) })
    : "";
  return `<button type="button" class="prod-row" data-product="${escapeHtml(row.id)}">
    <div class="glyph">${escapeHtml((row.symbol || "?").slice(0, 2))}</div>
    <div class="row-body">
      <div class="title-row"><div class="title">${escapeHtml(row.symbol)}</div><span class="pct-slot num">${escapeHtml(mark)}</span></div>
      <div class="sub">${escapeHtml(sub)}${held ? `<span class="prod-held">${escapeHtml(C.search.held)}</span>` : ""}</div>
    </div>
  </button>`;
}

function heldPosition(prod) {
  if (!prod) return null;
  const all = (state.portfolio && state.portfolio.positions) || [];
  const want = String(prod.symbol || "")
    .replace(/-PERP$/i, "")
    .toUpperCase();
  for (let idx = 0; idx < all.length; idx++) {
    const row = all[idx];
    const have = String(row.symbol || "")
      .replace(/-PERP$/i, "")
      .toUpperCase();
    const type = row.asset_type === "borrow" ? "lend" : row.asset_type;
    if (have === want && type === prod.product) return { row, idx };
  }
  return null;
}

function findProduct(id) {
  return (state.products || []).find((row) => row.id === id);
}

function bottomHtml() {
  if (!showBottomBar()) return "";
  const label = state.sheet || state.view !== "main" ? C.bottom.inner : C.bottom.launch;
  const mic =
    state.view === "main" && !state.sheet && !isVoiceHome()
      ? `<button type="button" class="voice-btn" id="voiceBtn" aria-label="${escapeHtml(C.voice.hold)}">🎙</button>`
      : "";
  return `<div class="bottom-row"><button type="button" class="bottom-bar" id="bottomBtn">${escapeHtml(label)}</button>${mic}</div>`;
}

function toastHtml() {
  if (!state.toast) return "";
  return `<div class="toast">${escapeHtml(state.toast)}</div>`;
}

function showToast(msg) {
  state.toast = msg;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    state.toast = null;
    paint();
  }, 3200);
  paint();
}

function closeToChat() {
  if (tg && typeof tg.close === "function") tg.close();
}

function goBack() {
  haptic("impact", "light");
  if (state.searchOpen) {
    closeSearch();
    return;
  }
  if (state.sheet === "picker") {
    state.sheet = state.productId ? "product" : "position";
    paint();
    return;
  }
  if (state.sheet) {
    state.sheet = null;
    state.productId = "";
    paint();
    return;
  }
  if (state.view === "chart") {
    destroyChart();
    state.view = "main";
    const url = new URL(location.href);
    url.pathname = "/";
    url.searchParams.delete("symbol");
    url.searchParams.delete("period");
    history.pushState({}, "", url);
    paint();
    return;
  }
  if (state.view !== "main") {
    state.view = "main";
    paint();
    return;
  }
  if (state.tab === "portfolio") {
    state.tab = "ledger";
    paint();
    return;
  }
  closeToChat();
}

function closeSearch() {
  state.searchOpen = false;
  state.search = "";
  paint();
}

function bindSearch() {
  const search = document.getElementById("search");
  function open() {
    if (state.searchOpen) return;
    state.searchOpen = true;
    paint();
    const el = document.getElementById("search");
    if (el) el.focus();
  }
  if (search) {
    search.onfocus = open;
    search.onclick = open;
    search.oninput = () => {
      state.search = search.value;
      state.searchOpen = true;
      paint();
      const el = document.getElementById("search");
      if (el) {
        el.focus();
        el.setSelectionRange(state.search.length, state.search.length);
      }
    };
  }
  const clear = document.getElementById("searchClear");
  if (clear) {
    clear.onclick = (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      closeSearch();
    };
  }
  app.querySelectorAll("[data-product]").forEach((el) => {
    el.onclick = () => openProduct(el.getAttribute("data-product"));
  });
}

function openProduct(id) {
  const prod = findProduct(id);
  if (!prod) return;
  state.searchOpen = false;
  state.search = "";
  const held = heldPosition(prod);
  if (held) {
    state.productId = "";
    state.sheet = "position";
    state.posIdx = held.idx;
  } else {
    state.posIdx = -1;
    state.productId = id;
    state.sheet = "product";
  }
  state.detent = "half";
  paint();
}

function openProductChart(symbol) {
  haptic("select");
  state.searchOpen = false;
  state.sheet = null;
  state.view = "chart";
  const url = new URL(location.href);
  url.pathname = "/chart";
  url.searchParams.set("symbol", symbol);
  url.searchParams.set("period", "d");
  history.pushState({}, "", url);
  loadChartView({ symbol, period: "d" });
}

function bindChrome() {
  const back = document.getElementById("backBtn");
  if (back) back.onclick = goBack;
  const bottom = document.getElementById("bottomBtn");
  if (bottom) bottom.onclick = closeToChat;
  const more = document.getElementById("moreBtn");
  if (more) {
    more.onclick = () => {
      if (voiceHomeOn() && state.tab === "ledger" && !state.searchOpen) {
        state.searchOpen = true;
        paint();
        const el = document.getElementById("search");
        if (el) el.focus();
      }
    };
  }
  bindSearch();
  bindVoice();
  const header = document.querySelector(".header");
  if (header) {
    document.documentElement.style.setProperty("--header-h", header.offsetHeight + "px");
  }
  syncTgBackButton();
}

let tgBackBound = false;
function syncTgBackButton() {
  if (!tg || !tg.BackButton) return;
  if (hasInAppBack()) {
    tg.BackButton.show();
    if (!tgBackBound) {
      tgBackBound = true;
      tg.BackButton.onClick(goBack);
    }
  } else if (typeof tg.BackButton.hide === "function") {
    tg.BackButton.hide();
  }
}

let lastMainPaintKey = "";

function mainPaintKey() {
  const rows = instructions();
  return JSON.stringify({
    view: state.view,
    tab: state.tab,
    compact: state.compact,
    status: state.ledgerStatus,
    holding: state.summary && state.summary.holding,
    needs: state.summary && state.summary.needs_you,
    toast: state.toast,
    sheet: state.sheet,
    openSwipe: state.openSwipe,
    earlier: state.earlierOpen,
    search: state.searchOpen,
    near: state.nearMatch && state.nearMatch.asked_entity,
    voicePhase: state.voice && state.voice.phase,
    rows: rows.map((r) => [
      r.instruction_id,
      r.status,
      r.display_status,
      r.sentence,
      r.progress_pct,
      r.receipt,
      r.execute_at,
      r.updated_at,
      r.status_changed_at,
      r.distance && r.distance.pct,
    ]),
    fill: Object.keys(state.fillUx)
      .sort()
      .map((id) => {
        const u = state.fillUx[id];
        return [id, u && u.revealed, u && u.fillStartedAt, u && u.waitUntil];
      }),
  });
}

function captureLedgerScroll() {
  const led = document.getElementById("homeLedger");
  return {
    ledger: led ? led.scrollTop : 0,
    win: window.scrollY,
    hadLedger: Boolean(led),
  };
}

function restoreLedgerScroll(saved) {
  const led = document.getElementById("homeLedger");
  if (led && saved && saved.hadLedger) led.scrollTop = saved.ledger || 0;
  else if (!led && saved && saved.win) window.scrollTo(0, saved.win);
}

function paint() {
  if (state.voice && state.voice.phase === "listening" && !voiceFinalizing) {
    return;
  }
  if (state.view === "chart") return;
  if (state.view === "compose") return renderCompose();
  if (state.view === "sent") return renderSent();
  if (state.view === "blocked") return renderBlocked();
  renderMain();
}

function renderMain() {
  const voiceHome = isVoiceHome();
  const savedScroll = captureLedgerScroll();
  document.body.className = [
    state.sheet || state.searchOpen ? "locked" : "",
    voiceHome ? "home-v7" : "",
  ]
    .filter(Boolean)
    .join(" ");
  const hb = heartbeatText();
  const held = heldCount();
  const tab = state.tab;
  const compact = state.compact && !state.sheet && tab === "ledger";
  const showSeg = !voiceHomeOn() || tab === "portfolio";
  const inner =
    tab === "ledger"
      ? voiceHome
        ? voiceHomeHtml(hb)
        : ledgerHtml(hb, compact)
      : portfolioHtml(hb);
  app.className = voiceHome ? "home-v7-app" : "";
  app.innerHTML =
    (voiceHome ? `<div class="app-home">` : "") +
    headerHtml("root") +
    (showSeg
      ? `<div class="seg">
      <button type="button" class="${tab === "ledger" ? "on" : ""}" data-tab="ledger">${escapeHtml(C.header.tabLedger)}<span class="held">${held}</span></button>
      <button type="button" class="${tab === "portfolio" ? "on" : ""}" data-tab="portfolio">${escapeHtml(C.header.tabPortfolio)}</button>
    </div>`
      : "") +
    inner +
    (voiceHome ? `</div>` : "") +
    searchMenuHtml() +
    (state.sheet ? sheetHtml() : "") +
    toastHtml() +
    bottomHtml();
  bindChrome();
  app.querySelectorAll("[data-tab]").forEach((btn) => {
    btn.onclick = () => {
      haptic("select");
      state.tab = btn.getAttribute("data-tab");
      state.openSwipe = "";
      paint();
    };
  });
  bindLedger();
  bindPortfolio();
  bindSheet();
  bindHomeActs();
  restoreLedgerScroll(savedScroll);
  lastMainPaintKey = mainPaintKey();
}

function homeHeartbeatText() {
  if (state.ledgerStatus === "loading") return { text: C.heartbeat.loading, dot: "well" };
  if (state.ledgerStatus === "error") return { text: C.heartbeat.error, dot: "neg" };
  if (state.ledgerStatus === "stale") return { text: C.heartbeat.stale, dot: "warn" };
  const held = heldCount();
  if (!held) return { text: C.heartbeat.empty, dot: "accent" };
  const n = relCheck(state.summary.last_check_at);
  return {
    text: fillCopy(C.heartbeat.holdingOk, { held, n: n == null ? "—" : n }),
    dot: "accent",
  };
}

function homeStripHtml(hb) {
  const p = state.portfolio;
  const chg = p && p.total_change_24h_pct != null ? Number(p.total_change_24h_pct) : null;
  const chgCls = chg == null ? "" : chg < 0 ? "down" : "up";
  const chgTxt =
    chg == null ? "" : fillCopy(C.strip.chg24h, { chg: (chg < 0 ? "" : "+") + chg + "%" });
  const risk = p && p.risk ? p.risk.liquidation_score : "—";
  const band = p && p.risk ? p.risk.band || "safe" : "";
  const line = p
    ? `<div class="home-strip-line"><span class="lab">${escapeHtml(C.strip.label)}</span><span class="val num">${usd(p.total_usd_value)}</span><span class="chg num ${chgCls}">${escapeHtml(chgTxt)}</span><span class="meta">${escapeHtml(fillCopy(C.strip.riskBand, { score: risk, band }))}</span></div>`
    : "";
  return `<div class="home-strip" id="strip">${line}<div class="heartbeat"><span class="dot ${hb.dot}"></span>${escapeHtml(hb.text)}</div></div>`;
}

function micSvg() {
  return `<svg class="mic-svg" width="44" height="44" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true"><rect x="9" y="3" width="6" height="11" rx="3"></rect><path d="M5 11a7 7 0 0 0 14 0"></path><path d="M12 18v3"></path></svg>`;
}

function voiceStatusText() {
  const phase = state.voice.phase;
  if (phase === "listening") {
    if (voiceFinalizing) return C.voice.finalizing || C.voice.processing;
    if (!voiceReady) {
      if (voiceChirping) return C.voice.ready || C.voice.starting;
      if (voiceCaptureArmed && !voiceStreamReady) return C.voice.connecting || C.voice.starting;
      return C.voice.starting;
    }
    if (!voiceHadSpeech) return C.voice.speakNow || C.voice.listening;
    const s = Math.floor((state.voice.heldMs || 0) / 1000);
    const m = Math.floor(s / 60);
    const ss = String(s % 60).padStart(2, "0");
    return fillCopy(C.voice.listening, { m, ss });
  }
  if (phase === "drafted") return C.voice.drafted;
  if (phase === "sending") return C.voice.processing || C.voice.sending;
  return voiceMode() === "tap" ? C.voice.tapIdle : C.voice.hold;
}

function voiceLevelHtml() {
  return `<div class="voice-level" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></div>`;
}

function voiceDockHtml() {
  const phase = state.voice.phase;
  const nudge = state.voice.nudge
    ? `<div class="voice-nudge" id="voiceNudge">${escapeHtml(state.voice.nudge)}</div>`
    : `<div class="voice-nudge" id="voiceNudge" hidden></div>`;
  const words = `<div class="listen-words" id="liveWords"${
    showLiveWords() ? "" : " hidden"
  }>${liveWordsInnerHtml()}</div>`;
  const typePulse = state.voice.typePulse ? " type-pulse" : "";
  return (
    `<div class="home-ledger-wrap">` +
    `<div class="home-ledger${state.voice.phase === "listening" && !voiceFinalizing ? " dim" : ""}" id="homeLedger">` +
    ledgerZonesHtml() +
    `</div>` +
    `<div class="listen-scrim${state.voice.phase === "listening" && !voiceFinalizing ? " on" : ""}" id="listenScrim">` +
    `<div class="listen-lab">${escapeHtml(
      voiceFinalizing
        ? C.listening.finalizing || C.listening.label
        : !voiceReady
          ? C.listening.opening || C.listening.label
          : C.listening.label,
    )}</div>` +
    words +
    `</div>` +
    `</div>` +
    `<div class="voice-dock ${phase}${phase === "listening" && (!voiceReady || voiceFinalizing) ? " arming" : ""}" id="voiceDock">` +
    nudge +
    `<div class="voice-hero-wrap">` +
    voiceLevelHtml() +
    `<button type="button" class="voice-hero" id="voiceBtn" aria-label="${escapeHtml(voiceStatusText())}">` +
    `<div class="voice-halo"></div>` +
    `<div class="voice-arm" aria-hidden="true"><i></i></div>` +
    `<div class="rip"></div><div class="rip d2"></div>` +
    micSvg() +
    `<div class="wave"><i></i><i></i><i></i><i></i><i></i><i></i></div>` +
    `<div class="spin sending-spin"></div>` +
    `<div class="check">✓</div>` +
    `</button>` +
    voiceLevelHtml() +
    `</div>` +
    `<div class="meter queue-fill voice-wait" id="voiceWaitMeter" hidden><span></span></div>` +
    `<div class="voice-status" id="voiceStatus">${escapeHtml(voiceStatusText())}</div>` +
    `</div>` +
    nearMatchHtml() +
    `<div class="home-acts">` +
    `<button type="button" class="home-act${typePulse}" id="typeInstead">${escapeHtml(C.homeActs.type)}</button>` +
    `<button type="button" class="home-act port" id="goPortfolio">${escapeHtml(C.homeActs.portfolio)}</button>` +
    `</div>`
  );
}

function voiceHomeHtml() {
  const hb = homeHeartbeatText();
  return homeStripHtml(hb) + voiceDockHtml();
}

function nearMatchHtml() {
  const nm = state.nearMatch;
  if (!nm) return "";
  const word = nm.asked_entity || "";
  const frame = nm.message || fillCopy(C.cant.nearMatchFrame, { word });
  const controls = Array.isArray(nm.controls) ? nm.controls : [];
  const chips = controls
    .map((label) => {
      const escape = /meant /i.test(String(label));
      return `<button type="button" class="near-chip${escape ? " escape" : ""}" data-near="${escapeHtml(String(label))}">${escapeHtml(String(label))}</button>`;
    })
    .join("");
  return `<div class="near-match"><div class="frame">${escapeHtml(frame)}</div><div class="near-match-chips">${chips}</div></div>`;
}

function bindHomeActs() {
  const typeBtn = document.getElementById("typeInstead");
  if (typeBtn) {
    typeBtn.onclick = () => {
      haptic("select");
      openCompose({
        kind: "text",
        message: "",
        slide: true,
        typed: true,
        button: C.compose.sendWatch,
      });
    };
  }
  const port = document.getElementById("goPortfolio");
  if (port) {
    port.onclick = () => {
      haptic("select");
      state.tab = "portfolio";
      paint();
    };
  }
  app.querySelectorAll("[data-near]").forEach((btn) => {
    btn.onclick = () => {
      const label = btn.getAttribute("data-near") || "";
      if (label) sendNearMatchChoice(label);
    };
  });
}

function ledgerZonesHtml() {
  const rows = instructions();
  const needs = rows.filter((r) => zoneOf(r) === "needs");
  const queued = rows.filter((r) => zoneOf(r) === "queued");
  const watch = rows.filter((r) => zoneOf(r) === "watch");
  const cant = rows.filter((r) => zoneOf(r) === "cant");
  const done = rows.filter((r) => zoneOf(r) === "done");
  const earlier = rows.filter((r) => zoneOf(r) === "earlier");
  const paused = watch.filter((r) => r.status === "paused").length;
  const watchingN = watch.length - paused;

  if (state.ledgerStatus === "loading") {
    return `<div class="skel"></div><div class="skel" style="width:70%"></div>`;
  }
  if (state.ledgerStatus === "error" && !rows.length) {
    return `<p class="edge">${escapeHtml(C.errorRow)}</p>`;
  }
  if (!rows.length) {
    return `<p class="teach">${escapeHtml(C.emptyTeach)}</p>` + ledgerFooterHtml();
  }
  return (
    zoneBlock("needs", C.zones.needsYou, "lab-accent", needs.length, needs) +
    zoneBlock("queued", C.zones.queued, "lab-accent", queued.length, queued) +
    zoneBlock("watch", C.zones.watching, "lab-faint", fillCopy(C.zones.watchingCount, { w: watchingN, p: paused }), watch) +
    zoneBlock("cant", C.zones.cant, "lab-faint", cant.length, cant) +
    zoneBlock("done", C.zones.doneToday, "lab-pos", done.length, done) +
    (earlier.length
      ? `<div class="zone-h" id="earlierToggle"><span class="lab lab-faint">${escapeHtml(C.zones.earlier)} ${state.earlierOpen ? "▴" : "▾"}</span><span class="n">${escapeHtml(C.zones.earlierSub)}</span></div>` +
        (state.earlierOpen ? zoneRows(earlier) : "")
      : "") +
    `<p class="footer-line">${escapeHtml(voiceHomeOn() ? C.ledgerFooter : C.ledgerFooterLegacy)}</p>` +
    `<button type="button" class="nav-row" id="introduceRow">${escapeHtml(C.share.introduce)}</button>`
  );
}

function ledgerHtml(hb, compact) {
  const p = state.portfolio;
  const chg = p && p.total_change_24h_pct != null ? Number(p.total_change_24h_pct) : null;
  const chgCls = chg == null ? "" : chg < 0 ? "down" : "up";
  const chgTxt = chg == null ? "" : (chg < 0 ? "" : "+") + chg + "%";
  const risk = p && p.risk ? p.risk.liquidation_score : "—";
  const free = p && p.dollarpower ? usd(p.dollarpower.committed_usd) : "—";
  const strip = p
    ? `<div class="strip" id="strip"><span class="val num">${usd(p.total_usd_value)}</span><span class="chg num ${chgCls}">${escapeHtml(chgTxt)}</span><span class="meta">${escapeHtml(fillCopy(C.strip.riskFree, { risk, free }))}</span><span class="go">${escapeHtml(C.strip.trail)}</span></div>`
    : "";
  const rows = instructions();
  const needs = rows.filter((r) => zoneOf(r) === "needs");
  const queued = rows.filter((r) => zoneOf(r) === "queued");
  const watch = rows.filter((r) => zoneOf(r) === "watch");
  const cant = rows.filter((r) => zoneOf(r) === "cant");
  const done = rows.filter((r) => zoneOf(r) === "done");
  const paused = watch.filter((r) => r.status === "paused").length;
  const watchingN = watch.length - paused;

  if (compact) {
    return (
      `<div class="launch-label"><span>${escapeHtml(C.launch.label)}</span><span class="num">${new Date().toISOString().slice(11, 16)} UTC</span></div>` +
      reportLine("!", C.launch.needs, needs) +
      reportLine("⚙", C.launch.motion, queued) +
      `<div class="report"><span class="g">◎</span><span>${escapeHtml(
        fillCopy(C.launch.watching, {
          n: watchingN,
          p: paused,
          list: watch.map((r) => r.sentence).slice(0, 3).join(" · ") || "—",
        }),
      )}</span></div>` +
      reportLine("✓", C.launch.done, done, "pos") +
      (cant.length ? reportLine("·", C.launch.cant, cant, "faint") : "") +
      `<div class="heartbeat"><span class="dot ${hb.dot}"></span>${escapeHtml(hb.text)}</div>` +
      strip +
      `<p class="teach">${escapeHtml(C.launch.hint)}</p>`
    );
  }

  return (
    strip +
    `<div class="heartbeat"><span class="dot ${hb.dot}"></span>${escapeHtml(hb.text)}</div>` +
    nearMatchHtml() +
    ledgerZonesHtml()
  );
}

function ledgerFooterHtml() {
  const line = voiceHomeOn() ? C.ledgerFooter : C.ledgerFooterLegacy;
  return (
    `<p class="footer-line">${escapeHtml(line)}</p>` +
    `<button type="button" class="nav-row" id="introduceRow">${escapeHtml(C.share.introduce)}</button>`
  );
}

function reportLine(g, tmpl, rows, cls) {
  const what = rows[0] ? rows[0].sentence : "—";
  const pct = rows[0] && rows[0].progress_pct != null ? rows[0].progress_pct : "—";
  return `<div class="report"><span class="g ${cls || ""}">${g}</span><span>${escapeHtml(
    fillCopy(tmpl, { n: rows.length, what, pct, receipts: rows.map((r) => r.receipt || r.sentence).join(", ") }),
  )}</span></div>`;
}

function zoneBlock(id, label, labCls, count, rows) {
  if (!rows.length) return "";
  return `<div class="zone-h"><span class="lab ${labCls}">${escapeHtml(label)}</span><span class="n">${escapeHtml(String(count))}</span></div>${zoneRows(rows)}`;
}

function zoneRows(rows) {
  return rows
    .map((row, i) => {
      const g = glyph(row);
      const view = queueView(row);
      const queued = view && view.zone === "queued";
      const archiveSwipe =
        archivable(row) && !state.pending[row.instruction_id] && !queued;
      const actionSwipe =
        (row.status === "watching" || row.status === "paused") &&
        !state.pending[row.instruction_id];
      const swipable = archiveSwipe || actionSwipe;
      const pct = view && view.showMeter ? view.fillPct : clientProgress(row);
      const rem = view && view.showCountdown ? view.remainingDisplay : remainingSecs(row);
      const meter =
        queued && view.showMeter && pct != null
          ? `<div class="meter queue-fill"><span style="width:${Number(pct)}%"></span></div>`
          : row.status === "watching" && row.distance && state.ledgerStatus !== "stale"
            ? `<div class="meter ${row.distance.near ? "warn" : ""}"><span style="width:${row.distance.pct}%"></span></div>`
            : "";
      const value =
        queued && view.phase === "wait" && rem != null
          ? `<span class="count-chip num">${escapeHtml(String(rem))}</span>`
          : queued && view.phase === "sliced" && pct != null
            ? `<span class="pct-slot num">${escapeHtml(String(pct))}%</span>`
            : queued
              ? ""
              : row.display_status
                ? `<span class="chip ${row.voice_draft ? "warn" : chipClass(row.status)}">${escapeHtml(row.display_status)}</span>`
                : "";
      const open = state.openSwipe === row.instruction_id;
      const canCancel = view && view.zone === "queued" ? view.cancellable : cancellable(row);
      const chips = archiveSwipe
        ? `<div class="swipe-under"><button type="button" class="swipe-chip archive" data-act="archive" data-id="${escapeHtml(row.instruction_id)}">${escapeHtml(C.instruction.archive)}</button></div>`
        : actionSwipe
          ? `<div class="swipe-under"><button type="button" class="swipe-chip primary" data-act="${row.status === "paused" ? "resume" : "pause"}" data-id="${escapeHtml(row.instruction_id)}">${row.status === "paused" ? "Resume" : "Pause"}</button><button type="button" class="swipe-chip ask" data-act="ask" data-id="${escapeHtml(row.instruction_id)}">Ask</button></div>`
          : "";
      const rowCls = [
        i === rows.length - 1 ? "last" : "",
        queued && view.phase === "fill" ? "is-queued" : "",
        row.status === "done" && !queued ? "is-done" : "",
        row.status === "cant" ? "is-cant" : "",
        row.status === "misheard" ? "is-misheard" : "",
        (row.queued_local || row.voice_draft) && row.status !== "misheard" ? "voice-draft" : "",
        (row.queued_local || row.voice_draft) && Date.now() - (row.voice_landed_at || 0) < 500
          ? "fresh"
          : "",
      ]
        .filter(Boolean)
        .join(" ");
      const swipeKind = archiveSwipe ? "archive" : actionSwipe ? "1" : "0";
      return `<div class="row ${rowCls}" data-row="${escapeHtml(row.instruction_id)}" data-swipe="${swipeKind}" data-phase="${escapeHtml((view && view.phase) || row.status)}">
        ${chips}
        <div class="row-front" style="${open ? `transform:translateX(-${archiveSwipe ? 88 : 140}px)` : ""}">
          <div class="glyph ${g.cls}">${g.spin ? '<div class="spin"></div>' : escapeHtml(g.g)}</div>
          <div class="row-body">
            <div class="title-row"><div class="title">${escapeHtml(row.sentence)}</div>${value}${canCancel ? `<button type="button" class="row-x" data-cancel="${escapeHtml(row.instruction_id)}" aria-label="${escapeHtml(C.instruction.cancel)}">×</button>` : ""}</div>
            <div class="sub">${escapeHtml(subLine(row))}</div>
            ${meter}
          </div>
          ${swipable ? '<div class="grip"><i></i></div>' : ""}
        </div>
      </div>`;
    })
    .join("");
}

function bindLedger() {
  const strip = document.getElementById("strip");
  if (strip) {
    strip.onclick = () => {
      state.tab = "portfolio";
      paint();
    };
  }
  const earlier = document.getElementById("earlierToggle");
  if (earlier) {
    earlier.onclick = () => {
      state.earlierOpen = !state.earlierOpen;
      paint();
    };
  }
  const hint = app.querySelector(".teach");
  if (hint && state.compact) {
    hint.onclick = () => {
      state.compact = false;
      paint();
    };
  }
  app.querySelectorAll(".row[data-row]").forEach((el) => bindRowSwipe(el, false));
  app.querySelectorAll("[data-cancel]").forEach((btn) => {
    btn.addEventListener("pointerdown", (ev) => ev.stopPropagation());
    btn.onclick = (ev) => {
      ev.stopPropagation();
      const row = instructions().find((r) => r.instruction_id === btn.getAttribute("data-cancel"));
      if (row) cancelInPlace(row);
    };
  });
  const introduce = document.getElementById("introduceRow");
  if (introduce) introduce.onclick = introduceAomi;
}

function bindRowSwipe(el, isPosition) {
  const id = el.getAttribute("data-row");
  const swipeMode = el.getAttribute("data-swipe") || "0";
  const swipable = swipeMode === "1" || swipeMode === "archive";
  const front = el.querySelector(".row-front");
  const reveal =
    isPosition && el.getAttribute("data-ask-only") === "1"
      ? 70
      : swipeMode === "archive"
        ? 88
        : 140;
  let x0 = 0;
  let y0 = 0;
  let dx = 0;
  let t0 = 0;
  let tracking = false;
  let aborted = false;

  function rubber(v) {
    const cap = isPosition ? 260 : reveal;
    if (v > 0) return 0;
    const mag = -v;
    if (mag <= cap) return v;
    return -(cap + (mag - cap) * 0.22);
  }

  el.querySelectorAll("[data-act]").forEach((btn) => {
    btn.onclick = (ev) => {
      ev.stopPropagation();
      onRowAct(id, btn.getAttribute("data-act"), isPosition);
    };
  });

  front.addEventListener("click", () => {
    if (Date.now() < suppressClickUntil) return;
    if (state.openSwipe && state.openSwipe !== id) {
      state.openSwipe = "";
      paint();
      return;
    }
    if (state.openSwipe === id) {
      state.openSwipe = "";
      paint();
      return;
    }
    if (isPosition) {
      state.sheet = "position";
      state.posIdx = Number(id);
      state.detent = "half";
      paint();
      return;
    }
    openInstruction(id);
  });

  if (!swipable) return;

  front.addEventListener("pointerdown", (ev) => {
    x0 = ev.clientX;
    y0 = ev.clientY;
    dx = 0;
    t0 = performance.now();
    tracking = true;
    aborted = false;
    try {
      front.setPointerCapture(ev.pointerId);
    } catch (_) {
      /* capture optional */
    }
    front.style.transition = "none";
  });
  front.addEventListener(
    "pointermove",
    (ev) => {
      if (!tracking) return;
      const mx = ev.clientX - x0;
      const my = ev.clientY - y0;
      if (!aborted && Math.abs(my) > 12 && Math.abs(my) > Math.abs(mx)) {
        aborted = true;
        front.style.transform = "";
        return;
      }
      if (Math.abs(mx) > 6 && Math.abs(mx) > Math.abs(my) * 1.2) {
        ev.preventDefault();
        dx = rubber(mx);
        if (dx < -8) haptic("impact", "light");
        front.style.transform = `translateX(${dx}px)`;
      }
    },
    { passive: false },
  );
  function end() {
    if (!tracking) return;
    tracking = false;
    const dt = Math.max(1, performance.now() - t0);
    const vel = dx / dt;
    front.style.transition = "transform 220ms cubic-bezier(.2,.8,.3,1)";
    suppressClickUntil = Date.now() + 120;
    if (isPosition && (dx < -230 || vel < -0.9)) {
      haptic("impact", "medium");
      onRowAct(id, "primary", true);
      return;
    }
    if (swipeMode === "archive" && !aborted && (dx < -(reveal + 24) || (dx < -56 && vel < -0.85))) {
      front.style.transform = `translateX(-${el.getBoundingClientRect().width || 320}px)`;
      archiveInPlace(id);
      return;
    }
    if (dx < -(reveal * 0.5) || vel < -0.9) {
      state.openSwipe = id;
    } else {
      state.openSwipe = "";
    }
    paint();
  }
  front.addEventListener("pointerup", end);
  front.addEventListener("pointercancel", end);
}

function onRowAct(id, act, isPosition) {
  if (isPosition) {
    const p = (state.portfolio.positions || [])[Number(id)];
    if (!p) return;
    const acts = positionActs(p);
    if (act === "ask") return openCompose({ kind: "question", message: acts.ask, slide: false });
    if (act === "primary" && acts.primary) {
      return openCompose({
        kind: "imperative",
        message: acts.primary.msg,
        note: fillCopy(C.compose.noteImperative, { delta: acts.primary.delta || "moves portfolio risk" }),
        slide: true,
        button: acts.primary.label,
      });
    }
    return;
  }
  const row = instructions().find((r) => r.instruction_id === id);
  if (!row) return;
  if (act === "archive") {
    return archiveInPlace(row);
  }
  if (act === "ask") {
    return openCompose({
      kind: "question",
      message: fillCopy(C.drafts.askPrefix, { sentence: row.sentence }),
      slide: false,
      instruction_id: id,
    });
  }
  if (act === "pause" || act === "resume") {
    return openCompose({
      kind: act,
      message: fillCopy(act === "pause" ? C.drafts.pause : C.drafts.resume, { sentence: row.sentence }),
      note: act === "pause" ? C.compose.notePause : C.compose.noteResume,
      slide: true,
      instruction_id: id,
      button: act === "pause" ? C.compose.sendPause : C.compose.sendResume,
    });
  }
}

function portfolioHtml(hb) {
  const p = state.portfolio;
  if (!p) {
    return `<div class="heartbeat"><span class="dot well"></span>${escapeHtml(C.heartbeat.loading)}</div><div class="skel"></div>`;
  }
  const all = p.positions || [];
  if (!all.length) {
    const hbEmpty = heartbeatText();
    return (
      `<div class="hero"><div class="hero-val num">${usd(p.total_usd_value)}</div></div>` +
      `<div class="heartbeat tap" id="hbTap"><span class="dot ${hbEmpty.dot}"></span>${escapeHtml(hbEmpty.text)}</div>` +
      `<div class="center"><p>${escapeHtml(C.portfolioEmpty)}</p><p class="sub">${escapeHtml(C.portfolioEmptySub)}</p></div>` +
      `<p class="footer-line">${escapeHtml(C.portfolio.footer)}</p>`
    );
  }
  const chg = p.total_change_24h_pct != null ? Number(p.total_change_24h_pct) : null;
  const groups = [
    ["holdings", C.portfolio.holdings],
    ["positions", C.portfolio.openPositions],
    ["lending", C.portfolio.lending],
  ];
  const risk = p.risk || {};
  const floor = p.floor || "—";
  return (
    `<div class="hero"><div class="hero-val num">${usd(p.total_usd_value)}</div><div class="hero-sub num ${chg != null && chg < 0 ? "down" : "up"}">${chg == null ? "—" : (chg < 0 ? "" : "+") + chg + "%"}</div></div>` +
    `<div class="margin"><div class="lab">Available margin</div><div class="val num">${usd(p.dollarpower && p.dollarpower.committed_usd)}</div><div class="stack"><span style="width:${escapeHtml((p.dollarpower && p.dollarpower.fill_pct) || "0")}%"></span></div></div>` +
    `<div class="risk-line" id="riskLine">${escapeHtml(
      fillCopy(C.portfolio.riskLine, {
        n: risk.liquidation_score,
        band: risk.band || "",
        d: risk.distance_from_floor_pct != null ? risk.distance_from_floor_pct + "%" : "—",
        floor,
      }),
    )}</div>` +
    (state.riskOpen
      ? `<div class="facts"><div>${escapeHtml(fillCopy(C.portfolio.riskFloor, { floor }))}</div><div class="ask" id="riskAsk">${escapeHtml(C.portfolio.riskAsk)}</div></div>`
      : "") +
    `<div class="heartbeat tap" id="hbTap"><span class="dot ${hb.dot}"></span>${escapeHtml(hb.text)}</div>` +
    groups
      .map(([key, label]) => {
        const rows = all
          .map((row, idx) => ({ row, idx }))
          .filter(({ row }) => (row.group || groupFallback(row)) === key);
        if (!rows.length) return "";
        return `<div class="sec-h">${escapeHtml(label)}</div>${rows
          .map(({ row, idx }) => positionRowHtml(row, idx, idx === rows[rows.length - 1].idx))
          .join("")}`;
      })
      .join("") +
    `<p class="footer-line">${escapeHtml(C.portfolio.footer)}</p>`
  );
}

function groupFallback(row) {
  if (row.asset_type === "perp") return "positions";
  if (row.asset_type === "lend" || row.asset_type === "borrow") return "lending";
  return "holdings";
}

function jobline(row) {
  const extra = row.extra || "";
  if (row.watch_count > 0) return fillCopy(C.portfolio.jobline, { n: row.watch_count, extra });
  if (state.flags.jobline_negative && extra) return fillCopy(C.portfolio.joblineNeg, { extra });
  return extra;
}

function positionRowHtml(row, idx) {
  const askOnly = row.can_exit === false ? "1" : "0";
  const swipable = "1";
  const open = state.openSwipe === String(idx);
  const primary = positionActs(row).primary;
  const under = `<div class="swipe-under">${
    row.can_exit !== false && primary
      ? `<button type="button" class="swipe-chip primary" data-act="primary" data-id="${idx}">${escapeHtml(primary.label)}</button>`
      : ""
  }<button type="button" class="swipe-chip ask" data-act="ask" data-id="${idx}">Ask</button></div>`;
  return `<div class="row pos" data-row="${idx}" data-swipe="${swipable}" data-ask-only="${askOnly}">
    ${under}
    <div class="row-front" style="${open ? `transform:translateX(-${row.can_exit === false ? 70 : 140}px)` : ""}">
      <div class="glyph">${escapeHtml((row.symbol || "?").slice(0, 2))}</div>
      <div class="row-body">
        <div class="title-row"><div class="title">${escapeHtml(row.symbol)}</div><span class="pct-slot num">${usd(row.usd_value)}</span></div>
        <div class="sub">${escapeHtml(row.quantity + " · " + (jobline(row) || row.asset_type))}</div>
      </div>
      <div class="grip"><i></i></div>
    </div>
  </div>`;
}

function bindPortfolio() {
  const risk = document.getElementById("riskLine");
  if (risk) {
    risk.onclick = () => {
      state.riskOpen = !state.riskOpen;
      paint();
    };
  }
  const ask = document.getElementById("riskAsk");
  if (ask) {
    ask.onclick = () =>
      openCompose({ kind: "question", message: "Walk me through my risk.", slide: false });
  }
  const hb = document.getElementById("hbTap");
  if (hb) {
    hb.onclick = () => {
      state.tab = "ledger";
      paint();
    };
  }
  app.querySelectorAll(".row.pos").forEach((el) => bindRowSwipe(el, true));
}

function positionActs(p) {
  const qty = p.quantity;
  const sym = p.symbol;
  const type = p.asset_type;
  if (type === "lend") {
    return {
      primary: null,
      ask: `What happens when my ${sym} lend matures?`,
      watch: true,
    };
  }
  if (type === "perp") {
    return {
      primary: {
        label: "Close at market",
        msg: `Close my ${sym} ${p.side || "long"} (${qty}) at market.`,
        delta: "moves portfolio risk",
      },
      extra: [
        {
          label: "Reduce by half",
          msg: `Reduce my ${sym} ${p.side || "long"} by half.`,
          delta: "moves portfolio risk",
        },
        { label: "Increase to 5×", gated: true },
      ],
      ask: `Walk me through my ${sym} position.`,
      watch: true,
    };
  }
  return {
    primary: {
      label: "Sell at market",
      msg: `Sell my ${qty} ${sym} at market.`,
      delta: "moves portfolio risk",
    },
    extra: [
      {
        label: "Sell half",
        msg: `Sell ${qty} ${sym} at market — half.`,
        delta: "moves portfolio risk",
      },
    ],
    ask: `Walk me through my ${sym} spot position.`,
    watch: true,
  };
}

function watchDrafts(p) {
  const sym = p.symbol;
  const floor = state.portfolio && state.portfolio.floor;
  const out = [];
  if (p.asset_type === "perp" && floor) {
    const level = (Number(floor) * 1.1).toFixed(0);
    out.push({ tag: "TELL", text: `If ${sym} drops to $${level} (floor +10%), tell me`, fire: "tell" });
    out.push({ tag: "ACT", text: `If ${sym} drops to $${level}, close half`, fire: "act" });
    out.push({ tag: "TELL", text: "If funding turns positive, tell me", fire: "tell" });
  } else if (p.asset_type === "lend") {
    out.push({
      tag: "TELL",
      text: `The day before maturity, remind me to choose a roll`,
      fire: "tell",
    });
  } else {
    out.push({ tag: "ACT", text: `If ${sym} touches a third below, sell a third of the spot`, fire: "act" });
    out.push({ tag: "TELL", text: `If ${sym} drops 5% in a day, tell me`, fire: "tell" });
  }
  return out;
}

function sheetBackHtml() {
  return `<button type="button" class="header-btn sheet-back" id="sheetX" aria-label="Back">${backIconHtml()}</button>`;
}

function sheetHtml() {
  const half = state.sheet === "position" || state.sheet === "product" ? 280 : state.sheet === "instruction" ? 260 : 0;
  const y = state.sheet === "pick" || state.sheet === "picker" ? 0 : state.detent === "full" ? 0 : half;
  if (state.sheet === "position" || (state.sheet === "picker" && !state.productId)) return positionSheet(y);
  if (state.sheet === "product" || (state.sheet === "picker" && state.productId)) return productSheet(y);
  if (state.sheet === "instruction") return instructionSheet(y);
  return "";
}

function positionSheet(y) {
  const p = (state.portfolio.positions || [])[state.posIdx];
  if (!p) return "";
  const acts = positionActs(p);
  const picker = state.sheet === "picker";
  if (picker) {
    const drafts = watchDrafts(p);
    return `<div class="scrim" id="scrim"></div>
      <div class="sheet pick" id="sheet" style="transform:translateY(${y}px)">
        <div class="handle" id="handle"></div>
        <div class="sheet-h">${sheetBackHtml()}<h2>${escapeHtml(fillCopy(C.picker.title, { position: p.symbol }))}</h2></div>
        <div class="sheet-body">
          <p class="note">${escapeHtml(C.picker.sub)}</p>
          ${drafts
            .map(
              (d, i) =>
                `<div class="draft" data-draft="${i}"><span class="tag-pill">${escapeHtml(d.tag)}</span><span>${escapeHtml(d.text)}</span></div>`,
            )
            .join("")}
          <p class="hint">${escapeHtml(C.picker.footer)}</p>
        </div>
      </div>`;
  }
  const extras = (acts.extra || [])
    .map((a) => {
      if (a.gated) {
        return `<button type="button" class="act" data-gated="1"><span>${escapeHtml(a.label)}</span><span class="tag">${escapeHtml(C.position.gated)}</span></button>`;
      }
      return `<button type="button" class="act extra-act" data-msg="${escapeHtml(a.msg)}" data-label="${escapeHtml(a.label)}"><span>${escapeHtml(a.label)}</span><span class="tag">${escapeHtml(C.instruction.tagSlides)}</span></button>`;
    })
    .join("");
  return `<div class="scrim" id="scrim"></div>
    <div class="sheet pos" id="sheet" style="transform:translateY(${y}px)">
      <div class="handle" id="handle"></div>
      <div class="sheet-h">${sheetBackHtml()}<div><h2>${escapeHtml(p.symbol)}</h2><div class="sub">${usd(p.usd_value)} · ${escapeHtml(p.quantity)}</div></div></div>
      <div class="sheet-body">
        <div class="fact-card">${escapeHtml(jobline(p) || p.asset_type)}${p.watch_count ? `<div class="k">${escapeHtml(fillCopy(C.position.watched, { n: p.watch_count }))}</div>` : ""}</div>
        <div class="act-lab">${escapeHtml(C.position.actsLabel)}</div>
        ${
          acts.primary
            ? `<button type="button" class="act" id="primaryAct"><span>${escapeHtml(acts.primary.label)}</span><span class="tag">${escapeHtml(C.instruction.tagSlides)}</span></button>`
            : ""
        }
        ${extras}
        <button type="button" class="act" id="watchAct"><span>${escapeHtml(C.position.watchThis)}</span><span class="tag">›</span></button>
        <button type="button" class="act" id="askAct"><span>${escapeHtml(C.instruction.ask)}</span><span class="tag">${escapeHtml(C.instruction.tagTap)}</span></button>
        <p class="hint">${escapeHtml(C.position.footer)}</p>
        <p class="hint">${escapeHtml(state.detent === "full" ? C.instruction.detentFull : C.instruction.detentHalf)}</p>
      </div>
    </div>`;
}

function productActs(prod) {
  const sym = prod.symbol;
  if (prod.product === "lend") {
    return {
      primary: {
        label: C.search.lend,
        msg: fillCopy(C.search.lendMsg, { symbol: sym }),
        delta: "moves portfolio risk",
      },
      ask: fillCopy(C.search.askLend, { symbol: sym }),
      watch: true,
    };
  }
  if (prod.product === "perp") {
    return {
      primary: {
        label: C.search.openLong,
        msg: fillCopy(C.search.longMsg, { symbol: sym }),
        delta: "moves portfolio risk",
      },
      extra: [
        {
          label: C.search.openShort,
          msg: fillCopy(C.search.shortMsg, { symbol: sym }),
          delta: "moves portfolio risk",
        },
      ],
      ask: fillCopy(C.search.askPerp, { symbol: sym }),
      watch: true,
    };
  }
  return {
    primary: {
      label: C.search.buy,
      msg: fillCopy(C.search.buyMsg, { symbol: sym }),
      delta: "moves portfolio risk",
    },
    extra: [
      {
        label: C.search.sell,
        msg: fillCopy(C.search.sellMsg, { symbol: sym }),
        delta: "moves portfolio risk",
      },
    ],
    ask: fillCopy(C.search.askSpot, { symbol: sym }),
    watch: true,
  };
}

function productWatchDrafts(prod) {
  const held = heldPosition(prod);
  if (held) return watchDrafts(held.row);
  const sym = prod.symbol;
  if (prod.product === "perp") {
    return [
      { tag: "TELL", text: `If ${sym} drops 5% in a day, tell me`, fire: "tell" },
      { tag: "ACT", text: `If ${sym} drops 8%, close the perp`, fire: "act" },
    ];
  }
  if (prod.product === "lend") {
    return [
      {
        tag: "TELL",
        text: `The day before ${sym} maturity, remind me to choose a roll`,
        fire: "tell",
      },
    ];
  }
  return [
    { tag: "ACT", text: `If ${sym} touches a third below, sell a third of the spot`, fire: "act" },
    { tag: "TELL", text: `If ${sym} drops 5% in a day, tell me`, fire: "tell" },
  ];
}

function productSheet(y) {
  const p = findProduct(state.productId);
  if (!p) return "";
  const acts = productActs(p);
  const picker = state.sheet === "picker";
  if (picker) {
    const drafts = productWatchDrafts(p);
    return `<div class="scrim" id="scrim"></div>
      <div class="sheet pick" id="sheet" style="transform:translateY(${y}px)">
        <div class="handle" id="handle"></div>
        <div class="sheet-h">${sheetBackHtml()}<h2>${escapeHtml(fillCopy(C.picker.title, { position: p.symbol }))}</h2></div>
        <div class="sheet-body">
          <p class="note">${escapeHtml(C.picker.sub)}</p>
          ${drafts
            .map(
              (d, i) =>
                `<div class="draft" data-draft="${i}"><span class="tag-pill">${escapeHtml(d.tag)}</span><span>${escapeHtml(d.text)}</span></div>`,
            )
            .join("")}
          <p class="hint">${escapeHtml(C.picker.footer)}</p>
        </div>
      </div>`;
  }
  const extras = (acts.extra || [])
    .map(
      (a) =>
        `<button type="button" class="act extra-act" data-msg="${escapeHtml(a.msg)}" data-label="${escapeHtml(a.label)}"><span>${escapeHtml(a.label)}</span><span class="tag">${escapeHtml(C.instruction.tagSlides)}</span></button>`,
    )
    .join("");
  const kind = productKindLabel(p.product);
  const mark = p.mark_price ? fillCopy(C.search.mark, { price: usd(p.mark_price) }) : "";
  return `<div class="scrim" id="scrim"></div>
    <div class="sheet prod" id="sheet" style="transform:translateY(${y}px)">
      <div class="handle" id="handle"></div>
      <div class="sheet-h">${sheetBackHtml()}<div><h2>${escapeHtml(p.symbol)}</h2><div class="sub">${escapeHtml(kind)}${mark ? " · " + escapeHtml(mark) : ""}</div></div></div>
      <div class="sheet-body">
        <div class="act-lab">${escapeHtml(C.position.actsLabel)}</div>
        ${
          acts.primary
            ? `<button type="button" class="act" id="primaryAct"><span>${escapeHtml(acts.primary.label)}</span><span class="tag">${escapeHtml(C.instruction.tagSlides)}</span></button>`
            : ""
        }
        ${extras}
        <button type="button" class="act" id="watchAct"><span>${escapeHtml(C.position.watchThis)}</span><span class="tag">›</span></button>
        <button type="button" class="act" id="askAct"><span>${escapeHtml(C.instruction.ask)}</span><span class="tag">${escapeHtml(C.instruction.tagTap)}</span></button>
        <button type="button" class="act" id="chartAct"><span>${escapeHtml(C.search.chart)}</span><span class="tag">↗</span></button>
        <p class="hint">${escapeHtml(C.position.footer)}</p>
        <p class="hint">${escapeHtml(state.detent === "full" ? C.instruction.detentFull : C.instruction.detentHalf)}</p>
      </div>
    </div>`;
}

function instructionSheet(y) {
  const row = instructions().find((r) => r.instruction_id === state.insId);
  if (!row) return "";
  const needs = row.status === "awaiting_confirm" || row.status === "triggered";
  const executing = row.status === "executing";
  const cant = row.status === "cant";
  const facts = cant
    ? [
        [C.cant.factAsked, row.asked_entity || row.params?.asked || row.sentence],
        [C.cant.factAnswerLabel, row.params?.answer || C.cant.factAnswer],
        [C.cant.factTradesLabel, row.params?.world_trades || C.cant.factTrades],
      ]
    : [
        row.params && row.params.resolved ? ["condition", row.params.resolved] : null,
        row.check_stats && row.check_stats.checks_7d
          ? ["checks", String(row.check_stats.checks_7d)]
          : null,
        ["id", taskIdOf(row)],
        ["expires", fmtDate(row.expires_at)],
      ].filter(Boolean);
  const trail = row.trail || [];
  const acts = cant
    ? `<div class="act-lab">${escapeHtml(C.instruction.actsLabel)}</div>` +
      `<button type="button" class="act" id="askIns"><span>${escapeHtml(C.instruction.ask)}</span><span class="tag">${escapeHtml(C.instruction.tagTap)}</span></button>`
    : needs
    ? `<div class="act-lab">${escapeHtml(C.instruction.awaitingLabel)}</div><p class="note">${escapeHtml(C.instruction.awaitingNote)}</p><button type="button" class="act accent" id="openThread"><span>${escapeHtml(C.instruction.openThread)}</span></button>` +
      (cancellable(row)
        ? `<button type="button" class="act" id="cancelAct"><span>${escapeHtml(C.instruction.cancel)}</span><span class="tag">×</span></button>`
        : "")
    : `<div class="act-lab">${escapeHtml(C.instruction.actsLabel)}</div>` +
      (row.status === "watching"
        ? `<button type="button" class="act" id="pauseAct"><span>${escapeHtml(C.instruction.pause)}</span><span class="tag">${escapeHtml(C.instruction.tagSlides)}</span></button>`
        : "") +
      (row.status === "paused"
        ? `<button type="button" class="act" id="resumeAct"><span>${escapeHtml(C.instruction.resume)}</span><span class="tag">${escapeHtml(C.instruction.tagSlides)}</span></button>`
        : "") +
      (cancellable(row)
        ? `<button type="button" class="act" id="cancelAct"><span>${escapeHtml(C.instruction.cancel)}</span><span class="tag">×</span></button>`
        : "") +
      (!executing
        ? `<button type="button" class="act" id="askIns"><span>${escapeHtml(C.instruction.ask)}</span><span class="tag">${escapeHtml(C.instruction.tagTap)}</span></button>`
        : `<button type="button" class="act" id="askIns"><span>${escapeHtml(C.instruction.askRun)}</span><span class="tag">${escapeHtml(C.instruction.tagTap)}</span></button>`) +
      `<button type="button" class="act accent" id="openThread"><span>${escapeHtml(C.instruction.openThread)}</span><span class="tag">${executing ? escapeHtml(C.instruction.tagHalt) : ""}</span></button>`;
  return `<div class="scrim" id="scrim"></div>
    <div class="sheet ins" id="sheet" style="transform:translateY(${y}px)">
      <div class="handle" id="handle"></div>
      <div class="sheet-h">${sheetBackHtml()}<div><h2>${escapeHtml(row.sentence)}</h2><div class="chip ${chipClass(row.status)}">${escapeHtml(row.display_status || (clientProgress(row) != null ? clientProgress(row) + "%" : ""))}</div></div></div>
      <div class="sheet-body">
        <div class="fact-card">${facts.map(([k, v]) => `<div class="k">${escapeHtml(k)}</div><div>${escapeHtml(v)}</div>`).join("")}</div>
        ${acts}
        ${
          state.detent === "full"
            ? `<div class="act-lab">${escapeHtml(C.instruction.trailLabel)}</div>${trail
                .map(
                  (t) =>
                    `<div class="trail-row"><div class="trail-meta">${escapeHtml(fmtDate(t.at))} · ${escapeHtml(t.actor)}${t.origin === "voice" ? `<span class="trail-voice">voice</span>` : ""}</div><div class="trail-line">${escapeHtml(t.line)}${t.signed ? `<span class="signed">signed</span>` : ""}</div></div>`,
                )
                .join("")}<p class="hint">${escapeHtml(C.instruction.sheetFooter)}</p>`
            : `<p class="hint">${escapeHtml(C.instruction.detentHalf)}</p>`
        }
      </div>
    </div>`;
}

function bindSheet() {
  const scrim = document.getElementById("scrim");
  const sheet = document.getElementById("sheet");
  const handle = document.getElementById("handle");
  const x = document.getElementById("sheetX");
  if (scrim) scrim.onclick = () => { state.sheet = null; state.productId = ""; paint(); };
  if (x) {
    x.onclick = () => {
      if (state.sheet === "picker") state.sheet = state.productId ? "product" : "position";
      else {
        state.sheet = null;
        state.productId = "";
      }
      paint();
    };
  }
  const primary = document.getElementById("primaryAct");
  if (primary) {
    primary.onclick = () => {
      if (state.productId) {
        const p = findProduct(state.productId);
        const acts = productActs(p);
        return openCompose({
          kind: "imperative",
          message: acts.primary.msg,
          note: fillCopy(C.compose.noteImperative, { delta: acts.primary.delta }),
          slide: true,
          button: acts.primary.label,
        });
      }
      const p = (state.portfolio.positions || [])[state.posIdx];
      const acts = positionActs(p);
      openCompose({
        kind: "imperative",
        message: acts.primary.msg,
        note: fillCopy(C.compose.noteImperative, { delta: acts.primary.delta }),
        slide: true,
        button: acts.primary.label,
      });
    };
  }
  app.querySelectorAll(".extra-act").forEach((btn) => {
    btn.onclick = () =>
      openCompose({
        kind: "imperative",
        message: btn.getAttribute("data-msg"),
        note: fillCopy(C.compose.noteImperative, { delta: "moves portfolio risk" }),
        slide: true,
        button: btn.getAttribute("data-label"),
      });
  });
  app.querySelectorAll("[data-gated]").forEach((btn) => {
    btn.onclick = () => {
      state.blocked = { act: "Increase to 5×", n: "—", floor: (state.portfolio && state.portfolio.floor) || "—" };
      state.view = "blocked";
      state.sheet = null;
      paint();
    };
  });
  const watchAct = document.getElementById("watchAct");
  if (watchAct) {
    watchAct.onclick = () => {
      haptic("select");
      state.sheet = "picker";
      paint();
    };
  }
  const askAct = document.getElementById("askAct");
  if (askAct) {
    askAct.onclick = () => {
      if (state.productId) {
        const p = findProduct(state.productId);
        return openCompose({ kind: "question", message: productActs(p).ask, slide: false });
      }
      const p = (state.portfolio.positions || [])[state.posIdx];
      openCompose({ kind: "question", message: positionActs(p).ask, slide: false });
    };
  }
  const chartAct = document.getElementById("chartAct");
  if (chartAct) {
    chartAct.onclick = () => {
      const p = findProduct(state.productId);
      if (p) openProductChart(p.symbol);
    };
  }
  app.querySelectorAll("[data-draft]").forEach((el) => {
    el.onclick = () => {
      haptic("select");
      if (state.productId) {
        const p = findProduct(state.productId);
        const d = productWatchDrafts(p)[Number(el.getAttribute("data-draft"))];
        return openCompose({
          kind: "conditional",
          message: d.text,
          fire_kind: d.fire,
          note: d.fire === "act" ? C.compose.noteWatchAct : C.compose.noteWatchTell,
          slide: true,
          button: C.compose.sendWatch,
          instrument: p.symbol,
        });
      }
      const p = (state.portfolio.positions || [])[state.posIdx];
      const d = watchDrafts(p)[Number(el.getAttribute("data-draft"))];
      openCompose({
        kind: "conditional",
        message: d.text,
        fire_kind: d.fire,
        note: d.fire === "act" ? C.compose.noteWatchAct : C.compose.noteWatchTell,
        slide: true,
        button: C.compose.sendWatch,
        instrument: p.symbol,
      });
    };
  });
  const pauseAct = document.getElementById("pauseAct");
  if (pauseAct) pauseAct.onclick = () => onRowAct(state.insId, "pause", false);
  const resumeAct = document.getElementById("resumeAct");
  if (resumeAct) resumeAct.onclick = () => onRowAct(state.insId, "resume", false);
  const cancelAct = document.getElementById("cancelAct");
  if (cancelAct) {
    cancelAct.onclick = () => {
      const row = instructions().find((r) => r.instruction_id === state.insId);
      if (row) cancelInPlace(row);
    };
  }
  const askIns = document.getElementById("askIns");
  if (askIns) askIns.onclick = () => onRowAct(state.insId, "ask", false);
  const openThread = document.getElementById("openThread");
  if (openThread) openThread.onclick = () => openThreadLink();
  if (sheet) bindSheetDrag(sheet, handle);
}

function bindSheetDrag(sheet, handle) {
  const kind = state.sheet;
  const half = kind === "position" || kind === "product" ? 280 : kind === "instruction" ? 260 : 0;
  const container = kind === "position" ? 620 : kind === "instruction" ? 640 : 400;
  if (kind === "picker") {
    if (handle) handle.onclick = () => {};
    return;
  }
  let y0 = 0;
  let start = state.detent === "full" ? 0 : half;
  let y = start;
  let t0 = 0;
  let tracking = false;
  function setY(v) {
    y = v < 0 ? v * 0.18 : v;
    sheet.style.transition = "none";
    sheet.style.transform = `translateY(${y}px)`;
  }
  function onDown(ev) {
    if (ev.target && ev.target.closest && ev.target.closest("button, .act, .draft, a, input, textarea")) {
      return;
    }
    y0 = ev.clientY;
    start = state.detent === "full" ? 0 : half;
    t0 = performance.now();
    tracking = true;
    sheet.setPointerCapture(ev.pointerId);
  }
  function onMove(ev) {
    if (!tracking) return;
    const dy = ev.clientY - y0;
    if (Math.abs(dy) > 8) setY(start + dy);
  }
  function onUp() {
    if (!tracking) return;
    tracking = false;
    const dt = Math.max(1, performance.now() - t0);
    const vel = (y - start) / dt;
    sheet.style.transition = "transform 240ms cubic-bezier(.2,.8,.3,1)";
    if (vel > 0.8 || y > half + 130) {
      state.sheet = null;
      state.productId = "";
      paint();
      return;
    }
    if (y < half * 0.55 || vel < -0.6) {
      haptic("select");
      state.detent = "full";
    } else {
      haptic("select");
      state.detent = "half";
    }
    paint();
  }
  sheet.addEventListener("pointerdown", onDown);
  sheet.addEventListener("pointermove", onMove);
  sheet.addEventListener("pointerup", onUp);
  sheet.addEventListener("pointercancel", onUp);
  if (handle) {
    handle.onclick = () => {
      state.detent = state.detent === "full" ? "half" : "full";
      haptic("select");
      paint();
    };
  }
}

async function openInstruction(id) {
  const row = instructions().find((r) => r.instruction_id === id);
  if (row && row.status === "misheard") return;
  state.insId = id;
  state.sheet = "instruction";
  state.detent = "half";
  paint();
  try {
    const one = await api("/api/v1/mini-app/ledger/" + encodeURIComponent(id));
    const ins = one.instruction || one;
    if (ins && ins.instruction_id) {
      const idx = state.ledger.findIndex((r) => r.instruction_id === ins.instruction_id);
      if (idx >= 0) state.ledger[idx] = { ...state.ledger[idx], ...ins };
      else state.ledger.push(ins);
      if (state.sheet === "instruction" && state.insId === id) paint();
    }
  } catch (_) {
    /* keep the list card */
  }
}

function openCompose(payload) {
  state.compose = payload;
  state.view = "compose";
  state.sheet = null;
  state.searchOpen = false;
  paint();
}

function renderCompose() {
  const c = state.compose;
  document.body.className = "";
  const slide = c.slide && !reduceMotion;
  const body = c.typed
    ? `<textarea class="compose-input" id="typeInput">${escapeHtml(c.message || "")}</textarea>`
    : `<p class="msg">${escapeHtml(c.message)}</p>`;
  app.innerHTML =
    headerHtml("inner") +
    `<div class="screen">
      <div class="kicker">${escapeHtml(C.compose.label)}</div>
      ${body}
      <p class="note">${escapeHtml(C.compose.disclaimer)}</p>
      ${c.note ? `<p class="note">${escapeHtml(c.note)}</p>` : ""}
      ${
        slide
          ? `<div class="rail" id="rail"><div class="rail-fill" id="railFill"></div><div class="rail-lab" id="railLab">${escapeHtml(fillCopy(C.compose.slideLabel, { button: c.button || C.compose.sendWatch }))}</div><div class="thumb" id="thumb">⟶</div></div><p class="hint">${escapeHtml(C.compose.slideHint)}</p>`
          : `<button type="button" class="primary-btn" id="sendTap">${escapeHtml(c.button || C.compose.ask)}</button><p class="hint">${escapeHtml(C.compose.tapHint)}</p>`
      }
    </div>` +
    bottomHtml();
  bindChrome();
  const input = document.getElementById("typeInput");
  if (input) {
    input.oninput = () => {
      state.compose.message = input.value;
    };
    input.focus();
  }
  const tap = document.getElementById("sendTap");
  if (tap) tap.onclick = () => doSend();
  const thumb = document.getElementById("thumb");
  if (thumb) bindSlide(thumb);
}

function bindSlide(thumb) {
  const rail = document.getElementById("rail");
  const fillEl = document.getElementById("railFill");
  const lab = document.getElementById("railLab");
  const max = () => rail.clientWidth - 64 - 8;
  let x0 = 0;
  let x = 0;
  let tracking = false;
  thumb.addEventListener("pointerdown", (ev) => {
    x0 = ev.clientX;
    tracking = true;
    thumb.setPointerCapture(ev.pointerId);
    thumb.style.transition = "none";
  });
  thumb.addEventListener("pointermove", (ev) => {
    if (!tracking) return;
    x = Math.max(0, Math.min(max(), ev.clientX - x0));
    const pct = x / max();
    thumb.style.transform = `translateX(${x}px)`;
    fillEl.style.transform = `translateX(${-100 + pct * 100}%)`;
    lab.style.opacity = String(1 - Math.min(1, pct / 0.6));
  });
  function end() {
    if (!tracking) return;
    tracking = false;
    if (x / max() >= 0.9) {
      haptic("notify", "success");
      doSend();
      return;
    }
    thumb.style.transition = "transform 250ms cubic-bezier(.2,.8,.3,1)";
    fillEl.style.transition = "transform 250ms cubic-bezier(.2,.8,.3,1)";
    thumb.style.transform = "translateX(0)";
    fillEl.style.transform = "translateX(-100%)";
    lab.style.opacity = "1";
  }
  thumb.addEventListener("pointerup", end);
  thumb.addEventListener("pointercancel", end);
}

async function doSend() {
  const c = state.compose;
  if (c.typed) {
    const input = document.getElementById("typeInput");
    if (input) c.message = input.value;
    if (!String(c.message || "").trim()) return;
  }
  const correlation_id = newId();
  const payload = {
    correlation_id,
    kind: c.kind,
    message: c.message,
    instruction_id: c.instruction_id || undefined,
    fire_kind: c.fire_kind,
    instrument: c.instrument,
  };
  const inTelegram = tg && tg.initData && typeof tg.sendData === "function";
  if (inTelegram) {
    try {
      tg.sendData(JSON.stringify(payload));
    } catch (_) {
      /* host may still ingest via webhook */
    }
  }
  let recorded = null;
  if (!inTelegram || previewState()) {
    try {
      recorded = await api("/api/v1/mini-app/compose", {
        method: "POST",
        body: payload,
      });
    } catch (_) {
      recorded = null;
    }
  }
  if (c.kind === "pause" || c.kind === "resume") {
    if (c.instruction_id) state.pending[c.instruction_id] = c.kind;
  }
  const heardKind = recorded && (recorded.kind || recorded.voice_kind);
  if (
    heardKind === "cant" ||
    heardKind === "near_match" ||
    heardKind === "unclear"
  ) {
    state.compose = null;
    state.view = "main";
    applyHeardOutcome(recorded, c.message, correlation_id);
    return;
  }
  if (heardKind === "resolved") {
    const rewritten =
      (recorded && (recorded.rewritten_text || recorded.remaining_text)) || c.message;
    c.message = rewritten;
  }
  if (c.kind !== "question") {
    const id =
      (recorded && recorded.instruction && recorded.instruction.instruction_id) ||
      c.instruction_id ||
      correlation_id;
    landQueuedTask(c.message, correlation_id, id, { kind: c.kind || "trade" });
    state.sent = { id, kind: c.kind, message: c.message, question: false };
  } else {
    state.sent = { id: null, kind: c.kind, message: c.message, question: true };
  }
  state.view = "sent";
  paint();
  refreshLedger();
}

function renderSent() {
  document.body.className = "";
  const s = state.sent;
  const row = s.id ? instructions().find((r) => r.instruction_id === s.id) : null;
  app.innerHTML =
    headerHtml("inner") +
    `<div class="screen">
      <div class="kicker">${escapeHtml(C.sent.label)}</div>
      <h2>${escapeHtml(C.sent.headline)}</h2>
      <p class="msg">${escapeHtml(s.message)}</p>
      ${
        s.question
          ? `<p class="note">${escapeHtml(C.sent.askNote)}</p>`
          : `<div class="mini-card" id="sentCard"><div class="title">${escapeHtml((row && row.sentence) || s.message)}</div><div class="chip">${escapeHtml((row && row.display_status) || C.zones.queued)}</div><p class="sub">${escapeHtml((row && subLine(row)) || fillCopy(C.sub.pendingExecute, { n: 3 }))}</p></div><p class="note">${escapeHtml(C.sent.cardNote)}</p>`
      }
      <button type="button" class="ghost-btn" id="openThread">${escapeHtml(C.sent.openThread)}</button>
    </div>` +
    bottomHtml();
  bindChrome();
  const card = document.getElementById("sentCard");
  if (card) {
    card.onclick = () => {
      state.view = "main";
      state.tab = "ledger";
      state.sheet = "instruction";
      state.insId = s.id;
      state.detent = "half";
      paint();
    };
  }
  const t = document.getElementById("openThread");
  if (t) t.onclick = () => openThreadLink();
}

function renderBlocked() {
  document.body.className = "";
  const b = state.blocked || {};
  app.innerHTML =
    headerHtml("inner") +
    `<div class="screen">
      <div class="kicker">${escapeHtml(C.gate.label)}</div>
      <h2>${escapeHtml(C.gate.headline)}</h2>
      <p class="msg">${escapeHtml(fillCopy(C.gate.line, { act: b.act || "This", n: b.n || "—", floor: b.floor || "—" }))}</p>
      <p class="note">${escapeHtml(C.gate.note)}</p>
      <button type="button" class="ghost-btn" id="gateAsk">${escapeHtml(C.gate.act)}</button>
      <p class="hint">${escapeHtml(C.gate.footer)}</p>
    </div>` +
    bottomHtml();
  bindChrome();
  const ask = document.getElementById("gateAsk");
  if (ask) {
    ask.onclick = () =>
      openCompose({ kind: "question", message: "Walk me through this policy gate.", slide: false });
  }
}

function openThreadLink() {
  if (tg && typeof tg.close === "function") tg.close();
}

function introduceAomi() {
  haptic("select");
  (async () => {
    try {
      const data = await api("/api/v1/mini-app/share", { method: "POST", body: {} });
      if (
        data.prepared_inline_message_id &&
        tg &&
        typeof tg.shareMessage === "function"
      ) {
        tg.shareMessage(data.prepared_inline_message_id);
        return;
      }
      const url = data.fallback_url;
      if (url && tg && typeof tg.openTelegramLink === "function") {
        tg.openTelegramLink(url);
        return;
      }
      if (tg && typeof tg.sendData === "function") {
        tg.sendData(C.share.intent);
      }
    } catch (_) {
      /* nav only — stay silent */
    }
  })();
}

let voiceRecorder = null;
let voiceChunks = [];
let voiceStream = null;
let voiceWanted = false;
let voiceReady = false;
let voiceCaptureArmed = false;
let voiceReadyTimer = null;
let voiceStartedAt = 0;
let voiceTick = null;
let voiceNudgeTimer = null;
let voiceDraftTimer = null;
const voiceMisheardTimers = new Map();
const MISHEARD_MS = 3000;
let livePollTimer = null;
let livePollInFlight = false;
let livePollSeq = 0;
let liveAppliedSeq = 0;
let liveFromServer = false;
let livePcmChunks = [];
let livePcmSamples = 0;
let livePcmRate = 48000;
let liveStreamRate = 48000;
let liveCapture = null;
let voiceAnalyser = null;
let voiceAudioCtx = null;
let voiceLevelRaf = 0;
let voiceLevelSmoothed = 0;
let voiceLevelEls = [];
let voiceWaveBars = [];
let voiceFinalizing = false;
let voiceFlushing = false;
let voiceFlushWait = null;
let voiceStreamSock = null;
let voiceStreamReady = false;
let voiceStreamQueue = [];
let voiceStreamOnFinal = null;
let voiceStreamFallbackTimer = null;
let voiceHeardCommitted = "";
let voiceOnCue = null;
let voiceChirping = false;
let voiceHoldArmed = false;
let voiceHadSpeech = false;
let voiceSpeechAt = 0;
let voiceKeepAliveAt = 0;
let voiceWaitAt = 0;
let voiceFlushAt = 0;

function bindVoice() {
  const btn = document.getElementById("voiceBtn");
  if (!btn) return;
  btn.addEventListener("contextmenu", (ev) => ev.preventDefault());
  btn.addEventListener("pointerdown", (ev) => {
    if (ev.button != null && ev.button !== 0) return;
    ev.preventDefault();
    ev.stopPropagation();
    try {
      btn.setPointerCapture(ev.pointerId);
    } catch (_) {
      /* capture optional */
    }
    onVoiceDown(btn);
  });
  btn.addEventListener("pointermove", (ev) => {
    if (voiceMode() !== "hold") return;
    if (voiceFinalizing) return;
    if (state.voice.phase !== "listening") return;
    if (!pointInVoiceControl(btn, ev.clientX, ev.clientY, slideCancelPad())) {
      ev.preventDefault();
      cancelVoice("slide");
    }
  });
  btn.addEventListener("pointerup", (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    onVoiceUp(btn);
  });
  btn.addEventListener("pointercancel", (ev) => {
    ev.preventDefault();
    if (voiceFinalizing) return;
    if (voiceMode() !== "hold" || state.voice.phase !== "listening") return;
    const held = voiceStartedAt ? Date.now() - voiceStartedAt : 0;
    if (voiceReady && held >= 600) {
      commitVoice();
      return;
    }
    cancelVoice("slide");
  });
}

function slideCancelPad() {
  const pad = Number(window.SLIDE_CANCEL_PAD_PX);
  return Number.isFinite(pad) ? pad : 48;
}

function pointInVoiceControl(btn, x, y, extra) {
  if (typeof window.pointInVoiceHit === "function") {
    const r = btn.getBoundingClientRect();
    return window.pointInVoiceHit(r.width, r.height, x, y, r.left, r.top, extra);
  }
  return pointInCircle(btn, x, y);
}

function pointInCircle(btn, x, y) {
  return pointInVoiceControl(btn, x, y, 0);
}

function onVoiceDown(btn) {
  ensureAudioCtx();
  if (state.voice.phase === "drafted" || state.voice.phase === "sending") {
    clearTimeout(voiceDraftTimer);
    state.voice.phase = "idle";
    syncVoiceDom();
  }
  if (voiceMode() === "tap") {
    if (state.voice.phase === "listening") {
      commitVoice();
      return;
    }
    beginListening(btn);
    return;
  }
  beginListening(btn);
}

function onVoiceUp() {
  if (voiceMode() === "tap") return;
  if (voiceFinalizing) return;
  if (state.voice.phase !== "listening") return;
  if (!voiceReady) {
    cancelVoice("short");
    return;
  }
  const held = voiceStartedAt ? Date.now() - voiceStartedAt : 0;
  if (held < 600) {
    cancelVoice("short");
    return;
  }
  commitVoice();
}

function showNudge(msg) {
  state.voice.nudge = msg;
  const el = document.getElementById("voiceNudge");
  if (el) {
    el.textContent = msg;
    el.hidden = false;
  }
  clearTimeout(voiceNudgeTimer);
  voiceNudgeTimer = setTimeout(() => {
    state.voice.nudge = "";
    const n = document.getElementById("voiceNudge");
    if (n) n.hidden = true;
  }, 2000);
}

function pulseTypeInstead() {
  state.voice.typePulse = true;
  const btn = document.getElementById("typeInstead");
  if (btn) {
    btn.classList.add("type-pulse");
    setTimeout(() => {
      state.voice.typePulse = false;
      btn.classList.remove("type-pulse");
    }, 1200);
  }
}

function syncVoiceDom() {
  const dock = document.getElementById("voiceDock");
  const ledger = document.getElementById("homeLedger");
  const scrim = document.getElementById("listenScrim");
  const status = document.getElementById("voiceStatus");
  const btn = document.getElementById("voiceBtn");
  const listening = state.voice.phase === "listening";
  const overlay = listening && !voiceFinalizing;
  const arming = listening && (!voiceReady || voiceFinalizing);
  if (dock) {
    dock.className =
      "voice-dock " +
      state.voice.phase +
      (arming ? " arming" : "") +
      (voiceAnalyser && !arming ? " live-level" : "");
  }
  if (ledger) ledger.classList.toggle("dim", overlay);
  if (scrim) scrim.classList.toggle("on", overlay);
  const lab = scrim && scrim.querySelector(".listen-lab");
  if (lab) {
    lab.textContent = voiceFinalizing
      ? C.listening.finalizing || C.listening.label
      : !voiceReady
        ? C.listening.opening || C.listening.label
        : C.listening.label;
  }
  if (status) status.textContent = voiceStatusText();
  if (btn) {
    btn.classList.toggle("hot", listening && !arming);
    btn.setAttribute("aria-label", voiceStatusText());
  }
  const words = document.getElementById("liveWords");
  if (words) {
    const show = showLiveWords();
    words.hidden = !show;
    if (show) words.innerHTML = liveWordsInnerHtml();
  }
  paintVoiceWait();
}

function startVoiceTick() {
  clearInterval(voiceTick);
  voiceTick = setInterval(() => {
    if (state.voice.phase !== "listening" && state.voice.phase !== "sending") return;
    if (voiceFinalizing || !voiceReady) {
      const status = document.getElementById("voiceStatus");
      if (status) status.textContent = voiceStatusText();
      paintVoiceWait();
      return;
    }
    state.voice.heldMs = Date.now() - (voiceStartedAt || Date.now());
    const status = document.getElementById("voiceStatus");
    if (status) status.textContent = voiceStatusText();
    paintVoiceWait();
  }, 110);
}

function paintVoiceWait() {
  const el = document.getElementById("voiceWaitMeter");
  if (!el) return;
  const bar = el.querySelector("span");
  const phase = state.voice.phase;
  let mode = null;
  let pct = 0;
  if (voiceFlushing) {
    mode = "det";
    const dur = Math.max(1, Number(window.PCM_FLUSH_MS) || 400);
    pct = Math.min(100, ((Date.now() - (voiceFlushAt || Date.now())) / dur) * 100);
  } else if (phase === "sending") {
    mode = "indet";
  } else if (phase === "listening" && !voiceReady) {
    if (voiceChirping) {
      mode = "det";
      const dur =
        Math.max(1, Number(window.CHIRP_MS) || 180) + Math.max(0, Number(window.CHIRP_TAIL_MS) || 40);
      pct = Math.min(100, ((Date.now() - (voiceWaitAt || Date.now())) / dur) * 100);
    } else {
      mode = "indet";
    }
  }
  if (!mode) {
    el.hidden = true;
    el.classList.remove("is-indet");
    return;
  }
  el.hidden = false;
  el.classList.toggle("is-indet", mode === "indet");
  if (bar) bar.style.width = mode === "indet" ? "36%" : pct.toFixed(1) + "%";
}

function ensureAudioCtx() {
  const AC = window.AudioContext || window.webkitAudioContext;
  if (!AC) return null;
  if (!voiceAudioCtx || voiceAudioCtx.state === "closed") {
    try {
      voiceAudioCtx = new AC({ latencyHint: "interactive", sampleRate: 48000 });
    } catch (_) {
      try {
        voiceAudioCtx = new AC({ latencyHint: "interactive" });
      } catch (__) {
        try {
          voiceAudioCtx = new AC();
        } catch (___) {
          voiceAudioCtx = null;
          return null;
        }
      }
    }
  }
  if (voiceAudioCtx.state === "suspended") {
    voiceAudioCtx.resume().catch(() => {});
  }
  return voiceAudioCtx;
}

function stopVoiceOnSound() {
  if (!voiceOnCue) return;
  const cue = voiceOnCue;
  voiceOnCue = null;
  cue.oscs.forEach((osc) => {
    try {
      osc.stop();
    } catch (_) {
      /* already stopped */
    }
  });
  try {
    cue.master.disconnect();
  } catch (_) {
    /* already disconnected */
  }
}

function playVoiceOnSound(done) {
  stopVoiceOnSound();
  const ctx = ensureAudioCtx();
  const finish = () => {
    if (typeof done === "function") done();
  };
  if (!ctx) {
    finish();
    return;
  }
  const fire = () => {
    if (!voiceWanted || state.voice.phase !== "listening") {
      finish();
      return;
    }
    const now = ctx.currentTime;
    const master = ctx.createGain();
    master.gain.setValueAtTime(0.22, now);
    master.connect(ctx.destination);
    const oscs = [];
    const chirp = (freq, start, dur) => {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = "sine";
      osc.frequency.setValueAtTime(freq, start);
      gain.gain.setValueAtTime(0.0001, start);
      gain.gain.exponentialRampToValueAtTime(1, start + 0.008);
      gain.gain.exponentialRampToValueAtTime(0.0001, start + dur);
      osc.connect(gain);
      gain.connect(master);
      osc.start(start);
      osc.stop(start + dur + 0.02);
      oscs.push(osc);
      return osc;
    };
    chirp(880, now, 0.055);
    const last = chirp(1318.5, now + 0.07, 0.085);
    voiceOnCue = { master, oscs };
    last.onended = () => {
      if (voiceOnCue && voiceOnCue.master === master) stopVoiceOnSound();
      finish();
    };
  };
  if (ctx.state === "suspended") {
    ctx.resume().then(fire).catch(() => finish());
    return;
  }
  fire();
}

function mapVoiceLevel(rms, peak) {
  const floor = 0.004;
  const body = Math.max(0, rms - floor) * 5.4;
  const spike = Math.max(0, peak - 0.03) * 1.1;
  return Math.min(1, Math.pow(Math.max(body, body * 0.72 + spike * 0.5), 0.62));
}

function paintVoiceLevel(level) {
  const litFrac = Math.max(0, Math.min(1, level));
  voiceLevelEls.forEach((col) => {
    const segs = col.children;
    const n = segs.length;
    const lit = Math.round(litFrac * n);
    for (let i = 0; i < n; i++) {
      const on = i < lit;
      segs[i].classList.toggle("on", on);
      segs[i].classList.toggle("hot", on && i >= n - 2 && litFrac > 0.82);
    }
  });
  const wrap = document.querySelector(".voice-hero-wrap");
  if (wrap) wrap.style.setProperty("--voice-lvl", litFrac.toFixed(3));
}

function paintWaveFromTime(time, level) {
  const n = voiceWaveBars.length;
  if (!n || !time || !time.length) return;
  const slice = Math.max(1, Math.floor(time.length / n));
  for (let i = 0; i < n; i++) {
    let peak = 0;
    const start = i * slice;
    const end = Math.min(time.length, start + slice);
    for (let j = start; j < end; j++) {
      const a = Math.abs((time[j] - 128) / 128);
      if (a > peak) peak = a;
    }
    voiceWaveBars[i].style.height = 8 + Math.max(peak, level * 0.28) * 44 + "px";
  }
}

function resetVoiceLevelDom() {
  paintVoiceLevel(0);
  voiceWaveBars.forEach((bar) => {
    bar.style.height = "";
  });
  const wrap = document.querySelector(".voice-hero-wrap");
  if (wrap) wrap.style.removeProperty("--voice-lvl");
}

function tickVoiceLevel() {
  if (!voiceAnalyser) return;
  const { an, time } = voiceAnalyser;
  an.getByteTimeDomainData(time);
  let sum = 0;
  let peak = 0;
  for (let i = 0; i < time.length; i++) {
    const x = (time[i] - 128) / 128;
    sum += x * x;
    const a = Math.abs(x);
    if (a > peak) peak = a;
  }
  const mapped = mapVoiceLevel(Math.sqrt(sum / time.length), peak);
  const follow = mapped > voiceLevelSmoothed ? 0.55 : 0.18;
  voiceLevelSmoothed += (mapped - voiceLevelSmoothed) * (reduceMotion ? 1 : follow);
  paintVoiceLevel(voiceLevelSmoothed);
  paintWaveFromTime(time, voiceLevelSmoothed);
}

function startLevelLoop() {
  stopLevelLoop();
  voiceLevelSmoothed = 0;
  voiceLevelEls = Array.from(document.querySelectorAll("#voiceDock .voice-level"));
  voiceWaveBars = Array.from(document.querySelectorAll("#voiceBtn .wave i"));
  const dock = document.getElementById("voiceDock");
  if (dock) dock.classList.add("live-level");
  const loop = () => {
    voiceLevelRaf = requestAnimationFrame(loop);
    tickVoiceLevel();
  };
  voiceLevelRaf = requestAnimationFrame(loop);
}

function stopLevelLoop() {
  if (voiceLevelRaf) cancelAnimationFrame(voiceLevelRaf);
  voiceLevelRaf = 0;
  voiceLevelSmoothed = 0;
  const dock = document.getElementById("voiceDock");
  if (dock) dock.classList.remove("live-level");
  resetVoiceLevelDom();
  voiceLevelEls = [];
  voiceWaveBars = [];
}

function disconnectAnalyserNodes() {
  stopLevelLoop();
  if (liveCapture) {
    try {
      liveCapture.proc.disconnect();
    } catch (_) {
      /* ignore */
    }
    try {
      liveCapture.sink.disconnect();
    } catch (_) {
      /* ignore */
    }
    try {
      if (liveCapture.mute) liveCapture.mute.disconnect();
    } catch (_) {
      /* ignore */
    }
    liveCapture.proc.onaudioprocess = null;
    liveCapture = null;
  }
  if (voiceAnalyser && voiceAnalyser.src) {
    try {
      voiceAnalyser.src.disconnect();
    } catch (_) {
      /* ignore */
    }
  }
  voiceAnalyser = null;
  livePcmChunks = [];
  livePcmSamples = 0;
}

function startAnalyser(stream) {
  disconnectAnalyserNodes();
  try {
    const ctx = ensureAudioCtx();
    if (!ctx) return;
    const src = ctx.createMediaStreamSource(stream);
    const an = ctx.createAnalyser();
    an.fftSize = 256;
    an.smoothingTimeConstant = 0.18;
    src.connect(an);
    voiceAnalyser = { ctx, src, an, time: new Uint8Array(an.fftSize) };
    startLevelLoop();
    if (typeof ctx.createScriptProcessor !== "function") return;
    const proc = ctx.createScriptProcessor(2048, 1, 1);
    const sink = ctx.createMediaStreamDestination();
    const mute = ctx.createGain();
    mute.gain.value = 0;
    src.connect(proc);
    proc.connect(sink);
    proc.connect(mute);
    mute.connect(ctx.destination);
    livePcmChunks = [];
    livePcmSamples = 0;
    livePcmRate = Math.round(ctx.sampleRate || 48000);
    liveStreamRate =
      typeof window.declaredStreamRate === "function"
        ? window.declaredStreamRate(livePcmRate)
        : livePcmRate;
    openVoiceStream();
    proc.onaudioprocess = (ev) => {
      if (!voiceWanted && !voiceFlushing) return;
      const input = ev.inputBuffer.getChannelData(0);
      if (!voiceHoldArmed && !voiceFlushing) {
        cueVoiceReady();
        sendStreamKeepAlive();
        return;
      }
      const speechFn = typeof window.isSpeechFrame === "function" ? window.isSpeechFrame : null;
      if (speechFn ? speechFn(input) : false) {
        voiceHadSpeech = true;
        voiceSpeechAt = Date.now();
      }
      livePcmChunks.push(new Float32Array(input));
      livePcmSamples += input.length;
      if (voiceFlushing || (speechFn ? speechFn(input) : true)) {
        sendPcmToStream(input);
      } else {
        sendStreamKeepAlive();
      }
      if (typeof voiceFlushWait === "function") voiceFlushWait();
    };
    liveCapture = { proc, sink, mute };
  } catch (_) {
    if (!voiceAnalyser) voiceAnalyser = null;
  }
}

function stopAnalyser() {
  disconnectAnalyserNodes();
}

function snapshotLivePcm() {
  const snap =
    typeof window.snapshotPcm === "function"
      ? window.snapshotPcm(livePcmChunks, livePcmSamples, livePcmRate)
      : { chunks: livePcmChunks.slice(), samples: livePcmSamples, rate: livePcmRate };
  return snap;
}

function wavFromPcm(chunks, sampleRate) {
  return typeof window.encodeWavFromPcm === "function"
    ? window.encodeWavFromPcm(chunks, sampleRate)
    : null;
}

function liveWordsInnerHtml() {
  const caret = `<span class="listen-caret">${escapeHtml(C.listening.caret)}</span>`;
  const raw = state.voice.transcriptRaw || "";
  if (!raw) return state.voice.phase === "sending" ? "" : caret;
  const annotate =
    typeof window.annotateLiveTranscript === "function" ? window.annotateLiveTranscript : null;
  const body = !annotate
    ? escapeHtml(state.voice.transcript || raw)
    : annotate(raw)
        .map((span) => {
          const display = escapeHtml(span.display);
          if (!span.rewritten) return display;
          return `<s class="listen-from">${escapeHtml(span.surface)}</s> <span class="listen-to">${display}</span>`;
        })
        .join(" ");
  if (state.voice.phase === "sending" || voiceFinalizing) return body;
  return body + caret;
}

function paintLiveWords() {
  const words = document.getElementById("liveWords");
  if (!words) return;
  const show = showLiveWords();
  words.hidden = !show;
  if (show) words.innerHTML = liveWordsInnerHtml();
}

function applyLiveTranscript(text, isFinal, replace, confidence) {
  if (!voiceFinalizing) {
    const recentFn =
      typeof window.speechHeardRecently === "function" ? window.speechHeardRecently : null;
    const recent = recentFn
      ? recentFn(voiceSpeechAt, Date.now())
      : voiceSpeechAt && Date.now() - voiceSpeechAt < 1500;
    if (!recent) return;
    const confFn = typeof window.liveConfidenceOk === "function" ? window.liveConfidenceOk : null;
    if (confFn ? !confFn(confidence) : Number(confidence) > 0 && Number(confidence) < 0.55) return;
  }
  const raw = String(text || "").trim();
  const paintFn =
    typeof window.shouldPaintInterim === "function" ? window.shouldPaintInterim : null;
  const correct = typeof window.correctLiveTranscript === "function" ? window.correctLiveTranscript : null;
  let display = raw;
  if (replace) {
    if (paintFn && !paintFn(raw, state.voice.transcriptRaw, false)) return;
    voiceHeardCommitted = "";
  } else if (typeof window.foldStreamTranscript === "function") {
    const folded = window.foldStreamTranscript(
      voiceHeardCommitted,
      raw,
      isFinal,
      state.voice.transcriptRaw,
    );
    if (paintFn && !paintFn(folded.display, state.voice.transcriptRaw, isFinal)) return;
    voiceHeardCommitted = folded.committed;
    display = folded.display;
  } else if (paintFn && !paintFn(raw, state.voice.transcriptRaw, isFinal)) {
    return;
  }
  if (!display) return;
  state.voice.transcriptRaw = display;
  state.voice.transcript = correct ? correct(display) : display;
  paintLiveWords();
}

function voiceStreamUrl(sampleRate) {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  const token = encodeURIComponent(sessionToken || "");
  const rate = Math.round(Number(sampleRate) || liveStreamRate || livePcmRate || 48000);
  return `${proto}//${location.host}/api/v1/mini-app/voice/stream?sample_rate=${rate}&access_token=${token}`;
}

function sendPcmToStream(input) {
  if (voiceFinalizing && !voiceFlushing) return;
  if (!voiceHoldArmed && !voiceFlushing) {
    sendStreamKeepAlive();
    return;
  }
  const toInt16 = window.floatToInt16;
  const toBytes = window.int16Bytes;
  if (typeof toInt16 !== "function" || typeof toBytes !== "function") return;
  let samples = input;
  if (
    typeof window.resampleForStream === "function" &&
    liveStreamRate &&
    livePcmRate &&
    liveStreamRate !== livePcmRate
  ) {
    samples = window.resampleForStream(input, livePcmRate, liveStreamRate);
  }
  const bytes = toBytes(toInt16(samples));
  if (voiceStreamSock && voiceStreamSock.readyState === 1) {
    try {
      voiceStreamSock.send(bytes);
    } catch (_) {
      /* keep recording */
    }
    return;
  }
  // Keep the start of the hold (usually "buy"/"sell") until Deepgram is ready.
  // Do not buffer seconds of audio — a burst dump is heard as noise.
  if (voiceWanted && typeof window.enqueueStreamPcm === "function") {
    window.enqueueStreamPcm(voiceStreamQueue, bytes);
  } else if (voiceWanted && voiceStreamQueue.length < 10) {
    voiceStreamQueue.push(bytes);
  }
}

function sendStreamKeepAlive() {
  if (!voiceStreamSock || voiceStreamSock.readyState !== 1) return;
  const gap = Number(window.STREAM_KEEPALIVE_MS) || 3000;
  const now = Date.now();
  if (voiceKeepAliveAt && now - voiceKeepAliveAt < gap) return;
  voiceKeepAliveAt = now;
  try {
    voiceStreamSock.send(JSON.stringify({ type: "KeepAlive" }));
  } catch (_) {
    /* keep recording */
  }
}

function flushVoiceStreamQueue() {
  if (!voiceStreamSock || voiceStreamSock.readyState !== 1) return;
  while (voiceStreamQueue.length) {
    try {
      voiceStreamSock.send(voiceStreamQueue.shift());
    } catch (_) {
      break;
    }
  }
}

function closeVoiceStream(opts) {
  voiceStreamOnFinal = null;
  voiceStreamReady = false;
  if (!opts || !opts.keepQueue) voiceStreamQueue = [];
  if (voiceStreamFallbackTimer) {
    clearTimeout(voiceStreamFallbackTimer);
    voiceStreamFallbackTimer = null;
  }
  const sock = voiceStreamSock;
  voiceStreamSock = null;
  if (!sock) return;
  try {
    sock.close();
  } catch (_) {
    /* ignore */
  }
}

function openVoiceStream() {
  if (
    voiceStreamSock &&
    (voiceStreamSock.readyState === 0 || voiceStreamSock.readyState === 1)
  ) {
    return;
  }
  closeVoiceStream({ keepQueue: true });
  if (!liveWordsOn() || !sessionToken) {
    startLivePoll();
    return;
  }
  let sock;
  try {
    sock = new WebSocket(voiceStreamUrl(liveStreamRate));
  } catch (_) {
    startLivePoll();
    return;
  }
  voiceStreamSock = sock;
  sock.onopen = () => {
    if (voiceStreamSock !== sock) return;
    stopLivePoll();
  };
  sock.onmessage = (ev) => {
    if (voiceStreamSock !== sock) return;
    let msg;
    try {
      msg = JSON.parse(ev.data);
    } catch (_) {
      return;
    }
    if (msg && msg.type === "ready") {
      voiceStreamReady = true;
      stopLivePoll();
      flushVoiceStreamQueue();
      return;
    }
    if (msg && msg.type === "transcript" && msg.text) {
      liveFromServer = true;
      applyLiveTranscript(msg.text, Boolean(msg.is_final), false, msg.confidence);
    }
    if (msg && msg.type === "error" && !voiceFinalizing) {
      voiceStreamReady = false;
      startLivePoll();
    }
  };
  sock.onerror = () => {
    if (voiceStreamSock !== sock || voiceFinalizing) return;
    voiceStreamReady = false;
    startLivePoll();
  };
  sock.onclose = () => {
    if (voiceStreamSock === sock) {
      voiceStreamSock = null;
      voiceStreamReady = false;
    }
    if (voiceStreamOnFinal) {
      const done = voiceStreamOnFinal;
      voiceStreamOnFinal = null;
      done(state.voice.transcriptRaw || "");
    }
  };
  voiceStreamFallbackTimer = setTimeout(() => {
    voiceStreamFallbackTimer = null;
    if (voiceStreamReady || !voiceWanted || voiceFinalizing) return;
    if (voiceStreamSock && (voiceStreamSock.readyState === 0 || voiceStreamSock.readyState === 1)) {
      return;
    }
    startLivePoll();
  }, 1500);
}

function waitPcmFlushFrames() {
  const need = Math.max(
    1,
    Math.round(Number(window.PCM_FLUSH_FRAMES) || 8),
  );
  const limit = Math.max(
    40,
    Math.round(Number(window.PCM_FLUSH_MS) || 400),
  );
  return new Promise((resolve) => {
    let got = 0;
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      voiceFlushWait = null;
      clearTimeout(timer);
      resolve();
    };
    voiceFlushWait = () => {
      got += 1;
      if (got >= need) finish();
    };
    const timer = setTimeout(finish, limit);
  });
}

function discardVoiceStream() {
  voiceStreamOnFinal = null;
  if (voiceStreamSock && voiceStreamSock.readyState === 1) {
    try {
      voiceStreamSock.send(JSON.stringify({ type: "close" }));
    } catch (_) {
      /* captions are overlay-only */
    }
  }
  closeVoiceStream();
}

function shouldLivePoll() {
  return (
    liveWordsOn() &&
    voiceWanted &&
    voiceReady &&
    !voiceStreamReady &&
    !voiceFinalizing &&
    state.voice.phase === "listening"
  );
}

function stopLivePoll() {
  livePollInFlight = false;
  livePollSeq = 0;
  liveAppliedSeq = 0;
  if (livePollTimer) {
    clearTimeout(livePollTimer);
    livePollTimer = null;
  }
}

function startLivePoll() {
  stopLivePoll();
  if (!liveWordsOn() || voiceStreamReady) return;
  scheduleLivePoll(160);
}

function scheduleLivePoll(delay) {
  if (livePollTimer) clearTimeout(livePollTimer);
  livePollTimer = setTimeout(tickLivePoll, delay);
}

function liveCaptionBlob() {
  if (livePcmSamples >= livePcmRate * 0.18) {
    const wav = wavFromPcm(livePcmChunks.slice(), livePcmRate);
    if (wav && wav.size >= 4000) return wav;
  }
  if (voiceRecorder && typeof voiceRecorder.requestData === "function" && voiceRecorder.state === "recording") {
    try {
      voiceRecorder.requestData();
    } catch (_) {
      /* ignore */
    }
  }
  if (!voiceChunks.length) return null;
  const mime = (voiceRecorder && voiceRecorder.mimeType) || "audio/webm";
  const blob = new Blob(voiceChunks.slice(), { type: mime });
  return blob.size >= 1200 ? blob : null;
}

async function tickLivePoll() {
  livePollTimer = null;
  if (!shouldLivePoll()) return;
  if (livePollInFlight) {
    scheduleLivePoll(180);
    return;
  }
  const blob = liveCaptionBlob();
  if (!blob) {
    scheduleLivePoll(160);
    return;
  }
  const seq = ++livePollSeq;
  livePollInFlight = true;
  try {
    const audio_base64 = await blobToBase64(blob);
    if (!shouldLivePoll()) return;
    const out = await api("/api/v1/mini-app/voice/live", {
      method: "POST",
      body: { audio_base64, mime: blob.type || "audio/wav" },
    });
    if (seq < liveAppliedSeq) return;
    liveAppliedSeq = seq;
    const text = String((out && out.text) || "").trim();
    if (text) {
      liveFromServer = true;
      applyLiveTranscript(text, false, true);
    }
  } catch (_) {
    /* keep last caption */
  } finally {
    livePollInFlight = false;
    if (shouldLivePoll()) scheduleLivePoll(120);
  }
}

function cueVoiceReady() {
  if (voiceChirping || voiceReady || !voiceWanted || state.voice.phase !== "listening") return;
  if (!voiceCaptureArmed) return;
  voiceChirping = true;
  voiceWaitAt = Date.now();
  clearTimeout(voiceReadyTimer);
  voiceReadyTimer = null;
  const chirp = Number(window.CHIRP_MS) || 180;
  const tail = Number(window.CHIRP_TAIL_MS) || 40;
  const armAfterChirp = () => {
    if (!voiceWanted || voiceReady || state.voice.phase !== "listening") return;
    clearTimeout(voiceReadyTimer);
    voiceReadyTimer = setTimeout(markVoiceReady, tail);
  };
  playVoiceOnSound(armAfterChirp);
  // If onended never fires, still arm — do not add a second pause on top of the beep.
  voiceReadyTimer = setTimeout(markVoiceReady, chirp + tail);
  syncVoiceDom();
}

function markVoiceReady() {
  if (voiceReady || !voiceWanted || state.voice.phase !== "listening") return;
  if (!voiceCaptureArmed) return;
  clearTimeout(voiceReadyTimer);
  voiceReadyTimer = null;
  voiceChirping = false;
  livePcmChunks = [];
  livePcmSamples = 0;
  voiceChunks = [];
  startHoldRecorder();
  voiceHoldArmed = true;
  voiceReady = true;
  voiceStartedAt = Date.now();
  openVoiceStream();
  haptic("impact", "light");
  const btn = document.getElementById("voiceBtn");
  if (btn) btn.classList.add("hot");
  syncVoiceDom();
}

function beginListening(btn) {
  if (state.voice.phase === "listening") return;
  if (state.voice.micDenied) {
    onMicDenied();
    return;
  }
  voiceWanted = true;
  voiceReady = false;
  voiceCaptureArmed = false;
  voiceChirping = false;
  voiceHoldArmed = false;
  voiceHadSpeech = false;
  voiceSpeechAt = 0;
  voiceKeepAliveAt = 0;
  voiceWaitAt = 0;
  voiceFlushAt = 0;
  voiceFinalizing = false;
  voiceFlushing = false;
  voiceFlushWait = null;
  voiceStreamReady = false;
  voiceStreamQueue = [];
  voiceHeardCommitted = "";
  clearTimeout(voiceReadyTimer);
  voiceReadyTimer = null;
  voiceChunks = [];
  voiceStartedAt = Date.now();
  state.voice.phase = "listening";
  state.voice.heldMs = 0;
  state.voice.transcript = "";
  state.voice.transcriptRaw = "";
  state.voice.nudge = "";
  haptic("impact", "light");
  syncVoiceDom();
  startVoiceTick();
  if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
    onMicDenied();
    return;
  }
  navigator.mediaDevices
    .getUserMedia({
      audio: {
        channelCount: 1,
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
        voiceIsolation: true,
      },
    })
    .catch(() => navigator.mediaDevices.getUserMedia({ audio: true }))
    .then((stream) => {
      if (!voiceWanted || state.voice.phase !== "listening") {
        stream.getTracks().forEach((t) => t.stop());
        return;
      }
      stream.getAudioTracks().forEach((track) => {
        try {
          track.contentHint = "speech";
        } catch (_) {
          /* optional */
        }
      });
      voiceStream = stream;
      startAnalyser(stream);
      if (!bindHoldRecorder(stream)) return;
      voiceCaptureArmed = true;
      voiceReadyTimer = setTimeout(cueVoiceReady, 280);
      if (livePcmSamples > 0) cueVoiceReady();
      syncVoiceDom();
    })
    .catch(() => {
      onMicDenied();
    });
}

function bindHoldRecorder(stream) {
  const mime = MediaRecorder.isTypeSupported("audio/webm;codecs=opus")
    ? "audio/webm;codecs=opus"
    : MediaRecorder.isTypeSupported("audio/webm")
      ? "audio/webm"
      : "";
  try {
    voiceRecorder = mime
      ? new MediaRecorder(stream, { mimeType: mime })
      : new MediaRecorder(stream);
  } catch (_) {
    onMicDenied();
    return false;
  }
  voiceRecorder.ondataavailable = (ev) => {
    if (ev.data && ev.data.size) voiceChunks.push(ev.data);
  };
  return true;
}

function startHoldRecorder() {
  if (!voiceRecorder || voiceRecorder.state !== "inactive") return;
  try {
    voiceRecorder.start(100);
  } catch (_) {
    try {
      voiceRecorder.start();
    } catch (__) {
      /* MediaRecorder optional — PCM snapshot still covers the hold */
    }
  }
}

function onMicDenied() {
  state.voice.micDenied = true;
  teardownVoice();
  state.voice.phase = "idle";
  syncVoiceDom();
  if (isVoiceHome()) {
    showNudge(C.voice.micDenied);
    pulseTypeInstead();
  } else {
    showToast(C.toasts.voiceDenied);
  }
}

function closeCaptureImmediate() {
  stopVoiceOnSound();
  voiceWanted = false;
  voiceReady = false;
  voiceCaptureArmed = false;
  voiceChirping = false;
  voiceHoldArmed = false;
  voiceFlushing = false;
  voiceFlushWait = null;
  voiceStartedAt = 0;
  clearTimeout(voiceReadyTimer);
  voiceReadyTimer = null;
  clearInterval(voiceTick);
  voiceTick = null;
  stopLivePoll();
  closeVoiceStream();
  stopAnalyser();
  if (voiceStream) {
    voiceStream.getTracks().forEach((t) => t.stop());
    voiceStream = null;
  }
  const btn = document.getElementById("voiceBtn");
  if (btn) btn.classList.remove("hot");
}

function teardownVoice() {
  closeCaptureImmediate();
  voiceFinalizing = false;
  try {
    if (voiceRecorder && voiceRecorder.state !== "inactive") voiceRecorder.stop();
  } catch (_) {
    /* ignore */
  }
  voiceRecorder = null;
  voiceChunks = [];
}

function cancelVoice(reason) {
  if (voiceFinalizing) return;
  teardownVoice();
  state.voice.phase = "idle";
  state.voice.transcript = "";
  state.voice.transcriptRaw = "";
  voiceHeardCommitted = "";
  syncVoiceDom();
  if (reason === "slide") showNudge(C.voice.slideOff);
  else if (reason === "short") showNudge(C.voice.shortTap);
}

function stopRecorderBlob(recorder, mime, chunks) {
  return new Promise((resolve) => {
    if (!recorder || recorder.state === "inactive") {
      resolve(chunks && chunks.length ? new Blob(chunks.slice(), { type: mime }) : null);
      return;
    }
    recorder.onstop = () => {
      resolve(new Blob(voiceChunks.slice(), { type: mime }));
    };
    try {
      if (typeof recorder.requestData === "function" && recorder.state === "recording") {
        recorder.requestData();
      }
      recorder.stop();
    } catch (_) {
      resolve(chunks && chunks.length ? new Blob(chunks.slice(), { type: mime }) : null);
    }
  });
}

async function submitVoiceBlob(blob, mime, started, correlation_id, liveCaption) {
  if (!blob || !blob.size) {
    dropOptimistic(correlation_id);
    state.voice.phase = "idle";
    paint();
    showToast(C.toasts.voiceEmpty);
    return;
  }
  try {
    const audio_base64 = await blobToBase64(blob);
    const duration_secs = started ? (Date.now() - started) / 1000 : undefined;
    const liveRaw = String(liveCaption || "").trim();
    const live_text =
      liveRaw &&
      !(typeof window.isPlaceholderTranscript === "function" && window.isPlaceholderTranscript(liveRaw))
        ? liveRaw
        : undefined;
    if (live_text) landQueuedTask(live_text, correlation_id, correlation_id, { voice: true });
    const out = await api("/api/v1/mini-app/voice", {
      method: "POST",
      body: {
        audio_base64,
        mime: blob.type || mime,
        duration_secs,
        live_text,
      },
    });
    const heard = String((out && (out.transcript || out.heard_echo)) || "").trim();
    const kind =
      (out && (out.voice_kind || (out.heard_handled && out.heard_handled.kind))) || "";
    if (heard) {
      const payload =
        (out && out.send_payload) || {
          kind: "voice",
          message: heard,
          utterance_id: out && out.utterance_id,
          correlation_id: (out && out.correlation_id) || correlation_id,
        };
      payload.message = heard;
      const inTelegram = tg && tg.initData && typeof tg.sendData === "function";
      const skipSend = out && out.skip_send_data;
      if (inTelegram && !skipSend) {
        try {
          tg.sendData(JSON.stringify(payload));
        } catch (_) {
          /* host may still ingest via webhook */
        }
      }
      if (kind === "cant" || kind === "unclear" || kind === "near_match") {
        applyHeardOutcome(out.heard_handled || out, heard, correlation_id);
        burstPoll();
      } else {
        landVoiceDraft(heard, correlation_id, (out && out.instruction_id) || correlation_id);
        burstPoll();
      }
    } else if (live_text) {
      landVoiceDraft(live_text, correlation_id, correlation_id);
      burstPoll();
    } else {
      landMisheard(correlation_id);
    }
  } catch (_) {
    dropOptimistic(correlation_id);
    state.voice.phase = "idle";
    paint();
    showToast(C.toasts.voiceFailed);
  }
}

async function commitVoice() {
  if (voiceFinalizing) return;
  if (!voiceWanted || !voiceReady) {
    cancelVoice("short");
    return;
  }
  const held = voiceStartedAt ? Date.now() - voiceStartedAt : 0;
  if (voiceMode() === "hold" && held < 600) {
    cancelVoice("short");
    return;
  }
  const recorder = voiceRecorder;
  const mime = (recorder && recorder.mimeType) || "audio/webm";
  const started = voiceStartedAt;
  const correlation_id = newId();
  const webmChunks = voiceChunks.slice();
  haptic("notify", "success");
  const liveNow = String(state.voice.transcript || state.voice.transcriptRaw || "").trim();
  const placeholder =
    liveNow &&
    !(typeof window.isPlaceholderTranscript === "function" && window.isPlaceholderTranscript(liveNow))
      ? liveNow
      : C.draftRow.hearing;
  landQueuedTask(placeholder, correlation_id, correlation_id, { voice: true });
  voiceFinalizing = true;
  voiceFlushing = true;
  voiceFlushAt = Date.now();
  stopLivePoll();
  state.voice.phase = "sending";
  paint();
  syncVoiceDom();
  try {
    await waitPcmFlushFrames();
    const pcm = snapshotLivePcm();
    const hadSpeech =
      voiceHadSpeech ||
      (typeof window.holdHadSpeech === "function" && window.holdHadSpeech(pcm.chunks));
    if (!hadSpeech && pcm.samples > 0) {
      dropOptimistic(correlation_id);
      teardownVoice();
      state.voice.phase = "idle";
      paint();
      showToast(C.toasts.voiceEmpty);
      return;
    }
    const liveCaption = String(state.voice.transcript || state.voice.transcriptRaw || "").trim();
    if (liveCaption) {
      landQueuedTask(liveCaption, correlation_id, correlation_id, { voice: true });
      const title = document.querySelector(`[data-row="${correlation_id}"] .title`);
      if (title) title.textContent = liveCaption;
    }
    const rawWav = wavFromPcm(pcm.chunks, pcm.rate);
    const webm = await stopRecorderBlob(recorder, mime, webmChunks);
    const heldSec = started ? (Date.now() - started) / 1000 : 0;
    const picked =
      typeof window.pickHoldAudio === "function"
        ? window.pickHoldAudio(rawWav, webm, pcm.rate, heldSec)
        : rawWav && rawWav.size
          ? rawWav
          : webm;
    const blob =
      picked === rawWav && rawWav && typeof window.padHoldPcm === "function"
        ? wavFromPcm(window.padHoldPcm(pcm.chunks, pcm.rate), pcm.rate) || rawWav
        : picked;
    voiceFlushing = false;
    voiceWanted = false;
    discardVoiceStream();
    closeCaptureImmediate();
    voiceRecorder = null;
    voiceChunks = [];
    state.voice.phase = "sending";
    syncVoiceDom();
    await submitVoiceBlob(blob, (blob && blob.type) || mime, started, correlation_id, liveCaption);
  } finally {
    voiceFinalizing = false;
    if (state.view === "main") paint();
  }
}

function landVoiceDraft(text, correlation_id, instruction_id) {
  landQueuedTask(text, correlation_id, instruction_id, { voice: true });
  state.voice.phase = "drafted";
  state.voice.transcript = "";
  state.voice.transcriptRaw = "";
  paint();
  clearTimeout(voiceDraftTimer);
  voiceDraftTimer = setTimeout(() => {
    if (state.voice.phase === "drafted") {
      state.voice.phase = "idle";
      paint();
    }
  }, 2400);
}

function landMisheard(correlation_id) {
  const existing = state.optimistic.find(
    (r) => r.instruction_id === correlation_id || r.correlation_id === correlation_id,
  );
  const now = nowSecs();
  const row = existing || {
    instruction_id: correlation_id,
    correlation_id,
    kind: "voice",
    fire_kind: "act",
    queued_local: true,
    created_at: now,
  };
  row.sentence = C.draftRow.misheard || "Misheard, try again";
  row.status = "misheard";
  row.display_status = "";
  row.updated_at = now;
  row.execute_at = null;
  row.queued_local = true;
  delete state.fillUx[row.instruction_id];
  if (!existing) {
    state.optimistic = [row].concat(
      state.optimistic.filter(
        (r) => r.instruction_id !== correlation_id && r.correlation_id !== correlation_id,
      ),
    );
  }
  state.voice.phase = "idle";
  state.voice.transcript = "";
  state.voice.transcriptRaw = "";
  paint();
  const id = row.instruction_id || correlation_id;
  const prev = voiceMisheardTimers.get(id);
  if (prev) clearTimeout(prev);
  voiceMisheardTimers.set(
    id,
    setTimeout(() => {
      voiceMisheardTimers.delete(id);
      dropOptimistic(correlation_id, id);
      paint();
    }, MISHEARD_MS),
  );
}

function applyHeardOutcome(handled, fallbackText, correlation_id) {
  const kind = handled && (handled.kind || handled.voice_kind);
  state.voice.phase = "idle";
  state.voice.transcript = "";
  if (kind === "near_match") {
    dropOptimistic(correlation_id);
    state.nearMatch = handled;
    paint();
    return;
  }
  state.nearMatch = null;
  if (kind === "unclear") {
    landMisheard(correlation_id);
    return;
  }
  if (kind === "cant") {
    dropOptimistic(correlation_id);
    const ins = handled.instruction;
    if (ins && ins.instruction_id) {
      const row = {
        ...ins,
        status: "cant",
        display_status: ins.display_status || "can't",
        correlation_id: correlation_id || ins.correlation_id,
        created_at: ins.created_at || nowSecs(),
        updated_at: ins.updated_at || nowSecs(),
      };
      state.optimistic = state.optimistic
        .filter(
          (r) =>
            r.instruction_id !== row.instruction_id &&
            r.correlation_id !== (correlation_id || row.correlation_id),
        )
        .concat([row]);
    }
    paint();
    refreshLedger();
    return;
  }
  if (kind === "resolved") {
    const text =
      (handled && (handled.rewritten_text || handled.remaining_text)) || fallbackText;
    landVoiceDraft(text, correlation_id, handled && handled.instruction_id);
    return;
  }
  paint();
}

async function sendNearMatchChoice(label) {
  haptic("select");
  const correlation_id = newId();
  const payload = {
    correlation_id,
    kind: "text",
    message: label,
  };
  const inTelegram = tg && tg.initData && typeof tg.sendData === "function";
  if (inTelegram) {
    try {
      tg.sendData(JSON.stringify(payload));
    } catch (_) {
      /* host may still ingest via webhook */
    }
  }
  state.nearMatch = null;
  let recorded = null;
  if (!inTelegram || previewState()) {
    try {
      recorded = await api("/api/v1/mini-app/compose", {
        method: "POST",
        body: payload,
      });
    } catch (_) {
      recorded = null;
    }
  }
  if (recorded) applyHeardOutcome(recorded, label, correlation_id);
  else paint();
  refreshLedger();
}

function blobToBase64(blob) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onloadend = () => {
      const data = String(reader.result || "");
      const comma = data.indexOf(",");
      resolve(comma >= 0 ? data.slice(comma + 1) : data);
    };
    reader.onerror = reject;
    reader.readAsDataURL(blob);
  });
}

async function api(path, opts) {
  const headers = { Authorization: "Bearer " + sessionToken };
  const init = { headers };
  if (opts && opts.method) {
    init.method = opts.method;
    headers["Content-Type"] = "application/json";
    init.body = JSON.stringify(opts.body || {});
  }
  const res = await fetch(path, init);
  if (res.status === 401) throw new Error("unauthorized");
  if (res.status === 404) throw new Error("not_found");
  if (!res.ok) throw new Error("http");
  return res.json();
}

async function ensureSession(initData) {
  if (sessionToken) return sessionToken;
  const authRes = await fetch("/api/v1/mini-app/auth", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ init_data: initData }),
  });
  if (authRes.status === 401) throw new Error("unauthorized");
  if (!authRes.ok) throw new Error("auth");
  const auth = await authRes.json();
  sessionToken = auth.token;
  return sessionToken;
}

async function refreshLedger() {
  try {
    const [sum, led] = await Promise.all([
      api("/api/v1/mini-app/ledger/summary"),
      api("/api/v1/mini-app/ledger"),
    ]);
    const prev = Object.fromEntries(instructions().map((r) => [r.instruction_id, r.status]));
    state.summary = {
      holding: sum.holding,
      needs_you: sum.needs_you,
      last_check_at: sum.last_check_at,
    };
    const priorById = Object.fromEntries(
      (state.ledger || []).map((row) => [row.instruction_id, row]),
    );
    const optimisticBySentence = new Map();
    for (const row of state.optimistic) {
      const sentence = String(row.sentence || "").trim().toLowerCase();
      if (sentence) optimisticBySentence.set(sentence, row);
    }
    state.ledger = (led.instructions || []).map((row) => {
      const prior = priorById[row.instruction_id];
      let next = row;
      if (prior && prior.trail && !row.trail) next = { ...row, trail: prior.trail };
      const opt =
        state.optimistic.find(
          (r) =>
            r.instruction_id === row.instruction_id ||
            (r.correlation_id && row.correlation_id && r.correlation_id === row.correlation_id),
        ) || optimisticBySentence.get(String(row.sentence || "").trim().toLowerCase());
      if (opt) {
        adoptFillUx(opt.instruction_id, row.instruction_id);
      }
      return next;
    });
    state.ledgerStatus = "ok";
    const ids = new Set(state.ledger.map((r) => r.instruction_id));
    const cor = new Set(
      state.ledger.map((r) => r.correlation_id).filter(Boolean),
    );
    state.optimistic = state.optimistic.filter((r) => {
      if (ids.has(r.instruction_id) || (r.correlation_id && cor.has(r.correlation_id))) {
        return false;
      }
      const sentence = String(r.sentence || "").trim().toLowerCase();
      if (sentence && (r.queued_local || r.voice_draft)) {
        const matched = state.ledger.some((led) => {
          const s = String(led.sentence || "").trim().toLowerCase();
          return Boolean(s && s === sentence);
        });
        return !matched;
      }
      return true;
    });
    for (const row of state.ledger) {
      if (state.pending[row.instruction_id] && row.status !== prev[row.instruction_id]) {
        delete state.pending[row.instruction_id];
        if (row.status === "paused") showToast(C.toasts.paused);
        if (row.status === "watching") showToast(fillCopy(C.toasts.watching, { date: fmtDate(row.expires_at) }));
      } else if (prev[row.instruction_id] && prev[row.instruction_id] !== row.status) {
        if (row.status === "awaiting_confirm") showToast(fillCopy(C.toasts.trigger, { detail: row.sentence }));
        if (row.status === "done") {
          const view = queueView(row);
          if (!view || view.zone !== "queued") showToast(C.toasts.executed);
        }
      }
    }
    const liveIds = new Set(instructions().map((r) => r.instruction_id));
    for (const id of Object.keys(state.fillUx)) {
      if (!liveIds.has(id)) delete state.fillUx[id];
    }
    if (
      (state.view === "main" || state.view === "sent") &&
      !(state.voice.phase === "listening" && !voiceFinalizing)
    ) {
      const key = mainPaintKey();
      const skip = state.view === "main" && key === lastMainPaintKey;
      if (!skip) paint();
    }
    maybeFlushDue();
    tunePoll();
  } catch (err) {
    if (err.message === "unauthorized") return renderUnauthorized();
    if (!state.ledger.length) state.ledgerStatus = "error";
    else state.ledgerStatus = "stale";
    if (
      state.view === "main" &&
      !(state.voice.phase === "listening" && !voiceFinalizing)
    ) {
      paint();
    }
  }
}

function patchLiveClock() {
  maybeFlushDue();
  if (
    state.view !== "main" ||
    state.searchOpen ||
    (state.voice.phase === "listening" && !voiceFinalizing)
  ) {
    tuneAgeClock();
    return;
  }
  const now = Date.now();
  let needsPaint = false;
  const hb = isVoiceHome() ? homeHeartbeatText() : heartbeatText();
  document.querySelectorAll(".heartbeat").forEach((el) => {
    el.innerHTML = `<span class="dot ${hb.dot}"></span>${escapeHtml(hb.text)}`;
  });
  document.querySelectorAll("[data-row]").forEach((el) => {
    const id = el.getAttribute("data-row");
    const row = instructions().find((r) => r.instruction_id === id);
    if (!row) return;
    const prevPhase = el.getAttribute("data-phase");
    const view = queueView(row, now);
    const preview = previewState();
    if (
      preview &&
      preview !== "dev" &&
      row.status === "pending_execute" &&
      view &&
      view.phase === "fill" &&
      view.fillPct >= 100
    ) {
      row.status = "done";
      row.display_status = "done";
      row.receipt = row.receipt || "filled";
      row.status_changed_at = Math.floor(now / 1000);
    }
    const shown = queueView(row, now);
    const ux = state.fillUx[id];
    if (ux && ux.revealed && !ux.toasted && ux.fillStartedAt && row.status === "done") {
      ux.toasted = true;
      showToast(C.toasts.executed);
    }
    if (!shown) return;
    if (shown.phase !== prevPhase) needsPaint = true;
    if (shown.phase === "wait") {
      const n = shown.remainingDisplay;
      const glyphEl = el.querySelector(".glyph");
      if (glyphEl) glyphEl.textContent = n == null ? "·" : String(n);
      const sub = el.querySelector(".sub");
      if (sub) sub.textContent = fillCopy(C.sub.pendingExecute, { n: n == null ? "—" : n });
      const count = el.querySelector(".count-chip");
      if (count && n != null) count.textContent = String(n);
      const meter = el.querySelector(".meter span");
      if (meter) meter.style.width = "0%";
    } else if (shown.phase === "fill" || shown.phase === "sliced") {
      const meter = el.querySelector(".meter span");
      if (meter && shown.fillPct != null) meter.style.width = Number(shown.fillPct) + "%";
      const slot = el.querySelector(".pct-slot");
      if (slot && shown.fillPct != null) slot.textContent = String(shown.fillPct) + "%";
      if (shown.phase === "fill") {
        const sub = el.querySelector(".sub");
        if (sub) sub.textContent = C.sub.completing;
      }
    }
  });
  tuneAgeClock();
  if (needsPaint) paint();
}

function startPoll() {
  clearInterval(pollTimer);
  clearInterval(ageTimer);
  ageTimerMs = 0;
  pollTimer = setInterval(refreshLedger, 2000);
  tuneAgeClock();
}

function tuneAgeClock() {
  const filling =
    !reduceMotion &&
    instructions().some((row) => {
      const view = presentQueue(row, state.fillUx[row.instruction_id], Date.now(), {
        reduceMotion,
      });
      return view && view.phase === "fill";
    });
  const ms = filling ? 50 : 250;
  if (ageTimer && ageTimerMs === ms) return;
  ageTimerMs = ms;
  clearInterval(ageTimer);
  ageTimer = setInterval(patchLiveClock, ms);
}

function tunePoll() {
  const now = Date.now();
  const hot = instructions().some(
    (row) =>
      row.status === "pending_execute" ||
      row.status === "executing" ||
      isQueueHot(row, state.fillUx[row.instruction_id], now, { reduceMotion }),
  );
  clearInterval(pollTimer);
  pollTimer = setInterval(refreshLedger, hot ? 500 : 2000);
}

function burstPoll() {
  clearInterval(burstTimer);
  let n = 0;
  burstTimer = setInterval(() => {
    refreshLedger();
    if (++n >= 12) clearInterval(burstTimer);
  }, 500);
}

function maybeFlushDue() {
  const preview = previewState();
  if (preview && preview !== "dev") return;
  for (const row of state.ledger) {
    if (row.queued_local) continue;
    if (!isFlushDue(row)) continue;
    const key = flushKey(row);
    if (flushedIds.has(key)) continue;
    flushedIds.add(key);
    api("/api/v1/mini-app/compose", {
      method: "POST",
      body: { kind: "flush_execute", instruction_id: row.instruction_id, message: "" },
    })
      .then(() => refreshLedger())
      .catch(() => {
        flushedIds.delete(key);
      });
  }
}

function isFlushDue(row) {
  if (!row) return false;
  if (row.status === "pending_execute") {
    const rem = remainingQueueSecs(row, Date.now());
    return rem != null && rem <= 0;
  }
  if (row.status === "executing") {
    if (row.next_slice_at) return nowSecs() >= Number(row.next_slice_at);
    const fills = Array.isArray(row.child_fills) ? row.child_fills.length : 0;
    return fills === 0;
  }
  return false;
}

function flushKey(row) {
  return [
    row.instruction_id,
    row.status,
    row.slice_i || 0,
    row.next_slice_at || row.execute_at || 0,
  ].join(":");
}

function renderUnauthorized() {
  app.innerHTML =
    headerHtml("root") +
    `<div class="center"><p>${escapeHtml(C.unauthorized)}</p></div>` +
    bottomHtml();
  bindChrome();
}

function renderError(retry) {
  haptic("notify", "error");
  app.innerHTML =
    headerHtml("root") +
    `<div class="center"><p>${escapeHtml(C.loadError)}</p></div>
     <div class="pad"><button type="button" class="primary-btn" id="retry">${escapeHtml(C.retry)}</button></div>` +
    bottomHtml();
  bindChrome();
  const r = document.getElementById("retry");
  if (r) r.onclick = retry;
}

const PREVIEW_PORTFOLIO = {
  positions: [
    { symbol: "ETH", quantity: "2.35", usd_value: "8432.50", asset_type: "spot", group: "holdings", extra: "free collateral", can_exit: true, watch_count: 1, keywords: "ETH spot" },
    { symbol: "ETH-PERP", quantity: "1.20", usd_value: "4310.00", asset_type: "perp", group: "positions", extra: "floor $2410.00", can_exit: true, watch_count: 2, keywords: "ETH perp", side: "long" },
    { symbol: "USDC", quantity: "2000", usd_value: "2000.00", asset_type: "lend", group: "lending", extra: "fixed term", can_exit: false, watch_count: 1, keywords: "USDC lend" },
  ],
  dollarpower: { ratio: "6.8", equivalent_usd: "43100.00", committed_usd: "6338.00", fill_pct: "15", is_estimate: false },
  risk: { liquidation_score: 7.8, band: "safe", distance_from_floor_pct: "47", is_estimate: false },
  total_usd_value: "24761.18",
  total_change_24h_pct: "1.30",
  floor: "2410.00",
  flags: {
    primary_view: "ledger",
    jobline_negative: false,
    family: "blue",
    voice_home: true,
    voice_mode: "hold",
    live_words: true,
  },
};

const PREVIEW_PRODUCTS = [
  { id: "spot:ETH", symbol: "ETH", name: "Ether", product: "spot", quote_symbol: "USDT", mark_price: "3588.12", keywords: "ETH ether spot usdt" },
  { id: "perp:ETH", symbol: "ETH", name: "Ether", product: "perp", quote_symbol: "USDT", mark_price: "3588.12", keywords: "ETH ether perp perpetual usdt" },
  { id: "lend:ETH", symbol: "ETH", name: "Ether", product: "lend", mark_price: "3588.12", keywords: "ETH ether lend lending" },
  { id: "spot:WBTC", symbol: "WBTC", name: "Wrapped Bitcoin", product: "spot", quote_symbol: "USDT", mark_price: "97500.00", keywords: "WBTC wrapped bitcoin btc spot usdt" },
  { id: "perp:WBTC", symbol: "WBTC", name: "Wrapped Bitcoin", product: "perp", quote_symbol: "USDT", mark_price: "97500.00", keywords: "WBTC wrapped bitcoin btc perp perpetual usdt" },
  { id: "lend:WBTC", symbol: "WBTC", name: "Wrapped Bitcoin", product: "lend", mark_price: "97500.00", keywords: "WBTC wrapped bitcoin btc lend lending" },
  { id: "spot:SOL", symbol: "SOL", name: "Solana", product: "spot", quote_symbol: "USDT", mark_price: "178.40", keywords: "SOL solana spot usdt" },
  { id: "perp:SOL", symbol: "SOL", name: "Solana", product: "perp", quote_symbol: "USDT", mark_price: "178.40", keywords: "SOL solana perp perpetual usdt" },
  { id: "lend:SOL", symbol: "SOL", name: "Solana", product: "lend", mark_price: "178.40", keywords: "SOL solana lend lending" },
  { id: "lend:USDC", symbol: "USDC", name: "USD Coin", product: "lend", mark_price: "1", keywords: "USDC usd coin lend lending" },
  { id: "lend:USDT", symbol: "USDT", name: "Tether", product: "lend", mark_price: "1", keywords: "USDT tether lend lending" },
];

function previewBoot() {
  const pv = previewState();
  state.portfolio = PREVIEW_PORTFOLIO;
  state.products = PREVIEW_PRODUCTS;
  applyFlags(PREVIEW_PORTFOLIO.flags);
  if (pv === "empty") {
    state.ledger = [];
    state.summary = { holding: 0, needs_you: 0, last_check_at: null };
    state.ledgerStatus = "ok";
    state.compact = false;
    return paint();
  }
  if (pv === "error") {
    state.ledgerStatus = "error";
    state.compact = false;
    return paint();
  }
  if (pv === "unauthorized") return renderUnauthorized();
  state.ledger = [
    {
      instruction_id: "buy",
      task_id: "buy",
      status: "pending_execute",
      sentence: "Buy 0.1 ETH spot at market",
      kind: "trade",
      fire_kind: "act",
      execute_at: nowSecs() + 3,
      delay_secs: 3,
      progress_pct: 0,
      remaining_secs: 3,
    },
    {
      instruction_id: "fill",
      task_id: "fill",
      status: "executing",
      sentence: "Buy 10 ETH over the next hour",
      kind: "trade",
      fire_kind: "act",
      order_type: "twap",
      progress_pct: 40,
      slice_i: 2,
      slice_n: 5,
      avg_price: "3588.12",
    },
    {
      instruction_id: "roll",
      task_id: "roll",
      status: "awaiting_confirm",
      display_status: "needs you",
      sentence: "At maturity, roll the lend into the 30-day if the rate holds at 9% or better",
      kind: "conditional",
      fire_kind: "act",
      expires_at: nowSecs() + 86400 * 5,
    },
    {
      instruction_id: "perp",
      task_id: "perp",
      status: "watching",
      display_status: "watching",
      sentence: "If ETH touches $3,400, close half the perp",
      kind: "conditional",
      fire_kind: "act",
      check_stats: { last_check_at: nowSecs() - 4, checks_7d: 12 },
      expires_at: nowSecs() + 86400 * 18,
      distance: { mark: "3588", pct: 72, near: true },
    },
    {
      instruction_id: "floor",
      task_id: "floor",
      status: "watching",
      display_status: "watching",
      sentence: "If ETH drops to $2,650 (floor +10%), tell me",
      kind: "watch",
      fire_kind: "tell",
      check_stats: { last_check_at: nowSecs() - 4, checks_7d: 8 },
      expires_at: nowSecs() + 86400 * 26,
      distance: { mark: "3588", pct: 26, near: false },
    },
    {
      instruction_id: "cant-beef",
      task_id: "cant-beef",
      status: "cant",
      display_status: "can't",
      sentence: "Buy fifty of beef",
      kind: "cant",
      asked_entity: "beef",
      cant_kind: "no_market",
      status_changed_at: nowSecs() - 20,
      updated_at: nowSecs() - 20,
    },
    {
      instruction_id: "done-eth",
      task_id: "done-eth",
      status: "done",
      display_status: "done",
      sentence: "Buy 0.05 ETH spot at market",
      kind: "trade",
      receipt: "filled 0.05 ETH",
      status_changed_at: nowSecs() - 120,
      updated_at: nowSecs() - 120,
    },
  ];
  state.summary = { holding: 5, needs_you: 1, last_check_at: nowSecs() - 4 };
  state.ledgerStatus = "ok";
  state.compact = pv !== "loaded";
  loadSpeechOntology();
  paint();
  ageTimerMs = 0;
  tuneAgeClock();
}

function chartParams() {
  const q = new URLSearchParams(location.search);
  let symbol = q.get("symbol");
  let period = (q.get("period") || "").toLowerCase();
  const start = startParam();
  if (!symbol) {
    const parsed = parseChartStart(start);
    if (parsed) {
      symbol = parsed.symbol;
      period = period || parsed.period;
    }
  }
  const onChart =
    location.pathname === "/chart" || location.pathname.endsWith("/chart") || !!symbol;
  if (!onChart) return null;
  if (!symbol) symbol = "AAPL";
  if (period !== "d" && period !== "w" && period !== "m") period = "d";
  return { symbol, period };
}

function destroyChart() {
  if (chartHandle) {
    chartHandle.remove();
    chartHandle = null;
    candleSeries = null;
  }
}

function fmtChartPrice(v) {
  if (!Number.isFinite(v)) return "—";
  const d = Math.abs(v) >= 1000 ? 1 : Math.abs(v) >= 10 ? 2 : 4;
  return "$" + v.toLocaleString("en-US", { minimumFractionDigits: d, maximumFractionDigits: d });
}

function mountCandles(el, bars) {
  destroyChart();
  const LC = window.LightweightCharts;
  if (!LC || !el) return false;
  chartHandle = LC.createChart(el, {
    layout: {
      background: { color: "#0e1116" },
      textColor: "rgba(232,237,243,0.55)",
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
    },
    grid: {
      vertLines: { color: "rgba(151,168,190,0.08)" },
      horzLines: { color: "rgba(151,168,190,0.08)" },
    },
    rightPriceScale: { borderColor: "rgba(151,168,190,0.13)" },
    timeScale: { borderColor: "rgba(151,168,190,0.13)", timeVisible: true, secondsVisible: false },
    crosshair: { mode: LC.CrosshairMode.Normal },
    handleScroll: { mouseWheel: true, pressedMouseMove: true, horzTouchDrag: true },
    handleScale: { axisPressedMouseMove: true, pinch: true, mouseWheel: true },
  });
  candleSeries = chartHandle.addCandlestickSeries({
    upColor: "#46c08a",
    downColor: "#f07878",
    wickUpColor: "#46c08a",
    wickDownColor: "#f07878",
    borderVisible: false,
  });
  candleSeries.setData(
    bars
      .filter((b) => Number.isFinite(b.t) && Number.isFinite(b.o))
      .map((b) => ({ time: b.t, open: b.o, high: b.h, low: b.l, close: b.c })),
  );
  chartHandle.timeScale().fitContent();
  const ro = new ResizeObserver(() => {
    if (!chartHandle) return;
    chartHandle.applyOptions({ width: el.clientWidth, height: el.clientHeight });
  });
  ro.observe(el);
  chartHandle.applyOptions({ width: el.clientWidth, height: el.clientHeight });
  return true;
}

function renderChartShell(params, status, data) {
  document.body.className = "page-chart";
  document.title = params.symbol + " · World Markets";
  const last = data && data.candles && data.candles.length ? data.candles[data.candles.length - 1] : null;
  const first = data && data.candles && data.candles[0];
  const up = last && first ? last.c >= first.o : true;
  const px = last ? fmtChartPrice(last.c) : "—";
  const sub = data
    ? `${escapeHtml(data.period_label)} · ${escapeHtml(data.bar_label)} bars · pinch to zoom`
    : "Loading…";
  const periods = ["d", "w", "m"]
    .map((p) => {
      const label = p === "d" ? "1D" : p === "w" ? "1W" : "1M";
      const on = params.period === p ? " active" : "";
      return `<button type="button" class="${on}" data-period="${p}">${label}</button>`;
    })
    .join("");
  let body = "";
  if (status === "loading") body = `<p class="chart-sub">Loading…</p><div class="skel"></div>`;
  else if (status === "error") body = `<div class="center"><p>Could not load chart</p></div>`;
  else if (status === "empty") body = `<div class="center"><p>No bars for ${escapeHtml(params.symbol)}.</p></div>`;
  else body = `<div id="plot"></div>`;
  app.innerHTML = `${headerHtml("inner")}
    <div class="chart-page">
      <div class="chart-meta">
        <span class="chart-sym">${escapeHtml(params.symbol)}</span>
        <span class="chart-px num ${up ? "up" : "down"}">${px}</span>
      </div>
      <div class="chart-sub">${sub}</div>
      <div class="periods">${periods}</div>
      ${body}
    </div>`;
  bindChrome();
  app.querySelectorAll("[data-period]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const next = btn.getAttribute("data-period");
      if (!next || next === params.period) return;
      haptic("select");
      const url = new URL(location.href);
      url.pathname = "/chart";
      url.searchParams.set("symbol", params.symbol);
      url.searchParams.set("period", next);
      history.replaceState({}, "", url);
      loadChartView({ symbol: params.symbol, period: next });
    });
  });
  if (status === "ready") {
    const plot = document.getElementById("plot");
    if (!mountCandles(plot, data.candles)) {
      plot.innerHTML = `<div class="center"><p>Chart library failed to load.</p></div>`;
    }
  }
}

async function loadChartView(params) {
  state.view = "chart";
  renderChartShell(params, "loading", null);
  const initData = (tg && tg.initData) || (previewState() === "dev" ? "dev" : "");
  try {
    const token = await ensureSession(initData);
    sessionToken = token;
    const res = await fetch(
      "/api/v1/mini-app/chart?symbol=" +
        encodeURIComponent(params.symbol) +
        "&period=" +
        encodeURIComponent(params.period),
      { headers: { Authorization: "Bearer " + token } },
    );
    if (res.status === 401) return renderUnauthorized();
    if (res.status === 404) return renderChartShell(params, "empty", null);
    if (!res.ok) return renderChartShell(params, "error", null);
    const data = await res.json();
    if (!data.candles || data.candles.length === 0) return renderChartShell(params, "empty", data);
    params.symbol = data.symbol || params.symbol;
    renderChartShell(params, "ready", data);
  } catch (err) {
    if (err && err.message === "unauthorized") return renderUnauthorized();
    renderChartShell(params, "error", null);
  }
}

function warmMic() {
  if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) return;
  navigator.mediaDevices
    .getUserMedia({ audio: true })
    .then((stream) => stream.getTracks().forEach((t) => t.stop()))
    .catch(() => {
      /* permission prompt happens on first hold */
    });
}

function loadSpeechOntology() {
  const apply = (data) => {
    if (data && data.entries && typeof window.setOntologyEntries === "function") {
      window.setOntologyEntries(data.entries);
    }
  };
  fetch("/api/v1/mini-app/speech-ontology")
    .then((res) => (res.ok ? res.json() : null))
    .then(apply)
    .catch(() => {});
}

async function boot() {
  loadSpeechOntology();
  const preview = previewState();
  const chart = chartParams();
  if (chart) return loadChartView(chart);
  if (preview && preview !== "dev") return previewBoot();

  state.ledgerStatus = "loading";
  paint();
  const initData = (tg && tg.initData) || (preview === "dev" ? "dev" : "");
  try {
    await ensureSession(initData);
    const [port, catalog] = await Promise.all([
      api("/api/v1/mini-app/portfolio"),
      api("/api/v1/mini-app/products").catch(() => ({ products: [] })),
    ]);
    state.portfolio = port;
    state.products = catalog.products || [];
    if (port.flags) applyFlags(port.flags);
    if (state.flags.primary_view === "portfolio") state.tab = "portfolio";
    await refreshLedger();
    state.compact = Boolean(tg && !tg.isExpanded);
    const deep = instructionStart(startParam());
    if (deep) {
      try {
        const one = await api("/api/v1/mini-app/ledger/" + encodeURIComponent(deep));
        const ins = one.instruction || one;
        if (ins && ins.instruction_id) {
          const idx = state.ledger.findIndex((r) => r.instruction_id === ins.instruction_id);
          if (idx >= 0) state.ledger[idx] = { ...state.ledger[idx], ...ins };
          else state.ledger.push(ins);
          state.compact = false;
          state.sheet = "instruction";
          state.insId = ins.instruction_id;
          state.detent = "half";
        }
      } catch (_) {
        /* stale/foreign id — default view */
      }
    }
    paint();
    startPoll();
    warmMic();
  } catch (err) {
    if (err && err.message === "unauthorized") return renderUnauthorized();
    renderError(() => boot());
  }
}

boot();
