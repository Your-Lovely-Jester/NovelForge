//! Shared HTTP transport for source requests.
//!
//! Every app-level request to a source host goes through a per-host gate. The
//! gate enforces a conservative concurrency and spacing, adapts slowly after a
//! clean streak, and backs off immediately when the source answers 429/503.
//!
//! Some sources (NovelFull is the current example) reject reqwest's TLS
//! fingerprint with a Cloudflare 403 while the system `curl` binary receives
//! HTTP 200. Those profiles can fall back to curl without turning every request
//! into a browser render. A blocked source is put into a cooldown instead of
//! being retried in a tight loop.

use crate::error::{AppError, AppResult};
use crate::settings::SourceLimit;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;
use url::Url;

pub const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(15);
const MAX_COOLDOWN: Duration = Duration::from_secs(300);
const BLOCK_COOLDOWN: Duration = Duration::from_secs(300);
const RAMP_SUCCESSES: u32 = 20;
const RAMP_LATENCY_SAMPLES: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Metadata,
    Toc,
    Cover,
}

impl RequestKind {
    fn accept(self) -> &'static str {
        match self {
            Self::Cover => "image/avif,image/webp,image/apng,image/*,*/*;q=0.8",
            Self::Metadata | Self::Toc => "text/html,application/xhtml+xml",
        }
    }

    fn timeout(self) -> Duration {
        match self {
            Self::Cover => Duration::from_secs(14),
            Self::Metadata | Self::Toc => Duration::from_secs(18),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CurlMode {
    /// Never use curl; reqwest is the only transport.
    Never,
    /// Use curl after the host has returned a Cloudflare 403.
    OnBlock,
    /// The source is known to reject reqwest; use curl from the first request.
    Always,
}

#[derive(Debug, Clone)]
pub struct SourceProfile {
    pub id: &'static str,
    pub label: &'static str,
    pub safe_concurrency: usize,
    pub max_concurrency: usize,
    pub request_gap: Duration,
    pub chapter_rate: f32,
    pub max_retries: usize,
    curl_mode: CurlMode,
    pub note: &'static str,
}

impl SourceProfile {
    fn gap_for(&self, kind: RequestKind) -> Duration {
        match kind {
            // Discovery is deliberately sequential even when the host has a
            // higher chapter ceiling. One page at a time is what the live
            // sources were verified with.
            RequestKind::Toc => self.request_gap.max(Duration::from_millis(700)),
            RequestKind::Metadata => self.request_gap.max(Duration::from_millis(500)),
            RequestKind::Cover => (self.request_gap / 2).max(Duration::from_millis(150)),
        }
    }
}

/// Built-in starting points. The `note` is evidence, not a promise: a source
/// is only promoted above 1 concurrent request after the gate observes a long
/// clean streak, and the user can pin a lower value in Settings.
fn builtin_profile(host: &str) -> SourceProfile {
    let host = host.split(':').next().unwrap_or(host);
    let bare = host.strip_prefix("www.").unwrap_or(host);

    if bare == "novelfull.net" || bare.ends_with(".novelfull.net") {
        SourceProfile {
            id: "novelfull",
            label: "NovelFull",
            safe_concurrency: 1,
            max_concurrency: 1,
            request_gap: Duration::from_millis(900),
            chapter_rate: 2.0,
            max_retries: 3,
            curl_mode: CurlMode::Always,
            note: "reqwest rustls/native-tls both get HTTP 403; system curl got HTTP 200 over HTTP/1.1 and HTTP/2 (2026-11-15). Sequential only; 59 TOC pages completed without a 429.",
        }
    } else if bare == "novelfire.net" || bare.ends_with(".novelfire.net") {
        SourceProfile {
            id: "novelfire",
            label: "NovelFire",
            safe_concurrency: 1,
            max_concurrency: 2,
            request_gap: Duration::from_millis(900),
            chapter_rate: 2.0,
            max_retries: 3,
            curl_mode: CurlMode::OnBlock,
            note: "A 6-way TOC burst produced 429; sequential 900 ms pacing completed a live index.",
        }
    } else if bare == "novelphoenix.net" || bare.ends_with(".novelphoenix.net") {
        SourceProfile {
            id: "novelphoenix",
            label: "NovelPhoenix",
            safe_concurrency: 1,
            max_concurrency: 3,
            request_gap: Duration::from_millis(700),
            chapter_rate: 2.0,
            max_retries: 3,
            curl_mode: CurlMode::OnBlock,
            note: "Live index verified sequentially; no clean-response evidence for parallel bursts.",
        }
    } else if bare == "divinedaolibrary.com" || bare.ends_with(".divinedaolibrary.com") {
        SourceProfile {
            id: "divinedaolibrary",
            label: "Divine Dao Library",
            safe_concurrency: 1,
            max_concurrency: 3,
            request_gap: Duration::from_millis(700),
            chapter_rate: 2.0,
            max_retries: 3,
            curl_mode: CurlMode::OnBlock,
            note: "A 1,227-chapter live index completed sequentially; parallel bursts are not verified.",
        }
    } else if bare == "freewebnovel.com" || bare.ends_with(".freewebnovel.com") {
        SourceProfile {
            id: "freewebnovel",
            label: "FreeWebNovel",
            safe_concurrency: 2,
            max_concurrency: 4,
            request_gap: Duration::from_millis(700),
            chapter_rate: 2.0,
            max_retries: 3,
            curl_mode: CurlMode::OnBlock,
            note: "2,334-chapter live index completed from 59 TOC pages at 4 concurrent/700 ms with one retry and no 429; lncrawl 1–3 download completed (2026-11-15).",
        }
    } else if bare == "novelping.com" || bare.ends_with(".novelping.com") {
        SourceProfile {
            id: "novelping",
            label: "NovelPing",
            safe_concurrency: 1,
            max_concurrency: 2,
            request_gap: Duration::from_millis(700),
            chapter_rate: 2.0,
            max_retries: 3,
            curl_mode: CurlMode::OnBlock,
            note: "53-chapter archive index and a 1–3 download completed sequentially (2026-11-15); parallel bursts are not verified.",
        }
    } else if bare == "webnovel.com" || bare.ends_with(".webnovel.com") {
        SourceProfile {
            id: "webnovel",
            label: "Webnovel",
            safe_concurrency: 1,
            max_concurrency: 2,
            request_gap: Duration::from_millis(1000),
            chapter_rate: 2.0,
            max_retries: 3,
            curl_mode: CurlMode::OnBlock,
            note: "reqwest gets Cloudflare HTTP 403 while curl gets HTTP 200, so metadata uses the fallback; the catalog exposes 877 rows but only 832 links (45 locked), so a complete index cannot be proven.",
        }
    } else if bare == "novelbin.com" || bare.ends_with(".novelbin.com") {
        SourceProfile {
            id: "novelbin",
            label: "NovelBin",
            safe_concurrency: 1,
            max_concurrency: 2,
            request_gap: Duration::from_millis(900),
            chapter_rate: 2.0,
            max_retries: 3,
            curl_mode: CurlMode::OnBlock,
            note: "Cloudflare-protected clone; conservative defaults, not yet live-verified.",
        }
    } else {
        SourceProfile {
            id: "generic",
            label: "Generic source",
            safe_concurrency: 2,
            max_concurrency: 4,
            request_gap: Duration::from_millis(700),
            chapter_rate: 2.0,
            max_retries: 3,
            curl_mode: CurlMode::OnBlock,
            note: "Unverified host: starts at 2 concurrent, ramps to 4 only after 20 clean responses.",
        }
    }
}

fn limit_key_candidates(host: &str) -> [String; 4] {
    let bare = host.split(':').next().unwrap_or(host);
    let no_www = |value: &str| value.strip_prefix("www.").unwrap_or(value).to_string();
    [
        host.to_string(),
        bare.to_string(),
        no_www(host),
        no_www(bare),
    ]
}

fn configured_limit(host: &str) -> Option<SourceLimit> {
    let overrides = config().read().ok()?;
    // Settings normalize `www.` away, but the request host may still carry it
    // (or a port the override omits), so try every equivalent key.
    limit_key_candidates(host)
        .iter()
        .find_map(|candidate| overrides.get(candidate).cloned())
}

fn apply_limit(profile: &mut SourceProfile, limit: &SourceLimit) {
    profile.safe_concurrency = limit.concurrency as usize;
    // A user override is a hard ceiling, not a starting point. The gate must
    // never ramp past a value the user pinned.
    profile.max_concurrency = limit.concurrency as usize;
    profile.request_gap = Duration::from_millis(limit.request_gap_ms);
    profile.chapter_rate = limit.chapter_rate;
    profile.note = "Configured by a per-host override in Settings.";
}

fn profile_for_host(host: &str) -> SourceProfile {
    let mut profile = builtin_profile(host);
    if let Some(limit) = configured_limit(host) {
        apply_limit(&mut profile, &limit);
    }
    profile
}

fn apply_profile(profile: &mut SourceProfile, next: SourceProfile) {
    profile.safe_concurrency = next.safe_concurrency;
    profile.max_concurrency = next.max_concurrency;
    profile.request_gap = next.request_gap;
    profile.chapter_rate = next.chapter_rate;
    profile.max_retries = next.max_retries;
    profile.curl_mode = next.curl_mode;
    profile.note = next.note;
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportHostReport {
    pub host: String,
    pub profile: String,
    pub label: String,
    pub safe_concurrency: usize,
    pub max_concurrency: usize,
    pub current_concurrency: usize,
    pub request_gap_ms: u64,
    pub chapter_rate: f32,
    pub requests: u64,
    pub retries: u64,
    pub status_429: u64,
    pub status_503: u64,
    pub blocked: u64,
    pub curl_requests: u64,
    pub curl_fallbacks: u64,
    pub avg_latency_ms: u64,
    pub note: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportReport {
    pub hosts: Vec<TransportHostReport>,
}

/// The body is all release builds consume; `status`, `via` and `elapsed` are
/// read by the debug-only QA probes, so they are marked dead there instead of
/// growing `#[cfg]` variants of the struct.
#[derive(Debug)]
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub struct TransportResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub via: &'static str,
    pub elapsed: Duration,
}

impl TransportResponse {
    pub fn text(self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Via {
    Reqwest,
    Curl,
}

impl Via {
    fn label(self) -> &'static str {
        match self {
            Self::Reqwest => "reqwest",
            Self::Curl => "curl",
        }
    }
}

struct RawResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    via: Via,
}

impl RawResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .rev()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn is_cloudflare(&self) -> bool {
        if self.status != 403 {
            return false;
        }
        let server = self.header("server").unwrap_or_default();
        if server.to_ascii_lowercase().contains("cloudflare") || self.header("cf-ray").is_some() {
            return true;
        }
        let body = String::from_utf8_lossy(&self.body).to_ascii_lowercase();
        body.contains("just a moment")
            || body.contains("attention required")
            || body.contains("cf-chl")
            || body.contains("cloudflare")
    }
}

#[derive(Default)]
struct Counters {
    requests: AtomicU64,
    retries: AtomicU64,
    status_429: AtomicU64,
    status_503: AtomicU64,
    blocked: AtomicU64,
    curl_requests: AtomicU64,
    curl_fallbacks: AtomicU64,
    total_latency_ms: AtomicU64,
}

struct GateState {
    profile: SourceProfile,
    active: usize,
    limit: usize,
    next_slot: Instant,
    cooldown_until: Instant,
    blocked_until: Option<Instant>,
    consecutive_successes: u32,
    consecutive_failures: u32,
    recent_latencies: VecDeque<u64>,
    curl_required: bool,
}

struct HostGate {
    state: Mutex<GateState>,
    notify: Notify,
    counters: Counters,
}

impl HostGate {
    fn new(profile: SourceProfile) -> Self {
        let limit = profile.safe_concurrency.max(1);
        Self {
            state: Mutex::new(GateState {
                profile,
                active: 0,
                limit,
                next_slot: Instant::now(),
                cooldown_until: Instant::now(),
                blocked_until: None,
                consecutive_successes: 0,
                consecutive_failures: 0,
                recent_latencies: VecDeque::new(),
                curl_required: false,
            }),
            notify: Notify::new(),
            counters: Counters::default(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, GateState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn max_attempts(&self) -> usize {
        self.lock().profile.max_retries + 1
    }

    fn use_curl(&self) -> bool {
        let state = self.lock();
        match state.profile.curl_mode {
            CurlMode::Never => false,
            CurlMode::Always => true,
            CurlMode::OnBlock => state.curl_required,
        }
    }

    fn is_curl_fallback(&self) -> bool {
        let state = self.lock();
        state.profile.curl_mode == CurlMode::OnBlock && state.curl_required
    }

    fn can_curl_fallback(&self) -> bool {
        let state = self.lock();
        state.profile.curl_mode != CurlMode::Never && curl_available()
    }

    fn mark_curl_required(&self) {
        self.lock().curl_required = true;
    }

    async fn acquire(&self, kind: RequestKind) -> AppResult<GatePermit<'_>> {
        loop {
            let now = Instant::now();
            let wait = {
                let mut state = self.lock();
                if let Some(until) = state.blocked_until {
                    if now < until {
                        let remaining = until.duration_since(now).as_secs().max(1);
                        return Err(AppError::msg(format!(
                            "Source is cooling down after an anti-bot block; retry in {remaining}s"
                        )));
                    }
                    state.blocked_until = None;
                }

                let gap = state.profile.gap_for(kind);
                let ready_at = state.next_slot.max(state.cooldown_until);
                if state.active < state.limit && now >= ready_at {
                    state.active += 1;
                    state.next_slot = now + gap;
                    return Ok(GatePermit { gate: self });
                }

                if state.active >= state.limit {
                    None
                } else {
                    Some(ready_at.saturating_duration_since(now))
                }
            };

            match wait {
                Some(duration) => {
                    tokio::time::sleep(duration.max(Duration::from_millis(1))).await;
                }
                None => {
                    // A permit is released with `notify_one`, but the timeout
                    // also wakes us if a cooldown expires without a release.
                    let _ =
                        tokio::time::timeout(Duration::from_millis(200), self.notify.notified())
                            .await;
                }
            }
        }
    }

    fn record_attempt(&self, elapsed: Duration, via: Via, fallback: bool) {
        self.counters.requests.fetch_add(1, Ordering::Relaxed);
        self.counters.total_latency_ms.fetch_add(
            elapsed.as_millis().min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
        if via == Via::Curl {
            self.counters.curl_requests.fetch_add(1, Ordering::Relaxed);
            if fallback {
                self.counters.curl_fallbacks.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn record_success(&self, elapsed: Duration) {
        let mut state = self.lock();
        state.consecutive_failures = 0;
        state.consecutive_successes = state.consecutive_successes.saturating_add(1);
        state
            .recent_latencies
            .push_back(elapsed.as_millis().min(u64::MAX as u128) as u64);
        while state.recent_latencies.len() > RAMP_LATENCY_SAMPLES {
            state.recent_latencies.pop_front();
        }

        if state.consecutive_successes >= RAMP_SUCCESSES
            && state.limit < state.profile.max_concurrency
        {
            let samples = state.recent_latencies.len().max(1) as u64;
            let total: u64 = state.recent_latencies.iter().sum();
            let average = total / samples;
            let slowest = state.recent_latencies.iter().copied().max().unwrap_or(0);
            // Ramp only when latency is stable; a host that is slowing down is
            // not a host that can take another concurrent request.
            if slowest <= average.saturating_mul(2).saturating_add(150) {
                state.limit += 1;
                state.consecutive_successes = 0;
                state.recent_latencies.clear();
            }
        }
    }

    fn record_rate_limited(&self, retry_after: Option<Duration>, status: u16) {
        if status == 429 {
            self.counters.status_429.fetch_add(1, Ordering::Relaxed);
        } else {
            self.counters.status_503.fetch_add(1, Ordering::Relaxed);
        }

        let wait = retry_after.unwrap_or(DEFAULT_RETRY_AFTER).min(MAX_COOLDOWN)
            + jitter(Duration::from_millis(750));
        let mut state = self.lock();
        state.limit = (state.limit / 2).max(1);
        state.consecutive_successes = 0;
        state.recent_latencies.clear();
        state.cooldown_until = Instant::now() + wait;
        self.notify.notify_waiters();
    }

    fn record_failure(&self) {
        let mut state = self.lock();
        state.consecutive_successes = 0;
        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        if state.consecutive_failures >= 3 {
            state.limit = (state.limit / 2).max(1);
            state.consecutive_failures = 0;
        }
    }

    fn mark_blocked(&self) {
        self.counters.blocked.fetch_add(1, Ordering::Relaxed);
        let until = Instant::now() + BLOCK_COOLDOWN;
        let mut state = self.lock();
        state.blocked_until = Some(until);
        state.cooldown_until = until;
        state.consecutive_successes = 0;
        self.notify.notify_waiters();
    }

    fn update_profile(&self, next: SourceProfile) {
        let mut state = self.lock();
        apply_profile(&mut state.profile, next);
        state.limit = state.profile.safe_concurrency.max(1);
        state.consecutive_successes = 0;
        state.recent_latencies.clear();
    }

    fn report(&self, host: &str) -> TransportHostReport {
        let state = self.lock();
        let requests = self.counters.requests.load(Ordering::Relaxed);
        let total_latency = self.counters.total_latency_ms.load(Ordering::Relaxed);
        TransportHostReport {
            host: host.to_string(),
            profile: state.profile.id.to_string(),
            label: state.profile.label.to_string(),
            safe_concurrency: state.profile.safe_concurrency,
            max_concurrency: state.profile.max_concurrency,
            current_concurrency: state.limit,
            request_gap_ms: state.profile.request_gap.as_millis() as u64,
            chapter_rate: state.profile.chapter_rate,
            requests,
            retries: self.counters.retries.load(Ordering::Relaxed),
            status_429: self.counters.status_429.load(Ordering::Relaxed),
            status_503: self.counters.status_503.load(Ordering::Relaxed),
            blocked: self.counters.blocked.load(Ordering::Relaxed),
            curl_requests: self.counters.curl_requests.load(Ordering::Relaxed),
            curl_fallbacks: self.counters.curl_fallbacks.load(Ordering::Relaxed),
            avg_latency_ms: total_latency.checked_div(requests).unwrap_or(0),
            note: state.profile.note.to_string(),
        }
    }
}

struct GatePermit<'a> {
    gate: &'a HostGate,
}

impl Drop for GatePermit<'_> {
    fn drop(&mut self) {
        let mut state = self.gate.lock();
        state.active = state.active.saturating_sub(1);
        drop(state);
        self.gate.notify.notify_one();
    }
}

fn gates() -> &'static Mutex<HashMap<String, std::sync::Arc<HostGate>>> {
    static GATES: OnceLock<Mutex<HashMap<String, std::sync::Arc<HostGate>>>> = OnceLock::new();
    GATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn config() -> &'static RwLock<HashMap<String, SourceLimit>> {
    static CONFIG: OnceLock<RwLock<HashMap<String, SourceLimit>>> = OnceLock::new();
    CONFIG.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Replaces the per-host override map. Existing gates are updated in place so
/// an active job is not orphaned; new limits apply to its next request.
pub fn configure(limits: &[SourceLimit]) {
    let next: HashMap<String, SourceLimit> = limits
        .iter()
        .filter(|limit| !limit.host.trim().is_empty())
        .map(|limit| (limit.host.trim().to_ascii_lowercase(), limit.clone()))
        .collect();
    if let Ok(mut guard) = config().write() {
        *guard = next;
    }

    let updates: Vec<(String, SourceProfile)> = {
        let Ok(map) = gates().lock() else {
            return;
        };
        map.keys()
            .map(|host| (host.clone(), profile_for_host(host)))
            .collect()
    };
    if let Ok(map) = gates().lock() {
        for (host, profile) in updates {
            if let Some(gate) = map.get(&host) {
                gate.update_profile(profile);
            }
        }
    }
}

fn host_key(url: &Url) -> String {
    let host = url.host_str().unwrap_or("unknown").to_ascii_lowercase();
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

fn gate_for(url: &Url) -> std::sync::Arc<HostGate> {
    let key = host_key(url);
    let mut map = gates()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(gate) = map.get(&key) {
        return std::sync::Arc::clone(gate);
    }
    let gate = std::sync::Arc::new(HostGate::new(profile_for_host(&key)));
    map.insert(key, std::sync::Arc::clone(&gate));
    gate
}

/// Fetches a source URL through the host gate. Success is a 2xx response; all
/// non-success statuses are turned into errors after the retry/backoff policy.
pub async fn fetch(
    client: &reqwest::Client,
    url: &str,
    kind: RequestKind,
) -> AppResult<TransportResponse> {
    let parsed =
        Url::parse(url).map_err(|error| AppError::msg(format!("Invalid source URL: {error}")))?;
    let gate = gate_for(&parsed);
    let attempts = gate.max_attempts();
    let mut last_error = String::new();

    for attempt in 0..attempts {
        if attempt > 0 {
            gate.counters.retries.fetch_add(1, Ordering::Relaxed);
        }

        let permit = gate.acquire(kind).await?;
        let use_curl = gate.use_curl();
        let fallback = use_curl && gate.is_curl_fallback();
        let started = Instant::now();
        let result = send_once(client, url, kind, use_curl).await;
        let elapsed = started.elapsed();
        drop(permit);

        match result {
            Ok(raw) => {
                gate.record_attempt(elapsed, raw.via, fallback);
                if (200..300).contains(&raw.status) {
                    gate.record_success(elapsed);
                    return Ok(TransportResponse {
                        status: raw.status,
                        body: raw.body,
                        via: raw.via.label(),
                        elapsed,
                    });
                }

                if raw.status == 429 || raw.status == 503 {
                    gate.record_rate_limited(
                        parse_retry_after(raw.header("retry-after")),
                        raw.status,
                    );
                    last_error = format!(
                        "Source returned HTTP {} {}",
                        raw.status,
                        status_text(raw.status)
                    );
                    if attempt + 1 < attempts {
                        continue;
                    }
                    return Err(AppError::msg(format!(
                        "{last_error}; backed off instead of retrying at full speed"
                    )));
                }

                if raw.status == 403 && raw.is_cloudflare() {
                    if gate.can_curl_fallback() && !use_curl {
                        gate.mark_curl_required();
                        last_error = "Cloudflare rejected the reqwest TLS fingerprint (HTTP 403)"
                            .to_string();
                        if attempt + 1 < attempts {
                            continue;
                        }
                    }
                    gate.mark_blocked();
                    return Err(AppError::msg(format!(
                        "Source blocked by Cloudflare (HTTP 403); {}. Retry later instead of hammering the source",
                        if use_curl {
                            "the curl fallback was also rejected"
                        } else {
                            "no fallback is available"
                        }
                    )));
                }

                if raw.status >= 500 || raw.status == 408 {
                    gate.record_failure();
                    last_error = format!(
                        "Source returned HTTP {} {}",
                        raw.status,
                        status_text(raw.status)
                    );
                    if attempt + 1 < attempts {
                        tokio::time::sleep(backoff(attempt)).await;
                        continue;
                    }
                    return Err(AppError::msg(format!(
                        "{last_error} after {attempts} attempts"
                    )));
                }

                return Err(AppError::msg(format!(
                    "Source returned HTTP {} {}",
                    raw.status,
                    status_text(raw.status)
                )));
            }
            Err((via, error)) => {
                gate.record_attempt(elapsed, via, fallback);
                gate.record_failure();
                last_error = error;
                if attempt + 1 < attempts {
                    tokio::time::sleep(backoff(attempt)).await;
                    continue;
                }
                return Err(AppError::msg(format!(
                    "Source request failed after {attempts} attempts: {last_error}"
                )));
            }
        }
    }

    Err(AppError::msg(format!(
        "Source request failed: {last_error}"
    )))
}

async fn send_once(
    client: &reqwest::Client,
    url: &str,
    kind: RequestKind,
    use_curl: bool,
) -> Result<RawResponse, (Via, String)> {
    if use_curl {
        return curl_get(url, kind)
            .await
            .map_err(|error| (Via::Curl, error));
    }

    let request = client
        .get(url)
        .header("Accept", kind.accept())
        .header("Accept-Language", "en-US,en;q=0.8");
    let response = tokio::time::timeout(kind.timeout(), request.send())
        .await
        .map_err(|_| (Via::Reqwest, "request timed out".to_string()))?
        .map_err(|error| (Via::Reqwest, error.to_string()))?;

    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let body = tokio::time::timeout(kind.timeout(), response.bytes())
        .await
        .map_err(|_| (Via::Reqwest, "response body timed out".to_string()))?
        .map_err(|error| (Via::Reqwest, error.to_string()))?
        .to_vec();

    Ok(RawResponse {
        status,
        headers,
        body,
        via: Via::Reqwest,
    })
}

async fn curl_get(url: &str, kind: RequestKind) -> Result<RawResponse, String> {
    if !curl_available() {
        return Err(
            "system curl is not available; install curl or configure a different source profile"
                .to_string(),
        );
    }

    let id = CURL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!("novelforge-curl-{}-{id}", std::process::id()));
    let body_path = base.with_extension("body");
    let header_path = base.with_extension("headers");
    let timeout = kind.timeout().as_secs().max(1).to_string();
    let args = vec![
        "--silent".to_string(),
        "--show-error".to_string(),
        "--location".to_string(),
        "--compressed".to_string(),
        "--max-time".to_string(),
        timeout,
        "--user-agent".to_string(),
        USER_AGENT.to_string(),
        "--header".to_string(),
        format!("Accept: {}", kind.accept()),
        "--header".to_string(),
        "Accept-Language: en-US,en;q=0.8".to_string(),
        "--output".to_string(),
        body_path.to_string_lossy().to_string(),
        "--dump-header".to_string(),
        header_path.to_string_lossy().to_string(),
        "--write-out".to_string(),
        "%{http_code}".to_string(),
        "--url".to_string(),
        url.to_string(),
    ];

    let mut command = tokio::process::Command::new("curl");
    command.args(&args).stdin(Stdio::null());
    crate::spawn::hide_console(&mut command);
    let output = command
        .output()
        .await
        .map_err(|error| format!("system curl failed to start: {error}"))?;

    let status_text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let headers_text = std::fs::read_to_string(&header_path).unwrap_or_default();
    let body = std::fs::read(&body_path).unwrap_or_default();
    let _ = std::fs::remove_file(&body_path);
    let _ = std::fs::remove_file(&header_path);

    let status = status_text.parse::<u16>().map_err(|_| {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            format!(
                "system curl returned no HTTP status (exit {})",
                output.status
            )
        } else {
            format!("system curl failed: {stderr}")
        }
    })?;
    if status == 0 {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "system curl returned HTTP 000 (timed out or connection failed)".to_string()
        } else {
            format!("system curl failed: {stderr}")
        });
    }

    Ok(RawResponse {
        status,
        headers: parse_headers(&headers_text),
        body,
        via: Via::Curl,
    })
}

fn parse_headers(raw: &str) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    for line in raw.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with("HTTP/") {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    headers
}

fn parse_retry_after(value: Option<&str>) -> Option<Duration> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    httpdate::parse_http_date(value)
        .ok()
        .and_then(|time| time.duration_since(SystemTime::now()).ok())
}

fn status_text(status: u16) -> &'static str {
    match status {
        403 => "Forbidden",
        408 => "Request Timeout",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "",
    }
}

fn backoff(attempt: usize) -> Duration {
    Duration::from_secs(1u64 << attempt.min(3)) + jitter(Duration::from_millis(400))
}

static JITTER: AtomicU64 = AtomicU64::new(0);

fn jitter(max: Duration) -> Duration {
    let max_ms = max.as_millis().max(1) as u64;
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::from(duration.subsec_nanos()))
        .unwrap_or(0)
        ^ JITTER.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed);
    Duration::from_millis(seed % max_ms)
}

