#![cfg(all(feature = "runway", feature = "replicate"))]
#[path = "../src/test_endpoints.rs"]
mod test_endpoints;
use axum::{
    body::Bytes,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::any,
    Router,
};
use base64::Engine as _;
use battersea_model::{
    adapter::{EngineAuthConfig, EngineBackendOptions},
    media::*,
    ErrorKind,
};
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct Reply {
    status: StatusCode,
    body: Vec<u8>,
    mime: &'static str,
    retry: Option<String>,
    location: Option<String>,
}
impl Reply {
    fn json(value: Value) -> Self {
        Self {
            status: StatusCode::OK,
            body: serde_json::to_vec(&value).unwrap(),
            mime: "application/json",
            retry: None,
            location: None,
        }
    }
    fn bytes(mime: &'static str, bytes: &[u8]) -> Self {
        Self {
            status: StatusCode::OK,
            body: bytes.into(),
            mime,
            retry: None,
            location: None,
        }
    }
    fn error(status: StatusCode) -> Self {
        Self {
            status,
            ..Self::json(json!({}))
        }
    }
}
type RecordedRequest = (String, String, HeaderMap, Vec<u8>);
#[derive(Clone, Default)]
struct Server {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    calls: Arc<Mutex<Vec<RecordedRequest>>>,
}
async fn handle(State(state): State<Server>, request: Request) -> impl IntoResponse {
    let (parts, body) = request.into_parts();
    let body = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
    state.calls.lock().unwrap().push((
        parts.method.to_string(),
        parts.uri.path().to_owned(),
        parts.headers,
        body.to_vec(),
    ));
    let reply = state
        .replies
        .lock()
        .unwrap()
        .pop_front()
        .unwrap_or_else(|| Reply::error(StatusCode::INTERNAL_SERVER_ERROR));
    let mut headers = HeaderMap::new();
    headers.insert("content-type", reply.mime.parse().unwrap());
    if let Some(retry) = reply.retry {
        headers.insert("retry-after", retry.parse().unwrap());
    }
    if let Some(location) = reply.location {
        headers.insert("location", location.parse().unwrap());
    }
    (reply.status, headers, Bytes::from(reply.body))
}
async fn server() -> (String, Server, tokio::task::JoinHandle<()>) {
    let state = Server::default();
    let listener = tokio::net::TcpListener::bind(test_endpoints::TEST_BIND_ADDRESS)
        .await
        .unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .fallback(any(handle))
        .with_state(state.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, state, task)
}
fn backend(provider: &str, url: &str, kind: MediaKind) -> MediaBackendConfig {
    MediaBackendConfig {
        id: "media-fixture".into(), provider: provider.into(), capability: match kind { MediaKind::Image => MediaCapability::ImageGeneration, MediaKind::Video => MediaCapability::VideoGeneration, MediaKind::Audio => MediaCapability::AudioGeneration },
        label: "Fixture".into(), enabled: true, endpoint: format!("{url}/v1"), model: if provider == "replicate" { "owner/model".into() } else { "gen4_image".into() },
        capabilities: MediaBackendCapabilities { supported_media_parameters: vec!["count".into()], aspect_ratio_options: vec![], size_options: vec![], supports_partial_image_streaming: false, estimated_generation_seconds: 1.0 },
        options: EngineBackendOptions { timeout_ms: Some(1000), extra: serde_json::from_value(json!({"pollIntervalMs":1,"pollRetryDelayMs":1,"maxPollRetryCount":2,"inputFields":{"prompt":"text","count":"num_outputs"},"maxPromptChars":10000,"defaultAspectRatio":"1:1"})).unwrap(), ..Default::default() },
        auth: EngineAuthConfig { api_key: Some("fixture-key".into()), version_header: Some("X-Runway-Version".into()), version: Some("2024-11-06".into()), auth_type: "bearer".into(), api_key_env: String::new(), header: None, has_api_key: true },
        short_description: String::new(), long_description: String::new(),
    }
}
fn request(kind: MediaKind) -> MediaRenderRequest {
    MediaRenderRequest {
        kind,
        prompt_text: "supplied fixture prompt".into(),
        negative_prompt: None,
        references: vec![],
        options: MediaGenerationHints::default(),
    }
}
fn update(provider: &str, status: &str, output: Value) -> Value {
    if provider == "replicate" {
        json!({"id":"accepted-job","status":status,"output":output,"completed_at":Utc::now().to_rfc3339()})
    } else {
        json!({"id":"accepted-job","status":status,"output":output})
    }
}
async fn submit(config: MediaBackendConfig, kind: MediaKind) -> Box<dyn MediaJob> {
    let adapter = battersea_providers::builtin_registry()
        .create_media(config)
        .unwrap();
    match adapter
        .submit(
            adapter.prepare(request(kind)).unwrap(),
            CancellationToken::new(),
            None,
            None,
        )
        .await
        .unwrap()
    {
        MediaSubmission::Pending(job) => job,
        _ => panic!("remote job expected"),
    }
}
fn enqueue(server: &Server, replies: impl IntoIterator<Item = Reply>) {
    server.replies.lock().unwrap().extend(replies);
}

