//! MiniMax Token Plan (formerly the Coding Plan): the quota of the plan's subscription key, saved in
//! Quota Control or exported as `MINIMAX_API_KEY`. MiniMax has no CLI or desktop login to reuse, so
//! nothing is read from disk or from the credential store; Windows, macOS and Linux behave the same.
//!
//! Endpoints (`GET`, `Authorization: Bearer <key>`, no group id):
//! - `/v1/token_plan/remains`, the documented Token Plan endpoint, then
//!   `/v1/api/openplatform/coding_plan/remains`, the Coding Plan endpoint it replaced, when the first
//!   is missing, refuses the key or finds no plan for it.
//! - On `https://api.minimax.io` for global keys and `https://api.minimaxi.com` for keys issued in
//!   mainland China. A key only works on the host of its own region, so a key the global host refuses
//!   is tried on the China host, and the endpoint that answered is remembered for 12 hours.
//!
//! MiniMax reports most failures as HTTP 200 with a `base_resp.status_code`: 1004 or 2049 is a
//! refused key, 2062 a key whose account has no active plan. Each `model_remains` lane (`general`
//! for the text models, `video` and other media on the larger plans) has an interval window
//! (5 hours for text, a day for media) and a weekly window, as a remaining percent or as counts; a
//! lane the plan leaves out comes as status 3 with everything left. `*_usage_count` is what was used
//! on the Token Plan endpoint but what is left on the Coding Plan one. The card's Session and Weekly
//! meters are the tightest lane of the 5-hour window and of the week; an answer with several lanes,
//! or with a media lane's day, adds one meter per lane and window after them.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use uc_core::{
    ErrorCategory, HttpRequest, HttpResponse, MetricLine, Provider, ProviderLink,
    SimpleProviderError, WidgetDescriptor,
};

use crate::service::{ApiKeyHelp, Connection, FetchContext, Reading, Service};
use crate::support::{http, lines, value};

pub(crate) struct MiniMax;

const NAME: &str = "MiniMax";
const TOKEN_PLAN: &str = "/v1/token_plan/remains";
const CODING_PLAN: &str = "/v1/api/openplatform/coding_plan/remains";
const ENDPOINT_MEMO: &str = "minimax.endpoint";
const SESSION: &str = "Session";
const WEEKLY: &str = "Weekly";
const REFUSED: &str =
    "MiniMax refused this API key. Use a Token Plan key, not a pay-as-you-go key.";
const NO_PLAN: &str =
    "This key's account has no active MiniMax Token Plan, so there is no quota to show.";
const NO_QUOTA: &str = "MiniMax returned no Token Plan quota for this key.";

/// A quota endpoint: the host of one region, and whether it is the older Coding Plan endpoint,
/// whose counts are what is left rather than what was used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Endpoint {
    host: &'static str,
    coding_plan: bool,
}

impl Endpoint {
    fn url(self) -> String {
        let path = if self.coding_plan {
            CODING_PLAN
        } else {
            TOKEN_PLAN
        };
        format!("{}{path}", self.host)
    }
}

/// The global host's endpoints, then the China mainland host's: indexes 0 and 1 are one region, 2
/// and 3 the other, and each region lists the Token Plan endpoint before the Coding Plan one.
const ENDPOINTS: [Endpoint; 4] = [
    Endpoint {
        host: "https://api.minimax.io",
        coding_plan: false,
    },
    Endpoint {
        host: "https://api.minimax.io",
        coding_plan: true,
    },
    Endpoint {
        host: "https://api.minimaxi.com",
        coding_plan: false,
    },
    Endpoint {
        host: "https://api.minimaxi.com",
        coding_plan: true,
    },
];

