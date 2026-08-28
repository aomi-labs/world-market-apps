/** Tool-filled copy. Numbers only from record fields — never estimated here. */

function mono(value) {
  if (value == null || value === "") return "`[#]`";
  return `\`${value}\``;
}

function dayMonth(unix) {
  if (!unix) return "";
  const d = new Date(Number(unix) * 1000);
  const months = [
    "Jan",
    "Feb",
    "Mar",
    "Apr",
    "May",
    "Jun",
    "Jul",
    "Aug",
    "Sep",
    "Oct",
    "Nov",
    "Dec",
  ];
  return `${d.getUTCDate()} ${months[d.getUTCMonth()]}`;
}

function daysLeft(expiresAt, now) {
  const secs = Number(expiresAt) - now;
  if (!Number.isFinite(secs) || secs <= 0) return "0";
  return String(Math.ceil(secs / 86400));
}

export function setMessage(watch, now = Math.floor(Date.now() / 1000)) {
  const pred = watch.predicate || {};
  const sym = pred.symbol || "";
  const mark = watch.mark_at_set;
  return [
    `Watching ${mono(sym)} for ${pred.resolved || "your trigger"}. Now ${mono(mark)}.`,
    `This is a heads-up, not a trade. I won't buy or sell anything. Expires in ${mono(daysLeft(watch.expires_at, now))} days.`,
  ].join("\n");
}

export function alreadyTrueMessage({ symbol, predicate, now: mark }) {
  const pred = predicate || {};
  const sym = pred.symbol || symbol || "";
  const level = pred.level || pred.resolved || "";
  return `That's already true — ${mono(sym)} is at ${mono(mark)}, past your ${mono(level)} level. Want the next crossing, or a different level?`;
}

export function clarifyMessage(symbol) {
  return `Happy to watch ${mono(symbol)} — but that trigger can mean different things. Want me to fire when it's up 5% in a day, or when it crosses a specific price? I'll store whichever you pick, exactly.`;
}

export function executionFoldedMessage(symbol) {
  return `I can watch ${mono(symbol)} for you, or I can help you set a conditional order — but those are different things. A watch just messages you. An order that fires on a trigger has to be signed on World, because it moves your money. Which do you want?`;
}

export function firedMessage(fire) {
  const pred = fire.predicate || {};
  const date = dayMonth(fire.created_at);
  const spent = fire.spent
    ? "This watch is done — it won't fire again unless you re-arm it."
    : "I'll message you again after this condition goes false, then true.";
  return [
    `${mono(pred.symbol)} just crossed ${pred.resolved || fire.original_phrase} — now ${mono(fire.live)}, the level you asked me to watch for on ${date}.`,
    spent,
  ].join("\n");
}

export function expiredMessage(fire) {
  const pred = fire.predicate || {};
  return `Your ${mono(pred.symbol)} watch (${pred.resolved || fire.original_phrase}, set ${dayMonth(fire.created_at)}) expired without firing. I've stopped watching. Nothing happened to your book.`;
}

export function bundleMessage(fires) {
  const rows = (fires || []).map((fire) => {
    const pred = fire.predicate || {};
    const done = fire.spent ? "done" : "still true";
    return `${pred.symbol || ""} ${pred.resolved || ""} → ${fire.live || ""} ${done}`.trim();
  });
  return [
    `${mono(String(fires.length))} more of your watches fired today. Here's all of them at once so I don't spam you:`,
    ...rows.map((row) => `> ${row}`),
  ].join("\n");
}

export function attachSetCopy(result, now) {
  if (result.execution_folded) {
    return {
      ...result,
      message: executionFoldedMessage(result.symbol || ""),
      controls: ["Just watch it", "Set it up on World ↗"],
    };
  }
  if (result.needs_clarification) {
    return {
      ...result,
      message: clarifyMessage(result.symbol || ""),
      controls: (result.options || []).map((o) => o.label),
    };
  }
  if (result.already_true && !result.stored) {
    return {
      ...result,
      already_true: true,
      now: result.now ?? result.watch?.mark_at_set ?? null,
      message: alreadyTrueMessage({
        symbol: result.symbol,
        predicate: result.predicate || result.watch?.predicate,
        now: result.now ?? result.watch?.mark_at_set,
      }),
      controls: ["Watch the next crossing", "Change the level"],
    };
  }
  if (result.ok && result.watch) {
    return {
      ...result,
      already_true: false,
      now: result.now ?? result.watch.mark_at_set ?? null,
      message: setMessage(result.watch, now),
      controls: ["Change the trigger", "Cancel this watch"],
    };
  }
  return result;
}