#[tokio::test]
async fn remote_jobs_poll_download_and_retain_the_same_result_without_resubmission() {
    for (provider, kind, queued, running, succeeded, mime) in [
        (
            "runway",
            MediaKind::Image,
            "PENDING",
            "RUNNING",
            "SUCCEEDED",
            "image/png",
        ),
        (
            "replicate",
            MediaKind::Image,
            "starting",
            "processing",
            "succeeded",
            "image/png",
        ),
        (
            "replicate",
            MediaKind::Video,
            "starting",
            "processing",
            "succeeded",
            "video/mp4",
        ),
        (
            "replicate",
            MediaKind::Audio,
            "starting",
            "processing",
            "succeeded",
            "audio/wav",
        ),
    ] {
        let (url, state, task) = server().await;
        let bytes = b"retained provider media";
        enqueue(
            &state,
            [
                Reply::json(update(provider, queued, Value::Null)),
                Reply::json(update(provider, running, Value::Null)),
                Reply::json(update(provider, succeeded, json!([format!("{url}/asset")]))),
                Reply::bytes(mime, bytes),
            ],
        );
        let mut job = submit(backend(provider, &url, kind.clone()), kind).await;
        assert_eq!(job.snapshot().status, MediaJobStatus::Queued);
        let calls = state.calls.lock().unwrap().len();
        for _ in 0..3 {
            let _ = job.snapshot();
        }
        assert_eq!(
            state.calls.lock().unwrap().len(),
            calls,
            "inspection performs no I/O"
        );
        assert_eq!(
            job.poll(CancellationToken::new()).await.unwrap(),
            MediaJobStatus::Running
        );
        let result = job.wait(CancellationToken::new()).await.unwrap();
        assert!(job.snapshot().outputs_retrieved);
        assert!(job.snapshot().output_expires_at.is_some());
        let encoded = result.assets[0].url.split_once(',').unwrap().1;
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap(),
            bytes
        );
        let calls = state.calls.lock().unwrap().len();
        assert_eq!(
            job.retrieve(CancellationToken::new()).await.unwrap(),
            result
        );
        job.cancel().await.unwrap();
        assert_eq!(
            job.poll(CancellationToken::new()).await.unwrap(),
            MediaJobStatus::Succeeded
        );
        assert_eq!(state.calls.lock().unwrap().len(), calls);
        assert_eq!(
            state
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.0 == "POST")
                .count(),
            1
        );
        if provider == "replicate" {
            assert_eq!(
                state.calls.lock().unwrap().last().unwrap().2["authorization"],
                "Bearer fixture-key"
            );
        }
        task.abort();
    }
}

