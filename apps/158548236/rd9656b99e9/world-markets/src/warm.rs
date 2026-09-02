//! Keep the live account RPC cache warm while the user is talking.
//!
//! The 500ms lookup path only works on cache hits. Hosted Aomi should call
//! `render_lookup` (or `warm_account`) at the start of every user message so
//! this module can prefetch immediately — even when the message is not a terse
//! token. While the session is active, a background refresh runs every minute.
//! Trades invalidate and rebuild the cache before the next lookup.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::client::WorldClient;

pub(crate) const REFRESH_EVERY: Duration = Duration::from_secs(60);
pub(crate) const IDLE_AFTER: Duration = Duration::from_secs(3 * 60);

struct WarmState {
    last_activity: Instant,
    last_refresh: Option<Instant>,
    account_id: Option<u64>,
    thread_started: bool,
    kick_inflight: bool,
}

#[derive(Clone)]
pub(crate) struct AccountWarmer {
    state: Arc<Mutex<WarmState>>,
}

impl Default for AccountWarmer {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(WarmState {
                last_activity: Instant::now(),
                last_refresh: None,
                account_id: None,
                thread_started: false,
                kick_inflight: false,
            })),
        }
    }
}

impl AccountWarmer {
    fn lock(&self) -> std::sync::MutexGuard<'_, WarmState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn touch(&self, account_id: Option<u64>) {
        let mut state = self.lock();
        state.last_activity = Instant::now();
        if account_id.is_some() {
            state.account_id = account_id;
        }
    }

    pub(crate) fn mark_refreshed(&self, account_id: u64) {
        let mut state = self.lock();
        state.account_id = Some(account_id);
        state.last_refresh = Some(Instant::now());
        state.kick_inflight = false;
    }

    pub(crate) fn clear_refresh(&self) {
        self.lock().last_refresh = None;
    }

    #[cfg(test)]
    pub(crate) fn never_refreshed(&self) -> bool {
        self.lock().last_refresh.is_none()
    }

    #[cfg(test)]
    pub(crate) fn account_id(&self) -> Option<u64> {
        self.lock().account_id
    }

    pub(crate) fn ensure_loop(&self, client: WorldClient) {
        if !background_enabled() {
            return;
        }
        {
            let mut state = self.lock();
            if state.thread_started {
                return;
            }
            state.thread_started = true;
        }
        let warmer = self.clone();
        let _ = thread::Builder::new()
            .name("world-account-warm".into())
            .spawn(move || {
                loop {
                    thread::sleep(Duration::from_secs(1));
                    let (idle, due, account_id) = {
                        let state = warmer.lock();
                        let idle = state.last_activity.elapsed() > IDLE_AFTER;
                        let due = state
                            .last_refresh
                            .map(|at| at.elapsed() >= REFRESH_EVERY)
                            .unwrap_or(true);
                        (idle, due, state.account_id)
                    };
                    if idle || !due {
                        continue;
                    }
                    let Some(account_id) = account_id else {
                        continue;
                    };
                    if prefetch_account(&client, account_id, true).is_ok() {
                        warmer.mark_refreshed(account_id);
                    }
                }
            });
    }

    /// First fill: do not wait for the 60s loop. Runs in the background so an
    /// unmatched chat message is not blocked on RPC.
    pub(crate) fn kick_prefetch(&self, client: WorldClient) {
        if !background_enabled() {
            return;
        }
        let account_id = {
            let mut state = self.lock();
            if !state.never_refresh_locked() || state.kick_inflight {
                return;
            }
            let Some(account_id) = state.account_id else {
                return;
            };
            state.kick_inflight = true;
            account_id
        };
        let warmer = self.clone();
        let _ = thread::Builder::new()
            .name("world-account-warm-kick".into())
            .spawn(move || {
                let ok = prefetch_account(&client, account_id, false).is_ok();
                let mut state = warmer.lock();
                state.kick_inflight = false;
                if ok {
                    state.account_id = Some(account_id);
                    state.last_refresh = Some(Instant::now());
                }
            });
    }
}

impl WarmState {
    fn never_refresh_locked(&self) -> bool {
        self.last_refresh.is_none()
    }
}

pub(crate) fn prefetch_account(
    client: &WorldClient,
    account_id: u64,
    invalidate: bool,
) -> Result<(), String> {
    if invalidate {
        client.invalidate_volatile();
    }
    let assets = client.assets()?;
    let account = client.account(account_id, &assets)?;
    let block = client.block_number()?;
    let metrics = crate::liquidation_risk::compute_metrics(client, &account, &assets, block)?;
    let _ = crate::lookups::compute_lookups(
        client,
        &account,
        &assets,
        &metrics.net_asset_value,
        block,
    )?;
    Ok(())
}

fn background_enabled() -> bool {
    let flag = std::env::var("WORLD_WARM_BG").ok();
    if cfg!(test) {
        flag.as_deref() == Some("1")
    } else {
        flag.as_deref() != Some("0")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_records_account_and_stays_unwarmed_until_mark() {
        let warmer = AccountWarmer::default();
        assert!(warmer.never_refreshed());
        warmer.touch(Some(17));
        assert_eq!(warmer.account_id(), Some(17));
        assert!(warmer.never_refreshed());
        warmer.mark_refreshed(17);
        assert!(!warmer.never_refreshed());
        warmer.clear_refresh();
        assert!(warmer.never_refreshed());
    }
}
