//! Allowlisted, typed, source-grounded agent tools and bounded execution limits.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Instant,
};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use llm_provider::{AgentProvider, ProviderRequest, ProviderResponse, ToolObservation};
use mes_application::{
    AgentAuditRepository, AgentRunStart, AgentToolAudit, DatasetReadRepository, MesReadRepository,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

pub const DEFAULT_MAX_ITERATIONS: usize = 6;
pub const DEFAULT_MAX_TOOL_CALLS: usize = 8;

#[derive(Debug, Clone, Copy)]
pub struct AgentRuntimeLimits {
    pub max_iterations: usize,
    pub max_tool_calls: usize,
    pub overall_timeout: std::time::Duration,
    pub provider_timeout: std::time::Duration,
    pub tool_timeout: std::time::Duration,
}

impl Default for AgentRuntimeLimits {
    fn default() -> Self {
        Self {
            max_iterations: DEFAULT_MAX_ITERATIONS,
            max_tool_calls: DEFAULT_MAX_TOOL_CALLS,
            overall_timeout: std::time::Duration::from_secs(45),
            provider_timeout: std::time::Duration::from_secs(10),
            tool_timeout: std::time::Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthorizedToolContext {
    pub user_id: String,
    pub tenant_id: Uuid,
    pub permitted_factory_ids: Vec<Uuid>,
    pub run_id: Uuid,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("malformed arguments: {0}")]
    InvalidArguments(String),
    #[error("capability denied")]
    Denied,
    #[error("source dataset not found")]
    NotFound,
    #[error("tool execution failed: {0}")]
    Execution(String),
}

#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &'static str;
    fn schema(&self) -> Value;
    async fn execute(
        &self,
        context: &AuthorizedToolContext,
        arguments: Value,
    ) -> Result<Value, ToolError>;
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DatasetSummaryArguments {
    dataset_key: String,
}

pub struct DatasetSummaryTool {
    repository: Arc<dyn DatasetReadRepository>,
}

#[derive(Debug, Clone, Copy)]
enum MesToolKind {
    WorkOrder,
    Production,
    Downtime,
    Quality,
}

pub struct MesTool {
    kind: MesToolKind,
    repository: Arc<dyn MesReadRepository>,
}

impl MesTool {
    #[must_use]
    pub fn work_order(repository: Arc<dyn MesReadRepository>) -> Self {
        Self {
            kind: MesToolKind::WorkOrder,
            repository,
        }
    }
    #[must_use]
    pub fn production(repository: Arc<dyn MesReadRepository>) -> Self {
        Self {
            kind: MesToolKind::Production,
            repository,
        }
    }
    #[must_use]
    pub fn downtime(repository: Arc<dyn MesReadRepository>) -> Self {
        Self {
            kind: MesToolKind::Downtime,
            repository,
        }
    }
    #[must_use]
    pub fn quality(repository: Arc<dyn MesReadRepository>) -> Self {
        Self {
            kind: MesToolKind::Quality,
            repository,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkOrderArguments {
    work_order_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SummaryArguments {
    line_id: Uuid,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
}

#[async_trait]
impl AgentTool for MesTool {
    fn name(&self) -> &'static str {
        match self.kind {
            MesToolKind::WorkOrder => "get_work_order",
            MesToolKind::Production => "get_production_summary",
            MesToolKind::Downtime => "get_equipment_downtime",
            MesToolKind::Quality => "get_quality_summary",
        }
    }

    fn schema(&self) -> Value {
        match self.kind {
            MesToolKind::WorkOrder => json!({
                "type":"object","additionalProperties":false,
                "required":["work_order_id"],
                "properties":{"work_order_id":{"type":"string","format":"uuid"}}
            }),
            _ => json!({
                "type":"object","additionalProperties":false,
                "required":["line_id","start_at","end_at"],
                "properties":{
                    "line_id":{"type":"string","format":"uuid"},
                    "start_at":{"type":"string","format":"date-time"},
                    "end_at":{"type":"string","format":"date-time"}
                }
            }),
        }
    }

    async fn execute(
        &self,
        context: &AuthorizedToolContext,
        arguments: Value,
    ) -> Result<Value, ToolError> {
        if !context.capabilities.iter().any(|item| item == "read:mes") {
            return Err(ToolError::Denied);
        }
        let factory_id = context
            .permitted_factory_ids
            .first()
            .copied()
            .ok_or(ToolError::Denied)?;
        match self.kind {
            MesToolKind::WorkOrder => {
                let args: WorkOrderArguments = parse_args(arguments)?;
                let value = self
                    .repository
                    .get_work_order(context.tenant_id, factory_id, args.work_order_id)
                    .await
                    .map_err(execution_error)?
                    .ok_or(ToolError::NotFound)?;
                serialize(value)
            }
            kind => {
                let args: SummaryArguments = parse_args(arguments)?;
                validate_window(args.start_at, args.end_at)?;
                let result = match kind {
                    MesToolKind::Production => serialize(
                        self.repository
                            .production_summary(
                                context.tenant_id,
                                factory_id,
                                args.line_id,
                                args.start_at,
                                args.end_at,
                                500,
                            )
                            .await
                            .map_err(execution_error)?
                            .ok_or(ToolError::NotFound)?,
                    ),
                    MesToolKind::Downtime => serialize(
                        self.repository
                            .downtime_summary(
                                context.tenant_id,
                                factory_id,
                                args.line_id,
                                args.start_at,
                                args.end_at,
                                500,
                            )
                            .await
                            .map_err(execution_error)?
                            .ok_or(ToolError::NotFound)?,
                    ),
                    MesToolKind::Quality => serialize(
                        self.repository
                            .quality_summary(
                                context.tenant_id,
                                factory_id,
                                args.line_id,
                                args.start_at,
                                args.end_at,
                                500,
                            )
                            .await
                            .map_err(execution_error)?
                            .ok_or(ToolError::NotFound)?,
                    ),
                    MesToolKind::WorkOrder => unreachable!(),
                }?;
                Ok(result)
            }
        }
    }
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn AgentTool>>,
}

impl ToolRegistry {
    pub fn register(&mut self, tool: Arc<dyn AgentTool>) {
        self.tools.insert(tool.name().to_owned(), tool);
    }

    fn schemas(&self) -> Vec<Value> {
        self.tools
            .values()
            .map(|tool| json!({"name": tool.name(), "parameters": tool.schema()}))
            .collect()
    }
}

pub struct AgentRuntime {
    provider: Arc<dyn AgentProvider>,
    audit: Arc<dyn AgentAuditRepository>,
    registry: ToolRegistry,
    limits: AgentRuntimeLimits,
    provider_name: String,
    data_policy: String,
}

#[derive(Debug, Clone)]
pub struct AgentRunRequest {
    pub agent_id: String,
    pub message: String,
    pub context: AuthorizedToolContext,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentRunResult {
    pub run_id: Uuid,
    pub status: String,
    pub answer: String,
    pub evidence: Vec<String>,
    pub tool_calls: usize,
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("audit persistence failed: {0}")]
    Audit(String),
    #[error("provider failed: {0}")]
    Provider(String),
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("tool failed: {0}")]
    Tool(String),
    #[error("execution limit exceeded")]
    LimitExceeded,
}

impl AgentRuntime {
    #[must_use]
    pub fn new(
        provider: Arc<dyn AgentProvider>,
        audit: Arc<dyn AgentAuditRepository>,
        registry: ToolRegistry,
    ) -> Self {
        Self {
            provider,
            audit,
            registry,
            limits: AgentRuntimeLimits::default(),
            provider_name: "unknown".to_owned(),
            data_policy: "local_only".to_owned(),
        }
    }

    #[must_use]
    pub const fn with_limits(mut self, limits: AgentRuntimeLimits) -> Self {
        self.limits = limits;
        self
    }

    #[must_use]
    pub fn with_provider_metadata(
        mut self,
        provider_name: impl Into<String>,
        data_policy: impl Into<String>,
    ) -> Self {
        self.provider_name = provider_name.into();
        self.data_policy = data_policy.into();
        self
    }

    /// Runs a bounded provider/tool lifecycle and persists its audit trail.
    ///
    /// # Errors
    ///
    /// Returns a typed error for provider, tool, audit, timeout, or limit failures.
    #[allow(clippy::too_many_lines)]
    pub async fn run(&self, request: AgentRunRequest) -> Result<AgentRunResult, RuntimeError> {
        let run_id = request.context.run_id;
        self.audit
            .start_run(&AgentRunStart {
                id: run_id,
                tenant_id: request.context.tenant_id,
                user_id: request.context.user_id.clone(),
                agent_id: request.agent_id,
                provider_name: self.provider_name.clone(),
                data_policy: self.data_policy.clone(),
            })
            .await
            .map_err(audit_error)?;
        let started = Instant::now();
        let mut observations = Vec::new();
        let mut tool_calls = 0;
        for _ in 0..self.limits.max_iterations {
            if started.elapsed() >= self.limits.overall_timeout {
                return self.fail(run_id, "timed_out", "overall_timeout").await;
            }
            let response = match tokio::time::timeout(
                self.limits.provider_timeout,
                self.provider.next_turn(ProviderRequest {
                    message: request.message.clone(),
                    tool_schemas: self.registry.schemas(),
                    observations: observations.clone(),
                }),
            )
            .await
            {
                Err(_) => return self.fail(run_id, "timed_out", "provider_timeout").await,
                Ok(Err(_)) => return self.fail(run_id, "failed", "provider_error").await,
                Ok(Ok(response)) => response,
            };
            match response {
                ProviderResponse::Final {
                    answer,
                    evidence: _,
                } => {
                    let evidence = grounded_evidence(&observations);
                    self.audit
                        .finish_run(run_id, "completed", Some(&answer), None)
                        .await
                        .map_err(audit_error)?;
                    return Ok(AgentRunResult {
                        run_id,
                        status: "completed".to_owned(),
                        answer,
                        evidence,
                        tool_calls,
                    });
                }
                ProviderResponse::ToolCalls(calls) => {
                    for call in calls {
                        tool_calls += 1;
                        if tool_calls > self.limits.max_tool_calls {
                            return self.fail(run_id, "failed", "tool_call_limit").await;
                        }
                        let Some(tool) = self.registry.tools.get(&call.tool_name) else {
                            self.record_audit(
                                run_id,
                                &call.tool_name,
                                call.arguments,
                                "denied",
                                None,
                                0,
                            )
                            .await?;
                            return self.fail(run_id, "failed", "unknown_tool").await;
                        };
                        let tool_started = Instant::now();
                        let result = match tokio::time::timeout(
                            self.limits.tool_timeout,
                            tool.execute(&request.context, call.arguments.clone()),
                        )
                        .await
                        {
                            Err(_) => {
                                self.record_audit(
                                    run_id,
                                    &call.tool_name,
                                    call.arguments,
                                    "authorized",
                                    None,
                                    i64::try_from(self.limits.tool_timeout.as_millis())
                                        .unwrap_or(i64::MAX),
                                )
                                .await?;
                                return self.fail(run_id, "timed_out", "tool_timeout").await;
                            }
                            Ok(Err(error)) => {
                                let decision = if matches!(error, ToolError::Denied) {
                                    "denied"
                                } else {
                                    "authorized"
                                };
                                self.record_audit(
                                    run_id,
                                    &call.tool_name,
                                    call.arguments,
                                    decision,
                                    None,
                                    i64::try_from(tool_started.elapsed().as_millis())
                                        .unwrap_or(i64::MAX),
                                )
                                .await?;
                                return self.fail(run_id, "failed", "tool_error").await;
                            }
                            Ok(Ok(result)) => result,
                        };
                        let duration_ms =
                            i64::try_from(tool_started.elapsed().as_millis()).unwrap_or(i64::MAX);
                        self.record_audit(
                            run_id,
                            &call.tool_name,
                            call.arguments.clone(),
                            "authorized",
                            Some(result.clone()),
                            duration_ms,
                        )
                        .await?;
                        observations.push(ToolObservation {
                            call_id: call.call_id,
                            tool_name: call.tool_name,
                            arguments: call.arguments,
                            result,
                        });
                    }
                }
            }
        }
        self.fail(run_id, "failed", "iteration_limit").await
    }

    async fn record_audit(
        &self,
        run_id: Uuid,
        tool_name: &str,
        arguments: Value,
        decision: &str,
        result: Option<Value>,
        duration_ms: i64,
    ) -> Result<(), RuntimeError> {
        self.audit
            .record_tool_call(&AgentToolAudit {
                id: Uuid::new_v4(),
                run_id,
                tool_name: tool_name.to_owned(),
                redacted_arguments: arguments,
                auth_decision: decision.to_owned(),
                result_summary: result,
                duration_ms,
            })
            .await
            .map_err(audit_error)
    }

    async fn fail<T>(&self, run_id: Uuid, status: &str, code: &str) -> Result<T, RuntimeError> {
        self.audit
            .finish_run(run_id, status, None, Some(code))
            .await
            .map_err(audit_error)?;
        Err(match code {
            "unknown_tool" => RuntimeError::UnknownTool(code.to_owned()),
            "provider_error" | "provider_timeout" => RuntimeError::Provider(code.to_owned()),
            "tool_call_limit" | "iteration_limit" | "overall_timeout" => {
                RuntimeError::LimitExceeded
            }
            _ => RuntimeError::Tool(code.to_owned()),
        })
    }
}

fn grounded_evidence(observations: &[ToolObservation]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut evidence = Vec::new();
    for observation in observations {
        collect_evidence(&observation.result, &mut evidence);
    }
    evidence.retain(|item| seen.insert(item.clone()));
    evidence
}

fn collect_evidence(value: &Value, output: &mut Vec<String>) {
    if let Some(items) = value.get("evidence").and_then(Value::as_array) {
        output.extend(
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned),
        );
    }
    for (field, id_field, prefix) in [
        ("records", "record_id", "production_record"),
        ("events", "event_id", "downtime_event"),
        ("inspections", "inspection_id", "quality_inspection"),
    ] {
        let Some(items) = value.get(field).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            if let Some(id) = item.get(id_field).and_then(Value::as_str) {
                output.push(format!("{prefix}:{id}"));
            }
        }
    }
}

fn parse_args<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, ToolError> {
    serde_json::from_value(value).map_err(|error| ToolError::InvalidArguments(error.to_string()))
}

fn serialize(value: impl Serialize) -> Result<Value, ToolError> {
    serde_json::to_value(value).map_err(execution_error)
}

fn validate_window(start: DateTime<Utc>, end: DateTime<Utc>) -> Result<(), ToolError> {
    let duration = end - start;
    if duration <= Duration::zero() || duration > Duration::days(31) {
        Err(ToolError::InvalidArguments(
            "time window must be positive and at most 31 days".to_owned(),
        ))
    } else {
        Ok(())
    }
}

fn execution_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Execution(error.to_string())
}

fn audit_error(error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Audit(error.to_string())
}

impl DatasetSummaryTool {
    #[must_use]
    pub fn new(repository: Arc<dyn DatasetReadRepository>) -> Self {
        Self { repository }
    }
}

#[async_trait]
impl AgentTool for DatasetSummaryTool {
    fn name(&self) -> &'static str {
        "get_dataset_ground_truth"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["dataset_key"],
            "properties": {
                "dataset_key": {
                    "type": "string",
                    "enum": ["uci-ai4i-2020", "uci-secom"]
                }
            }
        })
    }

    async fn execute(
        &self,
        context: &AuthorizedToolContext,
        arguments: Value,
    ) -> Result<Value, ToolError> {
        if !context
            .capabilities
            .iter()
            .any(|capability| capability == "read:evaluation_datasets")
        {
            return Err(ToolError::Denied);
        }
        let arguments: DatasetSummaryArguments = serde_json::from_value(arguments)
            .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
        if !matches!(
            arguments.dataset_key.as_str(),
            "uci-ai4i-2020" | "uci-secom"
        ) {
            return Err(ToolError::InvalidArguments(
                "dataset_key is not allowlisted".to_owned(),
            ));
        }
        let result = self
            .repository
            .dataset_ground_truth(&arguments.dataset_key)
            .await
            .map_err(|error| ToolError::Execution(error.to_string()))?
            .ok_or(ToolError::NotFound)?;
        serde_json::to_value(result).map_err(|error| ToolError::Execution(error.to_string()))
    }
}
