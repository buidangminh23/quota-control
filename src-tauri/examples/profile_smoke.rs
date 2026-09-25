use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use serde_json::{Value, json};
use tauri::webview::{NewWindowFeatures, NewWindowResponse};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const SCRIPT: &str = r#"
const phase = location.pathname.slice(1);
const read = () => ({storage: localStorage.getItem('profile-smoke'), cookie: document.cookie.split('; ').find(v => v.startsWith('profile-smoke='))?.split('=')[1] ?? null});
const put = value => { localStorage.setItem('profile-smoke', value); document.cookie = `profile-smoke=${value}; Path=/; Max-Age=3600; SameSite=Lax; Secure`; };
const report = checks => { document.title = 'profile-smoke:' + JSON.stringify({phase, ...checks}); };
try {
  if (phase === 'a-write') {
    const initial = read(); put('A'); const after = read();
    report({fresh: initial.storage === null && initial.cookie === null, storage: after.storage === 'A', cookie: after.cookie === 'A'});
  } else if (phase === 'b-write') {
    const initial = read(); put('B'); const after = read();
    report({isolated: initial.storage === null && initial.cookie === null, storage: after.storage === 'B', cookie: after.cookie === 'B'});
  } else if (phase === 'a-reopen') {
    const after = read(); report({storage: after.storage === 'A', cookie: after.cookie === 'A'});
  } else if (phase === 'popup') {
    const after = read(); localStorage.setItem('popup-smoke', 'A-popup');
    report({storage: after.storage === 'A', cookie: after.cookie === 'A'});
  }
} catch (_) { report({javascript: false}); }
"#;

fn build_window(
    app: &AppHandle,
    profile: PathBuf,
    label: &str,
    phase: &str,
    reports: Sender<Value>,
    features: Option<NewWindowFeatures>,
) -> tauri::Result<WebviewWindow> {
    let popup_app = app.clone();
    let popup_profile = profile.clone();
    let popup_reports = reports.clone();
    let initial = if features.is_some() {
        WebviewUrl::External("about:blank".parse().unwrap())
    } else {
        WebviewUrl::CustomProtocol(format!("profilesmoke://localhost/{phase}").parse().unwrap())
    };
    let mut builder = WebviewWindowBuilder::new(app, label, initial)
        .title("Profile isolation smoke test")
        .inner_size(320.0, 240.0)
        .visible(false)
        .focused(false)
        .skip_taskbar(true)
        .data_directory(profile)
        .use_https_scheme(true)
        .on_navigation(|url| {
            url.as_str() == "about:blank"
                || url.scheme() == "profilesmoke"
                || url.host_str() == Some("profilesmoke.localhost")
        })
        .on_document_title_changed(move |window, title| {
            if let Some(text) = title.strip_prefix("profile-smoke:")
                && let Ok(mut report) = serde_json::from_str::<Value>(text)
            {
                let hidden = window.is_visible().is_ok_and(|visible| !visible);
                if !hidden {
                    let _ = window.hide();
                }
                report["hidden"] = json!(hidden);
                let _ = reports.send(report);
            }
        })
        .on_new_window(move |url, features| {
            if url.host_str() != Some("profilesmoke.localhost") {
                return NewWindowResponse::Deny;
            }
            match build_window(
                &popup_app,
                popup_profile.clone(),
                "smoke-popup",
                "popup",
                popup_reports.clone(),
                Some(features),
            ) {
                Ok(window) => NewWindowResponse::Create { window },
                Err(error) => {
                    eprintln!("popup_build_failed: {error}");
                    NewWindowResponse::Deny
                }
            }
        });
    if let Some(features) = features {
        builder = builder.window_features(features);
    }
    builder.build()
}

fn expect(reports: &Receiver<Value>, phase: &str) -> anyhow::Result<()> {
    let report = reports.recv_timeout(Duration::from_secs(8))?;
    anyhow::ensure!(report["phase"] == phase, "unexpected test phase");
    let checks = report
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("invalid report"))?;
    let passed = checks
        .iter()
        .all(|(key, value)| key == "phase" || value == &json!(true));
    println!(
        "{}",
        json!({"phase": phase, "passed": passed, "checks": report})
    );
    anyhow::ensure!(passed, "profile isolation check failed");
    Ok(())
}

fn on_main(
    app: &AppHandle,
    action: impl FnOnce(AppHandle) -> anyhow::Result<()> + Send + 'static,
) -> anyhow::Result<()> {
    let (done, result) = mpsc::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = done.send(action(handle));
    })?;
    result.recv_timeout(Duration::from_secs(8))??;
    Ok(())
}