#[tokio::test]
async fn transient_poll_errors_retry_only_gets_and_respect_retry_after_bound() {
    for mode in ["server", "seconds", "date"] {
        let rate_limited = mode != "server";
        let (url, state, task) = server().await;
        let mut transient = Reply::error(if rate_limited {
            StatusCode::TOO_MANY_REQUESTS
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        });
        if rate_limited {
            transient.retry = Some(if mode == "date" {
                (Utc::now() + chrono::Duration::seconds(60)).to_rfc2822()
            } else {
                "60".into()
            });
        }
        enqueue(
            &state,
            [
                Reply::json(update("replicate", "starting", Value::Null)),
                transient,
            ],
        );
        if rate_limited {
            enqueue(
                &state,
                [Reply::json(update("replicate", "canceled", Value::Null))],
            );
        } else {
            enqueue(
                &state,
                [
                    Reply::json(update(
                        "replicate",
                        "succeeded",
                        json!([format!("{url}/asset")]),
                    )),
                    Reply::bytes("image/png", b"output"),
                ],
            );
        }
        let job = submit(
            backend("replicate", &url, MediaKind::Image),
            MediaKind::Image,
        )
        .await;
        let result = MediaSubmission::Pending(job)
            .wait(
                CancellationToken::new(),
                Some(tokio::time::Instant::now() + Duration::from_secs(2)),
            )
            .await;
        if rate_limited {
            assert_eq!(result.unwrap_err().classification, ErrorKind::RateLimit);
        } else {
            result.unwrap();
        }
        let calls = state.calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .filter(|c| c.1.ends_with("/predictions") && c.0 == "POST")
                .count(),
            1
        );
        assert_eq!(
            calls
                .iter()
                .filter(|c| c.0 == "GET" && c.1.contains("predictions"))
                .count(),
            if rate_limited { 1 } else { 2 }
        );
        task.abort();
    }
}

#[tokio::test]
async fn cancellation_deadline_and_drop_cleanup_target_the_accepted_job_once() {
    for mode in ["cancel", "deadline", "drop"] {
        let (url, state, task) = server().await;
        enqueue(
            &state,
            [
                Reply::json(update("replicate", "starting", Value::Null)),
                Reply::json(update("replicate", "canceled", Value::Null)),
            ],
        );
        let job = submit(
            backend("replicate", &url, MediaKind::Image),
            MediaKind::Image,
        )
        .await;
        if mode == "drop" {
            drop(job);
        } else {
            let token = CancellationToken::new();
            if mode == "cancel" {
                token.cancel();
            }
            let error = MediaSubmission::Pending(job)
                .wait(token, Some(tokio::time::Instant::now()))
                .await
                .unwrap_err();
            assert!(matches!(
                error.classification,
                ErrorKind::Cancelled | ErrorKind::Timeout
            ));
            assert_eq!(error.request_id.as_deref(), Some("accepted-job"));
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if state.calls.lock().unwrap().len() >= 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            state
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.1.ends_with("/cancel"))
                .count(),
            1
        );
        task.abort();
    }
}

#[tokio::test]
async fn retrieval_failure_is_repeatable_without_regenerating_or_publishing_partial_success() {
    let (url, state, task) = server().await;
    enqueue(
        &state,
        [
            Reply::json(update(
                "replicate",
                "succeeded",
                json!([format!("{url}/one"), format!("{url}/two")]),
            )),
            Reply::bytes("image/png", b"first"),
            Reply::error(StatusCode::SERVICE_UNAVAILABLE),
        ],
    );
    let mut job = submit(
        backend("replicate", &url, MediaKind::Image),
        MediaKind::Image,
    )
    .await;
    let retrieval_started = Utc::now();
    assert_eq!(
        job.retrieve(CancellationToken::new())
            .await
            .unwrap_err()
            .classification,
        ErrorKind::Retrieval
    );
    assert!(!job.snapshot().outputs_retrieved);
    enqueue(
        &state,
        [
            Reply::bytes("image/png", b"first"),
            Reply::bytes("image/png", b"second"),
        ],
    );
    let result = job.retrieve(CancellationToken::new()).await.unwrap();
    assert_eq!(result.assets.len(), 2);
    let timing = result
        .timing
        .expect("provider timing survives retrieval retries");
    assert!(
        chrono::DateTime::parse_from_rfc3339(&timing.completed_at).unwrap() <= retrieval_started,
        "provider timing excludes output download and retrieval retries"
    );
    assert_eq!(
        state
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.0 == "POST")
            .count(),
        1
    );
    task.abort();
}