export function attachFireCopy(fire) {
  return { ...fire, message: firedMessage(fire) };
}

export function attachExpireCopy(fire) {
  return { ...fire, message: expiredMessage(fire) };
}

export function attachBundleCopy(payload) {
  return { ...payload, message: bundleMessage(payload.fires) };
}

export const CANT = {
  wall_market:
    "World doesn't trade {entity}. It trades crypto — spot, perps, and lending — and nothing off-chain.",
  wall_scope:
    "That's outside what I do. I trade, watch, and report on World — nothing else.",
  repeat: "Still can't — World doesn't trade {entity}.",
  kept_line: "Kept for the record — it's in your ledger.",
  nearmatch_frame: 'Nothing called "{word}" trades on World. Close matches:',
  nearmatch_escape: "No — I meant {word}",
  unclear:
    "I didn't catch that — I trade crypto spot, perps, and lending on World. Say what you'd like to do, or `/p` for positions.",
};

export function fillCant(key, vars = {}) {
  return fillTemplate(CANT[key], vars);
}

export function wallMessage({ heard, entity, kind, repeat, index, total }) {
  if (repeat) {
    const line = fillCant("repeat", { entity });
    return total > 1 ? `${index}. ${line}` : line;
  }
  const line2 =
    kind === "out_of_scope"
      ? fillCant("wall_scope")
      : fillCant("wall_market", { entity });
  const body = [`heard: "${heard}"`, line2, fillCant("kept_line")].join("\n");
  return total > 1 ? `${index}. ${body}` : body;
}

export function nearMatchMessage(word) {
  return fillCant("nearmatch_frame", { word });
}

export function nearMatchEscape(word) {
  return fillCant("nearmatch_escape", { word });
}

/**
 * Introduction copy. Templates may interpolate only `first_name` and `ref_link`.
 * No positions, PnL, balances, or other account slots exist here.
 */
export const SHARE = {
  hint: "Forward the next message to them — and a voice note from you on top beats anything I could say.",
  m10_with_name:
    "I'm aomi — an AI on a recorded line. I watch, I execute inside signed limits, and I can do nothing my owner hasn't allowed.\n\n{first_name} thought you should meet me.\n\nTry me on paper — pick a number, nothing is real, you sign nothing.\n{ref_link}",
  m10_anon:
    "I'm aomi — an AI on a recorded line. I watch, I execute inside signed limits, and I can do nothing my owner hasn't allowed.\n\nA friend thought you should meet me.\n\nTry me on paper — pick a number, nothing is real, you sign nothing.\n{ref_link}",
  name_ask: "With your first name on it, or without?",
  already_user: "You two already know each other — this account is live.",
  revoke_ack: "Old invite link is dead. Here's your new one.",
  who_asked:
    "I don't track who opens it — that stays between you and them.",
  without_name: "without my name",
  paper: "Try it on paper ↗",
  cant: "What can't you do?",
  rate_limited: "Three new invite links a day. The current one still works.",
  introduce: "Introduce aomi to a friend ›",
  intent: "introduce yourself to my friend",
};

const SLOT = /\{(\w+)\}/g;

export function templateSlots(template) {
  return [...new Set([...String(template).matchAll(SLOT)].map((m) => m[1]))].sort();
}

export function fillTemplate(template, vars) {
  return String(template).replace(SLOT, (_, key) => {
    if (!Object.prototype.hasOwnProperty.call(vars, key)) {
      throw new Error(`no field for slot {${key}}`);
    }
    const value = vars[key];
    return value == null ? "" : String(value);
  });
}

export function renderM10({ includeName, firstName, refLink }) {
  if (includeName && firstName) {
    return fillTemplate(SHARE.m10_with_name, {
      first_name: firstName,
      ref_link: refLink,
    });
  }
  return fillTemplate(SHARE.m10_anon, { ref_link: refLink });
}

export function proseBlocks(text) {
  return String(text)
    .split(/\n\n+/)
    .map((block) => block.replace(/\n/g, " ").trim())
    .filter(Boolean);
}