fn drive(
    app: AppHandle,
    a: PathBuf,
    b: PathBuf,
    sender: Sender<Value>,
    reports: Receiver<Value>,
) -> anyhow::Result<()> {
    expect(&reports, "a-write")?;
    let next_sender = sender.clone();
    on_main(&app, move |handle| {
        build_window(&handle, b, "smoke-b", "b-write", next_sender, None)?;
        Ok(())
    })?;
    expect(&reports, "b-write")?;
    on_main(&app, move |handle| {
        handle
            .get_webview_window("smoke-a")
            .ok_or_else(|| anyhow::anyhow!("missing profile A"))?
            .destroy()?;
        Ok(())
    })?;
    std::thread::sleep(Duration::from_millis(500));
    on_main(&app, move |handle| {
        build_window(&handle, a, "smoke-a-reopened", "a-reopen", sender, None)?;
        Ok(())
    })?;
    expect(&reports, "a-reopen")?;
    on_main(&app, move |handle| {
        handle
            .get_webview_window("smoke-a-reopened")
            .ok_or_else(|| anyhow::anyhow!("missing reopened A"))?
            .eval("window.open('/popup', 'profile-smoke-popup', 'width=320,height=240')")?;
        Ok(())
    })?;
    expect(&reports, "popup")?;
    on_main(&app, move |handle| {
        handle.get_webview_window("smoke-b").ok_or_else(|| anyhow::anyhow!("missing profile B"))?
            .eval("document.title = 'profile-smoke:' + JSON.stringify({phase:'b-after-popup', storage:localStorage.getItem('profile-smoke')==='B', cookie:document.cookie.includes('profile-smoke=B'), popupIsolated:localStorage.getItem('popup-smoke')===null})")?;
        Ok(())
    })?;
    expect(&reports, "b-after-popup")?;
    on_main(&app, move |handle| {
        handle.get_webview_window("smoke-a-reopened").ok_or_else(|| anyhow::anyhow!("missing reopened A"))?
            .eval("document.title = 'profile-smoke:' + JSON.stringify({phase:'a-after-popup', popupShared:localStorage.getItem('popup-smoke')==='A-popup'})")?;
        Ok(())
    })?;
    expect(&reports, "a-after-popup")?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    anyhow::ensure!(
        cfg!(windows),
        "This smoke harness requires Windows WebView2"
    );
    let profiles = tempfile::Builder::new()
        .prefix("usage-control-profile-smoke-")
        .tempdir()?;
    let a = profiles.path().join("profile-a");
    let b = profiles.path().join("profile-b");
    std::fs::create_dir_all(&a)?;
    std::fs::create_dir_all(&b)?;
    let finished = Arc::new(AtomicBool::new(false));
    let success = Arc::new(AtomicBool::new(false));
    let end_state = finished.clone();
    let success_state = success.clone();
    let app = tauri::Builder::default()
        .register_uri_scheme_protocol("profilesmoke", |_context, _request| {
            tauri::http::Response::builder()
                .header("Content-Type", "text/html; charset=utf-8")
                .header("Cache-Control", "no-store")
                .header("Content-Security-Policy", "default-src 'none'; script-src 'unsafe-inline'")
                .body(format!("<!doctype html><meta charset=utf-8><title>Smoke</title><script>{SCRIPT}</script>").into_bytes())
                .unwrap()
        })
        .setup(move |app| {
            let (sender, reports) = mpsc::channel();
            let watchdog = app.handle().clone();
            let watchdog_done = finished.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(40));
                if !watchdog_done.load(Ordering::SeqCst) {
                    eprintln!("profile_smoke_timeout");
                    watchdog.exit(1);
                    std::thread::sleep(Duration::from_secs(3));
                    std::process::exit(1);
                }
            });
            build_window(app.handle(), a.clone(), "smoke-a", "a-write", sender.clone(), None)?;
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let result = drive(handle.clone(), a, b, sender, reports);
                success.store(result.is_ok(), Ordering::SeqCst);
                if let Err(error) = result { eprintln!("profile_smoke_failed: {error}"); }
                let _ = on_main(&handle, |app| {
                    for window in app.webview_windows().into_values() {
                        window.destroy()?;
                    }
                    Ok(())
                });
                handle.exit(if success.load(Ordering::SeqCst) { 0 } else { 1 });
            });
            Ok(())
        })
        .build(tauri::generate_context!())?;
    let code = app.run_return(|_, _| {});
    end_state.store(true, Ordering::SeqCst);
    let passed = code == 0 && success_state.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(500));
    println!(
        "{}",
        json!({"nativeWebView2ProfileSmoke": passed, "exitCode": code})
    );
    anyhow::ensure!(passed, "Native profile smoke did not pass");
    Ok(())
}