#[tokio::test]
async fn expired_removed_and_malformed_results_never_produce_success_or_new_generation() {
    for mode in ["expired", "removed", "missing", "wrong-mime"] {
        let (url, state, task) = server().await;
        let mut terminal = update("replicate", "succeeded", json!([format!("{url}/asset")]));
        if mode == "expired" {
            terminal["completed_at"] =
                json!((Utc::now() - chrono::Duration::hours(2)).to_rfc3339());
        }
        if mode == "removed" {
            terminal["data_removed"] = json!(true);
            terminal["output"] = Value::Null;
        }
        enqueue(&state, [Reply::json(terminal)]);
        if mode == "missing" {
            enqueue(&state, [Reply::error(StatusCode::GONE)]);
        }
        if mode == "wrong-mime" {
            enqueue(&state, [Reply::bytes("text/html", b"error page")]);
        }
        let mut job = submit(
            backend("replicate", &url, MediaKind::Image),
            MediaKind::Image,
        )
        .await;
        let error = job.retrieve(CancellationToken::new()).await.unwrap_err();
        assert!(matches!(
            error.classification,
            ErrorKind::Expired | ErrorKind::InvalidResponse
        ));
        assert!(!job.snapshot().outputs_retrieved);
        assert_eq!(
            state
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.0 == "POST")
                .count(),
            1
        );
        task.abort();
    }
}
fn signature(id: &str, timestamp: &str, body: &[u8], key: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
    mac.update(id.as_bytes());
    mac.update(b".");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    format!(
        "v1,{}",
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
    )
}
#[tokio::test]
async fn signed_callbacks_are_idempotent_reject_replays_and_preserve_terminal_results() {
    let (url, state, task) = server().await;
    enqueue(
        &state,
        [
            Reply::json(update("replicate", "starting", Value::Null)),
            Reply::bytes("image/png", b"output"),
        ],
    );
    let mut job = submit(
        backend("replicate", &url, MediaKind::Image),
        MediaKind::Image,
    )
    .await;
    let key = b"test signing key";
    let secret = format!(
        "whsec_{}",
        base64::engine::general_purpose::STANDARD.encode(key)
    );
    let now = Utc::now();
    let timestamp = now.timestamp().to_string();
    let id = "delivery-id";
    let body = serde_json::to_vec(&update(
        "replicate",
        "succeeded",
        json!([format!("{url}/asset")]),
    ))
    .unwrap();
    let signed = signature(id, &timestamp, &body, key);
    for _ in 0..2 {
        assert_eq!(
            job.webhook(
                MediaWebhook {
                    id,
                    timestamp: &timestamp,
                    signature: &signed,
                    body: &body
                },
                &secret,
                now
            )
            .await
            .unwrap(),
            MediaJobStatus::Succeeded
        );
    }
    let result = job.retrieve(CancellationToken::new()).await.unwrap();
    for (body, when, sig) in [
        (
            body.clone(),
            now + chrono::Duration::minutes(6),
            signed.clone(),
        ),
        (body.clone(), now, "v1,AAAA".into()),
    ] {
        assert_eq!(
            job.webhook(
                MediaWebhook {
                    id,
                    timestamp: &timestamp,
                    signature: &sig,
                    body: &body
                },
                &secret,
                when
            )
            .await
            .unwrap_err()
            .classification,
            ErrorKind::Authentication
        );
    }
    let late = serde_json::to_vec(&update("replicate", "processing", Value::Null)).unwrap();
    let sig = signature(id, &timestamp, &late, key);
    assert_eq!(
        job.webhook(
            MediaWebhook {
                id,
                timestamp: &timestamp,
                signature: &sig,
                body: &late
            },
            &secret,
            now
        )
        .await
        .unwrap(),
        MediaJobStatus::Succeeded
    );
    let wrong = serde_json::to_vec(&json!({"id":"another-job","status":"processing"})).unwrap();
    let sig = signature(id, &timestamp, &wrong, key);
    assert_eq!(
        job.webhook(
            MediaWebhook {
                id,
                timestamp: &timestamp,
                signature: &sig,
                body: &wrong
            },
            &secret,
            now
        )
        .await
        .unwrap_err()
        .classification,
        ErrorKind::InvalidResponse
    );
    assert_eq!(
        job.retrieve(CancellationToken::new()).await.unwrap(),
        result
    );
    assert_eq!(state.calls.lock().unwrap().len(), 2);
    task.abort();
}

