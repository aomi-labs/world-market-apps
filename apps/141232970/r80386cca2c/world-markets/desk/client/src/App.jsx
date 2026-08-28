import { useEffect, useMemo, useRef, useState } from "react";

const EMPTY_SLOTS = {
  side: null,
  quantity: null,
  instrument: null,
  order_type: null,
  limit_price: null,
  stop_price: null,
  duration: null,
};

function slotFilled(value) {
  if (value == null || value === "") return false;
  if (typeof value === "object") return Object.values(value).some((v) => v != null && v !== "");
  return true;
}

function formatMoney(value) {
  if (value == null || value === "") return "—";
  const n = Number(value);
  if (!Number.isFinite(n)) return String(value);
  return n.toLocaleString("en-US", { maximumFractionDigits: n >= 100 ? 2 : 4 });
}

function formatSlot(key, value) {
  if (!slotFilled(value)) return null;
  if (key === "quantity" && value && typeof value === "object") {
    if (value.kind === "dollars") return `$${formatMoney(value.value)}`;
    if (value.kind === "pct_of_position") return `${value.value}% of position`;
    return String(value.value);
  }
  if (key === "instrument" && value && typeof value === "object") {
    return { name: value.name, meta: `${value.symbol} · ${value.product}` };
  }
  if (key === "limit_price" || key === "stop_price") return formatMoney(value);
  if (key === "side") return String(value).toUpperCase();
  if (key === "order_type") return String(value).replace("_", " ").toUpperCase();
  if (key === "duration") return String(value).toUpperCase();
  return String(value);
}

function ageLabel(iso) {
  if (!iso) return null;
  const then = Date.parse(iso);
  if (!Number.isFinite(then)) return null;
  const sec = Math.max(1, Math.round((Date.now() - then) / 1000));
  if (sec < 60) return `${sec}s ago`;
  return `${Math.round(sec / 60)}m ago`;
}

function comparatorLabel(c) {
  return { lt: "below", lte: "at or below", gt: "above", gte: "at or above" }[c] || c;
}

function Ticket({ card }) {
  const slots = card?.slots || EMPTY_SLOTS;
  const state = card?.state || "assembling";
  const conf = card?.confidence || {};
  const keys = ["side", "quantity", "instrument", "order_type", "limit_price", "duration"];
  if (slotFilled(slots.stop_price)) keys.splice(5, 0, "stop_price");

  return (
    <article className={`card ticket ticket-${state}`}>
      <header className="card-head">
        <span className="kicker">Ticket</span>
        <span className="ticket-id">{card?.ticket_id || "no draft"}</span>
        <span className={`stamp stamp-${state}`}>
          {state === "stamped" ? "STAMPED" : state === "readback" ? "READBACK" : "ASSEMBLING"}
        </span>
      </header>
      <div className="slot-grid">
        {keys.map((key) => {
          const filled = slotFilled(slots[key]);
          const low = (conf[key] ?? 1) < 0.9 && filled;
          const display = formatSlot(key, slots[key]);
          const instrument = display && typeof display === "object";
          return (
            <div key={key} className={`slot ${filled ? "filled" : "empty"} ${low ? "low" : ""}`}>
              <span className="slot-label">
                {key.replace("_", " ")}
                {low ? <em>low</em> : null}
              </span>
              {instrument ? (
                <>
                  <span className="slot-value">{display.name}</span>
                  <span className="slot-meta">{display.meta}</span>
                </>
              ) : (
                <span className="slot-value">{display || "—"}</span>
              )}
            </div>
          );
        })}
      </div>
      {card?.consequence ? <p className="consequence">{card.consequence}</p> : null}
      {state === "readback" ? (
        <footer className="assent-legend">
          <span>
            <kbd>Done</kbd> place
          </span>
          <span>
            <kbd>Off</kbd> cancel
          </span>
          <span>
            <kbd>Hold</kbd> park
          </span>
        </footer>
      ) : null}
      {state === "stamped" ? <div className="stamp-mark" aria-hidden="true">WORLD</div> : null}
    </article>
  );
}

