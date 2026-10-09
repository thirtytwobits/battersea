use super::*;
use battersea_model::media::{MediaBackendCapabilities, MediaCapability, MediaGenerationHints};
use serde_json::json;
use std::io::{Read, Write};

fn backend(provider: &str) -> MediaBackendConfig {
    MediaBackendConfig {
        id: provider.into(),
        provider: provider.into(),
        label: provider.into(),
        capability: MediaCapability::ImageGeneration,
        enabled: true,
        model: "test-model".into(),
        endpoint: String::new(),
        capabilities: MediaBackendCapabilities {
            supported_media_parameters: match provider {
                "openai" => "backend count size aspect_ratio",
                "google" => "backend aspect_ratio",
                "runway" => "backend count aspect_ratio seed duration_seconds",
                _ => unreachable!(),
            }
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
            supports_partial_image_streaming: provider == "openai",
            ..MediaBackendCapabilities::mock()
        },
        options: EngineBackendOptions {
            timeout_ms: Some(2000),
            extra: BTreeMap::from([
                ("quality".into(), json!("high")),
                ("outputFormat".into(), json!("png")),
                ("defaultCount".into(), json!(1)),
                ("partialImages".into(), json!(3)),
                ("defaultMimeType".into(), json!("image/png")),
                ("imageSize".into(), json!("2K")),
                ("pollIntervalMs".into(), json!(1)),
                ("maxPollRetryCount".into(), json!(0)),
                ("maxPromptChars".into(), json!(1000)),
                ("defaultAspectRatio".into(), json!("1024:1024")),
            ]),
            ..Default::default()
        },
        auth: EngineAuthConfig {
            auth_type: "bearer".into(),
            api_key_env: String::new(),
            header: None,
            version_header: Some("X-Runway-Version".into()),
            version: Some("2024-11-06".into()),
            has_api_key: false,
            api_key: None,
        },
        short_description: String::new(),
        long_description: String::new(),
    }
}

