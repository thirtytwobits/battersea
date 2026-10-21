//! Copyright (c) Scott A Dixon
//! Explicit operations on a retained background response; the host owns its lifetime.
use super::*;
use battersea_model::{ConversationCommand, ConversationStep};

fn native_response(response: &Value) -> Result<(), EngineAdapterRequestError> {
    check_payload(response, "openai")?;
    serde_json::from_value::<async_openai::types::responses::Response>(response.clone())
        .map_err(|_| invalid("Invalid background Responses response."))?;
    if response["background"] != true
        || response["id"]
            .as_str()
            .is_none_or(|id| id.trim().is_empty())
    {
        return Err(invalid("Missing background response identity."));
    }
    Ok(())
}

impl OpenAiEngineAdapter {
    fn pending_checkpoint(
        &self,
        request: &ConversationRequest,
        previous: Option<&Value>,
        results: &[ConversationToolResult],
        state: &Value,
    ) -> Result<Checkpoint, EngineAdapterRequestError> {
        let binding = self.validate_durable_request(request, previous)?;
        let checkpoint = decode(state)?;
        native_response(&checkpoint.response)?;
        if !self.background
            || checkpoint.version != 1
            || checkpoint.binding != binding
            || checkpoint.request_revision
                != configuration_digest(&serde_json::to_value(request).expect("request"))
            || checkpoint.input != self.continuation_input(request, previous, results)?
            || !matches!(
                checkpoint.response["status"].as_str(),
                Some("queued" | "in_progress")
            )
            || !checkpoint.response["error"].is_null()
            || !checkpoint.response["incomplete_details"].is_null()
        {
            return Err(invalid(
                "Background response does not match its retained request.",
            ));
        }
        Ok(checkpoint)
    }

    pub(crate) fn validate_durable_step(
        &self,
        request: &ConversationRequest,
        previous: Option<&Value>,
        results: &[ConversationToolResult],
        command: &ConversationCommand,
        step: &ConversationStep,
    ) -> Result<ConversationBinding, EngineAdapterRequestError> {
        let binding = self.validate_durable_request(request, previous)?;
        let pending = match command {
            ConversationCommand::Start => None,
            ConversationCommand::Poll { state } | ConversationCommand::Cancel { state } => {
                Some(self.pending_checkpoint(request, previous, results, state)?)
            }
        };
        match (command, step) {
            (
                ConversationCommand::Start | ConversationCommand::Poll { .. },
                ConversationStep::Completed { turn },
            ) => {
                self.validate_durable_turn(request, previous, results, turn)?;
                if self.background {
                    let checkpoint = decode(&turn.state)?;
                    native_response(&checkpoint.response)?;
                    if pending
                        .as_ref()
                        .is_some_and(|p| p.response["id"] != checkpoint.response["id"])
                    {
                        return Err(invalid("Background response identity changed."));
                    }
                }
            }
            (
                ConversationCommand::Start | ConversationCommand::Poll { .. },
                ConversationStep::Pending { state },
            ) => {
                self.pending_checkpoint(request, previous, results, state)?;
                if let ConversationCommand::Poll { state: known } = command {
                    if state != known {
                        return Err(invalid("Background response handle changed."));
                    }
                }
            }
            (ConversationCommand::Cancel { .. }, ConversationStep::Cancelled { state }) => {
                native_response(state)?;
                if state["id"] != pending.expect("cancel handle").response["id"]
                    || !is_terminal_background_status(state["status"].as_str().unwrap_or_default())
                {
                    return Err(invalid(
                        "Background cancellation did not confirm a terminal response.",
                    ));
                }
            }
            _ => return Err(invalid("Invalid background response transition.")),
        }
        Ok(binding)
    }

