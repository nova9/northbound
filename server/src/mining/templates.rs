use std::{sync::Arc, time::Duration};

use axum::{
    body::Bytes,
    http::header,
    response::{IntoResponse, Response},
};
use tokio::{
    sync::watch,
    time::{Instant, MissedTickBehavior},
};

use super::MiningState;
use crate::error::ApiError;

const REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const REFRESH_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_CACHE_AGE: Duration = Duration::from_secs(5);
const LONG_POLL_TIMEOUT: Duration = Duration::from_secs(25);

pub(super) struct CachedTemplate {
    id: String,
    body: Bytes,
    fetched_at: Instant,
}

impl CachedTemplate {
    fn from_json(value: serde_json::Value) -> Result<Self, ApiError> {
        let id = value["longpollid"]
            .as_str()
            .ok_or_else(|| ApiError::unavailable("The mining template has no identifier."))?
            .to_owned();
        let body = serde_json::to_vec(&value)
            .map_err(|_| ApiError::unavailable("Could not encode the mining template."))?;
        Ok(Self {
            id,
            body: body.into(),
            fetched_at: Instant::now(),
        })
    }

    fn response(&self) -> Response {
        (
            [(header::CONTENT_TYPE, "application/json")],
            self.body.clone(),
        )
            .into_response()
    }
}

/// One sequential upstream poll serves every HTTP client. Core handles template
/// rebuilding; each successful poll also refreshes timestamp fields. No client
/// opens its own daemon long poll, and node requests have a shorter timeout than
/// the cache's maximum age.
pub async fn run_template_watcher(state: Arc<MiningState>, mut stop: watch::Receiver<bool>) {
    let mut interval = tokio::time::interval(REFRESH_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        if *stop.borrow() {
            break;
        }
        tokio::select! {
            _ = stop.changed() => break,
            _ = interval.tick() => {}
        }
        let refresh = async {
            let template = state.node.fetch_block_template().await?;
            Ok::<_, ApiError>(Arc::new(CachedTemplate::from_json(template)?))
        };
        let result = tokio::select! {
            _ = stop.changed() => break,
            result = tokio::time::timeout(REFRESH_TIMEOUT, refresh) => result,
        };
        match result {
            Ok(Ok(template)) => {
                state.templates.send_replace(Some(template));
            }
            Ok(Err(_)) => {
                state.templates.send_replace(None);
            }
            Err(_) => {
                eprintln!("Mining template refresh timed out");
                state.templates.send_replace(None);
            }
        }
    }
    state.templates.send_replace(None);
}

fn current_template(
    receiver: &watch::Receiver<Option<Arc<CachedTemplate>>>,
) -> Result<Arc<CachedTemplate>, ApiError> {
    let template = receiver
        .borrow()
        .clone()
        .ok_or_else(|| ApiError::unavailable("Mining work is unavailable; retry shortly."))?;
    if template.fetched_at.elapsed() >= MAX_CACHE_AGE {
        return Err(ApiError::unavailable(
            "Mining work has expired; retry shortly.",
        ));
    }
    Ok(template)
}

pub(super) async fn wait_for_template(
    state: &MiningState,
    previous_id: Option<&str>,
) -> Result<Response, ApiError> {
    let mut receiver = state.templates.subscribe();
    let wait = async {
        loop {
            // Mark this version seen before inspecting it, avoiding missed updates.
            let template = receiver.borrow_and_update().clone();
            let template = template.ok_or_else(|| {
                ApiError::unavailable("Mining work is unavailable; retry shortly.")
            })?;
            if template.fetched_at.elapsed() >= MAX_CACHE_AGE {
                return Err(ApiError::unavailable(
                    "Mining work has expired; retry shortly.",
                ));
            }
            if previous_id != Some(template.id.as_str()) {
                return Ok(template.response());
            }
            tokio::select! {
                result = receiver.changed() => {
                    result.map_err(|_| ApiError::unavailable("Mining service stopped."))?;
                }
                _ = tokio::time::sleep_until(template.fetched_at + MAX_CACHE_AGE) => {
                    // Recheck in case a refresh raced with the expiration timer.
                    current_template(&receiver)?;
                }
            }
        }
    };
    match tokio::time::timeout(LONG_POLL_TIMEOUT, wait).await {
        Ok(result) => result,
        // Return the latest fresh template even if its identifier is unchanged.
        Err(_) => Ok(current_template(&receiver)?.response()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use truenorth_rpc::NodeClient;

    fn state() -> Arc<MiningState> {
        Arc::new(MiningState::new(
            NodeClient::new("http://127.0.0.1:1", "unused-cookie".into(), "regtest").unwrap(),
        ))
    }

    fn publish(state: &MiningState, id: &str) {
        let template = CachedTemplate::from_json(json!({"longpollid": id, "extra": "preserved"}))
            .ok()
            .unwrap();
        state.templates.send_replace(Some(Arc::new(template)));
    }

    #[tokio::test]
    async fn one_update_wakes_all_waiters_and_preserves_template_fields() {
        let state = state();
        publish(&state, "old");
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let state = state.clone();
            tasks.spawn(async move { wait_for_template(&state, Some("old")).await.ok().unwrap() });
        }
        // Every waiter has subscribed and yielded before publishing the update.
        while state.templates.receiver_count() < 8 {
            tokio::task::yield_now().await;
        }
        assert_eq!(tasks.len(), 8);
        publish(&state, "new");
        while let Some(response) = tasks.join_next().await {
            let body = axum::body::to_bytes(response.unwrap().into_body(), 1024)
                .await
                .unwrap();
            let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(value, json!({"longpollid": "new", "extra": "preserved"}));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_clients_fail_when_cached_work_expires() {
        let state = state();
        publish(&state, "old");
        let started = Instant::now();
        let response = wait_for_template(&state, Some("old"))
            .await
            .err()
            .unwrap()
            .into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(started.elapsed(), MAX_CACHE_AGE);
        assert!(wait_for_template(&state, None).await.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_returns_fresh_work_even_when_identifier_does_not_change() {
        let state = state();
        publish(&state, "same");
        let (stop, mut stopped) = watch::channel(false);
        let refresh_state = state.clone();
        let refresh = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stopped.changed() => break,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => publish(&refresh_state, "same"),
                }
            }
        });
        let started = Instant::now();
        assert!(wait_for_template(&state, Some("same")).await.is_ok());
        assert_eq!(started.elapsed(), LONG_POLL_TIMEOUT);
        stop.send_replace(true);
        refresh.await.unwrap();
    }

    #[tokio::test]
    async fn invalidation_wakes_waiters_without_serving_the_old_template() {
        let state = state();
        publish(&state, "old");
        let waiting = state.clone();
        let task =
            tokio::spawn(async move { wait_for_template(&waiting, Some("old")).await.is_err() });
        while state.templates.receiver_count() < 1 {
            tokio::task::yield_now().await;
        }
        state.templates.send_replace(None);
        assert!(task.await.unwrap());
    }
}
