//! Copyright (c) Scott A Dixon
//! Run controlled MCP tools through a configured provider on both supported transports.
use battersea_model::{EngineTextStreamEvent, ToolCallOutcome};
use battersea_tools::{ToolApprovalDecision, ToolApprovalPolicy, ToolApprovalRequest};
use clap::{Parser, Subcommand};
use clapham_mcp_client::{McpConnection, McpHttpOptions};
use futures_util::StreamExt;
use mcp_example::{
    fixture::{self, Fault, Result},
    host,
};
use std::{path::PathBuf, sync::Arc, time::Duration};

struct FixturePolicy;
impl ToolApprovalPolicy for FixturePolicy {
    fn decide(&self, request: ToolApprovalRequest<'_>) -> ToolApprovalDecision {
        // The application created this controlled server and permits its write operation.
        if request.tool.name == "remote.write" {
            ToolApprovalDecision::Allow
        } else {
            ToolApprovalDecision::Deny
        }
    }
}

#[derive(Parser)]
#[command(
    about = "Demonstrate MCP tools in a provider conversation using local controlled servers"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    /// Serve the controlled MCP fixture over stdin/stdout.
    ServeStdio {
        #[arg(long)]
        ledger: PathBuf,
        #[arg(long, value_enum, default_value_t)]
        fault: Fault,
    },
}

async fn conversation(connection: &McpConnection, run_id: &str) -> Result<()> {
    let (binding, registry) = host::bind(connection).await?;
    let fixture = fixture::ProviderFixture::new(serde_json::json!({"value":1}), true);
    let provider = fixture::provider_http(fixture).await?;
    // The demonstration's time is explicit: retry backoff completes once it is scheduled, and the
    // operation and approval deadlines never elapse.
    let clock = battersea_model::ManualClock::new(1_800_000_000_000);
    let backoff = {
        let clock = clock.clone();
        tokio::spawn(async move {
            loop {
                clock
                    .advance_to_timer_within(std::time::Duration::from_millis(20))
                    .await;
            }
        })
    };
    let (adapter, chat) = host::provider(&provider.endpoint, Arc::new(clock.clone()))?;
    println!("MCP binding {}", binding.revision());
    let mut stream = adapter
        .stream_text(host::approved_request(
            &registry,
            chat,
            run_id,
            Arc::new(FixturePolicy),
            Arc::new(clock),
        )?)
        .await?;
    let mut completed = false;
    while let Some(event) = stream.next().await {
        match event? {
            EngineTextStreamEvent::ToolCallCompleted { ok, tool_call, .. } => {
                assert!(ok);
                assert_eq!(tool_call.unwrap().outcome, ToolCallOutcome::Completed);
                completed = true;
            }
            EngineTextStreamEvent::TextDelta { text } => println!("{text}"),
            _ => (),
        }
    }
    assert!(completed);
    backoff.abort();
    Ok(())
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    if let Some(Command::ServeStdio { ledger, fault }) = Cli::parse().command {
        return fixture::serve_stdio(ledger, fault).await;
    }
    let directory = tempfile::tempdir()?;
    let ledger = directory.path().join("effects");
    // MCP deadlines run on a clock no one advances; the fixtures answer every request.
    let clock: Arc<dyn battersea_model::Clock> =
        Arc::new(battersea_model::ManualClock::new(1_800_000_000_000));
    let stdio = host::StdioHost::start(
        &std::env::current_exe()?,
        &ledger,
        Fault::None,
        host::mcp_config(Duration::from_secs(10), clock.clone()),
    )
    .await?;
    conversation(&stdio.connection, "stdio-conversation").await?;
    stdio.close().await?;
    assert_eq!(fixture::effect_count(&ledger)?, 1);
    for sse in [false, true] {
        let server =
            fixture::mcp_http(fixture::McpFixture::new(ledger.clone(), Fault::None), sse).await?;
        let connection = McpConnection::http(
            McpHttpOptions::new(format!("{}/mcp", server.endpoint)),
            host::mcp_config(Duration::from_secs(10), clock.clone()),
        )
        .await?;
        conversation(
            &connection,
            if sse {
                "sse-conversation"
            } else {
                "json-conversation"
            },
        )
        .await?;
        connection.close();
    }
    assert_eq!(fixture::effect_count(&ledger)?, 3);
    println!("Three effects recorded across stdio, HTTP JSON and HTTP SSE; provider retries reused each result.");
    Ok(())
}
