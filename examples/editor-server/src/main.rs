mod runtime;
mod upgrade;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use battersea_flow::{
    document::{inspect_document, load_document, DocumentInspection},
    FlowDocument,
};
use battersea_runtime::{ActivationHost, ExecutionError};
use clap::{Parser, Subcommand};
use runtime::{Application, Error};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;
#[derive(Parser)]
#[command(about = "Battersea example editor server and activation client")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Initialise an empty application directory with example nodes and a flow.
    Init { root: PathBuf },
    /// Serve the application API on loopback. Reads require prior initialisation.
    Serve {
        root: PathBuf,
        #[arg(long, default_value_t = 18180)]
        port: u16,
    },
    /// Start an activation using the same endpoint as the editor.
    Activate {
        #[arg(long, default_value = "http://127.0.0.1:18180")]
        server: String,
        flow: String,
        node: String,
        text: String,
    },
    /// Inspect a saved flow's explicit upgrade and revision without changing it.
    InspectUpgrade {
        #[arg(long, default_value = "http://127.0.0.1:18180")]
        server: String,
        flow: String,
    },
    /// Upgrade a saved flow at the inspected revision, retaining its original.
    Upgrade {
        #[arg(long, default_value = "http://127.0.0.1:18180")]
        server: String,
        flow: String,
        #[arg(long)]
        expected_revision: String,
    },
    /// Read an activation and its ordered diagnostics.
    Inspect {
        #[arg(long, default_value = "http://127.0.0.1:18180")]
        server: String,
        id: String,
    },
    /// Request cancellation of an accepted activation.
    Cancel {
        #[arg(long, default_value = "http://127.0.0.1:18180")]
        server: String,
        id: String,
    },
}
#[derive(Clone)]
struct App {
    runtime: Arc<Application>,
    runs: Arc<Mutex<HashMap<String, LiveRun>>>,
    writes: Arc<Mutex<()>>,
}
struct LiveRun {
    status: String,
    token: CancellationToken,
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (
            if self.code == "invalid_request" {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            },
            Json(json!({"error":self.to_string()})),
        )
            .into_response()
    }
}
fn io(e: impl std::fmt::Display) -> Error {
    Error::internal(e.to_string())
}
fn key(value: &str) -> Result<&str, Error> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Err(Error::invalid_request("invalid key"))
    } else {
        Ok(value)
    }
}
fn initialise(root: &std::path::Path) -> Result<(), Error> {
    std::fs::create_dir(root).map_err(io)?;
    std::fs::create_dir(root.join("flows")).map_err(io)?;
    std::fs::create_dir(root.join("runs")).map_err(io)?;
    std::fs::write(
        root.join("nodes.json"),
        include_str!("../fixtures/nodes.json"),
    )
    .map_err(io)?;
    std::fs::write(
        root.join("flows/example.json"),
        include_str!("../fixtures/example.json"),
    )
    .map_err(io)
}
fn read(app: &App, name: &str) -> Result<FlowDocument, Error> {
    load_document(
        &std::fs::read_to_string(
            app.runtime
                .root
                .join("flows")
                .join(format!("{}.json", key(name)?)),
        )
        .map_err(io)?,
    )
    .map_err(io)
}
async fn catalogue(State(app): State<App>) -> Json<Value> {
    Json(json!(app.runtime.catalog.entries().collect::<Vec<_>>()))
}
async fn list(State(app): State<App>) -> Result<Json<Value>, Error> {
    let mut entries = vec![];
    for entry in std::fs::read_dir(app.runtime.root.join("flows")).map_err(io)? {
        let entry = entry.map_err(io)?;
        if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let name = entry
            .path()
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let source = std::fs::read_to_string(entry.path()).map_err(io)?;
        let summary = match inspect_document(&source) {
            Ok(DocumentInspection::Supported(flow)) => {
                json!({"flow_key": name, "title": flow.title})
            }
            Ok(DocumentInspection::Incompatible { version, title, .. }) => {
                json!({"flow_key": name, "title": title, "unavailable_reason": format!("Unsupported document version {version}")})
            }
            Err(error) => {
                json!({"flow_key": name, "title": name, "unavailable_reason": error.to_string()})
            }
        };
        entries.push(summary);
    }
    entries.sort_by(|a, b| a["flow_key"].as_str().cmp(&b["flow_key"].as_str()));
    Ok(Json(json!(entries)))
}
async fn read_flow(
    State(app): State<App>,
    Path(name): Path<String>,
) -> Result<Json<FlowDocument>, Error> {
    Ok(Json(read(&app, &name)?))
}
async fn validate(State(app): State<App>, Json(flow): Json<FlowDocument>) -> Json<Value> {
    Json(json!(app.runtime.catalog.validate(&flow)))
}
async fn save(
    State(app): State<App>,
    Json(flow): Json<FlowDocument>,
) -> Result<Json<Value>, Error> {
    let _guard = app.writes.lock().unwrap();
    key(&flow.flow_key)?;
    let validation = app.runtime.catalog.validate(&flow);
    if !validation.valid {
        return Err(Error::invalid_request(format!("{:?}", validation.issues)));
    }
    let path = app
        .runtime
        .root
        .join("flows")
        .join(format!("{}.json", flow.flow_key));
    match std::fs::read_to_string(&path) {
        Ok(source) => {
            load_document(&source).map_err(|e| Error::invalid_request(e.to_string()))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io(error)),
    }
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, serde_json::to_vec_pretty(&flow).map_err(io)?).map_err(io)?;
    std::fs::rename(tmp, path).map_err(io)?;
    Ok(Json(json!({"flow":flow})))
}
#[derive(Deserialize)]
struct CloneRequest {
    current_flow_key: String,
    next_flow_key: String,
    next_title: String,
}
async fn clone_flow(
    State(app): State<App>,
    Json(request): Json<CloneRequest>,
) -> Result<Json<Value>, Error> {
    let _guard = app.writes.lock().unwrap();
    let mut flow = read(&app, &request.current_flow_key)?;
    key(&request.next_flow_key)?;
    let path = app
        .runtime
        .root
        .join("flows")
        .join(format!("{}.json", request.next_flow_key));
    flow.flow_key = request.next_flow_key;
    flow.title = request.next_title;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io)?;
    file.write_all(&serde_json::to_vec_pretty(&flow).map_err(io)?)
        .map_err(io)?;
    Ok(Json(json!({"flow":flow})))
}
async fn delete_flow(
    State(app): State<App>,
    Path(name): Path<String>,
) -> Result<Json<Value>, Error> {
    let _guard = app.writes.lock().unwrap();
    std::fs::remove_file(
        app.runtime
            .root
            .join("flows")
            .join(format!("{}.json", key(&name)?)),
    )
    .map_err(io)?;
    Ok(Json(json!({})))
}
#[derive(Deserialize)]
struct Activation {
    flow_key: String,
    node_id: String,
    parameters: HashMap<String, Value>,
}
async fn activate(
    State(app): State<App>,
    Json(request): Json<Activation>,
) -> Result<Json<Value>, Error> {
    let flow = read(&app, &request.flow_key)?;
    let id = uuid::Uuid::new_v4().to_string();
    let mut run = app.runtime.run(id.clone(), &flow)?;
    app.runtime.validate_activation(&run)?;
    if !flow.nodes.iter().any(|n| n.id == request.node_id) {
        return Err(Error::invalid_request("node is not in the flow"));
    }
    let token = CancellationToken::new();
    app.runs.lock().unwrap().insert(
        id.clone(),
        LiveRun {
            status: "running".into(),
            token: token.clone(),
        },
    );
    let returned = json!({"id":id,"status":"running"});
    let (accepted, receiver) = tokio::sync::oneshot::channel();
    app.runtime
        .acceptance
        .lock()
        .unwrap()
        .insert(id.clone(), accepted);
    tokio::spawn(async move {
        let result = app
            .runtime
            .run_activation(&mut run, &request.node_id, request.parameters, token)
            .await;
        if let Some(sender) = app.runtime.acceptance.lock().unwrap().remove(&id) {
            let _ = sender.send(Err(result.as_ref().err().map_or_else(
                || "Activation did not publish acceptance".to_string(),
                ToString::to_string,
            )));
        }
        if let Err(error) = app.runtime.publish_retained_outcome(&run).await {
            *app.runtime.observation_error.lock().unwrap() = Some(error.to_string());
        }
        let status = match result {
            Ok(()) => "succeeded",
            Err(ref e) if e.code() == "cancelled" => "cancelled",
            Err(_) => "failed",
        };
        if let Some(record) = app.runs.lock().unwrap().get_mut(&id) {
            record.status = status.into();
        }
    });
    receiver.await.map_err(io)?.map_err(Error::internal)?;
    Ok(Json(returned))
}
async fn inspect(State(app): State<App>, Path(id): Path<String>) -> Result<Json<Value>, Error> {
    key(&id)?;
    let runs = app.runs.lock().unwrap();
    let live = runs
        .get(&id)
        .ok_or_else(|| Error::invalid_request("unknown activation"))?;
    let view = app.runtime.view.lock().unwrap();
    let mut events = Vec::new();
    for delta in view.history() {
        for change in &delta.changes {
            if let battersea_telemetry::view::Change::Upsert { record } = change {
                if record.context.activation_id != id {
                    continue;
                }
                use battersea_telemetry::view::{PortAction, State as RuntimeState};
                let phase = match &record.state {
                    RuntimeState::Port {
                        action: PortAction::Receive,
                        ..
                    } => "flow.token.receive",
                    RuntimeState::Port {
                        action: PortAction::Emit,
                        ..
                    } => "flow.token.emit",
                    RuntimeState::Activation { .. } => "activation.state",
                    RuntimeState::Node { .. } => "flow.node.state",
                    _ => "runtime.state",
                };
                events.push(json!({"activation_id":id,"node_id":record.context.node_id,"sequence":delta.cursor.revision,"phase":phase,"detail":record.state}));
            }
        }
    }
    if live.status != "running" {
        events.push(json!({"activation_id":id,"sequence":view.snapshot().cursor.revision,"phase":live.status}));
    }
    Ok(Json(
        json!({"id":id,"status":live.status,"events":events,"observation_error":*app.runtime.observation_error.lock().unwrap()}),
    ))
}
#[derive(Deserialize)]
struct RuntimeQuery {
    epoch: Option<String>,
    revision: Option<String>,
}
async fn runtime_view(
    State(app): State<App>,
    Query(query): Query<RuntimeQuery>,
) -> Result<Json<Value>, Error> {
    if let Some(error) = app.runtime.observation_error.lock().unwrap().clone() {
        return Err(Error::internal(error));
    }
    let view = app.runtime.view.lock().unwrap();
    let update = match (query.epoch, query.revision) {
        (Some(epoch), Some(revision)) => {
            view.updates(&battersea_telemetry::view::Cursor { epoch, revision })
        }
        (None, None) => battersea_telemetry::view::Update::Resync {
            snapshot: view.snapshot(),
        },
        _ => {
            return Err(Error::invalid_request(
                "Runtime cursor requires epoch and revision",
            ))
        }
    };
    Ok(Json(serde_json::to_value(update).map_err(io)?))
}
async fn cancel(State(app): State<App>, Path(id): Path<String>) -> Result<Json<Value>, Error> {
    let runs = app.runs.lock().unwrap();
    let run = runs
        .get(&id)
        .ok_or_else(|| Error::invalid_request("unknown activation"))?;
    run.token.cancel();
    Ok(Json(json!({"id":id})))
}
fn router(app: App) -> Router {
    Router::new()
        .route("/api/runtime", get(runtime_view))
        .route("/api/catalogue", get(catalogue))
        .route("/api/flows", get(list).put(save))
        .route("/api/flows/clone", post(clone_flow))
        .route("/api/flows/{key}", get(read_flow).delete(delete_flow))
        .route(
            "/api/flows/{key}/upgrade",
            get(upgrade::inspect).post(upgrade::apply),
        )
        .route("/api/validate", post(validate))
        .route("/api/activations", post(activate))
        .route("/api/activations/{id}", get(inspect).delete(cancel))
        .with_state(app)
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Command::Init { root } => initialise(&root)?,
        Command::Serve { root, port } => {
            let app = App {
                runtime: Arc::new(Application::new(root)?),
                runs: Arc::default(),
                writes: Arc::default(),
            };
            let listener =
                tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
            axum::serve(listener, router(app)).await?;
        }
        Command::Activate {
            server,
            flow,
            node,
            text,
        } => {
            println!(
                "{}",
                reqwest::Client::new()
                    .post(format!("{server}/api/activations"))
                    .json(&json!({"flow_key":flow,"node_id":node,"parameters":{"text":text}}))
                    .send()
                    .await?
                    .error_for_status()?
                    .text()
                    .await?
            );
        }
        Command::InspectUpgrade { server, flow } => println!(
            "{}",
            reqwest::get(format!("{server}/api/flows/{}/upgrade", key(&flow)?))
                .await?
                .error_for_status()?
                .text()
                .await?
        ),
        Command::Upgrade {
            server,
            flow,
            expected_revision,
        } => println!(
            "{}",
            reqwest::Client::new()
                .post(format!("{server}/api/flows/{}/upgrade", key(&flow)?))
                .json(&json!({"expected_revision": expected_revision}))
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?
        ),
        Command::Inspect { server, id } => println!(
            "{}",
            reqwest::get(format!("{server}/api/activations/{id}"))
                .await?
                .error_for_status()?
                .text()
                .await?
        ),
        Command::Cancel { server, id } => println!(
            "{}",
            reqwest::Client::new()
                .delete(format!("{server}/api/activations/{id}"))
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?
        ),
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn custom_node_runs_and_capture_survives_completion_without_product_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("app");
        initialise(&root).unwrap();
        let app = Application::new(root.clone()).unwrap();
        let flow = load_document(include_str!("../fixtures/example.json")).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let mut run = app.run(id.clone(), &flow).unwrap();
        let text = "A supplied input";
        app.run_activation(
            &mut run,
            "text",
            HashMap::from([("text".into(), json!(text))]),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            run.output
                .iter()
                .map(|value| (**value).clone())
                .collect::<Vec<_>>(),
            vec![json!(text.to_uppercase())]
        );
        let record: Value =
            serde_json::from_slice(&std::fs::read(root.join(format!("runs/{id}.json"))).unwrap())
                .unwrap();
        assert_eq!(record["capture"][0]["value"], *run.output[0]);
    }
    #[test]
    fn read_paths_preserve_documents_and_reject_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("app");
        initialise(&root).unwrap();
        let app = App {
            runtime: Arc::new(Application::new(root.clone()).unwrap()),
            runs: Arc::default(),
            writes: Arc::default(),
        };
        let path = root.join("flows/example.json");
        let before = std::fs::read(&path).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        read(&app, "example").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
        assert!(read(&app, "../nodes").is_err());
    }
    #[tokio::test]
    async fn incompatible_catalogue_entries_remain_visible_and_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("app");
        initialise(&root).unwrap();
        let app = App {
            runtime: Arc::new(Application::new(root.clone()).unwrap()),
            runs: Arc::default(),
            writes: Arc::default(),
        };
        let path = root.join("flows/future.json");
        let bytes = br#"{"version":999,"flow_key":"future","title":"Future document","nodes":"a different format"}"#;
        std::fs::write(&path, bytes).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let entries = list(State(app.clone())).await.unwrap().0;
        let entry = entries
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["flow_key"] == "future")
            .expect("retained entry");
        assert!(entry["unavailable_reason"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        assert!(read(&app, "future").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(
            std::fs::metadata(path).unwrap().modified().unwrap(),
            modified
        );
    }
    #[tokio::test]
    async fn acceptance_precedes_the_reply_and_cancellation_retains_the_same_identity() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("app");
        initialise(&root).unwrap();
        let app = App {
            runtime: Arc::new(Application::new(root.clone()).unwrap()),
            runs: Arc::default(),
            writes: Arc::default(),
        };
        let mut flow = read(&app, "example").unwrap();
        flow.nodes[0]
            .parameter_values
            .insert("delay_ms".into(), json!(10_000));
        let _ = save(State(app.clone()), Json(flow)).await.unwrap();
        let accepted = activate(
            State(app.clone()),
            Json(Activation {
                flow_key: "example".into(),
                node_id: "text".into(),
                parameters: HashMap::from([("text".into(), json!("cancel this"))]),
            }),
        )
        .await
        .unwrap()
        .0;
        let id = accepted["id"].as_str().unwrap().to_string();
        let path = root.join(format!("runs/{id}.json"));
        let record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(record["id"], id);
        let _ = cancel(State(app.clone()), Path(id.clone())).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let snapshot = inspect(State(app.clone()), Path(id.clone()))
                    .await
                    .unwrap()
                    .0;
                assert_eq!(snapshot["id"], id);
                if snapshot["status"] == "cancelled" {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let record: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(record["id"], id);
    }
}
