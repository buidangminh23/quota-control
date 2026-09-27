//! The browser half of a desktop sign-in: a listener on this computer's loopback interface that
//! receives an OAuth 2 authorization redirect, as the providers' own CLIs do, and answers the tab
//! with a small page saying how the sign-in ended. Nobody copies a code by hand.
//!
//! The listener only answers its own redirect path. A request with the wrong `state` gets an
//! "inactive" page and does not end the sign-in; the code itself is checked again with
//! [`validate`] before it is exchanged.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::{ErrorCategory, SimpleProviderError};

/// How long the browser tab waits for the account to be saved before it shows an interim page.
const PAGE_WAIT: Duration = Duration::from_secs(20);
const REQUEST_LIMIT: usize = 16_384;
const LOGIN_CANCELLED: &str = "This login was cancelled.";
const LOGIN_EXPIRED: &str = "This login has expired. Start again.";

fn auth_error(message: &str) -> SimpleProviderError {
    SimpleProviderError::new(ErrorCategory::AuthInvalid, message)
}

/// The error a sign-in ends with when it was cancelled or replaced by a newer one.
pub fn cancelled() -> SimpleProviderError {
    auth_error(LOGIN_CANCELLED)
}

/// The error a sign-in ends with when nobody finished it in the browser in time.
pub fn expired() -> SimpleProviderError {
    auth_error(LOGIN_EXPIRED)
}

/// True for the error a sign-in ends with when it was cancelled or replaced by a newer one.
pub fn is_cancelled(error: &SimpleProviderError) -> bool {
    error.message == LOGIN_CANCELLED
}

/// True for the error a sign-in ends with when nobody finished it in the browser in time.
pub fn is_expired(error: &SimpleProviderError) -> bool {
    error.message == LOGIN_EXPIRED
}

fn unavailable() -> SimpleProviderError {
    auth_error("Cannot start the local login callback.")
}

/// The language of the page the browser shows once it comes back to this computer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoginLanguage {
    #[default]
    English,
    Vietnamese,
}

impl LoginLanguage {
    pub fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("vi") {
            Self::Vietnamese
        } else {
            Self::English
        }
    }
}

/// The button in Quota Control that starts a new sign-in, which the result page points back to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignInAction {
    Google,
    GitHub,
}

impl SignInAction {
    fn label(self, language: LoginLanguage) -> &'static str {
        match (self, language) {
            (Self::Google, LoginLanguage::Vietnamese) => "Đăng nhập bằng Google",
            (Self::Google, LoginLanguage::English) => "Sign in with Google",
            (Self::GitHub, LoginLanguage::Vietnamese) => "Đăng nhập bằng GitHub",
            (Self::GitHub, LoginLanguage::English) => "Sign in with GitHub",
        }
    }
}

/// What the result page names: the provider signed in to, and the button that starts over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageBrand {
    pub product: String,
    pub action: SignInAction,
}

impl PageBrand {
    pub fn new(product: impl Into<String>, action: SignInAction) -> Self {
        Self {
            product: product.into(),
            action,
        }
    }
}

/// The browser came back with this sign-in's redirect. `page` receives how the sign-in ended,
/// which the tab then shows.
pub struct Callback {
    pub url: String,
    pub page: oneshot::Sender<LoginPage>,
}

pub type CallbackResult = Result<Callback, SimpleProviderError>;

/// Listeners for `http://localhost:<port>`: `localhost` resolves to both loopback addresses, so the
/// same port is taken on each when this computer has IPv6. Port 0 picks a free one.
pub async fn bind_localhost(port: u16) -> Result<Vec<TcpListener>, SimpleProviderError> {
    for _ in 0..8 {
        let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|_| unavailable())?;
        let bound = v4.local_addr().map_err(|_| unavailable())?.port();
        match TcpListener::bind((Ipv6Addr::LOCALHOST, bound)).await {
            Ok(v6) => return Ok(vec![v4, v6]),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse && port == 0 => continue,
            Err(_) => return Ok(vec![v4]),
        }
    }
    Err(unavailable())
}

/// A listener on `127.0.0.1` at the first of `ports` that is free, for clients registered with
/// fixed callback ports.
pub async fn bind_ipv4(ports: &[u16]) -> Result<TcpListener, SimpleProviderError> {
    for port in ports {
        if let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, *port)).await {
            return Ok(listener);
        }
    }
    Err(auth_error(
        "The login callback ports are in use. Finish the other login and try again.",
    ))
}

/// The port a listener took.
pub fn port(listener: &TcpListener) -> Result<u16, SimpleProviderError> {
    Ok(listener.local_addr().map_err(|_| unavailable())?.port())
}