static CURL_COUNTER: AtomicU64 = AtomicU64::new(0);
static CURL_STATE: AtomicU8 = AtomicU8::new(0);

fn curl_available() -> bool {
    match CURL_STATE.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => {
            let mut command = std::process::Command::new("curl");
            command
                .arg("--version")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let available = crate::spawn::hide_console_std(&mut command)
                .status()
                .map(|status| status.success())
                .unwrap_or(false);
            CURL_STATE.store(if available { 1 } else { 2 }, Ordering::Relaxed);
            available
        }
    }
}

fn report_for_host_key(host: &str) -> Option<TransportHostReport> {
    let map = gates().lock().ok()?;
    map.get(host).map(|gate| gate.report(host))
}

/// Reports every host that has handled an app-level request this session.
pub fn report() -> TransportReport {
    let Ok(map) = gates().lock() else {
        return TransportReport::default();
    };
    let mut hosts: Vec<TransportHostReport> =
        map.iter().map(|(host, gate)| gate.report(host)).collect();
    hosts.sort_by(|left, right| left.host.cmp(&right.host));
    TransportReport { hosts }
}

pub fn report_for_url(url: &str) -> Option<TransportHostReport> {
    let parsed = Url::parse(url).ok()?;
    report_for_host_key(&host_key(&parsed))
}

