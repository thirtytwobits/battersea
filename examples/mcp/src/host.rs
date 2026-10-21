//! Copyright (c) Scott A Dixon
//! Host-owned process lifecycle, profile selection and provider configuration.
use crate::fixture::{Fault, Result};
use battersea_mcp::{McpConfig, McpConnection, McpSelection, McpToolBinding};
use battersea_model::{engine::*, *};
use battersea_tools::{ApprovedToolExecutor, ToolApprovalPolicy, ToolApprovalScope, ToolRegistry};
use std::{
    path::Path,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

pub fn mcp_config(timeout: Duration) -> McpConfig {
    let mut config = McpConfig::new("remote", "example-account-1");
    config.operation_timeout = timeout;
    config.limits.message_bytes = 64 * 1024;
    config.limits.schema_bytes = 32 * 1024;
    config
}

pub struct StdioHost {
    pub connection: McpConnection,
    child: tokio::process::Child,
    shutdown: CancellationToken,
}
impl StdioHost {
    pub async fn start(
        executable: &Path,
        ledger: &Path,
        fault: Fault,
        config: McpConfig,
    ) -> Result<Self> {
        use clap::ValueEnum;
        let mut child = tokio::process::Command::new(executable)
            .arg("serve-stdio")
            .arg("--ledger")
            .arg(ledger)
            .arg("--fault")
            .arg(fault.to_possible_value().unwrap().get_name())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()?;
        let shutdown = CancellationToken::new();
        let connection = McpConnection::stdio(
            child.stdout.take().unwrap(),
            child.stdin.take().unwrap(),
            shutdown.clone(),
            config,
        )
        .await?;
        Ok(Self {
            connection,
            child,
            shutdown,
        })
    }
    pub async fn close(mut self) -> Result<()> {
        self.connection.close();
        self.shutdown.cancelled().await;
        self.child.kill().await?;
        Ok(())
    }
}

pub async fn bind(connection: &McpConnection) -> Result<(McpToolBinding, ToolRegistry)> {
    let catalogue = connection.discover().await?;
    if !catalogue.diagnostics().is_empty() {
        return Err("MCP fixture catalogue was refused".into());
    }
    let mut registry = ToolRegistry::default();
    let binding = catalogue.bind(
        vec![McpSelection::new("write", vec!["operator".into()])],
        &registry,
    )?;
    binding.register_into(&mut registry)?;
    Ok((binding, registry))
}

pub fn provider(endpoint: &str) -> Result<(Arc<dyn EngineAdapter>, EngineChatParameters)> {
    configured_provider(endpoint, "openai-chat-completions", false)
}

pub fn responses_provider(
    endpoint: &str,
) -> Result<(Arc<dyn EngineAdapter>, EngineChatParameters)> {
    configured_provider(endpoint, "openai-responses", true)
}

fn configured_provider(
    endpoint: &str,
    adapter: &str,
    stream: bool,
) -> Result<(Arc<dyn EngineAdapter>, EngineChatParameters)> {
    let mut protocols = ProtocolRegistry::default();
    battersea_providers::register_builtin_adapters(&mut protocols)?;
    let descriptor = protocols.adapter(adapter).unwrap();
    let chat = descriptor.chat.as_ref().unwrap();
    let mut capabilities = EngineBackendCapabilities::mock();
    capabilities.content = chat.content.clone();
    capabilities.supported_chat_parameters = chat.parameters.clone();
    let config: battersea_config::ProviderConfiguration = serde_json::from_value(
        serde_json::json!({
            "version":1,
            "providers":{"controlled":{"chat":{"adapter":adapter,"baseUrl":format!("{endpoint}/v1/"),"auth":{"type":"none"}}}},
            "backends":{"chat":{"provider":"controlled","model":"fixture","contextWindowTokens":8192,"capabilities":capabilities,
                "properties":{"stream":stream,"maxOutputTokens":1024,"temperature":0.7,"stopSequences":[],"toolExecution":"engine-orchestrated","maxToolRounds":2,"toolChoice":{"mode":"auto"},"contextOverflow":"error"},
                "options":{"timeoutMs":5000,"streamIdleTimeoutMs":3000,"maxRetries":1,"retryInitialDelayMs":1,"retryMaxDelayMs":20}}}
        }),
    )?;
    let parameters = battersea_config::resolve_chat_parameters(
        config.backends["chat"].properties.clone().unwrap(),
    )?;
    let configured = config.validate(&protocols)?.bind(
        &EnvironmentCredentials {
            scope_revision: "example-account-1".into(),
        },
        None,
    )?;
    Ok((configured.chat("chat").unwrap().clone(), parameters))
}

pub fn request(
    registry: &ToolRegistry,
    profile: &str,
    chat: EngineChatParameters,
) -> EngineTextStreamRequest {
    let permitted = registry.for_profile(profile);
    EngineTextStreamRequest {
        shared: EngineAdapterRequest {
            operation: EngineOperation::FlowTextStream,
            messages: vec![Message::text(
                Role::User,
                "Record the controlled effect".into(),
            )],
            chat,
            debug_rules: None,
        },
        local_tools: permitted.definitions(),
        local_tool_executor: Some(Arc::new(permitted)),
        max_tool_rounds: 2,
        mock_response: None,
    }
}

/// Apply the host's decision policy to this run's selected MCP binding.
pub fn approved_request(
    binding: &McpToolBinding,
    registry: &ToolRegistry,
    chat: EngineChatParameters,
    run_id: &str,
    policy: Arc<dyn ToolApprovalPolicy>,
) -> Result<EngineTextStreamRequest> {
    let executor = ApprovedToolExecutor::new(
        registry.for_profile("operator"),
        ToolApprovalScope {
            run_id: run_id.into(),
            binding_revision: binding.revision().into(),
            deadline: Instant::now() + Duration::from_secs(5),
        },
        policy,
        8,
    )?;
    let mut request = request(registry, "operator", chat);
    request.local_tools = executor.definitions();
    request.local_tool_executor = Some(Arc::new(executor));
    Ok(request)
}