/// The fields of one window of a lane, each as `[snake_case, camelCase]`.
struct Fields {
    left_percent: [&'static str; 2],
    total: [&'static str; 2],
    count: [&'static str; 2],
    status: [&'static str; 2],
    start: [&'static str; 2],
    end: [&'static str; 2],
    remains: [&'static str; 2],
}

const INTERVAL: Fields = Fields {
    left_percent: [
        "current_interval_remaining_percent",
        "currentIntervalRemainingPercent",
    ],
    total: ["current_interval_total_count", "currentIntervalTotalCount"],
    count: ["current_interval_usage_count", "currentIntervalUsageCount"],
    status: ["current_interval_status", "currentIntervalStatus"],
    start: ["start_time", "startTime"],
    end: ["end_time", "endTime"],
    remains: ["remains_time", "remainsTime"],
};

const WEEK: Fields = Fields {
    left_percent: [
        "current_weekly_remaining_percent",
        "currentWeeklyRemainingPercent",
    ],
    total: ["current_weekly_total_count", "currentWeeklyTotalCount"],
    count: ["current_weekly_usage_count", "currentWeeklyUsageCount"],
    status: ["current_weekly_status", "currentWeeklyStatus"],
    start: ["weekly_start_time", "weeklyStartTime"],
    end: ["weekly_end_time", "weeklyEndTime"],
    remains: ["weekly_remains_time", "weeklyRemainsTime"],
};

/// Where an answer may name the plan, at its top or under `data`.
const PLAN_TITLES: [&str; 10] = [
    "/current_subscribe_title",
    "/currentSubscribeTitle",
    "/plan_name",
    "/planName",
    "/combo_title",
    "/comboTitle",
    "/current_plan_title",
    "/currentPlanTitle",
    "/current_combo_card/title",
    "/currentComboCard/title",
];

#[async_trait]
impl Service for MiniMax {
    fn id(&self) -> &'static str {
        "minimax"
    }

    fn name(&self) -> &'static str {
        NAME
    }

    fn links(&self) -> Vec<ProviderLink> {
        vec![
            ProviderLink::new("Plans", "https://platform.minimax.io/subscribe/token-plan"),
            ProviderLink::new(
                "API Keys",
                "https://platform.minimax.io/user-center/basic-information/interface-key",
            ),
        ]
    }

    fn connection(&self) -> Connection {
        Connection::api_key(ApiKeyHelp {
            env: &["MINIMAX_API_KEY"],
            url: "https://platform.minimax.io/user-center/basic-information/interface-key",
            fields: &[],
        })
    }

    fn descriptors(&self, provider: &Provider) -> Vec<WidgetDescriptor> {
        [("session", SESSION), ("weekly", WEEKLY)]
            .into_iter()
            .map(|(suffix, title)| {
                WidgetDescriptor::percent(
                    format!("{}.{suffix}", provider.id),
                    provider,
                    title,
                    None,
                    None,
                )
                .exporting_progress(suffix, "percent")
            })
            .collect()
    }

    async fn fetch(&self, context: &FetchContext<'_>) -> Result<Reading, SimpleProviderError> {
        let key = context.secret.key().ok_or_else(|| {
            http::invalid("The saved MiniMax key is empty. Add the key again in Accounts.")
        })?;
        let remembered = remembered(context).await;
        let mut tally = Tally::default();
        let mut previous_region = None;
        for (step, index) in order(remembered).into_iter().enumerate() {
            let region = index / 2;
            if let Some(previous) = previous_region
                && previous != region
                && !tally.tries_other_region(previous)
            {
                break;
            }
            previous_region = Some(region);
            let from_memo = step == 0 && remembered.is_some();
            match attempt(context, key, ENDPOINTS[index]).await? {
                Attempt::Usage(reading) => {
                    context
                        .memo
                        .put(
                            ENDPOINT_MEMO,
                            json!(index),
                            Some(context.now + Duration::hours(12)),
                        )
                        .await;
                    return Ok(reading);
                }
                // The endpoint that answered last time: a passing failure is reported as it is.
                Attempt::Failed {
                    error,
                    transient: true,
                } if from_memo => return Err(error),
                outcome => {
                    if from_memo {
                        context.memo.remove(ENDPOINT_MEMO).await;
                    }
                    tally.record(region, outcome);
                }
            }
        }
        Err(tally.into_error())
    }
}

/// The endpoint that answered a refresh in the last 12 hours.
async fn remembered(context: &FetchContext<'_>) -> Option<usize> {
    let index = context
        .memo
        .get(ENDPOINT_MEMO, context.now)
        .await?
        .as_u64()?;
    usize::try_from(index)
        .ok()
        .filter(|index| *index < ENDPOINTS.len())
}

/// The remembered endpoint, the other endpoint of its region, then the other region's; without one,
/// every endpoint in `ENDPOINTS` order.
fn order(remembered: Option<usize>) -> Vec<usize> {
    let Some(first) = remembered else {
        return (0..ENDPOINTS.len()).collect();
    };
    let sibling = if first % 2 == 0 { first + 1 } else { first - 1 };
    let other = if first < 2 { 2 } else { 0 };
    vec![first, sibling, other, other + 1]
}

/// What one endpoint made of the key.
enum Attempt {
    Usage(Reading),
    /// HTTP 401 or 403, or a refused key in the body.
    Refused,
    /// A key MiniMax knows whose account has no active plan.
    NoPlan,
    /// Anything else. `transient` marks failures a later refresh may not meet again (the network,
    /// a server error, an unreadable answer).
    Failed {
        error: SimpleProviderError,
        transient: bool,
    },
}

async fn attempt(
    context: &FetchContext<'_>,
    key: &str,
    endpoint: Endpoint,
) -> Result<Attempt, SimpleProviderError> {
    let url = endpoint.url();
    let request = HttpRequest::get(url.as_str())
        .bearer(key)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json");
    let response = match http::send(context.http, request, NAME).await {
        Ok(response) => response,
        Err(error) => return Ok(failed(&url, error)),
    };
    match response.status {
        401 | 403 => return Ok(refused(&url)),
        429 => return Err(http::status_error(&response, NAME)),
        _ if !response.is_success() => {
            return Ok(failed(&url, http::status_error(&response, NAME)));
        }
        _ => {}
    }
    let body = match http::parse(&response, NAME) {
        Ok(body) => body,
        Err(error) => return Ok(failed(&url, error)),
    };
    if let Some((code, message)) = api_error(&body) {
        return answer_error(&url, code, &message);
    }
    let Some(lanes) = lanes(&body) else {
        return Ok(failed(&url, http::decoding(NAME)));
    };
    let read: Vec<Lane> = lanes
        .iter()
        .map(|lane| Lane::read(lane, endpoint.coding_plan, context.now))
        .collect();
    let meters = meters(&read, context.now);
    if meters.is_empty() {
        return Ok(failed(&url, http::not_available(NO_QUOTA)));
    }
    Ok(Attempt::Usage(Reading::new(plan(&body, &lanes), meters)))
}

fn refused(url: &str) -> Attempt {
    tracing::debug!(target: "minimax", "{url} refused the key");
    Attempt::Refused
}

fn failed(url: &str, error: SimpleProviderError) -> Attempt {
    tracing::debug!(target: "minimax", "{url} failed: {}", error.message);
    let transient = matches!(
        error.category,
        ErrorCategory::Network | ErrorCategory::Http5xx | ErrorCategory::Decoding
    );
    Attempt::Failed { error, transient }
}

/// A non-zero `base_resp.status_code` (at the top of the answer or under `data`) with its message
/// in lower case.
fn api_error(body: &Value) -> Option<(i64, String)> {
    [
        "/base_resp",
        "/baseResp",
        "/data/base_resp",
        "/data/baseResp",
    ]
    .iter()
    .filter_map(|pointer| body.pointer(pointer))
    .find_map(|status| {
        let code = value::number(status, "/status_code")
            .or_else(|| value::number(status, "/statusCode"))? as i64;
        let message = value::text(status, "/status_msg")
            .or_else(|| value::text(status, "/statusMsg"))
            .unwrap_or_default();
        (code != 0).then(|| (code, message.to_lowercase()))
    })
}

/// What a `base_resp` failure says about the key.
fn answer_error(url: &str, code: i64, message: &str) -> Result<Attempt, SimpleProviderError> {
    if code == 2062 || message.contains("no active") {
        return Ok(Attempt::NoPlan);
    }
    let refusal = [
        "authoriz",
        "api key",
        "api secret",
        "log in",
        "login",
        "cookie",
    ];
    if matches!(code, 1004 | 2049) || refusal.iter().any(|word| message.contains(word)) {
        return Ok(refused(url));
    }
    if matches!(code, 1002 | 1039 | 1041) || message.contains("rate limit") {
        return Err(rate_limited());
    }
    // Unknown error, request timeout, internal error, system error: MiniMax says to retry later.
    let error = if matches!(code, 1000 | 1001 | 1024 | 1033) {
        server_error(code)
    } else {
        http::not_available(format!("MiniMax did not return usage (error {code})."))
    };
    Ok(failed(url, error))
}

/// A MiniMax-side failure reported in an HTTP 200 body, counted as a server error so a later
/// refresh tries again.
fn server_error(code: i64) -> SimpleProviderError {
    SimpleProviderError::new(
        ErrorCategory::Http5xx,
        format!("MiniMax could not report usage right now (error {code})."),
    )
}

/// The error `status_error` gives an HTTP 429, for the same limit reported in an HTTP 200 body.
fn rate_limited() -> SimpleProviderError {
    http::status_error(
        &HttpResponse {
            status: 429,
            headers: Default::default(),
            body: Vec::new(),
        },
        NAME,
    )
}

/// What the endpoints tried so far said, for the error a refresh ends with when none answered.
#[derive(Default)]
struct Tally {
    /// Per region (global, China): whether an endpoint refused the key.
    refused: [bool; 2],
    no_plan: bool,
    /// The first other failure.
    failure: Option<SimpleProviderError>,
}

impl Tally {
    fn record(&mut self, region: usize, outcome: Attempt) {
        match outcome {
            Attempt::Usage(_) => {}
            Attempt::Refused => self.refused[region] = true,
            Attempt::NoPlan => self.no_plan = true,
            Attempt::Failed { error, .. } => {
                if self.failure.is_none() {
                    self.failure = Some(error);
                }
            }
        }
    }

    /// Whether to go on to the other region: only after this one refused the key, since a key
    /// works on the host of the region that issued it, and never once MiniMax knew the key.
    fn tries_other_region(&self, region: usize) -> bool {
        self.refused[region] && !self.no_plan
    }

    fn into_error(self) -> SimpleProviderError {
        if self.no_plan {
            http::not_available(NO_PLAN)
        } else if self.refused.contains(&true) {
            http::invalid(REFUSED)
        } else {
            self.failure.unwrap_or_else(|| http::decoding(NAME))
        }
    }
}

/// The `model_remains` lanes, at the top of the answer or under `data`; `None` when the answer has
/// no such field at all.
fn lanes(body: &Value) -> Option<Vec<&Value>> {
    let found = [
        "/model_remains",
        "/modelRemains",
        "/data/model_remains",
        "/data/modelRemains",
    ]
    .iter()
    .find_map(|pointer| body.pointer(pointer))?;
    Some(
        found
            .as_array()
            .map(|lanes| lanes.iter().collect())
            .unwrap_or_default(),
    )
}

/// One window of a lane as MiniMax reports it.
struct Window {
    left_percent: Option<f64>,
    total: Option<f64>,
    count: Option<f64>,
    status: Option<i64>,
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
    remains_ms: Option<f64>,
}

impl Window {
    fn read(lane: &Value, fields: &Fields) -> Self {
        let field = |names: [&str; 2]| {
            names
                .iter()
                .filter_map(|name| lane.get(*name))
                .find(|found| !found.is_null())
        };
        Self {
            left_percent: field(fields.left_percent).and_then(value::as_number),
            total: field(fields.total).and_then(value::as_number),
            count: field(fields.count).and_then(value::as_number),
            status: field(fields.status)
                .and_then(value::as_number)
                .map(|status| status as i64),
            start: field(fields.start).and_then(value::as_time),
            end: field(fields.end).and_then(value::as_time),
            remains_ms: field(fields.remains).and_then(value::as_number),
        }
    }

    /// Status 3 with the whole allowance left: a window the plan does not meter.
    fn unmetered(&self) -> bool {
        self.status == Some(3) && self.left_percent.is_some_and(|left| left >= 100.0)
    }

    /// An unmetered window without counts: a medium the plan leaves out (the video lane of Token
    /// Plan Plus).
    fn left_out(&self) -> bool {
        self.unmetered()
            && self.total.is_none_or(|total| total <= 0.0)
            && self.count.is_none_or(|count| count <= 0.0)
    }

    /// The used share: 100 minus the remaining percent, else from the counts, which are what was
    /// used on the Token Plan endpoint and what is left on the Coding Plan one.
    fn used_percent(&self, coding_plan: bool) -> Option<f64> {
        if let Some(left) = self.left_percent {
            return Some(100.0 - left.clamp(0.0, 100.0));
        }
        let total = self.total.filter(|total| *total > 0.0)?;
        let count = self.count?.clamp(0.0, total);
        let used = if coding_plan { total - count } else { count };
        Some(used / total * 100.0)
    }

    /// The window's end while it is ahead, else now plus the milliseconds MiniMax says remain.
    fn resets_at(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.end.filter(|end| *end > now).or_else(|| {
            let remains = self.remains_ms.filter(|remains| *remains > 0.0)?;
            now.checked_add_signed(Duration::try_milliseconds(remains.round() as i64)?)
        })
    }

    fn meter(&self, coding_plan: bool, now: DateTime<Utc>) -> Option<Meter> {
        if self.left_out() {
            return None;
        }
        Some(Meter {
            used: self.used_percent(coding_plan)?,
            resets_at: self.resets_at(now),
            span: self
                .start
                .zip(self.end)
                .map(|(start, end)| end - start)
                .filter(|span| *span > Duration::zero()),
        })
    }
}

/// A metered window: the used percent, the reset, and the length when MiniMax states its start
/// and end.
#[derive(Clone, Copy, Debug)]
struct Meter {
    used: f64,
    resets_at: Option<DateTime<Utc>>,
    span: Option<Duration>,
}

impl Meter {
    /// Whether this is the 5-hour window rather than a media lane's day: a span of at most 12
    /// hours, or, without start and end times, a reset at most 6 hours away.
    fn is_session(&self, now: DateTime<Utc>) -> bool {
        match self.span {
            Some(span) => span <= Duration::hours(12),
            None => self
                .resets_at
                .is_none_or(|reset| reset - now <= Duration::hours(6)),
        }
    }

    /// The stated length in milliseconds when it is plausible, else `fallback`.
    fn period_ms(&self, fallback: i64) -> i64 {
        self.span
            .map(|span| span.num_milliseconds())
            .filter(|ms| (lines::HOUR_MS..=2 * lines::WEEK_MS).contains(ms))
            .unwrap_or(fallback)
    }

    /// The more used of two windows, the first on a tie.
    fn tighter(self, other: Self) -> Self {
        if other.used > self.used { other } else { self }
    }
}

fn tightest(current: Option<Meter>, found: Option<Meter>) -> Option<Meter> {
    match (current, found) {
        (Some(current), Some(found)) => Some(current.tighter(found)),
        (current, found) => current.or(found),
    }
}

/// A lane's week.
#[derive(Clone, Copy, Debug)]
enum Week {
    Metered(Meter),
    /// The text lane's week that status 3 leaves unmetered.
    Unlimited,
}

/// A lane of `model_remains`, read: its name on the card, its interval window and its week.
struct Lane {
    label: Option<String>,
    interval: Option<Meter>,
    week: Option<Week>,
}

impl Lane {
    fn read(lane: &Value, coding_plan: bool, now: DateTime<Utc>) -> Self {
        let name = lane_name(lane);
        let week = Window::read(lane, &WEEK);
        let week = if is_text_lane(name) && week.unmetered() {
            Some(Week::Unlimited)
        } else {
            week.meter(coding_plan, now).map(Week::Metered)
        };
        Self {
            label: lane_label(name),
            interval: Window::read(lane, &INTERVAL).meter(coding_plan, now),
            week,
        }
    }

    fn metered_week(&self) -> Option<Meter> {
        match self.week {
            Some(Week::Metered(meter)) => Some(meter),
            _ => None,
        }
    }
}

/// The card's Session and Weekly meters, each the tightest lane of its window (a week only the text
/// lane leaves unmetered reads Unlimited), then the lanes' own meters.
fn meters(lanes: &[Lane], now: DateTime<Utc>) -> Vec<MetricLine> {
    let mut meters = Vec::new();
    if let Some(session) = lanes
        .iter()
        .filter_map(|lane| lane.interval)
        .filter(|meter| meter.is_session(now))
        .reduce(Meter::tighter)
    {
        meters.push(lines::percent(
            SESSION,
            session.used,
            session.resets_at,
            Some(session.period_ms(5 * lines::HOUR_MS)),
        ));
    }
    match lanes
        .iter()
        .filter_map(Lane::metered_week)
        .reduce(Meter::tighter)
    {
        Some(week) => meters.push(lines::percent(
            WEEKLY,
            week.used,
            week.resets_at,
            Some(lines::WEEK_MS),
        )),
        None if lanes
            .iter()
            .any(|lane| matches!(lane.week, Some(Week::Unlimited))) =>
        {
            meters.push(lines::badge(WEEKLY, "Unlimited"));
        }
        None => {}
    }
    meters.extend(lane_meters(lanes, now));
    meters
}

/// One meter per lane and window after the card's own, when the answer splits the quota into
/// several lanes (the text models and video on the larger plans) or has a window the Session meter
/// cannot show (a media lane's day), so each can be shown on its own. Lanes with the same name keep
/// their tightest window.
fn lane_meters(lanes: &[Lane], now: DateTime<Utc>) -> Vec<MetricLine> {
    let mut named: Vec<(&str, Option<Meter>, Option<Meter>)> = Vec::new();
    for lane in lanes {
        let (Some(label), interval, week) =
            (lane.label.as_deref(), lane.interval, lane.metered_week())
        else {
            continue;
        };
        if interval.is_none() && week.is_none() {
            continue;
        }
        match named.iter_mut().find(|(known, _, _)| *known == label) {
            Some(entry) => {
                entry.1 = tightest(entry.1, interval);
                entry.2 = tightest(entry.2, week);
            }
            None => named.push((label, interval, week)),
        }
    }
    let daily = named
        .iter()
        .any(|(_, interval, _)| interval.is_some_and(|meter| !meter.is_session(now)));
    if named.len() < 2 && !daily {
        return Vec::new();
    }
    named
        .into_iter()
        .flat_map(|(label, interval, week)| {
            let interval = interval.map(|meter| {
                let fallback = if meter.is_session(now) {
                    5 * lines::HOUR_MS
                } else {
                    lines::DAY_MS
                };
                lines::percent(
                    label,
                    meter.used,
                    meter.resets_at,
                    Some(meter.period_ms(fallback)),
                )
            });
            let week = week.map(|meter| {
                lines::percent(
                    &format!("{label} Weekly"),
                    meter.used,
                    meter.resets_at,
                    Some(lines::WEEK_MS),
                )
            });
            interval.into_iter().chain(week)
        })
        .collect()
}

fn lane_name(lane: &Value) -> &str {
    value::text(lane, "/model_name")
        .or_else(|| value::text(lane, "/modelName"))
        .unwrap_or_default()
}

/// The text models' lane: Token Plan's `general`, or a text model the Coding Plan answers name
/// (`MiniMax-M2.7`, `MiniMax-M*`, `M2.5-highspeed`).
fn is_text_lane(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "general"
        || name.contains("minimax-m")
        || name.starts_with("m2.")
        || name.starts_with("m3.")
}

/// A lane's name as MiniMax's dashboard shows it: `general` → General, `speech-2.8-hd` → Text to
/// Speech; an unknown model in title case.
fn lane_label(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let label = match lower.as_str() {
        "" => return None,
        "general" => "General",
        "video" => "Video",
        _ if is_text_lane(&lower) => "Text Generation",
        _ if lower.contains("speech") => "Text to Speech",
        _ if lower.contains("hailuo") && lower.contains("fast") => "Image to Video",
        _ if lower.contains("hailuo") => "Text to Video",
        _ if lower.starts_with("image-") => "Image Generation",
        _ if lower.contains("music") => "Music Generation",
        _ => return Some(title_case(name)),
    };
    Some(label.to_string())
}

/// `lyrics_generation-v2` → `Lyrics Generation V2`, keeping HD and TTS in capitals.
fn title_case(name: &str) -> String {
    name.split(['-', '_', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let lower = word.to_ascii_lowercase();
            if matches!(lower.as_str(), "hd" | "tts") {
                return lower.to_ascii_uppercase();
            }
            let mut characters = lower.chars();
            characters
                .next()
                .map(|first| first.to_uppercase().chain(characters).collect())
                .unwrap_or_default()
        })
        .collect::<Vec<String>>()
        .join(" ")
}

/// The plan: a title the answer carries, else Plus when the video lane comes as a medium the plan
/// leaves out, since Token Plan Plus is the plan without video generation.
fn plan(body: &Value, lanes: &[&Value]) -> Option<String> {
    let data = body.get("data").filter(|data| data.is_object());
    let title = [Some(body), data].into_iter().flatten().find_map(|scope| {
        PLAN_TITLES
            .iter()
            .find_map(|pointer| value::text(scope, pointer))
    });
    if let Some(title) = title {
        return format_plan(title);
    }
    let text = lanes.iter().any(|lane| is_text_lane(lane_name(lane)));
    let video_left_out = lanes.iter().any(|lane| {
        lane_name(lane).eq_ignore_ascii_case("video") && Window::read(lane, &INTERVAL).left_out()
    });
    (text && video_left_out).then(|| "Plus".to_string())
}

/// A plan title as its tier: `Token Plan · TokenPlanMax-年度会员` → `Max`. A title without a known
/// tier stays as MiniMax wrote it.
fn format_plan(title: &str) -> Option<String> {
    let title = title.trim();
    if title.is_empty() {
        return None;
    }
    let words = words(title);
    let tier = ["Ultra", "Max", "Plus", "Pro", "Starter"]
        .into_iter()
        .find(|tier| words.iter().any(|word| word.eq_ignore_ascii_case(tier)));
    Some(tier.map_or_else(|| title.to_string(), str::to_string))
}

/// The Latin words of a title, joined words split at capitals (`TokenPlanMax` → Token, Plan, Max),
/// with the brand left out, since its `Max` is no tier. Other scripts separate words
/// (`TokenPlanPlus年度会员` → Token, Plan, Plus).
fn words(title: &str) -> Vec<String> {
    let lower = title.to_ascii_lowercase();
    let mut cleaned = String::with_capacity(title.len());
    let mut rest = 0;
    while let Some(offset) = lower[rest..].find("minimax") {
        cleaned.push_str(&title[rest..rest + offset]);
        cleaned.push(' ');
        rest += offset + "minimax".len();
    }
    cleaned.push_str(&title[rest..]);
    let mut words = Vec::new();
    for chunk in cleaned.split(|character: char| !character.is_ascii_alphanumeric()) {
        let mut word = String::new();
        let mut after_lower = false;
        for character in chunk.chars() {
            if character.is_ascii_uppercase() && after_lower {
                words.push(std::mem::take(&mut word));
            }
            after_lower = character.is_ascii_lowercase() || character.is_ascii_digit();
            word.push(character);
        }
        if !word.is_empty() {
            words.push(word);
        }
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Roots;
    use crate::testing::{Scripted, context_at, header};
    use chrono::TimeZone;

    const GLOBAL_TOKEN: &str = "https://api.minimax.io/v1/token_plan/remains";
    const GLOBAL_CODING: &str = "https://api.minimax.io/v1/api/openplatform/coding_plan/remains";
    const CHINA_TOKEN: &str = "https://api.minimaxi.com/v1/token_plan/remains";
    const CHINA_CODING: &str = "https://api.minimaxi.com/v1/api/openplatform/coding_plan/remains";

    /// A Token Plan Plus answer as MiniMax sends it: the `general` lane, and the video lane as a
    /// medium the plan leaves out (status 3, everything left). Read at `1_780_282_340`.
    const PLUS: &str = r#"{
        "model_remains": [
            {"start_time": 1780279200000, "end_time": 1780297200000, "remains_time": 16659830,
             "current_interval_total_count": 0, "current_interval_usage_count": 0,
             "model_name": "general",
             "current_weekly_total_count": 0, "current_weekly_usage_count": 0,
             "weekly_start_time": 1780243200000, "weekly_end_time": 1780848000000,
             "weekly_remains_time": 567459830,
             "current_interval_status": 1, "current_interval_remaining_percent": 96,
             "current_weekly_status": 1, "current_weekly_remaining_percent": 99},
            {"start_time": 1780243200000, "end_time": 1780329600000, "remains_time": 49059830,
             "current_interval_total_count": 0, "current_interval_usage_count": 0,
             "model_name": "video",
             "current_weekly_total_count": 0, "current_weekly_usage_count": 0,
             "weekly_start_time": 1780243200000, "weekly_end_time": 1780848000000,
             "weekly_remains_time": 567459830,
             "current_interval_status": 3, "current_interval_remaining_percent": 100,
             "current_weekly_status": 3, "current_weekly_remaining_percent": 100}
        ],
        "base_resp": {"status_code": 0, "status_msg": "success"}
    }"#;

    /// Token Plan Plus on a boosted day: the week of the `general` lane unmetered (status 3).
    /// Read at `1_780_347_620`.
    const BOOSTED: &str = r#"{
        "model_remains": [
            {"start_time": 1780347600000, "end_time": 1780365600000, "remains_time": 4650822,
             "current_interval_total_count": 0, "current_interval_usage_count": 0,
             "model_name": "general",
             "current_weekly_total_count": 0, "current_weekly_usage_count": 0,
             "weekly_start_time": 1780243200000, "weekly_end_time": 1780848000000,
             "weekly_remains_time": 487050822,
             "current_interval_status": 1, "current_interval_remaining_percent": 99,
             "current_weekly_status": 3, "current_weekly_remaining_percent": 100,
             "interval_boost_permill": 2000, "weekly_boost_permill": 2000},
            {"start_time": 1780329600000, "end_time": 1780416000000, "remains_time": 55050822,
             "current_interval_total_count": 0, "current_interval_usage_count": 0,
             "model_name": "video",
             "current_weekly_total_count": 0, "current_weekly_usage_count": 0,
             "weekly_start_time": 1780243200000, "weekly_end_time": 1780848000000,
             "weekly_remains_time": 487050822,
             "current_interval_status": 3, "current_interval_remaining_percent": 100,
             "current_weekly_status": 3, "current_weekly_remaining_percent": 100}
        ],
        "base_resp": {"status_code": 0, "status_msg": "success"}
    }"#;

    const RATE_LIMITED: &str = "MiniMax is rate limiting usage requests. Waiting before retrying.";

    fn key() -> Value {
        json!({"apiKey": "sk-cp-test-key"})
    }

    fn at_seconds(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).unwrap()
    }

    fn at_millis(millis: i64) -> DateTime<Utc> {
        Utc.timestamp_millis_opt(millis).unwrap()
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, 10, 0, 0).unwrap()
    }

    fn today(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, hour, 0, 0).unwrap()
    }

    fn week_start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 21, 16, 0, 0).unwrap()
    }

    fn week_end() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 28, 16, 0, 0).unwrap()
    }

    fn ms(time: DateTime<Utc>) -> i64 {
        time.timestamp_millis()
    }

    fn urls(http: &Scripted) -> Vec<String> {
        http.requests()
            .into_iter()
            .map(|request| request.url)
            .collect()
    }

    fn progress(line: &MetricLine) -> (&str, f64, Option<DateTime<Utc>>, Option<i64>) {
        let MetricLine::Progress(line) = line else {
            panic!("expected a progress line")
        };
        (
            line.label.as_str(),
            line.used,
            line.resets_at,
            line.period_duration_ms,
        )
    }

    #[tokio::test]
    async fn reads_the_token_plan_windows_and_the_plus_plan() {
        let http = Scripted::new().on("GET", GLOBAL_TOKEN, 200, PLUS);
        let scope = context_at(&http, key(), at_seconds(1_780_282_340));
        let reading = MiniMax.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Plus"));
        assert_eq!(
            reading.lines,
            vec![
                lines::percent(
                    "Session",
                    4.0,
                    Some(at_millis(1_780_297_200_000)),
                    Some(5 * lines::HOUR_MS)
                ),
                lines::percent(
                    "Weekly",
                    1.0,
                    Some(at_millis(1_780_848_000_000)),
                    Some(lines::WEEK_MS)
                ),
            ]
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].url, GLOBAL_TOKEN);
        assert_eq!(
            header(&requests[0], "Authorization"),
            Some("Bearer sk-cp-test-key")
        );
        assert_eq!(header(&requests[0], "Accept"), Some("application/json"));
        assert!(requests[0].body.is_none());
    }

    #[tokio::test]
    async fn each_window_shows_its_tightest_lane_and_a_daily_video_quota_stays_out_of_the_session()
    {
        let body = json!({
            "base_resp": {"status_code": 0, "status_msg": "success"},
            "model_remains": [
                {
                    "model_name": "general",
                    "start_time": ms(today(8)), "end_time": ms(today(13)),
                    "remains_time": 3 * lines::HOUR_MS,
                    "current_interval_total_count": 0, "current_interval_usage_count": 0,
                    "current_interval_status": 1, "current_interval_remaining_percent": 96,
                    "weekly_start_time": ms(week_start()), "weekly_end_time": ms(week_end()),
                    "current_weekly_status": 1, "current_weekly_remaining_percent": 90
                },
                {
                    "model_name": "speech-2.8-hd",
                    "start_time": ms(today(8)), "end_time": ms(today(13)),
                    "current_interval_status": 1, "current_interval_remaining_percent": 70
                },
                {
                    "model_name": "video",
                    "start_time": ms(today(0)),
                    "end_time": ms(today(0) + Duration::days(1)),
                    "current_interval_status": 1, "current_interval_remaining_percent": 30,
                    "weekly_start_time": ms(week_start()), "weekly_end_time": ms(week_end()),
                    "current_weekly_status": 1, "current_weekly_remaining_percent": 80
                }
            ]
        });
        let http = Scripted::new().on("GET", GLOBAL_TOKEN, 200, &body.to_string());
        let scope = context_at(&http, key(), now());
        let reading = MiniMax.fetch(&scope.context()).await.unwrap();
        let five_hours = Some(5 * lines::HOUR_MS);
        let week = Some(lines::WEEK_MS);
        assert_eq!(reading.plan, None);
        assert_eq!(
            reading.lines,
            vec![
                lines::percent("Session", 30.0, Some(today(13)), five_hours),
                lines::percent("Weekly", 20.0, Some(week_end()), week),
                lines::percent("General", 4.0, Some(today(13)), five_hours),
                lines::percent("General Weekly", 10.0, Some(week_end()), week),
                lines::percent("Text to Speech", 30.0, Some(today(13)), five_hours),
                lines::percent(
                    "Video",
                    70.0,
                    Some(today(0) + Duration::days(1)),
                    Some(lines::DAY_MS)
                ),
                lines::percent("Video Weekly", 20.0, Some(week_end()), week),
            ]
        );
    }

    #[tokio::test]
    async fn coding_plan_counts_are_what_is_left() {
        let body = json!({
            "base_resp": {"status_code": 0},
            "current_subscribe_title": "Max",
            "model_remains": [{
                "model_name": "MiniMax-M2",
                "current_interval_total_count": 1000,
                "current_interval_usage_count": 250,
                "start_time": ms(today(8)), "end_time": ms(today(13)),
                "remains_time": 3 * lines::HOUR_MS,
                "current_weekly_total_count": 6000,
                "current_weekly_usage_count": 5376,
                "weekly_start_time": ms(week_start()), "weekly_end_time": ms(week_end())
            }]
        });
        let http = Scripted::new().on("GET", GLOBAL_TOKEN, 404, "{}").on(
            "GET",
            GLOBAL_CODING,
            200,
            &body.to_string(),
        );
        let scope = context_at(&http, key(), now());
        let reading = MiniMax.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Max"));
        assert_eq!(reading.lines.len(), 2);
        assert_eq!(
            progress(&reading.lines[0]),
            ("Session", 75.0, Some(today(13)), Some(5 * lines::HOUR_MS))
        );
        let weekly = progress(&reading.lines[1]);
        assert_eq!(weekly.0, "Weekly");
        assert!((weekly.1 - 10.4).abs() < 1e-9);
        assert_eq!(weekly.2, Some(week_end()));
        assert_eq!(urls(&http), [GLOBAL_TOKEN, GLOBAL_CODING]);
    }

    #[tokio::test]
    async fn token_plan_counts_are_what_was_used() {
        let body = json!({
            "base_resp": {"status_code": 0, "status_msg": "success"},
            "model_remains": [{
                "model_name": "MiniMax-M*",
                "current_interval_total_count": 1500,
                "current_interval_usage_count": 151,
                "remains_time": 2 * lines::HOUR_MS,
                "current_weekly_total_count": 15000,
                "current_weekly_usage_count": 1500,
                "weekly_remains_time": 48 * lines::HOUR_MS
            }]
        });
        let http = Scripted::new().on("GET", GLOBAL_TOKEN, 200, &body.to_string());
        let scope = context_at(&http, key(), now());
        let reading = MiniMax.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan, None);
        assert_eq!(reading.lines.len(), 2);
        let session = progress(&reading.lines[0]);
        assert_eq!(session.0, "Session");
        assert!((session.1 - 151.0 / 15.0).abs() < 1e-9);
        assert_eq!(session.2, Some(now() + Duration::hours(2)));
        assert_eq!(session.3, Some(5 * lines::HOUR_MS));
        let weekly = progress(&reading.lines[1]);
        assert_eq!(weekly.0, "Weekly");
        assert!((weekly.1 - 10.0).abs() < 1e-9);
        assert_eq!(weekly.2, Some(now() + Duration::hours(48)));
        assert_eq!(weekly.3, Some(lines::WEEK_MS));
    }

    #[tokio::test]
    async fn an_unmetered_week_on_the_text_lane_reads_unlimited() {
        let http = Scripted::new().on("GET", GLOBAL_TOKEN, 200, BOOSTED);
        let scope = context_at(&http, key(), at_seconds(1_780_347_620));
        let reading = MiniMax.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Plus"));
        assert_eq!(
            reading.lines,
            vec![
                lines::percent(
                    "Session",
                    1.0,
                    Some(at_millis(1_780_365_600_000)),
                    Some(5 * lines::HOUR_MS)
                ),
                lines::badge("Weekly", "Unlimited"),
            ]
        );
    }

    #[tokio::test]
    async fn camel_case_answers_under_data_name_the_plan() {
        let body = json!({
            "baseResp": {"statusCode": "0", "statusMsg": "success"},
            "data": {
                "currentSubscribeTitle": "Token Plan · TokenPlanPlus-年度会员",
                "modelRemains": [{
                    "modelName": "general",
                    "startTime": ms(today(8)), "endTime": ms(today(13)),
                    "currentIntervalStatus": 1, "currentIntervalRemainingPercent": "96",
                    "weeklyStartTime": ms(week_start()), "weeklyEndTime": ms(week_end()),
                    "currentWeeklyStatus": 1, "currentWeeklyRemainingPercent": "99"
                }]
            }
        });
        let http = Scripted::new().on("GET", GLOBAL_TOKEN, 200, &body.to_string());
        let scope = context_at(&http, key(), now());
        let reading = MiniMax.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Plus"));
        assert_eq!(
            reading.lines,
            vec![
                lines::percent("Session", 4.0, Some(today(13)), Some(5 * lines::HOUR_MS)),
                lines::percent("Weekly", 1.0, Some(week_end()), Some(lines::WEEK_MS)),
            ]
        );
    }

    #[tokio::test]
    async fn a_key_the_global_host_refuses_is_read_from_the_china_host_and_remembered() {
        let http = Scripted::new()
            .on("GET", GLOBAL_TOKEN, 401, "{}")
            .on("GET", GLOBAL_CODING, 401, "{}")
            .on("GET", CHINA_TOKEN, 200, PLUS);
        let scope = context_at(&http, key(), at_seconds(1_780_282_340));
        let reading = MiniMax.fetch(&scope.context()).await.unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Plus"));
        assert_eq!(urls(&http), [GLOBAL_TOKEN, GLOBAL_CODING, CHINA_TOKEN]);
        MiniMax.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            urls(&http),
            [GLOBAL_TOKEN, GLOBAL_CODING, CHINA_TOKEN, CHINA_TOKEN]
        );
    }

    #[tokio::test]
    async fn a_key_every_endpoint_refuses_is_rejected() {
        let http = Scripted::new()
            .on(
                "GET",
                GLOBAL_TOKEN,
                200,
                r#"{"base_resp":{"status_code":1004,"status_msg":"cookie is missing, log in again"}}"#,
            )
            .on("GET", GLOBAL_CODING, 404, "{}")
            .on("GET", CHINA_TOKEN, 403, "{}")
            .on("GET", CHINA_CODING, 401, "{}");
        let scope = context_at(&http, key(), now());
        let error = MiniMax.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert_eq!(error.message, REFUSED);
        assert_eq!(
            urls(&http),
            [GLOBAL_TOKEN, GLOBAL_CODING, CHINA_TOKEN, CHINA_CODING]
        );
    }

    #[tokio::test]
    async fn rate_limits_stop_the_refresh_at_once() {
        let http = Scripted::new().on("GET", GLOBAL_TOKEN, 429, "{}");
        let scope = context_at(&http, key(), now());
        let error = MiniMax.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::RateLimited);
        assert_eq!(error.message, RATE_LIMITED);
        assert_eq!(urls(&http), [GLOBAL_TOKEN]);

        let http = Scripted::new().on(
            "GET",
            GLOBAL_TOKEN,
            200,
            r#"{"base_resp":{"status_code":1002,"status_msg":"rate limit exceeded"}}"#,
        );
        let scope = context_at(&http, key(), now());
        let error = MiniMax.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::RateLimited);
        assert_eq!(error.message, RATE_LIMITED);
        assert_eq!(urls(&http), [GLOBAL_TOKEN]);
    }

    #[tokio::test]
    async fn a_key_without_a_plan_is_not_available_and_the_china_host_is_left_alone() {
        let http = Scripted::new()
            .on(
                "GET",
                GLOBAL_TOKEN,
                200,
                r#"{"model_remains":null,"base_resp":{"status_code":2062,"status_msg":"no active token plan subscription"}}"#,
            )
            .on("GET", GLOBAL_CODING, 404, "{}");
        let scope = context_at(&http, key(), now());
        let error = MiniMax.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
        assert_eq!(error.message, NO_PLAN);
        assert_eq!(urls(&http), [GLOBAL_TOKEN, GLOBAL_CODING]);
    }

    #[tokio::test]
    async fn an_answer_with_only_lanes_the_plan_leaves_out_is_not_available() {
        let body = json!({
            "base_resp": {"status_code": 0},
            "model_remains": [{
                "model_name": "video",
                "start_time": ms(today(0)), "end_time": ms(today(0) + Duration::days(1)),
                "current_interval_total_count": 0, "current_interval_usage_count": 0,
                "current_interval_status": 3, "current_interval_remaining_percent": 100,
                "current_weekly_status": 3, "current_weekly_remaining_percent": 100
            }]
        });
        let http = Scripted::new()
            .on("GET", GLOBAL_TOKEN, 200, &body.to_string())
            .on("GET", GLOBAL_CODING, 404, "{}");
        let scope = context_at(&http, key(), now());
        let error = MiniMax.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
        assert_eq!(error.message, NO_QUOTA);
        assert_eq!(urls(&http), [GLOBAL_TOKEN, GLOBAL_CODING]);
    }

    #[tokio::test]
    async fn a_lone_daily_lane_shows_as_its_own_meter() {
        let body = json!({
            "base_resp": {"status_code": 0},
            "model_remains": [{
                "model_name": "video",
                "start_time": ms(today(0)), "end_time": ms(today(0) + Duration::days(1)),
                "current_interval_status": 1, "current_interval_remaining_percent": 30
            }]
        });
        let http = Scripted::new().on("GET", GLOBAL_TOKEN, 200, &body.to_string());
        let scope = context_at(&http, key(), now());
        let reading = MiniMax.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            reading.lines,
            vec![lines::percent(
                "Video",
                70.0,
                Some(today(0) + Duration::days(1)),
                Some(lines::DAY_MS)
            )]
        );
    }

    #[tokio::test]
    async fn a_remembered_endpoint_reports_a_passing_failure_without_trying_others() {
        let http =
            Scripted::new()
                .on("GET", GLOBAL_TOKEN, 200, PLUS)
                .on("GET", GLOBAL_TOKEN, 503, "");
        let scope = context_at(&http, key(), at_seconds(1_780_282_340));
        MiniMax.fetch(&scope.context()).await.unwrap();
        let error = MiniMax.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Http5xx);
        assert_eq!(error.message, "MiniMax answered with HTTP 503.");
        assert_eq!(urls(&http), [GLOBAL_TOKEN, GLOBAL_TOKEN]);
    }

    #[tokio::test]
    async fn error_codes_in_the_body_are_reported_with_their_number() {
        let http = Scripted::new().on("GET", GLOBAL_TOKEN, 200, PLUS).on(
            "GET",
            GLOBAL_TOKEN,
            200,
            r#"{"base_resp":{"status_code":1024,"status_msg":"internal error"}}"#,
        );
        let scope = context_at(&http, key(), at_seconds(1_780_282_340));
        MiniMax.fetch(&scope.context()).await.unwrap();
        let error = MiniMax.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Http5xx);
        assert_eq!(
            error.message,
            "MiniMax could not report usage right now (error 1024)."
        );
        assert_eq!(urls(&http), [GLOBAL_TOKEN, GLOBAL_TOKEN]);

        let http = Scripted::new()
            .on(
                "GET",
                GLOBAL_TOKEN,
                200,
                r#"{"base_resp":{"status_code":1008,"status_msg":"insufficient balance"}}"#,
            )
            .on("GET", GLOBAL_CODING, 404, "{}");
        let scope = context_at(&http, key(), now());
        let error = MiniMax.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::NotAvailable);
        assert_eq!(error.message, "MiniMax did not return usage (error 1008).");
        assert_eq!(urls(&http), [GLOBAL_TOKEN, GLOBAL_CODING]);
    }

    #[tokio::test]
    async fn a_remembered_endpoint_that_refuses_the_key_is_forgotten() {
        let http = Scripted::new()
            .on("GET", GLOBAL_TOKEN, 200, PLUS)
            .on("GET", GLOBAL_TOKEN, 401, "{}")
            .on("GET", GLOBAL_CODING, 200, PLUS);
        let scope = context_at(&http, key(), at_seconds(1_780_282_340));
        MiniMax.fetch(&scope.context()).await.unwrap();
        MiniMax.fetch(&scope.context()).await.unwrap();
        MiniMax.fetch(&scope.context()).await.unwrap();
        assert_eq!(
            urls(&http),
            [GLOBAL_TOKEN, GLOBAL_TOKEN, GLOBAL_CODING, GLOBAL_CODING]
        );
    }

    #[tokio::test]
    async fn a_secret_without_a_key_sends_nothing() {
        let http = Scripted::new();
        let scope = context_at(&http, json!({}), now());
        let error = MiniMax.fetch(&scope.context()).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::AuthInvalid);
        assert!(http.requests().is_empty());
    }

    #[test]
    fn plan_titles_read_as_their_tier() {
        let cases = [
            ("Token Plan · TokenPlanMax-年度会员", "Max"),
            ("TokenPlanUltra-年度会员", "Ultra"),
            ("TokenPlanPlus年度会员", "Plus"),
            ("MiniMax Token Plan Plus", "Plus"),
            ("Coding Plan Pro", "Pro"),
            (" Enterprise Pack ", "Enterprise Pack"),
        ];
        for (title, expected) in cases {
            assert_eq!(format_plan(title).as_deref(), Some(expected), "{title}");
        }
        assert_eq!(format_plan("  "), None);
    }

    #[test]
    fn lanes_read_as_the_dashboard_names_them() {
        let cases = [
            ("general", "General"),
            ("video", "Video"),
            ("MiniMax-M2.7", "Text Generation"),
            ("speech-2.8-hd", "Text to Speech"),
            ("MiniMax-Hailuo-2.3-Fast", "Image to Video"),
            ("MiniMax-Hailuo-2.3", "Text to Video"),
            ("image-01", "Image Generation"),
            ("music-2.6", "Music Generation"),
            ("lyrics_generation-v2", "Lyrics Generation V2"),
        ];
        for (name, expected) in cases {
            assert_eq!(lane_label(name).as_deref(), Some(expected), "{name}");
        }
        assert_eq!(lane_label(""), None);
    }

    #[test]
    fn the_key_comes_from_accounts_or_minimax_api_key_and_nothing_is_read_from_disk() {
        let connection = MiniMax.connection();
        assert!(connection.login_from.is_none());
        let help = connection.api_key.unwrap();
        assert_eq!(help.env, ["MINIMAX_API_KEY"]);
        assert_eq!(
            help.url,
            "https://platform.minimax.io/user-center/basic-information/interface-key"
        );
        assert!(help.fields.is_empty());
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::under(dir.path()).with_var("MINIMAX_API_KEY", "sk-cp-test-key");
        assert!(MiniMax.discover(&roots).is_empty());
    }

    #[test]
    fn the_card_declares_session_and_weekly_meters() {
        let provider = Provider::new("minimax@abc", "MiniMax");
        let descriptors = MiniMax.descriptors(&provider);
        let ids: Vec<&str> = descriptors.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["minimax@abc.session", "minimax@abc.weekly"]);
        let labels: Vec<&str> = descriptors
            .iter()
            .map(|d| d.metric_label.as_str())
            .collect();
        assert_eq!(labels, ["Session", "Weekly"]);
        let exports: Vec<&str> = descriptors
            .iter()
            .flat_map(|d| d.limit_resources.iter().map(|r| r.key.as_str()))
            .collect();
        assert_eq!(exports, ["session", "weekly"]);
    }
}