/// A one-line report for job logs, QA output and `indexDetail`.
pub fn summary_for_url(url: &str) -> String {
    let Some(report) = report_for_url(url) else {
        return "no app-level requests recorded".to_string();
    };
    format!(
        "{} · {} request(s) · {} retr{} · {}×429 · {}×503 · {} curl · {} blocked · avg {} ms · concurrency {}/{} · gap {} ms · rate {}/s",
        report.label,
        report.requests,
        report.retries,
        if report.retries == 1 { "y" } else { "ies" },
        report.status_429,
        report.status_503,
        report.curl_requests,
        report.blocked,
        report.avg_latency_ms,
        report.current_concurrency,
        report.max_concurrency,
        report.request_gap_ms,
        report.chapter_rate
    )
}

/// The lncrawl chapter rate for a source URL. Settings overrides win; built-in
/// profiles keep the verified default of 2 req/s.
pub fn chapter_rate_for_url(url: &str) -> f32 {
    Url::parse(url)
        .ok()
        .map(|parsed| profile_for_host(&host_key(&parsed)).chapter_rate)
        .unwrap_or(2.0)
}

/// The profile identity for a source URL, used in transport log lines.
pub fn profile_label_for_url(url: &str) -> String {
    Url::parse(url)
        .ok()
        .map(|parsed| profile_for_host(&host_key(&parsed)).label.to_string())
        .unwrap_or_else(|| "Generic source".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn novelfull_profile_uses_curl_and_stays_sequential() {
        let profile = builtin_profile("novelfull.net");
        assert_eq!(profile.id, "novelfull");
        assert_eq!(profile.safe_concurrency, 1);
        assert_eq!(profile.max_concurrency, 1);
        assert_eq!(profile.curl_mode, CurlMode::Always);
        assert!(profile.note.contains("HTTP 403"));
    }

    #[test]
    fn freewebnovel_profile_matches_the_verified_index_run() {
        let profile = builtin_profile("www.freewebnovel.com");
        assert_eq!(profile.id, "freewebnovel");
        assert_eq!(profile.safe_concurrency, 2);
        assert_eq!(profile.max_concurrency, 4);
        assert_eq!(profile.request_gap, Duration::from_millis(700));
        assert!(profile.note.contains("2,334"));
    }

    #[test]
    fn generic_profile_is_conservative() {
        let profile = builtin_profile("example.com");
        assert_eq!(profile.safe_concurrency, 2);
        assert_eq!(profile.max_concurrency, 4);
        assert!(profile.request_gap >= Duration::from_millis(700));
    }

    #[test]
    fn retry_after_accepts_seconds_and_http_dates() {
        assert_eq!(parse_retry_after(Some("12")), Some(Duration::from_secs(12)));
        let future = SystemTime::now() + Duration::from_secs(30);
        let header = httpdate::fmt_http_date(future);
        let parsed = parse_retry_after(Some(&header)).expect("date header");
        assert!(parsed <= Duration::from_secs(30));
        assert!(parsed >= Duration::from_secs(25));
        assert_eq!(parse_retry_after(Some("not a date")), None);
    }

    #[test]
    fn per_host_override_pins_concurrency_and_rate() {
        let mut profile = builtin_profile("example.com");
        apply_limit(
            &mut profile,
            &SourceLimit {
                host: "example.com".to_string(),
                concurrency: 3,
                request_gap_ms: 1_234,
                chapter_rate: 1.5,
            },
        );
        assert_eq!(profile.safe_concurrency, 3);
        assert_eq!(profile.max_concurrency, 3);
        assert_eq!(profile.request_gap, Duration::from_millis(1_234));
        assert_eq!(profile.chapter_rate, 1.5);
        assert!(profile.note.contains("override"));
    }

    #[test]
    fn override_keys_match_www_and_port_variants() {
        let keys = limit_key_candidates("www.novelfull.net:8443");
        assert_eq!(keys[0], "www.novelfull.net:8443");
        assert_eq!(keys[1], "www.novelfull.net");
        assert_eq!(keys[2], "novelfull.net:8443");
        assert_eq!(keys[3], "novelfull.net");
        assert_eq!(
            limit_key_candidates("novelfull.net"),
            [
                "novelfull.net".to_string(),
                "novelfull.net".to_string(),
                "novelfull.net".to_string(),
                "novelfull.net".to_string()
            ]
        );
    }

    #[test]
    fn request_kinds_have_distinct_accept_headers() {
        assert!(RequestKind::Cover.accept().contains("image/"));
        assert!(RequestKind::Toc.accept().contains("text/html"));
        assert!(RequestKind::Toc.timeout() > RequestKind::Cover.timeout());
    }

    #[tokio::test]
    async fn rate_limit_halves_concurrency_and_honours_retry_after() {
        let gate = HostGate::new(builtin_profile("example.com"));
        assert_eq!(gate.lock().limit, 2);

        gate.record_rate_limited(Some(Duration::from_secs(1)), 429);
        {
            let state = gate.lock();
            assert_eq!(state.limit, 1);
            assert!(state.cooldown_until > Instant::now());
        }
        assert_eq!(gate.counters.status_429.load(Ordering::Relaxed), 1);

        let report = gate.report("example.com");
        assert_eq!(report.status_429, 1);
        assert_eq!(report.current_concurrency, 1);
    }

    #[tokio::test]
    async fn clean_streak_ramps_concurrency_slowly() {
        let gate = HostGate::new(builtin_profile("example.com"));
        for _ in 0..(RAMP_SUCCESSES - 1) {
            gate.record_success(Duration::from_millis(100));
        }
        assert_eq!(gate.lock().limit, 2);
        gate.record_success(Duration::from_millis(100));
        assert_eq!(gate.lock().limit, 3);
        for _ in 0..RAMP_SUCCESSES {
            gate.record_success(Duration::from_millis(100));
        }
        assert_eq!(gate.lock().limit, 4);
    }

    #[tokio::test]
    async fn blocked_host_fails_fast_instead_of_looping() {
        let gate = HostGate::new(builtin_profile("example.com"));
        gate.mark_blocked();
        let error = match gate.acquire(RequestKind::Toc).await {
            Ok(_) => panic!("blocked gate must not hand out a permit"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("cooling down"));
        assert_eq!(gate.report("example.com").blocked, 1);
    }

    #[tokio::test]
    async fn fetch_retries_429_with_retry_after() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        let requests = std::sync::Arc::new(AtomicU64::new(0));
        let seen = std::sync::Arc::clone(&requests);

        let server = tokio::spawn(async move {
            let mut first = true;
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut buffer = [0u8; 2048];
                let _ = socket.read(&mut buffer).await;
                seen.fetch_add(1, Ordering::Relaxed);
                let response = if first {
                    first = false;
                    "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbusy"
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                };
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("client");
        let url = format!("http://{address}/book");
        let started = Instant::now();
        let response = fetch(&client, &url, RequestKind::Toc)
            .await
            .expect("eventual success");
        let elapsed = started.elapsed();
        server.abort();

        assert_eq!(response.status, 200);
        assert_eq!(response.text(), "ok");
        assert_eq!(requests.load(Ordering::Relaxed), 2);
        assert!(elapsed >= Duration::from_secs(1));
        let report = report_for_url(&url).expect("report");
        assert_eq!(report.status_429, 1);
        assert_eq!(report.retries, 1);
        assert_eq!(report.current_concurrency, 1);
    }
}
