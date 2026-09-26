//! Spending one of Codex's banked limit resets from the popup. It happens only when the user presses
//! the button and confirms; nothing spends resets on a schedule, and the regular limits keep
//! resetting on the provider's own schedule. An attempt that got no definite answer keeps its request
//! id, so pressing again repeats the same request and the provider answers `already_redeemed`
//! instead of spending a second reset.

use std::collections::{HashMap, HashSet};

use parking_lot::Mutex;
use serde::Serialize;
use tauri::State;
use uc_core::{ErrorCategory, ProviderRuntime};

use crate::service::BackendService;

const REDEEMED: &str = "reset";
const ALREADY_REDEEMED: &str = "already_redeemed";
const NO_CREDIT: &str = "no_credit";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum RedeemResult {
    /// The limit came back.
    Reset {
        #[serde(rename = "resetType")]
        reset_type: Option<String>,
    },
    /// The provider refused for `code`; nothing was spent.
    Rejected { code: String },
    /// No definite answer; pressing again repeats the same request.
    Failed { category: ErrorCategory },
    /// Another press for this account is still running.
    InFlight,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Attempt {
    credit_id: String,
    request_id: String,
    unanswered: bool,
}

/// Tauri state: per account, the attempt waiting for a definite answer and whether one is running.
#[derive(Default)]
pub struct Redemptions {
    attempts: Mutex<HashMap<String, Attempt>>,
    running: Mutex<HashSet<String>>,
}

struct Running<'a> {
    redemptions: &'a Redemptions,
    provider_id: String,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.redemptions.running.lock().remove(&self.provider_id);
    }
}

impl Redemptions {
    fn start(&self, provider_id: &str) -> Option<Running<'_>> {
        self.running
            .lock()
            .insert(provider_id.to_string())
            .then(|| Running {
                redemptions: self,
                provider_id: provider_id.to_string(),
            })
    }

    async fn redeem(&self, runtime: &dyn ProviderRuntime, provider_id: &str) -> RedeemResult {
        let pending = self.attempts.lock().get(provider_id).cloned();
        let attempt = match pending {
            Some(attempt) => attempt,
            None => {
                let credits = match runtime.limit_reset_credits().await {
                    Ok(credits) => credits,
                    Err(error) => {
                        return RedeemResult::Failed {
                            category: error.category,
                        };
                    }
                };
                let Some(credit) = credits.into_iter().next() else {
                    return RedeemResult::Rejected {
                        code: NO_CREDIT.into(),
                    };
                };
                let attempt = Attempt {
                    credit_id: credit.id,
                    request_id: uuid::Uuid::new_v4().to_string(),
                    unanswered: false,
                };
                self.attempts
                    .lock()
                    .insert(provider_id.to_string(), attempt.clone());
                attempt
            }
        };
        match runtime
            .redeem_limit_reset(&attempt.credit_id, &attempt.request_id)
            .await
        {
            Ok(reply) => {
                self.attempts.lock().remove(provider_id);
                if reply.code == REDEEMED || (attempt.unanswered && reply.code == ALREADY_REDEEMED)
                {
                    RedeemResult::Reset {
                        reset_type: reply.reset_type,
                    }
                } else {
                    RedeemResult::Rejected { code: reply.code }
                }
            }
            Err(error) => {
                if let Some(stored) = self.attempts.lock().get_mut(provider_id) {
                    stored.unanswered = true;
                }
                RedeemResult::Failed {
                    category: error.category,
                }
            }
        }
    }
}