    pub(crate) async fn durable_step(
        &self,
        request: ConversationRequest,
        previous: Option<Value>,
        results: Vec<ConversationToolResult>,
        command: ConversationCommand,
    ) -> Result<ConversationStep, EngineAdapterRequestError> {
        let binding = self.validate_durable_request(&request, previous.as_ref())?;
        if !self.background {
            return match command {
                ConversationCommand::Start => self
                    .durable_turn(request, previous, results)
                    .await
                    .map(|turn| ConversationStep::Completed { turn }),
                _ => Err(invalid("Foreground response has no remote continuation.")),
            };
        }
        let input = self.continuation_input(&request, previous.as_ref(), &results)?;
        let headers = build_openai_headers(&self.backend)?;
        let (http, pending) = match &command {
            ConversationCommand::Start => {
                let mut body = to_background_body(build_local_tool_body(
                    &self.backend,
                    &request.to_request(),
                    json!(input),
                ));
                body.as_object_mut()
                    .expect("body")
                    .entry("include")
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
                    .expect("include")
                    .push(json!("reasoning.encrypted_content"));
                check_payload(&body, "openai")?;
                emit_request(
                    self.logger.as_ref(),
                    &self.context,
                    &request.shared,
                    None,
                    Some(body.clone()),
                )
                .await;
                (
                    self.http_client
                        .post(format!("{}/responses", self.base_url))
                        .headers(headers)
                        .json(&body),
                    None,
                )
            }
            ConversationCommand::Poll { state } | ConversationCommand::Cancel { state } => {
                let pending =
                    self.pending_checkpoint(&request, previous.as_ref(), &results, state)?;
                let id =
                    crate::connection::segment(pending.response["id"].as_str().expect("identity"));
                let url = format!("{}/responses/{id}", self.base_url);
                let http = if matches!(command, ConversationCommand::Cancel { .. }) {
                    self.http_client.post(format!("{url}/cancel"))
                } else {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        self.background_poll_interval_ms,
                    ))
                    .await;
                    self.http_client.get(url)
                };
                emit_request(
                    self.logger.as_ref(),
                    &self.context,
                    &request.shared,
                    None,
                    None,
                )
                .await;
                (http.headers(headers), Some(pending))
            }
        };
        let response: Value = crate::retry::send(http, &self.backend, self.logger.as_ref())
            .await?
            .bounded_json()
            .await
            .map_err(|_| invalid("Invalid background response payload."))?;
        native_response(&response)?;
        if pending
            .as_ref()
            .is_some_and(|p| p.response["id"] != response["id"])
        {
            return Err(invalid("Background response identity changed."));
        }
        let status = response["status"].as_str().unwrap_or_default();
        let step = if matches!(command, ConversationCommand::Cancel { .. }) {
            ConversationStep::Cancelled {
                state: response.clone(),
            }
        } else if matches!(status, "queued" | "in_progress") {
            if !response["error"].is_null() || !response["incomplete_details"].is_null() {
                return Err(invalid(
                    "Pending background response contains terminal details.",
                ));
            }
            let state = match &command {
                ConversationCommand::Poll { state } => state.clone(),
                _ => serde_json::to_value(Checkpoint {
                    version: 1,
                    binding: binding.clone(),
                    request_revision: configuration_digest(
                        &serde_json::to_value(&request).expect("request"),
                    ),
                    input,
                    response: response.clone(),
                })
                .expect("checkpoint"),
            };
            ConversationStep::Pending { state }
        } else {
            finalize_background_response(response.clone(), status)?;
            return self
                .completed_turn(&request, binding, input, response, false)
                .await
                .map(|turn| ConversationStep::Completed { turn });
        };
        self.validate_durable_step(&request, previous.as_ref(), &results, &command, &step)?;
        emit_response(
            self.logger.as_ref(),
            &self.context,
            request.shared.operation,
            200,
            EngineAdapterResponseDetail {
                ok: true,
                request_id: None,
                response_id: response["id"].as_str().map(ToOwned::to_owned),
                output_chars: None,
                usage: response.get("usage").cloned(),
                streaming: Some(false),
                stop_reason: None,
            },
        )
        .await;
        Ok(step)
    }
}
