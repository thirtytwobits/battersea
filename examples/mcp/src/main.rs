//! Copyright (c) Scott A Dixon
//! Run controlled MCP tools through a configured provider on both supported transports.
use battersea_mcp::{McpConnection, McpHttpOptions};
use battersea_model::{EngineTextStreamEvent, ToolCallOutcome};
use clap::{Parser, Subcommand};
use futures_util::StreamExt;
use mcp_example::{
    fixture::{self, Fault, Result},
    host,
};
use std::{path::PathBuf, time::Duration};

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

async fn conversation(connection: &McpConnection) -> Result<()> {
    let (binding, registry) = host::bind(connection).await?;
    let fixture = fixture::ProviderFixture::new(serde_json::json!({"value":1}), true);
    let provider = fixture::provider_http(fixture).await?;
    let (adapter, chat) = host::provider(&provider.endpoint)?;
    println!("MCP binding {}", binding.revision());
    let mut stream = adapter
        .stream_text(host::request(&registry, "operator", chat))
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
    Ok(())
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    if let Some(Command::ServeStdio { ledger, fault }) = Cli::parse().command {
        return fixture::serve_stdio(ledger, fault).await;
    }
    let directory = tempfile::tempdir()?;
    let ledger = directory.path().join("effects");
    let stdio = host::StdioHost::start(
        &std::env::current_exe()?,
        &ledger,
        Fault::None,
        host::mcp_config(Duration::from_secs(2)),
    )
    .await?;
    conversation(&stdio.connection).await?;
    stdio.close().await?;
    assert_eq!(fixture::effect_count(&ledger)?, 1);
    for sse in [false, true] {
        let server =
            fixture::mcp_http(fixture::McpFixture::new(ledger.clone(), Fault::None), sse).await?;
        let connection = McpConnection::http(
            McpHttpOptions::new(format!("{}/mcp", server.endpoint)),
            host::mcp_config(Duration::from_secs(2)),
        )
        .await?;
        conversation(&connection).await?;
        connection.close();
    }
    assert_eq!(fixture::effect_count(&ledger)?, 3);
    println!("Three effects recorded across stdio, HTTP JSON and HTTP SSE; provider retries reused each result.");
    Ok(())
}
