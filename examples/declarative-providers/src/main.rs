//! An independent consumer binds two configured services to one registered protocol.
use battersea_model::*;
use futures_util::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let configuration =
        battersea_config::parse_provider_configuration(include_str!("../providers.yaml"))?;
    let mut protocols = ProtocolRegistry::default();
    battersea_providers::register_builtin_adapters(&mut protocols)?;
    let providers = configuration.validate(&protocols)?.bind(
        &EnvironmentCredentials {
            scope_revision: "example-account-1".into(),
        },
        None,
    )?;
    for (id, backend) in &configuration.backends {
        let adapter = providers.chat(id).expect("configured backend");
        assert_eq!(adapter.describe_debug_context().provider, backend.provider);
        let request = EngineTextStreamRequest {
            shared: EngineAdapterRequest {
                operation: EngineOperation::FlowTextStream,
                messages: vec![Message::text(Role::User, "Hello".into())],
                chat: battersea_config::resolve_chat_parameters(
                    backend.properties.clone().unwrap(),
                )?,
                debug_rules: None,
            },
            local_tools: vec![],
            local_tool_executor: None,
            max_tool_rounds: 2,
            mock_response: None,
        };
        let mut stream = adapter.stream_text(request).await?;
        while let Some(event) = stream.next().await {
            if let EngineTextStreamEvent::TextDelta { text } = event? {
                print!("{text}");
            }
        }
        println!(" ({})", backend.provider);
    }
    Ok(())
}
