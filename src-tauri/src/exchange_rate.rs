use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uc_core::{HttpRequest, ReqwestHttpClient, SharedHttpClient};

const ENDPOINT: &str = "https://portal.vietcombank.com.vn/Usercontrols/TVPortal.TyGia/pXML.aspx";
const REFRESH_HOURS: i64 = 6;
const MAX_XML_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeRate {
    pub usd_to_vnd: f64,
    pub published_at: String,
    pub fetched_at: String,
    pub stale: bool,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredRate {
    rate: Option<ExchangeRate>,
    attempted_at: Option<DateTime<Utc>>,
    failed: bool,
}

pub struct ExchangeRateStore {
    path: PathBuf,
    state: Mutex<StoredRate>,
    http: SharedHttpClient,
    clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

impl ExchangeRateStore {
    pub fn new(root: PathBuf) -> Self {
        let path = root.join("exchange-rate.json");
        let mut state: StoredRate = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        if state.rate.as_ref().is_some_and(|rate| {
            !valid_rate(rate.usd_to_vnd)
                || DateTime::parse_from_rfc3339(&rate.published_at).is_err()
                || DateTime::parse_from_rfc3339(&rate.fetched_at).is_err()
        }) {
            state.rate = None;
        }
        Self {
            path,
            state: Mutex::new(state),
            http: ReqwestHttpClient::shared(),
            clock: Arc::new(Utc::now),
        }
    }

    #[cfg(test)]
    pub fn with_http(mut self, http: SharedHttpClient) -> Self {
        self.http = http;
        self
    }

    #[cfg(test)]
    pub fn with_clock(mut self, clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>) -> Self {
        self.clock = clock;
        self
    }

    pub async fn get(&self) -> Option<ExchangeRate> {
        let mut state = self.state.lock().await;
        let now = (self.clock)();
        let interval = chrono::Duration::hours(REFRESH_HOURS);
        if state
            .attempted_at
            .is_some_and(|last| now.signed_duration_since(last) < interval)
        {
            return cached_rate(&state, now);
        }
        state.attempted_at = Some(now);
        state.failed = true;
        if !self.persist(&state) {
            return cached_rate(&state, now);
        }
        let timeout = Duration::from_secs(20);
        let request = HttpRequest::get(ENDPOINT)
            .timeout(timeout)
            .max_response_bytes(MAX_XML_BYTES);
        if let Ok(Ok(response)) = tokio::time::timeout(timeout, self.http.send(request)).await
            && response.is_success()
            && let Some((usd_to_vnd, published_at)) = parse_rate(&response.body)
        {
            let previous = state.rate.clone();
            state.rate = Some(ExchangeRate {
                usd_to_vnd,
                published_at,
                fetched_at: (self.clock)().to_rfc3339(),
                stale: false,
            });
            state.failed = false;
            if !self.persist(&state) {
                state.rate = previous;
                state.failed = true;
            }
        }
        cached_rate(&state, (self.clock)())
    }

    fn persist(&self, state: &StoredRate) -> bool {
        serde_json::to_vec(state)
            .ok()
            .is_some_and(|bytes| uc_core::paths::write_atomic(&self.path, &bytes).is_ok())
    }
}

fn cached_rate(state: &StoredRate, now: DateTime<Utc>) -> Option<ExchangeRate> {
    let mut rate = state.rate.clone()?;
    let fetched_at = DateTime::parse_from_rfc3339(&rate.fetched_at).ok()?;
    rate.stale = state.failed
        || now.signed_duration_since(fetched_at) >= chrono::Duration::hours(REFRESH_HOURS);
    Some(rate)
}

fn valid_rate(value: f64) -> bool {
    value.is_finite() && (10_000.0..=100_000.0).contains(&value)
}

fn parse_rate(bytes: &[u8]) -> Option<(f64, String)> {
    if bytes.len() > MAX_XML_BYTES {
        return None;
    }
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut published = None;
    let mut rate = None;
    let mut depth = 0usize;
    let mut root_seen = false;
    loop {
        let event = reader.read_event().ok()?;
        let opening = matches!(event, Event::Start(_));
        match event {
            Event::Start(element) if element.name().as_ref() == b"DateTime" => {
                if published.is_some() || depth != 1 {
                    return None;
                }
                let text = reader.read_text(element.name()).ok()?;
                let text = text.decode().ok()?;
                let date =
                    NaiveDateTime::parse_from_str(text.trim(), "%m/%d/%Y %I:%M:%S %p").ok()?;
                published = Some(
                    FixedOffset::east_opt(7 * 3600)?
                        .from_local_datetime(&date)
                        .single()?
                        .to_rfc3339(),
                );
            }
            Event::Start(element) | Event::Empty(element)
                if element.name().as_ref() == b"Exrate" =>
            {
                if depth != 1 {
                    return None;
                }
                if opening {
                    depth += 1;
                }
                let mut code = None;
                let mut sell = None;
                for attribute in element.attributes() {
                    let attribute = attribute.ok()?;
                    match attribute.key.as_ref() {
                        b"CurrencyCode" => {
                            code = Some(
                                attribute
                                    .decoded_and_normalized_value(
                                        XmlVersion::Explicit1_0,
                                        reader.decoder(),
                                    )
                                    .ok()?
                                    .into_owned(),
                            );
                        }
                        b"Sell" => {
                            sell = Some(
                                attribute
                                    .decoded_and_normalized_value(
                                        XmlVersion::Explicit1_0,
                                        reader.decoder(),
                                    )
                                    .ok()?
                                    .into_owned(),
                            );
                        }
                        _ => {}
                    }
                }
                if code.as_deref() == Some("USD") {
                    if rate.is_some() {
                        return None;
                    }
                    let value = sell?.replace(',', "").parse::<f64>().ok()?;
                    if !valid_rate(value) {
                        return None;
                    }
                    rate = Some(value);
                }
            }
            Event::Start(element) => {
                if depth == 0 {
                    if root_seen || element.name().as_ref() != b"ExrateList" {
                        return None;
                    }
                    root_seen = true;
                }
                depth += 1;
            }
            Event::End(_) => depth = depth.checked_sub(1)?,
            Event::Empty(_) if depth == 0 => return None,
            Event::Text(text) if depth == 0 && !text.is_empty() => return None,
            Event::DocType(_) => return None,
            Event::Eof if depth == 0 && root_seen => break,
            Event::Eof => return None,
            _ => {}
        }
    }
    Some((rate?, published?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
    use uc_core::{HttpClient, HttpError, HttpResponse};

    const XML: &str = r#"<?xml version="1.0"?><ExrateList><DateTime>9/26/2026 4:07:07 PM</DateTime><Exrate CurrencyCode="EUR" Sell="30,000.00"/><Exrate Sell="26,170.00" CurrencyCode="USD"/></ExrateList>"#;

    struct MockHttp {
        responses: std::sync::Mutex<VecDeque<Result<HttpResponse, HttpError>>>,
        calls: AtomicUsize,
    }

    impl MockHttp {
        fn new(responses: Vec<Result<HttpResponse, HttpError>>) -> Arc<Self> {
            Arc::new(Self {
                responses: std::sync::Mutex::new(responses.into()),
                calls: AtomicUsize::new(0),
            })
        }
    }

    #[async_trait::async_trait]
    impl HttpClient for MockHttp {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
            assert_eq!(request.method, "GET");
            assert_eq!(request.url, ENDPOINT);
            assert_eq!(request.max_response_bytes, Some(MAX_XML_BYTES));
            assert!(request.headers.is_empty());
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            self.responses.lock().unwrap().pop_front().unwrap()
        }
    }

    fn success() -> Result<HttpResponse, HttpError> {
        Ok(HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: XML.as_bytes().to_vec(),
        })
    }

    fn fixture(
        root: &std::path::Path,
        http: Arc<MockHttp>,
        seconds: Arc<AtomicI64>,
    ) -> ExchangeRateStore {
        ExchangeRateStore::new(root.to_path_buf())
            .with_http(http)
            .with_clock(Arc::new(move || {
                DateTime::from_timestamp(1_790_400_000 + seconds.load(Ordering::SeqCst), 0).unwrap()
            }))
    }

    #[test]
    fn parses_usd_sell_and_vietnam_publication_time() {
        assert_eq!(
            parse_rate(XML.as_bytes()),
            Some((26_170.0, "2026-09-26T16:07:07+07:00".into()))
        );
        assert_eq!(
            parse_rate(br#"<ExrateList><Exrate CurrencyCode="USD" Sell="26,170.00"></Exrate><DateTime>9/26/2026 4:07:07 PM</DateTime></ExrateList>"#),
            parse_rate(XML.as_bytes())
        );
        assert!(parse_rate(XML.replace("USD", "GBP").as_bytes()).is_none());
        for invalid in ["NaN", "9,999.00", "100,001.00", "bad"] {
            assert!(parse_rate(XML.replace("26,170.00", invalid).as_bytes()).is_none());
        }
        assert!(parse_rate(XML.replace("9/26/2026", "2/30/2026").as_bytes()).is_none());
        assert!(parse_rate(XML.replace("</ExrateList>", "</wrong>").as_bytes()).is_none());
        assert!(parse_rate(XML.replace("</ExrateList>", "").as_bytes()).is_none());
        assert!(parse_rate(XML.replace("Sell=", "Missing=").as_bytes()).is_none());
        assert!(parse_rate(&vec![b' '; MAX_XML_BYTES + 1]).is_none());
    }

    #[tokio::test]
    async fn persists_success_and_throttles_concurrent_calls_and_restarts() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let http = MockHttp::new(vec![success()]);
        let store = fixture(root.path(), http.clone(), seconds.clone());
        let (first, second) = tokio::join!(store.get(), store.get());
        let first = first.unwrap();
        assert_eq!(first.usd_to_vnd, 26_170.0);
        assert!(!first.stale);
        assert_eq!(second, Some(first.clone()));
        assert_eq!(http.calls.load(Ordering::SeqCst), 1);
        let reopened = fixture(root.path(), http.clone(), seconds);
        assert_eq!(reopened.get().await, Some(first));
        assert_eq!(http.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn refreshes_at_six_hours_and_persists_failure_throttle() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let http = MockHttp::new(vec![success(), Err(HttpError::Timeout), success()]);
        let store = fixture(root.path(), http.clone(), seconds.clone());
        let first = store.get().await.unwrap();
        seconds.store(21_599, Ordering::SeqCst);
        assert!(!store.get().await.unwrap().stale);
        assert_eq!(http.calls.load(Ordering::SeqCst), 1);
        seconds.store(21_600, Ordering::SeqCst);
        let failed = store.get().await.unwrap();
        assert!(failed.stale);
        assert_eq!(failed.fetched_at, first.fetched_at);
        let reopened = fixture(root.path(), http.clone(), seconds.clone());
        assert!(reopened.get().await.unwrap().stale);
        assert_eq!(http.calls.load(Ordering::SeqCst), 2);
        seconds.store(43_200, Ordering::SeqCst);
        let recovered = reopened.get().await.unwrap();
        assert!(!recovered.stale);
        assert_ne!(recovered.fetched_at, first.fetched_at);
        assert_eq!(http.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn never_successful_returns_none_and_throttles_across_restart() {
        let root = tempfile::tempdir().unwrap();
        let seconds = Arc::new(AtomicI64::new(0));
        let http = MockHttp::new(vec![Err(HttpError::Timeout)]);
        assert!(
            fixture(root.path(), http.clone(), seconds.clone())
                .get()
                .await
                .is_none()
        );
        assert!(
            fixture(root.path(), http.clone(), seconds)
                .get()
                .await
                .is_none()
        );
        assert_eq!(http.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn invalid_xml_and_http_failure_keep_last_good_rate() {
        for response in [
            HttpResponse {
                status: 503,
                headers: HashMap::new(),
                body: XML.as_bytes().to_vec(),
            },
            HttpResponse {
                status: 200,
                headers: HashMap::new(),
                body: b"invalid".to_vec(),
            },
        ] {
            let root = tempfile::tempdir().unwrap();
            let seconds = Arc::new(AtomicI64::new(0));
            let http = MockHttp::new(vec![success(), Ok(response)]);
            let store = fixture(root.path(), http, seconds.clone());
            let first = store.get().await.unwrap();
            seconds.store(21_600, Ordering::SeqCst);
            let last = store.get().await.unwrap();
            assert!(last.stale);
            assert_eq!(last.usd_to_vnd, first.usd_to_vnd);
            assert_eq!(last.fetched_at, first.fetched_at);
        }
    }
}
