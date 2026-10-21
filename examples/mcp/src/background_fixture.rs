//! Copyright (c) Scott A Dixon
//! Controlled background jobs with explicit retrieval and cancellation.
use crate::fixture::{listen, HttpServer, ProviderFixture, Result};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

pub struct BackgroundFixture {
    pub provider: Arc<ProviderFixture>,
    pub operations: Mutex<Vec<String>>,
    pub retrievals: Mutex<Vec<Vec<String>>>,
    pub pending: AtomicBool,
    pub wrong_identity: AtomicBool,
    pub fail_poll: AtomicBool,
    pub fail_cancel: AtomicBool,
}
impl BackgroundFixture {
    pub fn new(provider: Arc<ProviderFixture>) -> Arc<Self> {
        Arc::new(Self {
            provider,
            operations: Mutex::new(vec![]),
            retrievals: Mutex::new(vec![]),
            pending: false.into(),
            wrong_identity: false.into(),
            fail_poll: false.into(),
            fail_cancel: false.into(),
        })
    }
    fn response(&self, id: &str, status: &str) -> Value {
        let output = if status != "completed" {
            json!([])
        } else if id == "job-1" {
            json!([
                {"type":"reasoning","id":"reasoning-1","summary":[],"encrypted_content":"opaque-reasoning"},
                {"type":"function_call","id":"function-1","call_id":"effect-1","name":"remote_write","arguments":self.provider.arguments.to_string(),"status":"completed","vendor":{"signature":"retained-call"}}
            ])
        } else {
            json!([{"type":"message","id":"message-2","role":"assistant","status":"completed","content":[{"type":"output_text","text":self.provider.final_text,"annotations":[]}]}])
        };
        json!({"id":id,"object":"response","created_at":1,"model":"fixture","background":true,"status":status,"output":output})
    }
}
pub async fn serve(fixture: Arc<BackgroundFixture>) -> Result<HttpServer> {
    async fn create(State(f): State<Arc<BackgroundFixture>>, Json(body): Json<Value>) -> Response {
        f.operations.lock().unwrap().push("create".into());
        let count = {
            let mut requests = f.provider.requests.lock().unwrap();
            requests.push(body);
            requests.len()
        };
        if count == 2 && f.provider.reject_continuation_once {
            return StatusCode::TOO_MANY_REQUESTS.into_response();
        }
        Json(f.response(&format!("job-{count}"), "queued")).into_response()
    }
    async fn poll(
        State(f): State<Arc<BackgroundFixture>>,
        Path(id): Path<String>,
        Query(query): Query<Vec<(String, String)>>,
    ) -> Response {
        let includes: Vec<_> = query
            .into_iter()
            .filter_map(|(key, value)| (key == "include[]").then_some(value))
            .collect();
        f.retrievals.lock().unwrap().push(includes.clone());
        f.operations.lock().unwrap().push(format!("poll:{id}"));
        if f.fail_poll.load(Ordering::SeqCst) {
            return StatusCode::BAD_REQUEST.into_response();
        }
        let status = if f.pending.load(Ordering::SeqCst) {
            "in_progress"
        } else {
            "completed"
        };
        let mut response = f.response(
            if f.wrong_identity.load(Ordering::SeqCst) {
                "wrong-job"
            } else {
                &id
            },
            status,
        );
        for output in response["output"].as_array_mut().unwrap() {
            if output["type"] == "reasoning"
                && !includes.iter().any(|v| v == "reasoning.encrypted_content")
            {
                output.as_object_mut().unwrap().remove("encrypted_content");
            }
            if output["type"] == "message"
                && includes.iter().any(|v| v == "message.output_text.logprobs")
            {
                output["content"][0]["logprobs"] =
                    json!([{"token":"ok","logprob":-0.1,"bytes":[111,107],"top_logprobs":[]}]);
            }
        }
        Json(response).into_response()
    }
    async fn cancel(State(f): State<Arc<BackgroundFixture>>, Path(id): Path<String>) -> Response {
        f.operations.lock().unwrap().push(format!("cancel:{id}"));
        if f.fail_cancel.load(Ordering::SeqCst) {
            return StatusCode::BAD_REQUEST.into_response();
        }
        Json(f.response(&id, "cancelled")).into_response()
    }
    listen(
        Router::new()
            .route("/v1/responses", post(create))
            .route("/v1/responses/{id}", get(poll))
            .route("/v1/responses/{id}/cancel", post(cancel))
            .with_state(fixture),
    )
    .await
}
