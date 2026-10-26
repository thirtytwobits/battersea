mod ops;
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
use runtime::{codes, Application, Error};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;
#[derive(Parser)]
#[command(
    about = "Battersea example editor server and the CLI assembled from its operations",
    after_help = "Any other command is one of the daemon's operations, such as `battersea run \
                  activate`. With a daemon running, `battersea --help` lists them."
)]
struct Cli {
    /// The application directory whose endpoint record names the running daemon.
    #[arg(long)]
    root: Option<PathBuf>,
    /// The daemon's MCP endpoint, instead of the endpoint record's.
    #[arg(long, env = "BATTERSEA_ENDPOINT")]
    endpoint: Option<String>,
    /// The bearer token, instead of the endpoint record's.
    #[arg(long, env = "BATTERSEA_TOKEN", hide_env_values = true)]
    token: Option<String>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Initialise an empty application directory with example nodes, a flow and local tokens.
    Init { root: PathBuf },
    /// Serve the application API and the operations' MCP endpoint on loopback, and write the
    /// endpoint record. Reads require prior initialisation.
    Serve {
        root: PathBuf,
        /// The loopback port; 0 chooses a free one.
        #[arg(long, default_value_t = 18180)]
        port: u16,
    },
    #[command(external_subcommand)]
    Operation(Vec<OsString>),
}
/// The issuer of the example's local tokens.
const ISSUER: &str = "urn:battersea:editor-server-example";
/// The endpoint record the daemon writes in its application directory.
const ENDPOINT_RECORD: &str = "daemon.json";
/// One local token's grant.
#[derive(Serialize, Deserialize)]
struct Grant {
    subject: String,
    scopes: Vec<String>,
}
/// The operator's token, holding every scope the example's operations require, and the agent's,
/// holding none of the administrative ones.
fn local_tokens() -> BTreeMap<String, Grant> {
    let grant = |subject: &str, scopes: &[&str]| Grant {
        subject: subject.into(),
        scopes: scopes.iter().map(|scope| scope.to_string()).collect(),
    };
    BTreeMap::from([
        (
            uuid::Uuid::new_v4().to_string(),
            grant(
                "operator",
                &[
                    "battersea:flows:read",
                    "battersea:flows:admin",
                    "battersea:runs:read",
                    "battersea:runs:write",
                    "battersea:runs:admin",
                ],
            ),
        ),
        (
            uuid::Uuid::new_v4().to_string(),
            grant(
                "agent",
                &[
                    "battersea:flows:read",
                    "battersea:runs:read",
                    "battersea:runs:write",
                ],
            ),
        ),
    ])
}
/// Writes `value` to `path`, readable only by its owner, replacing it atomically.
fn write_private(path: &std::path::Path, value: &Value) -> Result<(), Error> {
    use std::io::Write;
    let pending = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&pending).map_err(io)?;
    file.write_all(&serde_json::to_vec_pretty(value).map_err(io)?)
        .map_err(io)?;
    file.sync_all().map_err(io)?;
    std::fs::rename(pending, path).map_err(io)
}
/// The application's local tokens, created on first use for a directory initialised without them.
fn tokens(root: &std::path::Path) -> Result<BTreeMap<String, Grant>, Error> {
    let path = root.join("tokens.json");
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let tokens = local_tokens();
            write_private(&path, &json!(tokens))?;
            Ok(tokens)
        }
        Err(error) => Err(io(error)),
    }
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
        Err(Error::invalid_request(codes::INVALID_KEY, "invalid key"))
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
    .map_err(io)?;
    write_private(&root.join("tokens.json"), &json!(local_tokens()))
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
        return Err(Error::invalid_request(
            codes::INVALID_FLOW,
            format!("{:?}", validation.issues),
        ));
    }
    let path = app
        .runtime
        .root
        .join("flows")
        .join(format!("{}.json", flow.flow_key));
    match std::fs::read_to_string(&path) {
        Ok(source) => {
            load_document(&source)
                .map_err(|e| Error::invalid_request(codes::INVALID_DOCUMENT, e.to_string()))?;
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
/// Starts an activation and returns its identity once the runtime has accepted it.
async fn start(app: &App, request: Activation) -> Result<String, Error> {
    let flow = read(app, &request.flow_key)?;
    let id = uuid::Uuid::new_v4().to_string();
    let mut run = app.runtime.run(id.clone(), &flow)?;
    app.runtime.validate_activation(&run)?;
    if !flow.nodes.iter().any(|n| n.id == request.node_id) {
        return Err(Error::invalid_request(
            battersea_runtime::error_codes::UNKNOWN_NODE,
            "node is not in the flow",
        ));
    }
    let token = CancellationToken::new();
    app.runs.lock().unwrap().insert(
        id.clone(),
        LiveRun {
            status: "running".into(),
            token: token.clone(),
        },
    );
    let (accepted, receiver) = tokio::sync::oneshot::channel();
    app.runtime
        .acceptance
        .lock()
        .unwrap()
        .insert(id.clone(), accepted);
    let app = app.clone();
    let started = id.clone();
    tokio::spawn(async move {
        let id = started;
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
        app.runtime.changed.notify_waiters();
    });
    receiver.await.map_err(io)?.map_err(Error::internal)?;
    Ok(id)
}
async fn activate(
    State(app): State<App>,
    Json(request): Json<Activation>,
) -> Result<Json<Value>, Error> {
    let id = start(&app, request).await?;
    Ok(Json(json!({"id":id,"status":"running"})))
}
/// The position of one diagnostic in the runtime view: its delta's revision and its change's
/// index in that delta.
type Position = (u64, usize);
/// An activation's status, its retained diagnostics by position, and the revision the oldest
/// retained delta follows. Earlier deltas have been evicted from the view.
struct History {
    status: String,
    events: Vec<(Position, Value)>,
    follows: u64,
}
fn history(app: &App, id: &str) -> Result<History, Error> {
    key(id)?;
    let runs = app.runs.lock().unwrap();
    let live = runs
        .get(id)
        .ok_or_else(|| Error::invalid_request(codes::UNKNOWN_ACTIVATION, "unknown activation"))?;
    let view = app.runtime.view.lock().unwrap();
    let number = |revision: &str| revision.parse::<u64>().unwrap_or_default();
    let current = number(&view.snapshot().cursor.revision);
    let follows = view
        .history()
        .next()
        .map_or(current, |delta| number(&delta.base.revision));
    let mut events = Vec::new();
    for delta in view.history() {
        for (index, change) in delta.changes.iter().enumerate() {
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
                events.push((
                    (number(&delta.cursor.revision), index),
                    json!({"activation_id":id,"node_id":record.context.node_id,"sequence":delta.cursor.revision,"phase":phase,"detail":record.state}),
                ));
            }
        }
    }
    if live.status != "running" {
        events.push((
            (current, usize::MAX),
            json!({"activation_id":id,"sequence":current.to_string(),"phase":live.status}),
        ));
    }
    Ok(History {
        status: live.status.clone(),
        events,
        follows,
    })
}
/// An activation's status and its retained diagnostics in order, each carrying its identity.
fn snapshot(app: &App, id: &str) -> Result<(String, Vec<Value>), Error> {
    let history = history(app, id)?;
    Ok((
        history.status,
        history.events.into_iter().map(|(_, event)| event).collect(),
    ))
}
async fn inspect(State(app): State<App>, Path(id): Path<String>) -> Result<Json<Value>, Error> {
    let (status, events) = snapshot(&app, &id)?;
    Ok(Json(
        json!({"id":id,"status":status,"events":events,"observation_error":*app.runtime.observation_error.lock().unwrap()}),
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
                codes::INVALID_RUNTIME_CURSOR,
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
        .ok_or_else(|| Error::invalid_request(codes::UNKNOWN_ACTIVATION, "unknown activation"))?;
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
/// Serves the application API and the operations' MCP endpoint until interrupted.
async fn serve(root: PathBuf, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let tokens = Arc::new(tokens(&root)?);
    let app = App {
        runtime: Arc::new(Application::new(root.clone())?),
        runs: Arc::default(),
        writes: Arc::default(),
    };
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let audience = format!("http://{}", listener.local_addr()?);
    let host = Arc::new(ops::Host(app.clone()));
    let set = clapham::compose_with_options(
        [battersea_ops::BatterseaOps::new()
            .with_flows(host.clone())
            .with_runs(host)
            .contribution()],
        clapham::Bounds::default(),
        clapham::ComposeOptions {
            execution: clapham::ExecutionMode::Daemon,
            oauth: Some(clapham::OAuthResource {
                issuer: ISSUER.into(),
                audience: audience.clone(),
            }),
            ..clapham::ComposeOptions::default()
        },
    )?;
    let authority = {
        let (tokens, audience) = (tokens.clone(), audience.clone());
        move |credential: &clapham_mcp_server::Credential| {
            let token = credential.authorization()?.strip_prefix("Bearer ")?;
            let grant = tokens.get(token)?;
            Some(clapham::OAuthContext::from_verified(
                ISSUER,
                audience.as_str(),
                grant.subject.as_str(),
                grant.scopes.iter().map(String::as_str),
            ))
        }
    };
    let mcp = clapham_mcp_server::HttpServer::new(set, authority)?.into_router();
    let operator = tokens
        .iter()
        .find(|(_, grant)| grant.subject == "operator")
        .map(|(token, _)| token.clone())
        .ok_or("the application holds no operator token")?;
    write_private(
        &root.join(ENDPOINT_RECORD),
        &json!({"endpoint": format!("{audience}/mcp"), "token": operator}),
    )?;
    axum::serve(listener, router(app).merge(mcp)).await?;
    Ok(())
}
/// Runs one operation through Clapham's CLI against the daemon the endpoint names, and returns
/// its exit code. A refusal is one JSON line on stderr.
async fn operation(cli: Cli, args: Vec<OsString>) -> u8 {
    let usage = |message: String| {
        clapham::Refusal::not_dispatched("usage", clapham::Category::Usage, message)
    };
    let record = match &cli.root {
        Some(root) => std::fs::read(root.join(ENDPOINT_RECORD))
            .map_err(|error| format!("no endpoint record under {}: {error}", root.display()))
            .and_then(|bytes| {
                serde_json::from_slice::<Value>(&bytes).map_err(|error| error.to_string())
            })
            .map(Some),
        None => Ok(None),
    };
    let connected = async {
        let record = record.map_err(usage)?;
        let field = |name: &str| {
            record
                .as_ref()
                .and_then(|record| record[name].as_str())
                .map(String::from)
        };
        let endpoint = cli.endpoint.or_else(|| field("endpoint")).ok_or_else(|| {
            usage("name the daemon with --root, --endpoint or BATTERSEA_ENDPOINT".into())
        })?;
        let token = cli.token.or_else(|| field("token")).ok_or_else(|| {
            usage("supply a token with --root, --token or BATTERSEA_TOKEN".into())
        })?;
        let backend = clapham_cli::McpBackend::http(endpoint, token).await?;
        clapham_cli::Cli::connect("editor-server-example", backend, clapham::Bounds::default())
            .await
    };
    let assembled = match connected.await {
        Ok(assembled) => assembled,
        Err(refusal) => return clapham_cli::write_refusal(&refusal, std::io::stderr()),
    };
    let program = std::env::args_os().next().unwrap_or_default();
    use std::io::IsTerminal;
    assembled
        .execute_from(
            std::iter::once(program).chain(args),
            std::io::stdin(),
            std::io::stdout(),
            std::io::stderr(),
            std::io::stdout().is_terminal(),
            clapham::Cancellation::new(),
        )
        .await
}
#[tokio::main]
async fn main() -> Result<std::process::ExitCode, Box<dyn std::error::Error>> {
    let mut cli = Cli::parse();
    match std::mem::replace(&mut cli.command, Command::Operation(Vec::new())) {
        Command::Init { root } => initialise(&root)?,
        Command::Serve { root, port } => serve(root, port).await?,
        Command::Operation(args) => {
            return Ok(std::process::ExitCode::from(operation(cli, args).await))
        }
    }
    Ok(std::process::ExitCode::SUCCESS)
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
