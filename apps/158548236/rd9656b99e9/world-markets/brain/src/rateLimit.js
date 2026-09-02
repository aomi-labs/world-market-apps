const DEFAULT_CEILING = 5;
const WINDOW_SECS = 24 * 60 * 60;

export function ceiling() {
  const n = Number(process.env.WORLD_WATCH_FIRE_CEILING);
  return Number.isFinite(n) && n > 0 ? n : DEFAULT_CEILING;
}

export function emptyLimiter() {
  return { fires: [], held: [] };
}

function prune(state, now) {
  const cutoff = now - WINDOW_SECS;
  state.fires = (state.fires || []).filter((ts) => ts >= cutoff);
  state.held = state.held || [];
}

/**
 * Critical channels (guardian, negative-carry, partial-failure) skip this
 * limiter entirely — they never call it.
 *
 * A held fire is deferred, never dropped. When the ceiling is exceeded the
 * fire is appended to `held` and flushed as one bundle on the next fire or
 * a daily flush, whichever first.
 */
export function admit(state, fire, now = Math.floor(Date.now() / 1000)) {
  prune(state, now);
  const n = ceiling();
  const delivered = state.fires.length;
  if (delivered < n) {
    state.fires.push(now);
    return { action: "deliver", fire, held: [] };
  }
  state.held.push({ ...fire, held_at: now });
  return { action: "hold", fire, held: [...state.held] };
}

export function flushHeld(state, now = Math.floor(Date.now() / 1000)) {
  prune(state, now);
  if (!state.held.length) return null;
  const bundle = state.held.splice(0, state.held.length);
  return bundle;
}

export function dueForDailyFlush(state, now = Math.floor(Date.now() / 1000)) {
  if (!state.held?.length) return false;
  const oldest = Math.min(...state.held.map((item) => item.held_at || now));
  // Flush at least once per UTC day after the first hold.
  const heldDay = Math.floor(oldest / WINDOW_SECS);
  const today = Math.floor(now / WINDOW_SECS);
  return today > heldDay;
}
