pub fn trusted_handler<R: tauri::Runtime>(
    handler: impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        if invoke.message.webview_ref().label() != "popup" {
            invoke
                .resolver
                .reject("Application commands are only available from the Quota Control dashboard");
            return true;
        }
        handler(invoke)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tauri::Manager;
    use tauri::test::{INVOKE_KEY, mock_builder, mock_context, noop_assets};

    #[tauri::command]
    fn guarded_ping(calls: tauri::State<'_, AtomicUsize>) -> &'static str {
        calls.fetch_add(1, Ordering::SeqCst);
        "pong"
    }

    fn local_url() -> &'static str {
        if cfg!(any(windows, target_os = "android")) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        }
    }

    fn invoke(label: &str, origin: &str) -> (Result<String, serde_json::Value>, usize) {
        let app = mock_builder()
            .manage(AtomicUsize::new(0))
            .invoke_handler(super::trusted_handler(tauri::generate_handler![
                guarded_ping
            ]))
            .build(mock_context(noop_assets()))
            .unwrap();
        let webview = tauri::WebviewWindowBuilder::new(&app, label, Default::default())
            .build()
            .unwrap();
        let result = tauri::test::get_ipc_response(
            &webview,
            tauri::webview::InvokeRequest {
                cmd: "guarded_ping".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: origin.parse().unwrap(),
                body: tauri::ipc::InvokeBody::default(),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|response| response.deserialize::<String>().unwrap());
        let calls = app.state::<AtomicUsize>().load(Ordering::SeqCst);
        (result, calls)
    }

    #[test]
    fn bundled_popup_can_invoke_application_command() {
        let (result, calls) = invoke("popup", local_url());
        assert_eq!(result, Ok("pong".into()));
        assert_eq!(calls, 1);
    }

    #[test]
    fn chat_label_cannot_invoke_even_after_local_navigation() {
        let (result, calls) = invoke("chat-session", local_url());
        assert_eq!(
            result.unwrap_err(),
            "Application commands are only available from the Quota Control dashboard"
        );
        assert_eq!(calls, 0);
    }

    #[test]
    fn popup_label_does_not_bypass_remote_origin_acl() {
        let (result, calls) = invoke("popup", "https://chatgpt.com/");
        assert!(result.is_err());
        assert_eq!(calls, 0);
    }

    #[test]
    fn remote_chat_cannot_invoke_application_command() {
        let (result, calls) = invoke("chat-session", "https://claude.ai/");
        assert!(result.is_err());
        assert_eq!(calls, 0);
    }
}