#[tokio::test]
async fn malformed_poll_identity_or_status_stops_and_cleans_up_the_original_job() {
    for bad in [
        json!({"id":"different-job","status":"processing"}),
        json!({"id":"accepted-job","status":"unexpected"}),
    ] {
        let (url, state, task) = server().await;
        enqueue(
            &state,
            [
                Reply::json(update("replicate", "starting", Value::Null)),
                Reply::json(bad),
                Reply::json(update("replicate", "canceled", Value::Null)),
            ],
        );
        let job = submit(
            backend("replicate", &url, MediaKind::Image),
            MediaKind::Image,
        )
        .await;
        let error = MediaSubmission::Pending(job)
            .wait(CancellationToken::new(), None)
            .await
            .unwrap_err();
        assert_eq!(error.classification, ErrorKind::InvalidResponse);
        assert_eq!(error.request_id.as_deref(), Some("accepted-job"));
        assert!(state
            .calls
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .1
            .ends_with("accepted-job/cancel"));
        task.abort();
    }
}
#[tokio::test]
async fn failed_remote_cancellation_reports_uncertainty_and_does_not_double_cancel() {
    let (url, state, task) = server().await;
    enqueue(
        &state,
        [
            Reply::json(update("replicate", "starting", Value::Null)),
            Reply::error(StatusCode::SERVICE_UNAVAILABLE),
        ],
    );
    let job = submit(
        backend("replicate", &url, MediaKind::Image),
        MediaKind::Image,
    )
    .await;
    let token = CancellationToken::new();
    token.cancel();
    let error = MediaSubmission::Pending(job)
        .wait(token, None)
        .await
        .unwrap_err();
    assert_eq!(error.classification, ErrorKind::Cancelled);
    assert!(error.message.contains("Remote cancellation failed"));
    assert_eq!(error.request_id.as_deref(), Some("accepted-job"));
    tokio::task::yield_now().await;
    assert_eq!(state.calls.lock().unwrap().len(), 2);
    task.abort();
}
#[tokio::test]
async fn aggregate_output_bound_prevents_partial_publication() {
    let (url, state, task) = server().await;
    enqueue(
        &state,
        [
            Reply::json(update(
                "replicate",
                "succeeded",
                json!([format!("{url}/one"), format!("{url}/two")]),
            )),
            Reply::bytes("image/png", &[0; 64]),
            Reply::bytes("image/png", &[0; 64]),
        ],
    );
    let mut config = backend("replicate", &url, MediaKind::Image);
    config
        .options
        .extra
        .insert("maxOutputBytes".into(), json!(150));
    let mut job = submit(config, MediaKind::Image).await;
    assert_eq!(
        job.retrieve(CancellationToken::new())
            .await
            .unwrap_err()
            .classification,
        ErrorKind::Retrieval
    );
    assert!(!job.snapshot().outputs_retrieved);
    task.abort();
}
#[tokio::test]
async fn ambiguous_submission_is_not_replayed_even_with_retries_configured() {
    let (url, state, task) = server().await;
    enqueue(&state, [Reply::error(StatusCode::INTERNAL_SERVER_ERROR)]);
    let mut config = backend("replicate", &url, MediaKind::Image);
    config.options.max_retries = Some(10);
    let adapter = battersea_providers::builtin_registry()
        .create_media(config)
        .unwrap();
    let error = adapter
        .generate(
            request(MediaKind::Image),
            CancellationToken::new(),
            None,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.dispatch,
        battersea_model::adapter::error::DispatchState::Unknown
    );
    assert_eq!(state.calls.lock().unwrap().len(), 1);
    task.abort();
}
#[tokio::test]
async fn configured_model_mapping_and_callbacks_are_captured_in_the_exact_submit_body() {
    let (url, state, task) = server().await;
    enqueue(
        &state,
        [Reply::json(update("replicate", "canceled", Value::Null))],
    );
    let mut config = backend("replicate", &url, MediaKind::Image);
    config.model = "owner/model:abc123".into();
    config
        .options
        .extra
        .insert("webhookUrl".into(), json!("https://callback.example/jobs"));
    config
        .options
        .extra
        .insert("input".into(), json!({"format":"png"}));
    let adapter = battersea_providers::builtin_registry()
        .create_media(config)
        .unwrap();
    let mut input = request(MediaKind::Image);
    input.options.count = Some(2);
    let prepared = adapter.prepare(input.clone()).unwrap();
    let expected = prepared.body.clone();
    let _ = adapter
        .submit(prepared, CancellationToken::new(), None, None)
        .await
        .unwrap();
    let calls = state.calls.lock().unwrap();
    let body: Value = serde_json::from_slice(&calls[0].3).unwrap();
    assert_eq!(body, expected);
    assert_eq!(body["input"]["text"], input.prompt_text);
    assert_eq!(body["input"]["num_outputs"], input.options.count.unwrap());
    assert_eq!(calls[0].1, "/v1/predictions");
    assert!(body["webhook"].as_str().is_some());
    task.abort();
}

