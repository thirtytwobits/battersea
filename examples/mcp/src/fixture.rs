//! Copyright (c) Scott A Dixon
//! Controlled servers; the effect ledger belongs to this example application.
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use clap::ValueEnum;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum Fault {
    #[default]
    None,
    LostReply,
    Disconnect,
    ToolError,
    ProtocolError,
    Oversized,
    InvalidOutput,
}

pub fn effect_count(path: &Path) -> Result<u64> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text.parse()?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.into()),
    }
}

pub struct McpFixture {
    pub tools: Mutex<Vec<Value>>,
    pub fault: Fault,
    ledger: PathBuf,
    effects: Mutex<()>,
}
impl McpFixture {
    pub fn new(ledger: PathBuf, fault: Fault) -> Arc<Self> {
        Arc::new(Self {
            ledger,
            fault,
            effects: Mutex::new(()),
            tools: Mutex::new(vec![json!({
                "name":"write", "description":"Record one controlled effect",
                "inputSchema":{"type":"object","properties":{"value":{"type":"integer"}},"required":["value"],"additionalProperties":false},
                "outputSchema":{"type":"object","properties":{"count":{"type":"integer"}},"required":["count"]},
                "_meta":{"io.github.thirtytwobits.battersea/profiles":["forged"]}
            })]),
        })
    }
    pub fn response(&self, request: &Value) -> Result<Option<Value>> {
        let result = match request["method"].as_str() {
            Some("server/discover") => {
                json!({"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"private"})
            }
            Some("tools/list") => json!({"tools":self.tools.lock().unwrap().clone()}),
            Some("tools/call") => {
                let _lock = self.effects.lock().unwrap();
                let count = effect_count(&self.ledger)? + 1;
                std::fs::write(&self.ledger, count.to_string())?;
                match self.fault {
                    Fault::LostReply | Fault::Disconnect => return Ok(None),
                    Fault::ProtocolError => {
                        return Ok(Some(
                            json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32603,"message":"private server diagnostic"}}),
                        ))
                    }
                    _ => (),
                }
                json!({"content":[{"type":"text","text":if self.fault == Fault::Oversized { "x".repeat(128 * 1024) } else { "effect recorded".into() }},{"type":"resource_link","uri":"https://example.invalid/receipt","name":"Receipt"}],
                    "structuredContent":if self.fault == Fault::InvalidOutput { json!({"count":"invalid"}) } else { json!({"count":count}) },
                    "isError":self.fault == Fault::ToolError,
                    "_meta":{"hostReceipt":"private-receipt"}})
            }
            _ => return Ok(None),
        };
        Ok(Some(
            json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
        ))
    }
}

/// Serve the host-launched child using newline-delimited JSON on stdin/stdout.
pub async fn serve_stdio(ledger: PathBuf, fault: Fault) -> Result<()> {
    let fixture = McpFixture::new(ledger, fault);
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut output = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        let request: Value = serde_json::from_str(&line)?;
        if let Some(response) = fixture.response(&request)? {
            output.write_all(format!("{response}\n").as_bytes()).await?;
            output.flush().await?;
        } else if fault == Fault::Disconnect && request["method"] == "tools/call" {
            break;
        }
    }
    Ok(())
}

pub struct HttpServer {
    pub endpoint: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for HttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn listen(app: Router) -> Result<HttpServer> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Ok(HttpServer { endpoint, task })
}

pub async fn mcp_http(fixture: Arc<McpFixture>, sse: bool) -> Result<HttpServer> {
    async fn reply(
        State((fixture, sse)): State<(Arc<McpFixture>, bool)>,
        Json(request): Json<Value>,
    ) -> Response {
        let response = fixture.response(&request).expect("fixture ledger");
        if request["method"] == "tools/call" && fixture.fault == Fault::LostReply {
            return std::future::pending().await;
        }
        if request["method"] == "tools/call" && fixture.fault == Fault::Disconnect {
            let stream = futures_util::stream::once(async {
                Err::<String, _>(std::io::Error::other("fixture disconnect"))
            });
            return Response::builder()
                .header("content-type", "application/json")
                .body(axum::body::Body::from_stream(stream))
                .unwrap();
        }
        match response {
            Some(value) if sse => (
                [("content-type", "text/event-stream")],
                format!("data: {value}\n\n"),
            )
                .into_response(),
            Some(value) => Json(value).into_response(),
            None => StatusCode::ACCEPTED.into_response(),
        }
    }
    listen(
        Router::new()
            .route("/mcp", post(reply))
            .with_state((fixture, sse)),
    )
    .await
}

pub struct ProviderFixture {
    pub requests: Mutex<Vec<Value>>,
    pub arguments: Value,
    pub reject_continuation_once: bool,
}
impl ProviderFixture {
    pub fn new(arguments: Value, reject_continuation_once: bool) -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::new(Vec::new()),
            arguments,
            reject_continuation_once,
        })
    }
}
pub async fn provider_http(fixture: Arc<ProviderFixture>) -> Result<HttpServer> {
    async fn reply(
        State(fixture): State<Arc<ProviderFixture>>,
        Json(request): Json<Value>,
    ) -> Response {
        let turn = {
            let mut requests = fixture.requests.lock().unwrap();
            requests.push(request);
            requests.len()
        };
        if turn == 2 && fixture.reject_continuation_once {
            return (StatusCode::TOO_MANY_REQUESTS, Json(json!({}))).into_response();
        }
        let message = if turn == 1 {
            json!({"role":"assistant","content":null,"tool_calls":[{"id":"effect-1","type":"function","function":{"name":"remote_write","arguments":fixture.arguments.to_string()}}]})
        } else {
            json!({"role":"assistant","content":"Remote result received"})
        };
        Json(json!({"choices":[{"message":message,"finish_reason":if turn == 1 { "tool_calls" } else { "stop" }}],"usage":{"prompt_tokens":2,"completion_tokens":2,"total_tokens":4}})).into_response()
    }
    listen(
        Router::new()
            .route("/v1/chat/completions", post(reply))
            .with_state(fixture),
    )
    .await
}
