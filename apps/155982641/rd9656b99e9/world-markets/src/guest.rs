//! Guest referral, paper book, share card, and deposit funnel.
//!
//! Numbers on these surfaces come only from [`crate::reporting::Reporting`].
//! Simulated figures always carry `paper · live markets`. Guest copy never
//! names a referrer, never counts invites, and never renders a policy verdict.
//!
//! Host gaps (Telegram `start=` routing, PNG share-card renderer, world.inc
//! grant webhook) are listed in `docs/FUTURE-WORK.md`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::reporting::{DemoBook, Figure, RecommendedDeposit, Reporting};

pub(crate) const PAPER_LABEL: &str = "paper · live markets";
pub(crate) const WORLD_INC: &str = "world.inc";
pub(crate) const WORLD_INC_GUEST: &str = "world.inc/?from=guest";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) enum DoorOrder {
    #[default]
    BasisFirst,
    TrustFirst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) enum ConversionTiming {
    #[default]
    DayN,
    FirstPositiveCarry,
    OnRequestOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FunnelConfig {
    pub(crate) paper_start_equity: Decimal,
    pub(crate) conversion_window_days: u32,
    pub(crate) door_order: DoorOrder,
    pub(crate) conversion_timing: ConversionTiming,
    pub(crate) telegram_bot: String,
}

impl Default for FunnelConfig {
    fn default() -> Self {
        Self {
            paper_start_equity: Decimal::new(100, 0),
            conversion_window_days: 3,
            door_order: match std::env::var("WORLD_FUNNEL_DOOR_ORDER")
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str()
            {
                "trust_first" => DoorOrder::TrustFirst,
                _ => DoorOrder::BasisFirst,
            },
            conversion_timing: match std::env::var("WORLD_FUNNEL_CONVERSION_TIMING")
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str()
            {
                "first_positive_carry" => ConversionTiming::FirstPositiveCarry,
                "on_request_only" => ConversionTiming::OnRequestOnly,
                _ => ConversionTiming::DayN,
            },
            telegram_bot: std::env::var("WORLD_TELEGRAM_BOT")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "WorldMarketsBot".to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum GuestPhase {
    Greeting,
    Showcase,
    FireDrill,
    Paper,
    Upgraded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PaperPosition {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) opened_unix: u64,
    pub(crate) daily_carry_net: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PaperBook {
    pub(crate) starting_equity: String,
    pub(crate) positions: Vec<PaperPosition>,
    pub(crate) frozen: bool,
    pub(crate) frozen_carry_net: Option<String>,
    pub(crate) frozen_window_days: Option<u32>,
}

impl PaperBook {
    fn new(start: Decimal) -> Self {
        Self {
            starting_equity: start.normalize().to_string(),
            positions: Vec::new(),
            frozen: false,
            frozen_carry_net: None,
            frozen_window_days: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GuestSession {
    pub(crate) guest_id: String,
    pub(crate) phase: GuestPhase,
    pub(crate) paper: PaperBook,
    pub(crate) guardian_drill_sent: bool,
    pub(crate) upgrade_message_sent: bool,
}

impl GuestSession {
    fn new(guest_id: impl Into<String>, start: Decimal) -> Self {
        Self {
            guest_id: guest_id.into(),
            phase: GuestPhase::Greeting,
            paper: PaperBook::new(start),
            guardian_drill_sent: false,
            upgrade_message_sent: false,
        }
    }
}

#[derive(Clone)]
pub(crate) struct GuestStore {
    inner: Arc<Mutex<BTreeMap<String, GuestSession>>>,
    dir: PathBuf,
}

impl Default for GuestStore {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(BTreeMap::new())),
            dir: default_guest_dir(),
        }
    }
}

impl GuestStore {
    #[allow(dead_code)]
    pub(crate) fn memory() -> Self {
        Self {
            inner: Arc::new(Mutex::new(BTreeMap::new())),
            dir: PathBuf::new(),
        }
    }

    pub(crate) fn get_or_create(
        &self,
        guest_id: &str,
        start: Decimal,
    ) -> Result<GuestSession, String> {
        let mut map = self.lock()?;
        if let Some(existing) = map.get(guest_id) {
            return Ok(existing.clone());
        }
        if let Some(loaded) = self.load_file(guest_id) {
            map.insert(guest_id.to_string(), loaded.clone());
            return Ok(loaded);
        }
        let session = GuestSession::new(guest_id, start);
        map.insert(guest_id.to_string(), session.clone());
        drop(map);
        self.persist(&session)?;
        Ok(session)
    }

    pub(crate) fn save(&self, session: &GuestSession) -> Result<(), String> {
        let mut map = self.lock()?;
        map.insert(session.guest_id.clone(), session.clone());
        drop(map);
        self.persist(session)
    }

    fn persist(&self, session: &GuestSession) -> Result<(), String> {
        if self.dir.as_os_str().is_empty() {
            return Ok(());
        }
        fs::create_dir_all(&self.dir).map_err(|e| format!("[world-markets] guest dir: {e}"))?;
        let path = self
            .dir
            .join(format!("{}.json", sanitize(&session.guest_id)));
        let tmp = path.with_extension("json.tmp");
        let raw = serde_json::to_vec_pretty(session)
            .map_err(|e| format!("[world-markets] guest serialize: {e}"))?;
        fs::write(&tmp, raw).map_err(|e| format!("[world-markets] guest write: {e}"))?;
        fs::rename(&tmp, &path).map_err(|e| format!("[world-markets] guest persist: {e}"))
    }

    fn load_file(&self, guest_id: &str) -> Option<GuestSession> {
        if self.dir.as_os_str().is_empty() {
            return None;
        }
        let path = self.dir.join(format!("{}.json", sanitize(guest_id)));
        serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, GuestSession>>, String> {
        self.inner
            .lock()
            .map_err(|_| "[world-markets] guest store lock poisoned".to_string())
    }
}

fn default_guest_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("WORLD_GUEST_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("aomi/world-markets/guest");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/share/aomi/world-markets/guest");
    }
    std::env::temp_dir().join("aomi-world-markets-guest")
}

fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn opaque_token() -> String {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{n:x}").chars().rev().take(10).collect()
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct Control {
    pub(crate) label: String,
    pub(crate) action: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct RenderedSurface {
    pub(crate) surface: String,
    pub(crate) message: String,
    pub(crate) controls: Vec<Control>,
    pub(crate) silent: bool,
    pub(crate) link: Option<String>,
    pub(crate) image: Option<String>,
    pub(crate) image_status: String,
    pub(crate) simulated: bool,
    pub(crate) policy_verdict: Option<Value>,
}

impl RenderedSurface {
    fn spoken(surface: &str, message: impl Into<String>, controls: Vec<Control>) -> Self {
        Self {
            surface: surface.to_string(),
            message: message.into(),
            controls,
            silent: false,
            link: None,
            image: None,
            image_status: "none".to_string(),
            simulated: true,
            policy_verdict: None,
        }
    }
}

pub(crate) struct Funnel<'a, R: Reporting> {
    reporting: &'a R,
    store: &'a GuestStore,
    config: FunnelConfig,
}

impl<'a, R: Reporting> Funnel<'a, R> {
    pub(crate) fn new(reporting: &'a R, store: &'a GuestStore, config: FunnelConfig) -> Self {
        Self {
            reporting,
            store,
            config,
        }
    }

    fn deposit(&self) -> RecommendedDeposit {
        self.reporting.recommended_first_deposit()
    }

    fn about_deposit(&self) -> String {
        let d = self.deposit();
        format!("about `{}` {}", usd(&d.amount), d.rationale)
    }

    fn demo(&self) -> Result<DemoBook, String> {
        self.reporting.demo_book()
    }

    pub(crate) fn share(&self, image_available: bool) -> RenderedSurface {
        let link = format!(
            "https://t.me/{}?start=g_{}",
            self.config.telegram_bot,
            opaque_token()
        );
        let deposit = self.about_deposit();
        if image_available {
            let mut s = RenderedSurface::spoken(
                "share",
                format!(
                    "Scan or forward — they'll chat with me against a demo portfolio. Nothing real, nothing at risk. When they want the real thing: {WORLD_INC} on their phone — connect a wallet, make a first deposit ({deposit}), and grant the key.\n\n`{link}`"
                ),
                vec![ctrl("Copy link", "copy_link")],
            );
            s.simulated = false;
            s.link = Some(link);
            s.image_status = "ready".to_string();
            s.image = Some("share_card".into());
            s
        } else {
            let mut s = RenderedSurface::spoken(
                "share_fallback",
                format!(
                    "I couldn't render the share card just now. The link works on its own:\n`{link}` — they'll chat with me against a demo portfolio, nothing real."
                ),
                vec![ctrl("Copy link", "copy_link")],
            );
            s.simulated = false;
            s.link = Some(link);
            s.image_status = "unavailable".to_string();
            s
        }
    }

    pub(crate) fn render(&self, guest_id: &str, surface: &str) -> Result<RenderedSurface, String> {
        let mut session = self
            .store
            .get_or_create(guest_id, self.config.paper_start_equity)?;
        let out = match surface {
            "greeting" | "start" => {
                session.phase = GuestPhase::Greeting;
                self.greeting()
            }
            "showcase" | "basis" => {
                session.phase = GuestPhase::Showcase;
                self.or_demo_unavailable(self.showcase())
            }
            "fire_drill" | "drill" => {
                session.phase = GuestPhase::FireDrill;
                self.or_demo_unavailable(self.fire_drill())
            }
            "cant_do" => self.cant_do(),
            "paper_preview" => {
                session.phase = GuestPhase::Paper;
                self.or_demo_unavailable(self.paper_preview(&session))
            }
            "run_on_paper" | "paper_it" => {
                if session.paper.frozen {
                    self.paper_frozen(&session)?
                } else {
                    match self.open_basis_on_paper(&mut session) {
                        Ok(()) => self.or_demo_unavailable(self.paper_receipt(&session)),
                        Err(_) => self.demo_unavailable(),
                    }
                }
            }
            "paper" | "paper_report" => {
                if session.paper.frozen {
                    self.paper_frozen(&session)?
                } else {
                    self.paper_report(&session)?
                }
            }
            "keep_looking" | "keep_paper" | "keep_as_is" => self.keep_looking(),
            "make_mine" | "open_world" => self.make_mine(),
            "real_money" => self.real_money_wall(),
            "paper_executable" => self.paper_executable(),
            "deposit_less" => self.deposit_less(),
            "demo_unavailable" => self.demo_unavailable(),
            "exit" => self.exit(),
            "guardian_drill" => self.or_demo_unavailable(self.guardian_drill(&mut session)),
            "upgrade" => self.upgrade(&mut session)?,
            other => return Err(format!("[world-markets] unknown guest surface {other:?}")),
        };
        self.store.save(&session)?;
        Ok(out)
    }

    fn or_demo_unavailable(&self, result: Result<RenderedSurface, String>) -> RenderedSurface {
        result.unwrap_or_else(|_| self.demo_unavailable())
    }

    fn greeting(&self) -> RenderedSurface {
        let deposit = self.about_deposit();
        let message = format!(
            "You're talking to the World agent — invited by a friend.\n\nYou don't need an account to try me. I'll run a **paper portfolio** for you — simulated money, live markets, tracked over days. When you want the real thing: {WORLD_INC}, a first deposit ({deposit}), and one signature.\n\nWhat would you like to see?"
        );
        let basis = ctrl("The basis trade", "showcase");
        let trust = ctrl("What can't you do?", "cant_do");
        let mine = ctrl("Make it mine", "make_mine");
        let controls = match self.config.door_order {
            DoorOrder::BasisFirst => vec![basis, trust, mine],
            DoorOrder::TrustFirst => vec![trust, basis, mine],
        };
        RenderedSurface::spoken("greeting", message, controls)
    }

    fn showcase(&self) -> Result<RenderedSurface, String> {
        let book = self.demo()?;
        let dp = &book.dollarpower;
        let message = format!(
            "On the demo book, with `{committed}` committed:\n\nBorrow `{borrowed}` at `{apr}` → buy `{spot}` ETH spot → short `{short}` ETH-PERP, atomically. The long and short net out — that's why the borrow is possible.\nReturn is funding received minus borrow paid: currently about `{carry}`/day.\n\nRisk — the honest kind: funding flips (worst recent week `{worst}`/day), or the loan reprices at term. I watch both; if carry stays negative `{days}` days I close it and tell you. No approval needed — the plan would be in your entry receipt.\n\nSame trade on separate venues would need about `{effective}` committed. Here: `{committed}`. That ratio is what users call dollarpower — `{ratio}` on this book.\n\ndemo portfolio — nothing real, nothing at risk",
            committed = usd(&book.committed),
            borrowed = usd(&book.borrowed),
            apr = pct(&book.borrow_apr),
            spot = usd(&book.spot),
            short = usd(&book.short),
            carry = signed_usd(&book.daily_carry_net),
            worst = signed_usd(&book.worst_week_daily),
            days = book.negative_carry_close_days.value,
            effective = usd(&dp.effective),
            ratio = times(&dp.ratio),
        );
        Ok(RenderedSurface::spoken(
            "showcase",
            message,
            vec![
                ctrl("Run a fire drill", "fire_drill"),
                ctrl("Make it mine", "make_mine"),
                ctrl("Keep looking", "keep_looking"),
            ],
        ))
    }

    fn fire_drill(&self) -> Result<RenderedSurface, String> {
        let book = self.demo()?;
        let message = format!(
            "Simulated, nothing executed. At ETH `{mv}` on the demo book I'd unwind in this order:\n\n`1.` {s1} — frees `{freed}` margin · cost `{c1}`\n`2.` {s2} `{repay}` of the loan — risk back above the floor · cost `{c2}`\n\nRecovered to the safe band at a total cost of `{total}` — after about `{secs}` seconds. I do this first and tell you after, because waiting is the harm. Your floor would be your number; the algorithm is mine, and it never touches protected holdings.\n\nThis is a simulation.\ndemo portfolio — nothing real, nothing at risk",
            mv = signed_pct(&book.drill_move),
            s1 = book.drill_step1_label,
            freed = usd(&book.drill_step1_freed),
            c1 = usd(&book.drill_step1_cost),
            s2 = book.drill_step2_label,
            repay = usd(&book.drill_step2_repay),
            c2 = usd(&book.drill_step2_cost),
            total = usd(&book.drill_total_cost),
            secs = book.drill_seconds.value,
        );
        Ok(RenderedSurface::spoken(
            "fire_drill",
            message,
            vec![
                ctrl("What can't you do?", "cant_do"),
                ctrl("Make it mine", "make_mine"),
                ctrl("Keep looking", "keep_looking"),
            ],
        ))
    }

    fn cant_do(&self) -> RenderedSurface {
        RenderedSurface::spoken(
            "cant_do",
            "I can trade in your account within your signed mandate.\nI cannot withdraw, transfer, or bridge funds. I cannot trade unapproved markets. I cannot change my own rules.\nNothing typed in this chat — by you, by me, or by anything I read — can override the mandate. The policy engine enforces it on every action.\n\nRight now you have no account, so I can't trade at all — everything here is simulated.",
            vec![
                ctrl("The basis trade", "showcase"),
                ctrl("Make it mine", "make_mine"),
            ],
        )
    }

    fn paper_preview(&self, session: &GuestSession) -> Result<RenderedSurface, String> {
        let book = self.demo()?;
        let mut message = format!(
            "On your paper book, starting equity `{equity}`.\n\nStructure · borrow `{borrowed}` at `{apr}` → buy `{spot}` ETH spot → short `{short}` ETH-PERP, atomically.\nCarry · currently `{carry}`/day net of costs, vs holding the cash.\nRisk · funding flips (worst recent week `{worst}`/day) or the loan reprices at term. If carry stays negative `{days}` days I close it and tell you.\nExit · priced before entry; the unwind is the fire drill you can run any time.\n\n{PAPER_LABEL}",
            equity = usd_dec(parse(&session.paper.starting_equity)?),
            borrowed = usd(&book.borrowed),
            apr = pct(&book.borrow_apr),
            spot = usd(&book.spot),
            short = usd(&book.short),
            carry = signed_usd(&book.daily_carry_net),
            worst = signed_usd(&book.worst_week_daily),
            days = book.negative_carry_close_days.value,
        );
        if session.paper.frozen {
            message
                .push_str("\n\nThis paper book is frozen — read-only since you granted the key.");
        }
        Ok(RenderedSurface::spoken(
            "paper_preview",
            message,
            vec![
                ctrl("Run it on paper", "run_on_paper"),
                ctrl("Keep paper as is", "keep_paper"),
                ctrl("Make it mine — world.inc ↗", "make_mine"),
            ],
        ))
    }

    fn open_basis_on_paper(&self, session: &mut GuestSession) -> Result<(), String> {
        if session.paper.frozen {
            return Ok(());
        }
        let book = self.demo()?;
        session.phase = GuestPhase::Paper;
        session.paper.positions.push(PaperPosition {
            id: format!("basis-{}", session.paper.positions.len() + 1),
            label: "ETH basis (paper)".into(),
            opened_unix: now_unix(),
            daily_carry_net: book.daily_carry_net.value.clone(),
        });
        Ok(())
    }

    fn paper_receipt(&self, session: &GuestSession) -> Result<RenderedSurface, String> {
        let book = self.demo()?;
        self.with_conversion(
            session,
            "paper_receipt",
            format!(
                "On paper. Borrow `{b}` · long `{s}` ETH · short `{k}` ETH-PERP. Carry `{c}`/day net of costs.\n\n{PAPER_LABEL}",
                b = usd(&book.borrowed),
                s = usd(&book.spot),
                k = usd(&book.short),
                c = signed_usd(&book.daily_carry_net),
            ),
        )
    }

    fn paper_report(&self, session: &GuestSession) -> Result<RenderedSurface, String> {
        let (carry, window) = paper_carry(session, now_unix())?;
        let body = if session.paper.positions.is_empty() {
            format!(
                "Paper book · starting equity `{e}`. No positions yet.\n\n{PAPER_LABEL}",
                e = usd_dec(parse(&session.paper.starting_equity)?),
            )
        } else {
            format!(
                "Paper book · starting equity `{e}` · {n} position(s) · carry `{c}` net of costs over `{w}` days.\n\n{PAPER_LABEL}",
                e = usd_dec(parse(&session.paper.starting_equity)?),
                n = session.paper.positions.len(),
                c = signed_usd_dec(carry),
                w = window,
            )
        };
        self.with_conversion(session, "paper_report", body)
    }

    fn paper_frozen(&self, session: &GuestSession) -> Result<RenderedSurface, String> {
        let (carry, window) = paper_carry(session, now_unix())?;
        Ok(RenderedSurface::spoken(
            "paper_frozen",
            format!(
                "Paper book (frozen) · final carry `{c}` net of costs over `{w}` days. It accrues nothing after the key grant. Say `paper` anytime.\n\n{PAPER_LABEL}",
                c = signed_usd_dec(carry),
                w = window,
            ),
            vec![],
        ))
    }

    fn with_conversion(
        &self,
        session: &GuestSession,
        surface: &str,
        mut message: String,
    ) -> Result<RenderedSurface, String> {
        if let Some(line) = self.conversion_line(session, now_unix())? {
            message.push_str("\n\n");
            message.push_str(&line);
        }
        Ok(RenderedSurface::spoken(
            surface,
            message,
            vec![
                ctrl("Make it mine", "make_mine"),
                ctrl("Keep paper", "keep_paper"),
            ],
        ))
    }

    fn conversion_line(&self, session: &GuestSession, now: u64) -> Result<Option<String>, String> {
        if session.paper.frozen || session.paper.positions.is_empty() {
            return Ok(None);
        }
        let (carry, window) = paper_carry(session, now)?;
        let due = match self.config.conversion_timing {
            ConversionTiming::OnRequestOnly => false,
            ConversionTiming::DayN => window >= self.config.conversion_window_days,
            ConversionTiming::FirstPositiveCarry => carry > Decimal::ZERO,
        };
        if !due {
            return Ok(None);
        }
        let deposit = self.about_deposit();
        let lead = if carry.is_sign_negative() {
            format!(
                "If this were real you'd be down `{d}`; the mechanism and the exit are identical.",
                d = usd_dec(carry.abs()),
            )
        } else {
            format!(
                "Paper carry so far: `{c}` net of costs. If this were your real book, that's yours.",
                c = signed_usd_dec(carry),
            )
        };
        Ok(Some(format!(
            "{lead} {WORLD_INC} — {deposit}, one signature, and I'd run it for real."
        )))
    }

    fn keep_looking(&self) -> RenderedSurface {
        let mut s = RenderedSurface::spoken("keep_looking", "", vec![]);
        s.silent = true;
        s
    }

    fn make_mine(&self) -> RenderedSurface {
        let deposit = self.about_deposit();
        let mut s = RenderedSurface::spoken(
            "make_mine",
            format!(
                "If you want your own: {WORLD_INC} on your phone — connect your wallet, a first deposit ({deposit}), and grant me a trade-only key. One signature. Until then everything I show you is simulated."
            ),
            vec![
                ctrl("Open world.inc ↗", "open_world"),
                ctrl("Keep looking", "keep_looking"),
            ],
        );
        s.link = Some(format!("https://{WORLD_INC_GUEST}"));
        s
    }

    fn real_money_wall(&self) -> RenderedSurface {
        let deposit = self.about_deposit();
        let mut s = RenderedSurface::spoken(
            "real_money",
            format!(
                "I can't — you don't have an account yet, and I only trade with a key *you* sign. That's the whole design: I can't touch money nobody has granted me a key to.\n\nIf you want your own: {WORLD_INC} on your phone — connect your wallet, a first deposit ({deposit}), and grant me a trade-only key. One signature. Until then everything I show you is simulated."
            ),
            vec![
                ctrl("Open world.inc ↗", "open_world"),
                ctrl("Keep looking", "keep_looking"),
            ],
        );
        s.link = Some(format!("https://{WORLD_INC_GUEST}"));
        s
    }

    fn paper_executable(&self) -> RenderedSurface {
        RenderedSurface::spoken(
            "paper_executable",
            "I can't do it for real — you don't have an account yet, and I only trade with a key *you* sign. I **can** put it on your paper book, live markets, real costs — same preview, same receipts, nothing at risk.",
            vec![
                ctrl("Paper it", "paper_preview"),
                ctrl("Make it mine — world.inc ↗", "make_mine"),
            ],
        )
    }

    fn deposit_less(&self) -> RenderedSurface {
        let d = self.deposit();
        RenderedSurface::spoken(
            "deposit_less",
            format!(
                "Yes — there's no minimum. Below about `{amt}`, transaction costs take a visible bite out of a small book, which is why I recommend it. It's your call either way.",
                amt = usd(&d.amount),
            ),
            vec![
                ctrl("Open world.inc ↗", "open_world"),
                ctrl("Keep looking", "keep_looking"),
            ],
        )
    }

    fn demo_unavailable(&self) -> RenderedSurface {
        RenderedSurface::spoken(
            "demo_unavailable",
            "I can't reach the demo book right now, so I'd rather show you nothing than make up numbers. Try me again in a few minutes — or browse world.inc in the meantime.",
            vec![ctrl("Open world.inc ↗", "open_world")],
        )
    }

    fn exit(&self) -> RenderedSurface {
        RenderedSurface::spoken(
            "exit",
            "There's nothing of yours here to close — no account, no positions, no messages scheduled. Come back whenever; the demo book will still be here.",
            vec![],
        )
    }

    fn guardian_drill(&self, session: &mut GuestSession) -> Result<RenderedSurface, String> {
        let book = self.demo()?;
        let body = format!(
            "While you were offline, this is what I would have done — simulated, nothing executed. Floor breached on the paper book; I would have unwound in this order:\n\n`1.` {s1} — frees `{freed}` · cost `{c1}`\n`2.` {s2} `{repay}` — risk back above the floor · cost `{c2}`\n\nTotal cost `{total}`.\n\n{PAPER_LABEL}",
            s1 = book.drill_step1_label,
            freed = usd(&book.drill_step1_freed),
            c1 = usd(&book.drill_step1_cost),
            s2 = book.drill_step2_label,
            repay = usd(&book.drill_step2_repay),
            c2 = usd(&book.drill_step2_cost),
            total = usd(&book.drill_total_cost),
        );
        if session.guardian_drill_sent {
            return Ok(RenderedSurface::spoken(
                "guardian_drill_digest",
                format!("Digest line (already pushed once): {body}"),
                vec![],
            ));
        }
        session.guardian_drill_sent = true;
        Ok(RenderedSurface::spoken(
            "guardian_drill",
            body,
            vec![
                ctrl("Make it mine", "make_mine"),
                ctrl("Keep paper", "keep_paper"),
            ],
        ))
    }

    fn upgrade(&self, session: &mut GuestSession) -> Result<RenderedSurface, String> {
        if session.upgrade_message_sent {
            return Ok(RenderedSurface::spoken(
                "upgrade_already",
                "Key already granted in this chat — I won't repeat the welcome.",
                vec![],
            ));
        }
        let (carry, window) = paper_carry(session, now_unix())?;
        session.paper.frozen = true;
        session.paper.frozen_carry_net = Some(carry.normalize().to_string());
        session.paper.frozen_window_days = Some(window);
        session.phase = GuestPhase::Upgraded;
        session.upgrade_message_sent = true;
        let mut s = RenderedSurface::spoken(
            "upgrade",
            "Key granted — this chat is yours now. Everything above was simulated; from here, previews are your real book, and every action is checked against your signed mandate before it touches anything.\n\nYour paper book is still there if you want to compare — `paper` anytime. Start small. Pause anytime. Revoking the key is one signature on World.",
            vec![
                ctrl("What can't you do?", "cant_do"),
                ctrl("Preview my first trade", "preview_first_trade"),
            ],
        );
        s.simulated = false;
        Ok(s)
    }
}

pub(crate) fn paper_carry(session: &GuestSession, now: u64) -> Result<(Decimal, u32), String> {
    if let (Some(c), Some(w)) = (
        session.paper.frozen_carry_net.as_deref(),
        session.paper.frozen_window_days,
    ) {
        return Ok((parse(c)?, w));
    }
    if session.paper.positions.is_empty() {
        return Ok((Decimal::ZERO, 0));
    }
    let mut total = Decimal::ZERO;
    let mut window = 0u32;
    for pos in &session.paper.positions {
        let days = now.saturating_sub(pos.opened_unix) / 86_400;
        window = window.max(days as u32);
        total += parse(&pos.daily_carry_net)? * Decimal::from(days);
    }
    Ok((total, window))
}

pub(crate) fn guest_id_from_start(payload: &str) -> Option<String> {
    let p = payload.trim();
    let rest = p
        .strip_prefix("g_")
        .or_else(|| p.strip_prefix('g'))
        .unwrap_or(p);
    if rest.is_empty() {
        None
    } else {
        Some(format!("g_{rest}"))
    }
}

fn ctrl(label: &str, action: &str) -> Control {
    Control {
        label: label.to_string(),
        action: action.to_string(),
    }
}

fn parse(raw: &str) -> Result<Decimal, String> {
    Decimal::from_str(raw.trim_start_matches('+'))
        .map_err(|e| format!("[world-markets] invalid decimal {raw:?}: {e}"))
}

fn usd(fig: &Figure) -> String {
    usd_dec(parse(&fig.value).unwrap_or(Decimal::ZERO))
}

fn usd_dec(value: Decimal) -> String {
    let n = value.abs().normalize();
    if value.is_sign_negative() {
        format!("−${n}")
    } else {
        format!("${n}")
    }
}

fn signed_usd(fig: &Figure) -> String {
    signed_usd_dec(parse(&fig.value).unwrap_or(Decimal::ZERO))
}

fn signed_usd_dec(value: Decimal) -> String {
    if value.is_zero() {
        return "≈ $0".to_string();
    }
    let n = value.abs().normalize();
    if value.is_sign_negative() {
        format!("−${n}")
    } else {
        format!("+${n}")
    }
}

fn pct(fig: &Figure) -> String {
    format!("{}%", fig.value)
}

fn signed_pct(fig: &Figure) -> String {
    format!("{}%", fig.value.trim())
}

fn times(fig: &Figure) -> String {
    format!("{}×", fig.value.trim_end_matches('×').trim_end_matches('x'))
}

#[allow(dead_code)]
pub(crate) fn anti_goal_violations(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    let mut hits = Vec::new();
    for (needle, label) in [
        ("your code", "referral_code"),
        ("you'll get", "referral_reward"),
        ("invite count", "invite_count"),
        ("leaderboard", "leaderboard"),
        ("your friend signed up", "named_referrer"),
        ("ready to deposit", "standalone_conversion"),
        ("don't miss", "hype"),
        ("win rate", "win_rate"),
        ("streak", "streak"),
        ("the limit held", "policy_block_in_guest"),
        ("minimum $", "deposit_as_minimum"),
    ] {
        if lower.contains(needle) {
            hits.push(label);
        }
    }
    hits
}

pub(crate) fn to_tool_json(surface: &RenderedSurface) -> Value {
    json!({
        "source": "world-markets-reporting",
        "executable": false,
        "guest": true,
        "surface": surface.surface,
        "message": surface.message,
        "controls": surface.controls,
        "silent": surface.silent,
        "link": surface.link,
        "image": surface.image,
        "image_status": surface.image_status,
        "simulated": surface.simulated,
        "paper_label": if surface.simulated { Some(PAPER_LABEL) } else { None },
        "policy_verdict": surface.policy_verdict,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reporting::{FixtureReporting, ZeroEdgeReporting};

    fn funnel<'a>(r: &'a impl Reporting, store: &'a GuestStore) -> Funnel<'a, impl Reporting> {
        Funnel::new(r, store, FunnelConfig::default())
    }

    #[test]
    fn greeting_asks_no_wallet_and_names_the_deposit() {
        let r = FixtureReporting;
        let store = GuestStore::memory();
        let s = funnel(&r, &store).render("g_test", "greeting").unwrap();
        assert!(s.message.contains("invited by a friend"));
        assert!(s.message.contains("paper portfolio"));
        assert!(s.message.contains("clears transaction minimums"));
        assert!(s.message.contains("`$20`"));
        assert!(!s.message.to_lowercase().contains("connect a wallet first"));
        assert_eq!(s.controls[0].label, "The basis trade");
        assert!(s.policy_verdict.is_none());
        assert!(anti_goal_violations(&s.message).is_empty());
    }

    #[test]
    fn door_order_is_switchable() {
        let r = FixtureReporting;
        let store = GuestStore::memory();
        let cfg = FunnelConfig {
            door_order: DoorOrder::TrustFirst,
            ..FunnelConfig::default()
        };
        let s = Funnel::new(&r, &store, cfg)
            .render("g_e2", "greeting")
            .unwrap();
        assert_eq!(s.controls[0].label, "What can't you do?");
        assert_eq!(s.controls[1].label, "The basis trade");
    }

    #[test]
    fn share_fallback_has_no_referral_vocab() {
        let r = FixtureReporting;
        let store = GuestStore::memory();
        let s = funnel(&r, &store).share(false);
        assert_eq!(s.surface, "share_fallback");
        assert!(
            s.message
                .contains("I couldn't render the share card just now")
        );
        assert!(s.message.contains("?start=g_"));
        assert_eq!(s.image_status, "unavailable");
        assert!(anti_goal_violations(&s.message).is_empty());
    }

    #[test]
    fn share_normal_names_deposit_with_reason() {
        let r = FixtureReporting;
        let store = GuestStore::memory();
        let s = funnel(&r, &store).share(true);
        assert!(
            s.message
                .contains("about `$20` clears transaction minimums")
        );
        assert!(!s.message.contains("minimum $20"));
        assert!(anti_goal_violations(&s.message).is_empty());
    }

    #[test]
    fn showcase_names_risks_before_dollarpower() {
        let r = FixtureReporting;
        let store = GuestStore::memory();
        let s = funnel(&r, &store).render("g_show", "showcase").unwrap();
        let risk = s.message.find("Risk — the honest kind").unwrap();
        let dp = s.message.find("dollarpower").unwrap();
        assert!(risk < dp);
        assert_eq!(s.message.matches("dollarpower").count(), 1);
        assert!(s.message.contains("`$100`"));
        assert!(s.message.contains("`$900`"));
        assert!(anti_goal_violations(&s.message).is_empty());
    }

    #[test]
    fn paper_surfaces_carry_the_label() {
        let r = FixtureReporting;
        let store = GuestStore::memory();
        let f = funnel(&r, &store);
        assert!(
            f.render("g_p", "paper_preview")
                .unwrap()
                .message
                .contains(PAPER_LABEL)
        );
        assert!(
            f.render("g_p", "run_on_paper")
                .unwrap()
                .message
                .contains(PAPER_LABEL)
        );
        assert!(
            f.render("g_p", "paper")
                .unwrap()
                .message
                .contains(PAPER_LABEL)
        );
    }

    #[test]
    fn conversion_line_appends_after_window() {
        let r = FixtureReporting;
        let store = GuestStore::memory();
        let f = funnel(&r, &store);
        f.render("g_conv", "run_on_paper").unwrap();
        let mut session = store.get_or_create("g_conv", Decimal::new(100, 0)).unwrap();
        session.paper.positions[0].opened_unix = now_unix() - 4 * 86_400;
        store.save(&session).unwrap();
        let report = f.render("g_conv", "paper").unwrap();
        assert!(report.message.contains("Paper carry so far"));
        assert!(report.message.contains(PAPER_LABEL));
        assert_ne!(report.surface, "conversion_push");
    }

    #[test]
    fn conversion_on_request_only_never_appends() {
        let r = FixtureReporting;
        let store = GuestStore::memory();
        let cfg = FunnelConfig {
            conversion_timing: ConversionTiming::OnRequestOnly,
            ..FunnelConfig::default()
        };
        let f = Funnel::new(&r, &store, cfg);
        f.render("g_e4", "run_on_paper").unwrap();
        let mut session = store.get_or_create("g_e4", Decimal::new(100, 0)).unwrap();
        session.paper.positions[0].opened_unix = now_unix() - 10 * 86_400;
        store.save(&session).unwrap();
        let report = f.render("g_e4", "paper").unwrap();
        assert!(!report.message.contains("Paper carry so far"));
    }

    #[test]
    fn drawdown_conversion_is_not_loss_framed() {
        let r = ZeroEdgeReporting;
        let store = GuestStore::memory();
        let f = funnel(&r, &store);
        f.render("g_down", "run_on_paper").unwrap();
        let mut session = store.get_or_create("g_down", Decimal::new(100, 0)).unwrap();
        session.paper.positions[0].opened_unix = now_unix() - 4 * 86_400;
        session.paper.positions[0].daily_carry_net = "-0.35".into();
        store.save(&session).unwrap();
        let report = f.render("g_down", "paper").unwrap();
        assert!(report.message.contains("If this were real you'd be down"));
        assert!(
            report
                .message
                .contains("the mechanism and the exit are identical")
        );
        assert!(
            !report
                .message
                .to_lowercase()
                .contains("you would have lost")
        );
    }

    #[test]
    fn zero_edge_paper_reports_null_result() {
        let r = ZeroEdgeReporting;
        let store = GuestStore::memory();
        let f = funnel(&r, &store);
        f.render("g_zero", "run_on_paper").unwrap();
        let mut session = store.get_or_create("g_zero", Decimal::new(100, 0)).unwrap();
        session.paper.positions[0].opened_unix = now_unix() - 4 * 86_400;
        store.save(&session).unwrap();
        let report = f.render("g_zero", "paper").unwrap();
        assert!(report.message.contains("≈ $0"));
        assert!(!report.message.contains("+$"));
    }

    #[test]
    fn deposit_less_is_not_a_minimum() {
        let r = FixtureReporting;
        let store = GuestStore::memory();
        let s = funnel(&r, &store).render("g_less", "deposit_less").unwrap();
        assert!(s.message.starts_with("Yes — there's no minimum."));
        assert!(s.message.contains("`$20`"));
        assert!(s.message.contains("It's your call either way."));
        assert!(!s.message.contains("minimum $20"));
    }

    #[test]
    fn guest_blocks_are_setup_not_policy() {
        let r = FixtureReporting;
        let store = GuestStore::memory();
        let f = funnel(&r, &store);
        let a = f.render("g_w", "paper_executable").unwrap();
        let b = f.render("g_w", "real_money").unwrap();
        assert!(a.policy_verdict.is_none() && b.policy_verdict.is_none());
        assert!(!b.message.to_lowercase().contains("the limit held"));
        assert!(b.message.contains("I only trade with a key *you* sign"));
    }

    #[test]
    fn demo_unavailable_shows_the_law() {
        let r = FixtureReporting;
        let store = GuestStore::memory();
        let s = funnel(&r, &store)
            .render("g_fail", "demo_unavailable")
            .unwrap();
        assert!(
            s.message
                .contains("I'd rather show you nothing than make up numbers")
        );
    }

    #[test]
    fn upgrade_fires_once_and_freezes_paper() {
        let r = FixtureReporting;
        let store = GuestStore::memory();
        let f = funnel(&r, &store);
        f.render("g_up", "run_on_paper").unwrap();
        let first = f.render("g_up", "upgrade").unwrap();
        assert!(first.message.contains("this chat is yours now"));
        assert!(first.message.contains("`paper` anytime"));
        let session = store.get_or_create("g_up", Decimal::new(100, 0)).unwrap();
        assert!(session.paper.frozen);
        assert_eq!(session.phase, GuestPhase::Upgraded);
        assert_eq!(
            f.render("g_up", "upgrade").unwrap().surface,
            "upgrade_already"
        );
        assert_eq!(
            f.render("g_up", "run_on_paper").unwrap().surface,
            "paper_frozen"
        );
    }

    #[test]
    fn guardian_drill_is_one_push() {
        let r = FixtureReporting;
        let store = GuestStore::memory();
        let f = funnel(&r, &store);
        assert_eq!(
            f.render("g_g", "guardian_drill").unwrap().surface,
            "guardian_drill"
        );
        assert_eq!(
            f.render("g_g", "guardian_drill").unwrap().surface,
            "guardian_drill_digest"
        );
    }

    #[test]
    fn keep_looking_is_silent() {
        let r = FixtureReporting;
        let store = GuestStore::memory();
        let s = funnel(&r, &store).render("g_k", "keep_looking").unwrap();
        assert!(s.silent && s.message.is_empty());
    }

    #[test]
    fn start_payload_carries_no_referrer() {
        assert_eq!(guest_id_from_start("g_7K9X2Q").unwrap(), "g_7K9X2Q");
    }

    #[test]
    fn recommended_deposit_comes_from_reporting() {
        let d = FixtureReporting.recommended_first_deposit();
        assert_eq!(d.amount.value, "20");
        assert_eq!(d.rationale, "clears transaction minimums");
    }
}