/// Spend the reset that expires first for `provider_id`, then refresh that account.
#[tauri::command]
pub async fn redeem_limit_reset(
    service: State<'_, BackendService>,
    redemptions: State<'_, Redemptions>,
    provider_id: String,
) -> Result<RedeemResult, String> {
    service.validate_provider_ids(std::slice::from_ref(&provider_id))?;
    let engine = service.engine();
    let runtime = engine
        .runtime(&provider_id)
        .ok_or("This account is not connected.")?;
    let Some(_running) = redemptions.start(&provider_id) else {
        return Ok(RedeemResult::InFlight);
    };
    let result = redemptions.redeem(runtime.as_ref(), &provider_id).await;
    engine.refresh(&provider_id, true).await;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use async_trait::async_trait;
    use uc_core::{
        LimitResetCredit, LimitResetReply, Provider, ProviderSnapshot, RefreshContext,
        SimpleProviderError, WidgetDescriptor,
    };

    use super::*;

    struct Fake {
        provider: Provider,
        credits: Result<Vec<LimitResetCredit>, SimpleProviderError>,
        replies: Mutex<VecDeque<Result<LimitResetReply, SimpleProviderError>>>,
        calls: Mutex<Vec<(String, String)>>,
    }

    fn credit(id: &str) -> LimitResetCredit {
        LimitResetCredit {
            id: id.into(),
            expires_at: None,
        }
    }

    fn reply(code: &str) -> Result<LimitResetReply, SimpleProviderError> {
        Ok(LimitResetReply {
            code: code.into(),
            reset_type: (code == REDEEMED).then(|| "both".into()),
        })
    }

    fn network() -> SimpleProviderError {
        SimpleProviderError::new(ErrorCategory::Network, "offline")
    }

    fn fake(
        credits: Result<Vec<LimitResetCredit>, SimpleProviderError>,
        replies: Vec<Result<LimitResetReply, SimpleProviderError>>,
    ) -> Fake {
        Fake {
            provider: Provider::new("codex@fixture", "Codex"),
            credits,
            replies: Mutex::new(replies.into()),
            calls: Mutex::new(Vec::new()),
        }
    }

    #[async_trait]
    impl ProviderRuntime for Fake {
        fn provider(&self) -> &Provider {
            &self.provider
        }

        fn widget_descriptors(&self) -> Vec<WidgetDescriptor> {
            Vec::new()
        }

        async fn refresh(&self, _context: RefreshContext) -> ProviderSnapshot {
            ProviderSnapshot::make(&self.provider, None, Vec::new(), chrono::Utc::now())
        }

        async fn has_local_credentials(&self) -> bool {
            true
        }

        async fn limit_reset_credits(&self) -> Result<Vec<LimitResetCredit>, SimpleProviderError> {
            self.credits.clone()
        }

        async fn redeem_limit_reset(
            &self,
            credit_id: &str,
            request_id: &str,
        ) -> Result<LimitResetReply, SimpleProviderError> {
            self.calls
                .lock()
                .push((credit_id.to_string(), request_id.to_string()));
            self.replies
                .lock()
                .pop_front()
                .expect("an unexpected redeem call")
        }
    }

    #[tokio::test]
    async fn spends_the_first_listed_credit_once() {
        let runtime = fake(
            Ok(vec![credit("soonest"), credit("later")]),
            vec![reply(REDEEMED)],
        );
        let redemptions = Redemptions::default();
        assert_eq!(
            redemptions.redeem(&runtime, "codex@fixture").await,
            RedeemResult::Reset {
                reset_type: Some("both".into())
            }
        );
        let calls = runtime.calls.lock();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "soonest");
        assert_eq!(
            uuid::Uuid::parse_str(&calls[0].1)
                .unwrap()
                .get_version_num(),
            4
        );
        assert!(redemptions.attempts.lock().is_empty());
    }

    #[tokio::test]
    async fn a_retry_after_no_answer_repeats_the_same_request() {
        let runtime = fake(
            Ok(vec![credit("soonest")]),
            vec![Err(network()), reply(ALREADY_REDEEMED)],
        );
        let redemptions = Redemptions::default();
        assert_eq!(
            redemptions.redeem(&runtime, "codex@fixture").await,
            RedeemResult::Failed {
                category: ErrorCategory::Network
            }
        );
        assert_eq!(
            redemptions.redeem(&runtime, "codex@fixture").await,
            RedeemResult::Reset { reset_type: None }
        );
        let calls = runtime.calls.lock();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], calls[1]);
        assert!(redemptions.attempts.lock().is_empty());
    }

    #[tokio::test]
    async fn refusals_spend_nothing_and_clear_the_attempt() {
        for code in [ALREADY_REDEEMED, "nothing_to_reset", NO_CREDIT] {
            let runtime = fake(Ok(vec![credit("soonest")]), vec![reply(code)]);
            let redemptions = Redemptions::default();
            assert_eq!(
                redemptions.redeem(&runtime, "codex@fixture").await,
                RedeemResult::Rejected { code: code.into() }
            );
            assert!(redemptions.attempts.lock().is_empty());
        }
    }

    #[tokio::test]
    async fn no_credit_or_a_failed_listing_never_calls_redeem() {
        let empty = fake(Ok(Vec::new()), Vec::new());
        let redemptions = Redemptions::default();
        assert_eq!(
            redemptions.redeem(&empty, "codex@fixture").await,
            RedeemResult::Rejected {
                code: NO_CREDIT.into()
            }
        );
        let offline = fake(Err(network()), Vec::new());
        assert_eq!(
            redemptions.redeem(&offline, "codex@fixture").await,
            RedeemResult::Failed {
                category: ErrorCategory::Network
            }
        );
        assert!(empty.calls.lock().is_empty() && offline.calls.lock().is_empty());
        assert!(redemptions.attempts.lock().is_empty());
    }

    #[test]
    fn one_press_per_account_runs_at_a_time() {
        let redemptions = Redemptions::default();
        let first = redemptions.start("codex@fixture");
        assert!(first.is_some());
        assert!(redemptions.start("codex@fixture").is_none());
        assert!(redemptions.start("codex@other").is_some());
        drop(first);
        assert!(redemptions.start("codex@fixture").is_some());
    }

    #[test]
    fn serializes_results_for_the_popup() {
        let json = |result: RedeemResult| serde_json::to_value(result).unwrap();
        assert_eq!(
            json(RedeemResult::Reset {
                reset_type: Some("weekly".into())
            }),
            serde_json::json!({"status": "reset", "resetType": "weekly"})
        );
        assert_eq!(
            json(RedeemResult::Rejected {
                code: "nothing_to_reset".into()
            }),
            serde_json::json!({"status": "rejected", "code": "nothing_to_reset"})
        );
        assert_eq!(
            json(RedeemResult::Failed {
                category: ErrorCategory::Network
            }),
            serde_json::json!({"status": "failed", "category": "network"})
        );
        assert_eq!(
            json(RedeemResult::InFlight),
            serde_json::json!({"status": "inFlight"})
        );
    }
}
