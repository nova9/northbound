//! Background block-template poller and long-poll cache for solo miners.
//!
//! One background loop polls `truenorthd` every second and stores the pre-serialized
//! JSON bytes in a `tokio::sync::watch` channel. All mining HTTP clients read from
//! that shared cache so many miners never multiply RPC load on the node.

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
use truenorth_rpc::NodeClient;

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

    fn to_json_response(&self) -> Response {
        (
            [(header::CONTENT_TYPE, "application/json")],
            self.body.clone(),
        )
            .into_response()
    }
}

/// Polls `getblocktemplate` once per second and publishes fresh templates to all
/// waiting mining clients. Clears the cache on any failure or timeout.
pub async fn run_template_watcher(state: Arc<MiningState>, mut stop: watch::Receiver<bool>) {
    let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    while !*stop.borrow() {
        tokio::select! {
            _ = stop.changed() => break,
            _ = ticker.tick() => {}
        }

        let refreshed = tokio::select! {
            _ = stop.changed() => break,
            template = fetch_fresh_template(&state.node) => template,
        };

        state.templates.send_replace(refreshed);
    }

    state.templates.send_replace(None);
}

async fn fetch_fresh_template(node: &NodeClient) -> Option<Arc<CachedTemplate>> {
    let fetch = async {
        let value = node.fetch_block_template().await?;
        let template = CachedTemplate::from_json(value)?;
        Ok::<_, ApiError>(Arc::new(template))
    };

    match tokio::time::timeout(REFRESH_TIMEOUT, fetch).await {
        Ok(Ok(template)) => Some(template),
        Ok(Err(_)) => None,
        Err(_) => {
            eprintln!("Mining template refresh timed out");
            None
        }
    }
}

/// Returns the latest cached template immediately if `previous_id` is absent or
/// differs from the current `longpollid`. Otherwise waits up to 25 seconds for a
/// new template, returning the latest fresh template if the timeout elapses.
pub(super) async fn wait_for_template(
    state: &MiningState,
    previous_id: Option<&str>,
) -> Result<Response, ApiError> {
    let mut receiver = state.templates.subscribe();

    match tokio::time::timeout(
        LONG_POLL_TIMEOUT,
        wait_until_template_changes(&mut receiver, previous_id),
    )
    .await
    {
        Ok(result) => result,
        // Long-poll timed out: return the latest fresh template even if its ID is unchanged.
        Err(_) => Ok(read_fresh_template(&mut receiver)?.to_json_response()),
    }
}

async fn wait_until_template_changes(
    receiver: &mut watch::Receiver<Option<Arc<CachedTemplate>>>,
    previous_id: Option<&str>,
) -> Result<Response, ApiError> {
    loop {
        let template = read_fresh_template(receiver)?;
        if previous_id != Some(template.id.as_str()) {
            return Ok(template.to_json_response());
        }

        let expires_at = template.fetched_at + MAX_CACHE_AGE;
        tokio::select! {
            changed = receiver.changed() => {
                changed.map_err(|_| ApiError::unavailable("Mining service stopped."))?;
            }
            _ = tokio::time::sleep_until(expires_at) => {
                // Recheck on the next loop iteration in case a refresh raced with expiration.
            }
        }
    }
}

fn read_fresh_template(
    receiver: &mut watch::Receiver<Option<Arc<CachedTemplate>>>,
) -> Result<Arc<CachedTemplate>, ApiError> {
    // `borrow_and_update` marks this version seen before we inspect it,
    // preventing missed updates if a refresh lands right after we check.
    let template = receiver
        .borrow_and_update()
        .clone()
        .ok_or_else(|| ApiError::unavailable("Mining work is unavailable; retry shortly."))?;

    if template.fetched_at.elapsed() >= MAX_CACHE_AGE {
        return Err(ApiError::unavailable(
            "Mining work has expired; retry shortly.",
        ));
    }

    Ok(template)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn offline_state() -> Arc<MiningState> {
        let node =
            NodeClient::new("http://127.0.0.1:1", "unused-cookie".into(), "regtest").unwrap();
        Arc::new(MiningState::new(node))
    }

    fn publish_template(state: &MiningState, id: &str) {
        let template = CachedTemplate::from_json(json!({"longpollid": id, "extra": "preserved"}))
            .ok()
            .unwrap();
        state.templates.send_replace(Some(Arc::new(template)));
    }

    #[tokio::test]
    async fn one_update_wakes_all_waiters_and_preserves_template_fields() {
        let state = offline_state();
        publish_template(&state, "old");

        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let state = state.clone();
            tasks.spawn(async move { wait_for_template(&state, Some("old")).await.ok().unwrap() });
        }

        while state.templates.receiver_count() < 8 {
            tokio::task::yield_now().await;
        }
        assert_eq!(tasks.len(), 8);

        publish_template(&state, "new");

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
        let state = offline_state();
        publish_template(&state, "old");

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
        let state = offline_state();
        publish_template(&state, "same");

        let (stop, mut stopped) = watch::channel(false);
        let refresh_state = state.clone();
        let refresh = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stopped.changed() => break,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                        publish_template(&refresh_state, "same");
                    }
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
        let state = offline_state();
        publish_template(&state, "old");

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