#[tokio::test]
async fn runway_pending_job_keeps_remote_identity_through_completion_and_cancellation() {
    use axum::{
        extract::State,
        routing::{get, post},
        Json, Router,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    const JOB_ID: &str = "accepted-fixture-job";
    const ASSET_URL: &str = "https://example.invalid/fixture.png";
    let cancellations = Arc::new(AtomicUsize::new(0));
    let router = Router::new()
        .route(
            "/text_to_image",
            post(|| async { Json(json!({"id": JOB_ID})) }),
        )
        .route(
            &format!("/tasks/{JOB_ID}"),
            get(|| async {
                Json(json!({"id": JOB_ID, "status": "SUCCEEDED", "output": [ASSET_URL]}))
            })
            .delete(|State(count): State<Arc<AtomicUsize>>| async move {
                count.fetch_add(1, Ordering::SeqCst);
                axum::http::StatusCode::NO_CONTENT
            }),
        )
        .with_state(cancellations.clone());
    let listener = tokio::net::TcpListener::bind(crate::test_endpoints::TEST_BIND_ADDRESS)
        .await
        .unwrap();
    let mut config = backend("runway");
    config.endpoint = format!("http://{}", listener.local_addr().unwrap());
    config.capabilities.supports_partial_image_streaming = false;
    config.auth.api_key = Some("fixture-key".into());
    config.auth.has_api_key = true;
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let adapter = crate::builtin_registry().create_media(config).unwrap();
    for mode in ["complete", "cancel", "deadline"] {
        let prepared = adapter
            .prepare(MediaRenderRequest {
                kind: MediaKind::Image,
                prompt_text: "Fixture input".into(),
                negative_prompt: None,
                references: vec![],
                options: MediaGenerationHints::default(),
            })
            .unwrap();
        let submission = adapter
            .submit(prepared, CancellationToken::new(), None, None)
            .await
            .unwrap();
        let MediaSubmission::Pending(job) = &submission else {
            panic!("remote job must remain pending")
        };
        assert_eq!(job.id(), JOB_ID);
        let token = CancellationToken::new();
        if mode == "cancel" {
            token.cancel();
        }
        let result = submission
            .wait(
                token,
                Some(if mode == "deadline" {
                    tokio::time::Instant::now() - Duration::from_secs(1)
                } else {
                    tokio::time::Instant::now() + Duration::from_secs(5)
                }),
            )
            .await;
        if mode != "complete" {
            assert_eq!(
                result.unwrap_err().classification,
                if mode == "cancel" {
                    battersea_model::ErrorKind::Cancelled
                } else {
                    battersea_model::ErrorKind::Timeout
                }
            );
            assert_eq!(
                cancellations.load(Ordering::SeqCst),
                if mode == "cancel" { 1 } else { 2 }
            );
        } else {
            let result = result.unwrap();
            assert!(result.assets.iter().any(|asset| asset.url == ASSET_URL));
            assert_eq!(cancellations.load(Ordering::SeqCst), 0);
        }
    }
    server.abort();
}

#[test]
fn unsupported_media_parameters_fail_before_adapter_creation_or_submission() {
    let registry = crate::builtin_registry();
    let mut config = backend("openai");
    config
        .capabilities
        .supported_media_parameters
        .push("seed".into());
    assert_eq!(
        registry.create_media(config).err().unwrap().classification,
        battersea_model::ErrorKind::InvalidRequest
    );
    let request = MediaRenderRequest {
        kind: MediaKind::Image,
        prompt_text: "Fixture input".into(),
        negative_prompt: None,
        references: vec![],
        options: MediaGenerationHints {
            seed: Some(7),
            ..Default::default()
        },
    };
    assert_eq!(
        registry
            .prepare_media(&backend("openai"), request)
            .unwrap_err()
            .classification,
        battersea_model::ErrorKind::InvalidRequest
    );
}

/// How long the capture server waits for the submit under test.
///
/// A hang guard, not an assertion about how quickly a request is made. This
/// runs alongside the rest of the suite, and an accept that returns in
/// milliseconds on an idle machine has taken seconds under that load.
const SUBMIT_DEADLINE: Duration = Duration::from_secs(60);

/// Receive one real HTTP submit, then reject it to prevent polling or asset retrieval.
fn capture_server() -> (String, std::thread::JoinHandle<(String, Value)>) {
    let listener = std::net::TcpListener::bind(crate::test_endpoints::TEST_BIND_ADDRESS).unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + SUBMIT_DEADLINE;
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("No submit received: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream.set_read_timeout(Some(SUBMIT_DEADLINE)).unwrap();
        let mut bytes = Vec::new();
        let (headers, body) = loop {
            let mut chunk = [0; 4096];
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0, "incomplete HTTP request");
            bytes.extend_from_slice(&chunk[..count]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|n| n.trim().parse::<usize>().ok())
                    })
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    break (
                        headers,
                        serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap(),
                    );
                }
            }
        };
        stream
            .write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        (headers.lines().next().unwrap().to_string(), body)
    });
    (format!("http://{address}"), handle)
}

#[tokio::test]
async fn credential_free_capture_matches_every_image_transport_submit() {
    for (provider, router) in [
        ("openai", false),
        ("google", false),
        ("runway", false),
        ("runway", true),
    ] {
        for count in [1, 2] {
            let mut backend = backend(provider);
            if router {
                backend.options.transport = Some("model-router".into());
                backend
                    .options
                    .extra
                    .insert("configId".into(), json!("test-router"));
            }
            let request = MediaRenderRequest {
                kind: MediaKind::Image,
                prompt_text: "  Snow 雪\nwith a lantern.  ".into(),
                negative_prompt: None,
                references: Vec::new(),
                options: MediaGenerationHints {
                    count: (provider != "google").then_some(count),
                    aspect_ratio: Some("1:1".into()),
                    ..Default::default()
                },
            };
            let captured =
                prepare_media_request(&backend, request.clone()).expect("no credentials required");
            assert!(!captured.provider_prompt_text().is_empty());
            let (endpoint, server) = capture_server();
            backend.endpoint = endpoint;
            backend.auth.api_key = Some("local-test-key".into());
            backend.auth.has_api_key = true;
            let adapter = match provider {
                "openai" => create_openai_media_generator(backend),
                "google" => create_google_media_generator(backend),
                "runway" => create_runway_media_generator(backend),
                _ => unreachable!(),
            }
            .unwrap();
            assert!(adapter
                .generate(request, CancellationToken::new(), None, None)
                .await
                .is_err());
            let (start, sent) = server.join().unwrap();
            assert_eq!(
                &sent,
                captured.body(),
                "prepared values must be the actual HTTP submit for {provider}, router={router}"
            );
            assert!(start.contains(&format!("/{} ", captured.path().trim_start_matches('/'))));
            assert!(!serde_json::to_string(&sent)
                .unwrap()
                .contains("local-test-key"));
        }
    }
}