/// The authorization code of a redirect that came back to `redirect` with `expected_state`.
pub fn validate(
    raw: &str,
    redirect: &str,
    expected_state: &str,
) -> Result<String, SimpleProviderError> {
    let url = Url::parse(raw.trim()).map_err(|_| auth_error("The callback URL is invalid."))?;
    let expected = Url::parse(redirect).map_err(|_| auth_error("The callback URL is invalid."))?;
    if url.origin() != expected.origin()
        || url.path() != expected.path()
        || url.fragment().is_some()
    {
        return Err(auth_error(
            "The callback belongs to a different login endpoint.",
        ));
    }
    let params: Vec<_> = url.query_pairs().collect();
    if params.iter().any(|(key, _)| key == "error") {
        return Err(auth_error("The browser login was not authorized."));
    }
    let codes: Vec<_> = params.iter().filter(|(key, _)| key == "code").collect();
    let states: Vec<_> = params.iter().filter(|(key, _)| key == "state").collect();
    if codes.len() != 1 || states.len() != 1 {
        return Err(auth_error("The callback is missing its code or state."));
    }
    let code = codes[0].1.to_string();
    if states[0].1 != expected_state {
        return Err(auth_error("The callback state does not match this login."));
    }
    if code.is_empty() || code.len() > 8192 || code.chars().any(char::is_control) {
        return Err(auth_error("The authorization code is invalid."));
    }
    Ok(code)
}

/// What one sign-in's listener waits for.
pub struct CallbackContext {
    pub redirect: Url,
    pub state: String,
    pub brand: PageBrand,
    pub language: LoginLanguage,
}

#[derive(Debug, PartialEq, Eq)]
enum Arrival {
    Callback,
    Denied,
    Foreign,
    Elsewhere,
}

impl CallbackContext {
    /// What a request target means for this sign-in, judged on its path and state alone; the code
    /// itself is checked again before it is exchanged.
    fn classify(&self, target: &str) -> Arrival {
        let Some(query) = target
            .strip_prefix(self.redirect.path())
            .and_then(|rest| rest.strip_prefix('?'))
        else {
            return Arrival::Elsewhere;
        };
        let params: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        let states: Vec<_> = params.iter().filter(|(key, _)| key == "state").collect();
        if states.len() != 1 || states[0].1 != self.state {
            return Arrival::Foreign;
        }
        if params.iter().any(|(key, _)| key == "error") {
            return Arrival::Denied;
        }
        Arrival::Callback
    }
}

/// Accept connections until one carries this sign-in's callback, then stop listening. Each
/// connection is answered on its own task, so a browser that opens a spare connection first cannot
/// hold up the real one.
pub async fn serve(listeners: Vec<TcpListener>, context: CallbackContext) -> CallbackResult {
    let Some(first) = listeners.first() else {
        return Err(unavailable());
    };
    let context = Arc::new(context);
    let (sender, mut receiver) = mpsc::channel(1);
    loop {
        let accepted = tokio::select! {
            result = receiver.recv() => return result.unwrap_or_else(|| Err(unavailable())),
            accepted = first.accept() => accepted,
            accepted = async {
                match listeners.get(1) {
                    Some(listener) => listener.accept().await,
                    None => std::future::pending().await,
                }
            } => accepted,
        };
        let (socket, address) = accepted.map_err(|_| unavailable())?;
        if address.ip().is_loopback() {
            tokio::spawn(answer(socket, context.clone(), sender.clone()));
        }
    }
}

async fn answer(
    mut socket: TcpStream,
    context: Arc<CallbackContext>,
    sender: mpsc::Sender<CallbackResult>,
) {
    let Some(target) = read_target(&mut socket).await else {
        return;
    };
    let page = match context.classify(&target) {
        Arrival::Elsewhere => {
            respond(&mut socket, "404 Not Found", "").await;
            return;
        }
        Arrival::Foreign => LoginPage::Inactive,
        Arrival::Denied => {
            let _ = sender.try_send(Err(auth_error("The browser login was not authorized.")));
            LoginPage::Denied
        }
        Arrival::Callback => {
            let (page, outcome) = oneshot::channel();
            let url = format!(
                "{}{target}",
                context.redirect.origin().ascii_serialization()
            );
            if sender.try_send(Ok(Callback { url, page })).is_err() {
                LoginPage::Inactive
            } else {
                match tokio::time::timeout(PAGE_WAIT, outcome).await {
                    Ok(Ok(page)) => page,
                    Ok(Err(_)) => LoginPage::Cancelled,
                    Err(_) => LoginPage::Pending,
                }
            }
        }
    };
    let status = if page == LoginPage::Inactive {
        "400 Bad Request"
    } else {
        "200 OK"
    };
    respond(
        &mut socket,
        status,
        &page.html(&context.brand, context.language),
    )
    .await;
}