function Book({ card }) {
  const payload = card?.payload || {};
  const positions = payload.positions;
  if (Array.isArray(positions)) {
    return (
      <article className="card book">
        <header className="card-head">
          <span className="kicker">Book</span>
          <span className="stamp">WORLD</span>
        </header>
        <div className="book-totals">
          <div>
            <span className="slot-label">Equity</span>
            <span className="slot-value">{formatMoney(payload.equity)}</span>
          </div>
          <div>
            <span className="slot-label">Cash</span>
            <span className="slot-value">{formatMoney(payload.cash)}</span>
          </div>
        </div>
        {positions.length === 0 ? (
          <p className="empty-line">Flat.</p>
        ) : (
          <table className="book-table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Qty</th>
                <th>Mark</th>
                <th>Avg</th>
              </tr>
            </thead>
            <tbody>
              {positions.map((p) => (
                <tr key={`${p.symbol}-${p.product}`}>
                  <td>
                    {p.symbol}
                    <span className="slot-meta"> {p.product}</span>
                  </td>
                  <td>{p.quantity}</td>
                  <td>{formatMoney(p.mark)}</td>
                  <td>{formatMoney(p.avg_price)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </article>
    );
  }
  return (
    <article className="card book">
      <header className="card-head">
        <span className="kicker">Book</span>
        <span className="stamp">{ageLabel(payload.as_of) || "QUOTE"}</span>
      </header>
      <p className="quote-name">{payload.name || payload.symbol || "—"}</p>
      <p className="quote-meta">
        {payload.symbol} · {payload.product}
      </p>
      <p className="quote-mark">{formatMoney(payload.mark)}</p>
      <div className="book-totals quote-spread">
        <div>
          <span className="slot-label">Bid</span>
          <span className="slot-value">{formatMoney(payload.bid)}</span>
        </div>
        <div>
          <span className="slot-label">Ask</span>
          <span className="slot-value">{formatMoney(payload.ask)}</span>
        </div>
      </div>
    </article>
  );
}

function Registry({ card }) {
  const m = card?.payload?.mandate;
  if (!m) {
    return (
      <article className="card registry">
        <header className="card-head">
          <span className="kicker">Registry</span>
          <span className="stamp">EMPTY</span>
        </header>
        <p className="empty-line">No standing rule.</p>
      </article>
    );
  }
  const inst = m.trigger?.instrument;
  return (
    <article className="card registry">
      <header className="card-head">
        <span className="kicker">Registry</span>
        <span className={`stamp stamp-${m.status}`}>{String(m.status || "draft").toUpperCase()}</span>
      </header>
      <p className="quote-name">{m.name || "Unnamed rule"}</p>
      <dl className="receipt">
        <div>
          <dt>Trigger</dt>
          <dd>
            {inst?.name || inst?.symbol || m.trigger?.instrument_query || "—"}{" "}
            {comparatorLabel(m.trigger?.comparator)} {formatMoney(m.trigger?.price)}
          </dd>
        </div>
        <div>
          <dt>Action</dt>
          <dd>
            {m.action?.side || "—"} {m.action?.quantity?.value ?? ""} {inst?.symbol || ""}
          </dd>
        </div>
        <div>
          <dt>Expiry</dt>
          <dd>{m.expiry_days ? `${m.expiry_days} days` : "—"}</dd>
        </div>
        <div>
          <dt>Rationale</dt>
          <dd>{m.rationale_text || "—"}</dd>
        </div>
      </dl>
    </article>
  );
}

function Queue({ card }) {
  const p = card?.payload || {};
  const rows = [
    ["World", p.world],
    ["Names", p.names],
    ["Mandates", p.mandates],
    ["Decisions", p.decisions],
  ].filter(([, v]) => v);
  return (
    <article className="card queue">
      <header className="card-head">
        <span className="kicker">Queue</span>
        <span className="stamp">THE OPEN</span>
      </header>
      <ul className="queue-list">
        {rows.map(([label, text]) => (
          <li key={label}>
            <span className="slot-label">{label}</span>
            <span>{text}</span>
          </li>
        ))}
      </ul>
    </article>
  );
}

function Disambiguation({ card, onPick }) {
  const cands = card?.payload?.candidates || [];
  return (
    <article className="card disambiguation">
      <header className="card-head">
        <span className="kicker">Which name?</span>
        <span className="stamp">CHOOSE</span>
      </header>
      <ol className="choices">
        {cands.map((c, i) => (
          <li key={c.symbol + c.product}>
            <button type="button" onClick={() => onPick(`${c.product} ${c.symbol}`)}>
              <span className="choice-n">{i + 1}</span>
              <span>
                <strong>{c.name}</strong>
                <span className="slot-meta">
                  {c.symbol} · {c.product}
                  {c.last_price != null ? ` · ${formatMoney(c.last_price)}` : ""}
                </span>
              </span>
            </button>
          </li>
        ))}
      </ol>
    </article>
  );
}

function pickKind({ ticket, book, registry, queue, disambiguation, lastKind }) {
  if (disambiguation) return "disambiguation";
  if (ticket?.state === "readback") return "ticket";
  return lastKind || "ticket";
}

export default function App() {
  const [status, setStatus] = useState("offline");
  const [mic, setMic] = useState("ptt");
  const [sessionId, setSessionId] = useState("");
  const [speech, setSpeech] = useState([]);
  const [ticket, setTicket] = useState(null);
  const [book, setBook] = useState(null);
  const [registry, setRegistry] = useState(null);
  const [queue, setQueue] = useState(null);
  const [disambiguation, setDisambiguation] = useState(null);
  const [lastKind, setLastKind] = useState("ticket");
  const [draft, setDraft] = useState("");
  const [holding, setHolding] = useState(false);
  const wsRef = useRef(null);
  const inputRef = useRef(null);

  const wsUrl = useMemo(() => {
    const explicit = import.meta.env.VITE_DESK_WS;
    if (explicit) return explicit;
    if (import.meta.env.DEV) return "ws://127.0.0.1:8765/ws";
    const proto = location.protocol === "https:" ? "wss" : "ws";
    return `${proto}://${location.host}/ws`;
  }, []);

  useEffect(() => {
    const ws = new WebSocket(wsUrl);
    wsRef.current = ws;
    ws.onopen = () => {
      setStatus("connected");
      ws.send(JSON.stringify({ type: "hello" }));
    };
    ws.onclose = () => setStatus("offline");
    ws.onmessage = (ev) => {
      const msg = JSON.parse(ev.data);
      applyServerMessage(msg);
    };
    return () => ws.close();
  }, [wsUrl]);

  function applyServerMessage(msg) {
    if (!msg) return;
    if (msg.type === "hello" && msg.session_id) setSessionId(msg.session_id);
    const spoken = msg.text || msg.speech;
    if (spoken && msg.type !== "hello") {
      setSpeech((s) => [...s.slice(-12), { who: "desk", text: spoken }]);
    }
    const card = msg.card && typeof msg.card === "object" ? msg.card : null;
    if (card?.card === "ticket") setTicket(card);
    if (card?.card === "book") setBook(card);
    if (card?.card === "registry") setRegistry(card);
    if (card?.card === "queue") setQueue(card);
    if (card?.card === "disambiguation") setDisambiguation(card);
    else if (card?.card) setDisambiguation(null);
    if (card?.card) setLastKind(card.card);
    if (msg.earcon) {
      const audio = new Audio(`/earcons/${msg.earcon}.wav`);
      audio.play().catch(() => {});
    }
    if (msg.tts_complete) applyServerMessage(msg.tts_complete);
  }

  async function sendTranscript(text) {
    if (!text.trim()) return;
    setSpeech((s) => [...s.slice(-12), { who: "you", text }]);
    setDraft("");
    try {
      let sid = sessionId;
      if (!sid) {
        const created = await fetch("/api/session", { method: "POST" });
        sid = (await created.json()).session_id;
        setSessionId(sid);
      }
      const res = await fetch(`/api/inject/${sid}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ text }),
      });
      if (res.ok) applyServerMessage({ type: "turn", ...(await res.json()) });
    } catch {
      if (wsRef.current && wsRef.current.readyState === WebSocket.OPEN) {
        wsRef.current.send(JSON.stringify({ type: "transcript", text }));
      }
    }
  }

  function onPttDown() {
    setHolding(true);
  }
  function onPttUp() {
    setHolding(false);
    if (mic === "ptt") sendTranscript(inputRef.current?.value || draft);
  }

  const kind = pickKind({ ticket, book, registry, queue, disambiguation, lastKind });
  const lastDesk = [...speech].reverse().find((l) => l.who === "desk");

  return (
    <div className="desk">
      <header className="top">
        <div className="brand">
          <span className="mark" />
          The Desk
        </div>
        <div className="meta">
          <span className={`dot ${status}`} />
          {status}
          <span className="sep" />
          world
          <span className="sep" />
          {sessionId || "…"}
        </div>
        <div className="mic-toggle">
          <button className={mic === "ptt" ? "on" : ""} onClick={() => setMic("ptt")}>
            Push to talk
          </button>
          <button className={mic === "open" ? "on" : ""} onClick={() => setMic("open")}>
            Open mic
          </button>
        </div>
      </header>

      <p className="now">{lastDesk?.text || "The desk is here."}</p>

      <div className="stage">
        {kind === "book" && book ? <Book card={book} /> : null}
        {kind === "registry" && registry ? <Registry card={registry} /> : null}
        {kind === "queue" && queue ? <Queue card={queue} /> : null}
        {kind === "disambiguation" && disambiguation ? (
          <Disambiguation card={disambiguation} onPick={sendTranscript} />
        ) : null}
        {kind === "ticket" ? <Ticket card={ticket} /> : null}
      </div>

      <ol className="tape">
        {speech.slice(-6).map((line, i) => (
          <li key={`${i}-${line.text.slice(0, 12)}`} className={line.who}>
            <span className="who">{line.who}</span>
            {line.text}
          </li>
        ))}
      </ol>

      <form
        className={`composer ${holding ? "hot" : ""}`}
        onSubmit={(e) => {
          e.preventDefault();
          const value = inputRef.current?.value || draft;
          sendTranscript(value);
        }}
      >
        <button
          type="button"
          className="ptt"
          onMouseDown={onPttDown}
          onMouseUp={onPttUp}
          onMouseLeave={() => setHolding(false)}
        >
          {mic === "ptt" ? "Hold to talk" : "Open"}
        </button>
        <input
          ref={inputRef}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder={
            mic === "ptt"
              ? "Type the utterance, then release push-to-talk or press return"
              : "Open mic (text stand-in) — return sends"
          }
        />
        <button type="submit">Send</button>
      </form>
    </div>
  );
}