#[derive(Default)]
struct Reporter(Mutex<Vec<MediaGenerationActivityUpdate>>);
#[async_trait::async_trait]
impl MediaGenerationActivityReporter for Reporter {
    async fn report_activity(
        &self,
        update: MediaGenerationActivityUpdate,
    ) -> Result<(), battersea_model::EngineAdapterRequestError> {
        self.0.lock().unwrap().push(update);
        Ok(())
    }
}
#[tokio::test]
async fn runway_progress_is_attributed_to_each_requested_slot_and_bounded() {
    let (url, state, task) = server().await;
    let mut running = update("runway", "RUNNING", Value::Null);
    running["progress"] = json!(1.7);
    enqueue(
        &state,
        [
            Reply::json(update("runway", "PENDING", Value::Null)),
            Reply::json(running),
            Reply::error(StatusCode::NO_CONTENT),
        ],
    );
    let adapter = battersea_providers::builtin_registry()
        .create_media(backend("runway", &url, MediaKind::Image))
        .unwrap();
    let mut request = request(MediaKind::Image);
    request.options.count = Some(3);
    let reporter = Arc::new(Reporter::default());
    let MediaSubmission::Pending(mut job) = adapter
        .submit(
            adapter.prepare(request).unwrap(),
            CancellationToken::new(),
            Some(reporter.clone()),
            None,
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    job.poll(CancellationToken::new()).await.unwrap();
    let updates = reporter.0.lock().unwrap().clone();
    let progress: Vec<_> = updates.iter().filter(|u| u.progress.is_some()).collect();
    assert_eq!(progress.len(), 3);
    for (index, update) in progress.iter().enumerate() {
        assert_eq!(update.slot_index, Some(index as u8));
        assert!((0.0..=1.0).contains(&update.progress.unwrap()));
        assert_eq!(update.provider_job_id.as_deref(), Some(job.id()));
    }
    drop(updates);
    job.cancel().await.unwrap();
    task.abort();
}
#[tokio::test]
async fn malformed_accepted_submission_keeps_identity_and_disposes_remote_work() {
    let (url, state, task) = server().await;
    enqueue(
        &state,
        [
            Reply::json(update("replicate", "unknown", Value::Null)),
            Reply::json(update("replicate", "canceled", Value::Null)),
        ],
    );
    let adapter = battersea_providers::builtin_registry()
        .create_media(backend("replicate", &url, MediaKind::Image))
        .unwrap();
    let error = adapter
        .generate(
            request(MediaKind::Image),
            CancellationToken::new(),
            None,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(error.classification, ErrorKind::InvalidResponse);
    assert_eq!(error.request_id.as_deref(), Some("accepted-job"));
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if state.calls.lock().unwrap().len() == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(state
        .calls
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .1
        .ends_with("accepted-job/cancel"));
    task.abort();
}

#[tokio::test]
async fn asset_redirects_do_not_forward_provider_credentials_to_another_origin() {
    let (url, state, task) = server().await;
    let (other, other_state, other_task) = server().await;
    let mut redirect = Reply::error(StatusCode::FOUND);
    redirect.location = Some(format!("{other}/image"));
    enqueue(
        &state,
        [
            Reply::json(update(
                "replicate",
                "succeeded",
                json!([format!("{url}/redirect")]),
            )),
            redirect,
        ],
    );
    enqueue(&other_state, [Reply::bytes("image/png", b"output")]);
    let mut job = submit(
        backend("replicate", &url, MediaKind::Image),
        MediaKind::Image,
    )
    .await;
    job.retrieve(CancellationToken::new()).await.unwrap();
    assert!(state
        .calls
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .2
        .contains_key("authorization"));
    assert!(!other_state
        .calls
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .2
        .contains_key("authorization"));
    task.abort();
    other_task.abort();
}