/// The target of a `GET` request, once its head has arrived.
async fn read_target(socket: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), async {
        while buffer.len() < REQUEST_LIMIT && !buffer.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut chunk = [0; 1024];
            let size = socket.read(&mut chunk).await?;
            if size == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..size]);
        }
        Ok::<_, std::io::Error>(())
    })
    .await;
    if !matches!(read, Ok(Ok(()))) {
        return None;
    }
    let head = String::from_utf8_lossy(&buffer);
    let mut parts = head.lines().next()?.split_whitespace();
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("GET"), Some(target), Some(version), None)
            if version.starts_with("HTTP/1.") && target.starts_with('/') =>
        {
            Some(target.to_owned())
        }
        _ => None,
    }
}

async fn respond(socket: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        socket.write_all(response.as_bytes()).await?;
        socket.shutdown().await
    })
    .await;
}

/// The page the browser tab shows once the sign-in reaches this computer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginPage {
    Connected,
    Failed,
    Denied,
    Pending,
    Cancelled,
    Inactive,
}

impl LoginPage {
    fn text(self, brand: &PageBrand, language: LoginLanguage) -> (String, String) {
        use LoginLanguage::{English, Vietnamese};
        let product = &brand.product;
        let action = brand.action.label(language);
        match (self, language) {
            (Self::Connected, Vietnamese) => (
                format!("Đã kết nối {product}"),
                "Quota Control đã lưu tài khoản này và sẽ hiện hạn mức sau vài giây. Có thể đóng tab này.".into(),
            ),
            (Self::Connected, English) => (
                format!("{product} is connected"),
                "Quota Control saved this account and will show its limits in a few seconds. You can close this tab.".into(),
            ),
            (Self::Failed, Vietnamese) => (
                format!("Chưa kết nối được {product}"),
                "Quota Control không hoàn tất được lần đăng nhập này. Mở Quota Control để xem lý do rồi thử lại.".into(),
            ),
            (Self::Failed, English) => (
                format!("{product} was not connected"),
                "Quota Control could not finish this sign-in. Open Quota Control to see why, then try again.".into(),
            ),
            (Self::Denied, Vietnamese) => (
                "Chưa cấp quyền cho Quota Control".into(),
                format!("Lần này chưa được cho phép. Muốn thử lại, bấm {action} trong Quota Control."),
            ),
            (Self::Denied, English) => (
                "Access was not granted".into(),
                format!("To try again, choose {action} in Quota Control."),
            ),
            (Self::Pending, Vietnamese) => (
                format!("Đang hoàn tất đăng nhập {product}"),
                "Quay lại Quota Control để xem kết quả. Có thể đóng tab này.".into(),
            ),
            (Self::Pending, English) => (
                format!("Finishing the {product} sign-in"),
                "Return to Quota Control to see the result. You can close this tab.".into(),
            ),
            (Self::Cancelled, Vietnamese) => (
                "Lần đăng nhập này đã bị hủy".into(),
                format!("Quota Control đã dừng lần đăng nhập này. Bấm {action} để bắt đầu lại."),
            ),
            (Self::Cancelled, English) => (
                "This sign-in was cancelled".into(),
                format!("Quota Control stopped this sign-in. Choose {action} to start again."),
            ),
            (Self::Inactive, Vietnamese) => (
                "Lần đăng nhập này không còn hiệu lực".into(),
                format!("Bấm {action} trong Quota Control để bắt đầu lại."),
            ),
            (Self::Inactive, English) => (
                "This sign-in is no longer active".into(),
                format!("Choose {action} in Quota Control to start again."),
            ),
        }
    }

    pub fn html(self, brand: &PageBrand, language: LoginLanguage) -> String {
        let (title, message) = self.text(brand, language);
        let lang = match language {
            LoginLanguage::Vietnamese => "vi",
            LoginLanguage::English => "en",
        };
        let accent = match self {
            Self::Connected => "#1f9d55",
            Self::Pending => "#c98a00",
            _ => "#d14343",
        };
        let title = escape(&title);
        let message = escape(&message);
        format!(
            r#"<!doctype html><html lang="{lang}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>{title} · Quota Control</title><style>:root{{color-scheme:light dark;--bg:#f4f5f7;--card:#fff;--text:#1d1f23;--muted:#5b626d}}@media (prefers-color-scheme:dark){{:root{{--bg:#16181c;--card:#212429;--text:#eceef1;--muted:#a5abb4}}}}body{{margin:0;min-height:100vh;display:grid;place-items:center;background:var(--bg);color:var(--text);font:16px/1.5 system-ui,"Segoe UI",Roboto,sans-serif}}main{{box-sizing:border-box;width:min(440px,calc(100vw - 32px));padding:32px;background:var(--card);border-radius:16px;box-shadow:0 1px 3px rgba(0,0,0,.14)}}.app{{margin:0 0 8px;font-size:13px;font-weight:600;color:var(--muted)}}h1{{margin:0 0 12px;font-size:22px;line-height:1.3}}h1::before{{content:"";display:inline-block;width:10px;height:10px;margin-right:12px;border-radius:50%;background:{accent};vertical-align:middle}}p{{margin:0;color:var(--muted)}}</style></head><body><main><p class="app">Quota Control</p><h1>{title}</h1><p>{message}</p></main></body></html>"#
        )
    }
}

/// Text placed in the page as it is: provider names are this app's own, but nothing that reaches
/// HTML should be able to open a tag.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> CallbackContext {
        CallbackContext {
            redirect: Url::parse("http://localhost:4545/callback").unwrap(),
            state: "expected".into(),
            brand: PageBrand::new("Claude", SignInAction::Google),
            language: LoginLanguage::English,
        }
    }

    #[test]
    fn callbacks_reject_conflicting_state_and_wrong_origins() {
        let redirect = "http://localhost:4545/callback";
        for callback in [
            "http://attacker.invalid/callback?code=fixture&state=expected",
            "http://localhost:4546/callback?code=fixture&state=expected",
            "http://localhost:4545/other?code=fixture&state=expected",
            "http://localhost:4545/callback?code=fixture&state=expected&state=other",
            "http://localhost:4545/callback?code=fixture&state=other",
            "http://localhost:4545/callback?error=access_denied&state=expected",
            "fixture#expected",
        ] {
            assert!(validate(callback, redirect, "expected").is_err());
        }
        assert_eq!(
            validate(
                "http://localhost:4545/callback?code=fixture&state=expected",
                redirect,
                "expected"
            )
            .unwrap(),
            "fixture"
        );
    }

    #[test]
    fn listener_answers_only_its_own_path_and_state() {
        let context = context();
        assert_eq!(context.classify("/favicon.ico"), Arrival::Elsewhere);
        assert_eq!(
            context.classify("/callbacks?code=a&state=expected"),
            Arrival::Elsewhere
        );
        assert_eq!(
            context.classify("/callback?code=a&state=other"),
            Arrival::Foreign
        );
        assert_eq!(
            context.classify("/callback?error=access_denied&state=expected"),
            Arrival::Denied
        );
        assert_eq!(
            context.classify("/callback?code=a&state=expected"),
            Arrival::Callback
        );
    }

    #[test]
    fn result_pages_follow_the_language_and_the_action() {
        let claude = PageBrand::new("Claude", SignInAction::Google);
        let english = LoginPage::Connected.html(&claude, LoginLanguage::English);
        assert!(english.contains("<html lang=\"en\">") && english.contains("Claude is connected"));
        let codex = PageBrand::new("Codex", SignInAction::Google);
        let vietnamese = LoginPage::Denied.html(&codex, LoginLanguage::parse("vi"));
        assert!(vietnamese.contains("<html lang=\"vi\">") && vietnamese.contains("Chưa cấp quyền"));
        assert!(vietnamese.contains("bấm Đăng nhập bằng Google trong Quota Control"));
        let kiro = PageBrand::new("Kiro", SignInAction::GitHub);
        let inactive = LoginPage::Inactive.html(&kiro, LoginLanguage::English);
        assert!(inactive.contains("Choose Sign in with GitHub in Quota Control"));
    }

    #[test]
    fn page_text_cannot_open_a_tag() {
        let brand = PageBrand::new("<b>x</b>", SignInAction::Google);
        let page = LoginPage::Connected.html(&brand, LoginLanguage::English);
        assert!(page.contains("&lt;b&gt;x&lt;/b&gt; is connected"));
        assert!(!page.contains("<b>x</b>"));
    }

    #[tokio::test]
    async fn a_fixed_port_list_takes_the_first_free_port() {
        let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let busy = taken.local_addr().unwrap().port();
        let free = {
            let probe = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            probe.local_addr().unwrap().port()
        };
        let listener = bind_ipv4(&[busy, free]).await.unwrap();
        assert_eq!(port(&listener).unwrap(), free);
        assert!(bind_ipv4(&[busy]).await.is_err());
    }
}
