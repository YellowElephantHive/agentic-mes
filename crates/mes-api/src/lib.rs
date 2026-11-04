use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration as StdDuration, Instant},
};

use agent_core::{
    AgentRunRequest, AgentRuntime, AgentRuntimeLimits, AuthorizedToolContext, MesTool, ToolRegistry,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header::WWW_AUTHENTICATE},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Duration, Utc};
use llm_provider::{AgentProvider, ModelToolCall, ScriptedMockProvider};
use mes_application::{
    ACTION_PLAN_VALIDATION_POLICY_VERSION, ActionPlanDraft, ActionPlanRepository,
    ActionPlanValidationCommand, AgentAuditRepository, ApplicationError,
    AuthorizationAuditRepository, AuthorizationDecisionAudit, DispositionProposalDraft,
    DispositionProposalRepository, DispositionReviewCommand, DispositionTransitionCommand,
    MesReadRepository, ProposalStatus, ProposalType, ReviewDecision, compile_action_plan,
};
use mes_infra::PostgresMesRepository;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use uuid::Uuid;

pub mod auth;
mod disposition;
use auth::{AuthError, AuthenticatedPrincipal, IdentityService};

const MAX_WINDOW_DAYS: i64 = 31;
const MAX_EVIDENCE_ROWS: i64 = 500;

#[derive(Clone)]
pub struct AppState {
    pub repository: Arc<PostgresMesRepository>,
    identity_service: Arc<IdentityService>,
    rate_limiter: Arc<FixedWindowRateLimiter>,
    provider: ProviderSelection,
    data_policy: LlmDataPolicy,
    runtime_limits: AgentRuntimeLimits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LlmDataPolicy {
    LocalOnly,
    ExternalApproved,
}

#[derive(Clone)]
enum ProviderSelection {
    Mock,
    Ollama(Arc<dyn AgentProvider>),
    Http(Arc<dyn AgentProvider>),
}

impl ProviderSelection {
    const fn name(&self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::Ollama(_) => "ollama",
            Self::Http(_) => "http",
        }
    }
}

impl AppState {
    #[must_use]
    pub fn development(repository: Arc<PostgresMesRepository>) -> Self {
        let identity_service = Arc::new(IdentityService::development(repository.clone()));
        Self {
            repository,
            identity_service,
            rate_limiter: Arc::new(FixedWindowRateLimiter::new(120, StdDuration::from_secs(60))),
            provider: ProviderSelection::Mock,
            data_policy: LlmDataPolicy::LocalOnly,
            runtime_limits: AgentRuntimeLimits::default(),
        }
    }

    #[must_use]
    pub fn with_identity_service(mut self, service: IdentityService) -> Self {
        self.identity_service = Arc::new(service);
        self
    }

    #[must_use]
    pub fn with_rate_limit(mut self, requests: usize, window: StdDuration) -> Self {
        self.rate_limiter = Arc::new(FixedWindowRateLimiter::new(requests, window));
        self
    }

    #[must_use]
    pub const fn with_runtime_limits(mut self, limits: AgentRuntimeLimits) -> Self {
        self.runtime_limits = limits;
        self
    }

    #[must_use]
    pub fn with_ollama_provider(mut self, provider: Arc<dyn AgentProvider>) -> Self {
        self.provider = ProviderSelection::Ollama(provider);
        self.data_policy = LlmDataPolicy::LocalOnly;
        self
    }

    /// Enables an external HTTP provider only after explicit data-sharing approval.
    ///
    /// # Errors
    ///
    /// Returns an error when the policy is not `ExternalApproved`.
    pub fn with_http_provider(
        mut self,
        provider: Arc<dyn AgentProvider>,
        policy: LlmDataPolicy,
    ) -> Result<Self, &'static str> {
        if policy != LlmDataPolicy::ExternalApproved {
            return Err("external HTTP provider requires LLM_DATA_POLICY=external_approved");
        }
        self.provider = ProviderSelection::Http(provider);
        self.data_policy = policy;
        Ok(self)
    }
}

#[derive(Debug)]
struct FixedWindowRateLimiter {
    requests: usize,
    window: StdDuration,
    entries: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl FixedWindowRateLimiter {
    fn new(requests: usize, window: StdDuration) -> Self {
        Self {
            requests,
            window,
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn allow(&self, key: &str) -> bool {
        let Ok(mut entries) = self.entries.lock() else {
            return false;
        };
        let now = Instant::now();
        let requests = entries.entry(key.to_owned()).or_default();
        while requests
            .front()
            .is_some_and(|timestamp| now.duration_since(*timestamp) >= self.window)
        {
            requests.pop_front();
        }
        if requests.len() >= self.requests {
            return false;
        }
        requests.push_back(now);
        true
    }
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

#[derive(Debug, Deserialize)]
struct SummaryQuery {
    line_id: Uuid,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateAgentRun {
    agent_id: String,
    factory_id: Uuid,
    line_id: Uuid,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateDispositionProposal {
    proposal_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateActionPlan {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewDispositionProposal {
    decision: String,
    rationale: Option<String>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    request_id: Uuid,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            request_id: Uuid::new_v4(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let is_unauthorized = self.status == StatusCode::UNAUTHORIZED;
        let mut response = (
            self.status,
            Json(json!({
                "error": {
                    "code": self.code,
                    "message": self.message,
                    "request_id": self.request_id,
                }
            })),
        )
            .into_response();
        if is_unauthorized {
            response.headers_mut().insert(
                WWW_AUTHENTICATE,
                HeaderValue::from_static("Bearer realm=\"agentic-mes\""),
            );
        }
        response
    }
}

/// Builds the M2 HTTP router.
pub fn router(state: AppState) -> Router {
    let request_id = HeaderName::from_static("x-request-id");
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/api/v1/work-orders/{id}", get(get_work_order))
        .route("/api/v1/production/summary", get(production_summary))
        .route("/api/v1/equipment/downtime", get(downtime_summary))
        .route("/api/v1/quality/summary", get(quality_summary))
        .route("/api/v1/agent/runs", post(create_agent_run))
        .route("/api/v1/agent/runs/{run_id}", get(get_agent_run))
        .route(
            "/api/v1/agent/runs/{run_id}/tool-calls",
            get(get_agent_tool_calls),
        )
        .route(
            "/api/v1/agent/runs/{run_id}/disposition-proposals",
            post(create_disposition_proposal),
        )
        .route(
            "/api/v1/disposition-proposals/{proposal_id}",
            get(get_disposition_proposal),
        )
        .route(
            "/api/v1/disposition-proposals/{proposal_id}/submit",
            post(submit_disposition_proposal),
        )
        .route(
            "/api/v1/disposition-proposals/{proposal_id}/cancel",
            post(cancel_disposition_proposal),
        )
        .route(
            "/api/v1/disposition-proposals/{proposal_id}/reviews",
            post(review_disposition_proposal),
        )
        .route(
            "/api/v1/disposition-proposals/{proposal_id}/action-plans",
            post(create_action_plan),
        )
        .route("/api/v1/action-plans/{plan_id}", get(get_action_plan))
        .route(
            "/api/v1/action-plans/{plan_id}/validate",
            post(validate_action_plan),
        )
        .with_state(state)
        .layer(PropagateRequestIdLayer::new(request_id.clone()))
        .layer(TraceLayer::new_for_http())
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
        .layer(DefaultBodyLimit::max(64 * 1024))
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "mes-api",
    })
}

async fn readyz(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(state.repository.pool())
        .await
        .map_err(|error| {
            tracing::error!(%error, "readiness database check failed");
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "not_ready",
                "database is not ready",
            )
        })?;
    if !state.identity_service.is_ready().await {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_ready",
            "authentication provider is not ready",
        ));
    }
    Ok(Json(json!({
        "status": "ready",
        "database": "ready",
        "provider": {
            "type": state.provider.name(),
            "status": "configured"
        },
        "authentication": {
            "type": state.identity_service.provider_name(),
            "status": "ready"
        },
        "llm_data_policy": match state.data_policy {
            LlmDataPolicy::LocalOnly => "local_only",
            LlmDataPolicy::ExternalApproved => "external_approved",
        }
    })))
}

async fn get_work_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let identity = identity(&state, &headers).await?;
    enforce_capabilities(
        &state,
        &headers,
        &identity,
        &["read:mes"],
        "mes.work_order.read",
        "work_order",
        Some(id.to_string()),
    )
    .await?;
    let result = state
        .repository
        .get_work_order(identity.tenant_id, identity.factory_id(), id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    Ok(Json(json!({"data": result})))
}

async fn production_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SummaryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    validate_window(&query)?;
    let identity = identity(&state, &headers).await?;
    enforce_capabilities(
        &state,
        &headers,
        &identity,
        &["read:mes"],
        "mes.production.read",
        "production_line",
        Some(query.line_id.to_string()),
    )
    .await?;
    let result = state
        .repository
        .production_summary(
            identity.tenant_id,
            identity.factory_id(),
            query.line_id,
            query.start_at,
            query.end_at,
            MAX_EVIDENCE_ROWS,
        )
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    Ok(Json(json!({"data": result})))
}

async fn downtime_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SummaryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    validate_window(&query)?;
    let identity = identity(&state, &headers).await?;
    enforce_capabilities(
        &state,
        &headers,
        &identity,
        &["read:mes"],
        "mes.downtime.read",
        "production_line",
        Some(query.line_id.to_string()),
    )
    .await?;
    let result = state
        .repository
        .downtime_summary(
            identity.tenant_id,
            identity.factory_id(),
            query.line_id,
            query.start_at,
            query.end_at,
            MAX_EVIDENCE_ROWS,
        )
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    Ok(Json(json!({"data": result})))
}

async fn quality_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SummaryQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    validate_window(&query)?;
    let identity = identity(&state, &headers).await?;
    enforce_capabilities(
        &state,
        &headers,
        &identity,
        &["read:mes"],
        "mes.quality.read",
        "production_line",
        Some(query.line_id.to_string()),
    )
    .await?;
    let result = state
        .repository
        .quality_summary(
            identity.tenant_id,
            identity.factory_id(),
            query.line_id,
            query.start_at,
            query.end_at,
            MAX_EVIDENCE_ROWS,
        )
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    Ok(Json(json!({"data": result})))
}

#[allow(clippy::too_many_lines)]
async fn create_agent_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateAgentRun>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let identity = identity(&state, &headers).await?;
    if missing_any_capability(&identity, &["run:agent", "read:mes"]) {
        record_authorization(
            &state,
            &headers,
            &identity,
            "agent_run.create",
            "factory",
            Some(request.factory_id.to_string()),
            "denied",
            Some("missing_capability"),
        )
        .await?;
        return Err(authorization_denied());
    }
    if request.factory_id != identity.factory_id() {
        record_authorization(
            &state,
            &headers,
            &identity,
            "agent_run.create",
            "factory",
            Some(request.factory_id.to_string()),
            "denied",
            Some("scope_mismatch"),
        )
        .await?;
        return Err(not_found());
    }
    record_authorization(
        &state,
        &headers,
        &identity,
        "agent_run.create",
        "factory",
        Some(request.factory_id.to_string()),
        "authorized",
        None,
    )
    .await?;
    if request.agent_id != "production-analyst" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unsupported_agent",
            "only the read-only production-analyst agent is available",
        ));
    }
    if request.message.trim().is_empty() || request.message.len() > 4_000 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_message",
            "message must contain 1 to 4000 characters",
        ));
    }
    let window = request.end_at - request.start_at;
    validate_window(&SummaryQuery {
        line_id: request.line_id,
        start_at: request.start_at,
        end_at: request.end_at,
    })?;
    let baseline_start = request.start_at - window;
    let provider: Arc<dyn AgentProvider> = match &state.provider {
        ProviderSelection::Mock => Arc::new(ScriptedMockProvider::new(
            vec![
                summary_tool_call(
                    "baseline",
                    "get_production_summary",
                    request.line_id,
                    baseline_start,
                    request.start_at,
                ),
                summary_tool_call(
                    "current",
                    "get_production_summary",
                    request.line_id,
                    request.start_at,
                    request.end_at,
                ),
                summary_tool_call(
                    "downtime",
                    "get_equipment_downtime",
                    request.line_id,
                    request.start_at,
                    request.end_at,
                ),
                summary_tool_call(
                    "quality",
                    "get_quality_summary",
                    request.line_id,
                    request.start_at,
                    request.end_at,
                ),
            ],
            "Insufficient source observations for a grounded answer.",
        )),
        ProviderSelection::Ollama(provider) | ProviderSelection::Http(provider) => provider.clone(),
    };
    let mut registry = ToolRegistry::default();
    registry.register(Arc::new(MesTool::work_order(state.repository.clone())));
    registry.register(Arc::new(MesTool::production(state.repository.clone())));
    registry.register(Arc::new(MesTool::downtime(state.repository.clone())));
    registry.register(Arc::new(MesTool::quality(state.repository.clone())));
    let run_id = Uuid::new_v4();
    let grounded_message = format!(
        "Untrusted user question: {}\n\nTrusted required calls for line_id={}; do not answer until all four exist: A get_production_summary start_at={} end_at={}; B get_production_summary start_at={} end_at={}; C get_equipment_downtime start_at={} end_at={}; D get_quality_summary start_at={} end_at={}. Do not call downtime or quality for the baseline. Never repeat a completed call. Do not invent IDs, measurements, or causes.",
        request.message,
        request.line_id,
        baseline_start.to_rfc3339(),
        request.start_at.to_rfc3339(),
        request.start_at.to_rfc3339(),
        request.end_at.to_rfc3339(),
        request.start_at.to_rfc3339(),
        request.end_at.to_rfc3339(),
        request.start_at.to_rfc3339(),
        request.end_at.to_rfc3339(),
    );
    let policy_name = match state.data_policy {
        LlmDataPolicy::LocalOnly => "local_only",
        LlmDataPolicy::ExternalApproved => "external_approved",
    };
    let result = AgentRuntime::new(provider, state.repository.clone(), registry)
        .with_limits(state.runtime_limits)
        .with_provider_metadata(state.provider.name(), policy_name)
        .run(AgentRunRequest {
            agent_id: request.agent_id,
            message: grounded_message,
            context: AuthorizedToolContext {
                user_id: identity.user_id.to_string(),
                tenant_id: identity.tenant_id,
                permitted_factory_ids: identity.permitted_factory_ids.clone(),
                run_id,
                capabilities: identity.capabilities.clone(),
            },
        })
        .await
        .map_err(|error| {
            tracing::warn!(%run_id, %error, "agent run failed safely");
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "agent_run_failed",
                "agent run failed; inspect its audit record using the returned request ID",
            )
        })?;
    let calls = state
        .repository
        .get_tool_calls(identity.tenant_id, run_id)
        .await
        .map_err(internal_error)?;
    let required = [
        ("get_production_summary", baseline_start, request.start_at),
        ("get_production_summary", request.start_at, request.end_at),
        ("get_equipment_downtime", request.start_at, request.end_at),
        ("get_quality_summary", request.start_at, request.end_at),
    ];
    if !required.iter().all(|(tool, start, end)| {
        calls.iter().any(|call| {
            call.tool_name == *tool
                && call_matches_scope(&call.redacted_arguments, request.line_id, *start, *end)
        })
    }) {
        state
            .repository
            .finish_run(run_id, "failed", None, Some("incomplete_tool_evidence"))
            .await
            .map_err(internal_error)?;
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "incomplete_tool_evidence",
            "provider answered before all required source queries completed",
        ));
    }
    Ok(Json(json!({"data": result})))
}

fn call_matches_scope(
    arguments: &serde_json::Value,
    line_id: Uuid,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
) -> bool {
    arguments
        .get("line_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        == Some(line_id)
        && arguments
            .get("start_at")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| value.parse::<DateTime<Utc>>().ok())
            == Some(start_at)
        && arguments
            .get("end_at")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| value.parse::<DateTime<Utc>>().ok())
            == Some(end_at)
}

async fn get_agent_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let identity = identity(&state, &headers).await?;
    if !has_agent_run_read_capability(&identity) {
        record_authorization(
            &state,
            &headers,
            &identity,
            "agent_run.read",
            "agent_run",
            Some(run_id.to_string()),
            "denied",
            Some("missing_capability"),
        )
        .await?;
        return Err(authorization_denied());
    }
    let run = state
        .repository
        .get_run(identity.tenant_id, run_id)
        .await
        .map_err(internal_error)?;
    let Some(run) = run else {
        record_authorization(
            &state,
            &headers,
            &identity,
            "agent_run.read",
            "agent_run",
            Some(run_id.to_string()),
            "authorized",
            None,
        )
        .await?;
        return Err(not_found());
    };
    if !can_read_agent_run(&identity, &run.user_id) {
        record_authorization(
            &state,
            &headers,
            &identity,
            "agent_run.read",
            "agent_run",
            Some(run_id.to_string()),
            "denied",
            Some("resource_scope"),
        )
        .await?;
        return Err(not_found());
    }
    record_authorization(
        &state,
        &headers,
        &identity,
        "agent_run.read",
        "agent_run",
        Some(run_id.to_string()),
        "authorized",
        None,
    )
    .await?;
    Ok(Json(json!({"data": run})))
}

async fn get_agent_tool_calls(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let identity = identity(&state, &headers).await?;
    if !has_agent_run_read_capability(&identity) {
        record_authorization(
            &state,
            &headers,
            &identity,
            "agent_tool_calls.read",
            "agent_run",
            Some(run_id.to_string()),
            "denied",
            Some("missing_capability"),
        )
        .await?;
        return Err(authorization_denied());
    }
    let run = state
        .repository
        .get_run(identity.tenant_id, run_id)
        .await
        .map_err(internal_error)?;
    let Some(run) = run else {
        record_authorization(
            &state,
            &headers,
            &identity,
            "agent_tool_calls.read",
            "agent_run",
            Some(run_id.to_string()),
            "authorized",
            None,
        )
        .await?;
        return Err(not_found());
    };
    if !can_read_agent_run(&identity, &run.user_id) {
        record_authorization(
            &state,
            &headers,
            &identity,
            "agent_tool_calls.read",
            "agent_run",
            Some(run_id.to_string()),
            "denied",
            Some("resource_scope"),
        )
        .await?;
        return Err(not_found());
    }
    record_authorization(
        &state,
        &headers,
        &identity,
        "agent_tool_calls.read",
        "agent_run",
        Some(run_id.to_string()),
        "authorized",
        None,
    )
    .await?;
    let calls = state
        .repository
        .get_tool_calls(identity.tenant_id, run_id)
        .await
        .map_err(internal_error)?;
    Ok(Json(json!({"data": calls})))
}

async fn create_disposition_proposal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<Uuid>,
    Json(request): Json<CreateDispositionProposal>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let identity = identity(&state, &headers).await?;
    enforce_capabilities(
        &state,
        &headers,
        &identity,
        &["create:disposition_proposals"],
        "disposition_proposal.create",
        "agent_run",
        Some(run_id.to_string()),
    )
    .await?;
    let run = state
        .repository
        .get_run(identity.tenant_id, run_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    if !can_read_agent_run(&identity, &run.user_id) {
        return Err(not_found());
    }
    if run.status != "completed" || run.factory_id != Some(identity.factory_id()) {
        return Err(invalid_disposition(
            "source run must be completed in the selected factory",
        ));
    }
    let proposal_type = request
        .proposal_type
        .parse::<ProposalType>()
        .map_err(|_| invalid_disposition("unknown proposal type"))?;
    let tool_calls = state
        .repository
        .get_tool_calls(identity.tenant_id, run_id)
        .await
        .map_err(internal_error)?;
    let generated = disposition::generate_from_executed_evidence(proposal_type, &tool_calls)
        .map_err(invalid_disposition)?;
    let idempotency_key = idempotency_key(&headers)?;
    let request_fingerprint = serde_json::to_string(&json!({
        "run_id": run_id,
        "proposal_type": proposal_type,
    }))
    .map_err(internal_error)?;
    let result = state
        .repository
        .create_disposition_draft(
            &DispositionProposalDraft {
                id: Uuid::new_v4(),
                tenant_id: identity.tenant_id,
                factory_id: identity.factory_id(),
                initiated_by_user_id: identity.user_id,
                source_run_id: run_id,
                proposal_type,
                target_refs: generated.target_refs,
                evidence_refs: generated.evidence_refs,
                evidence_state: generated.evidence_state,
                recommended_actions: generated.recommended_actions,
                summary: generated.summary,
                expires_at: Utc::now() + Duration::hours(24),
            },
            &idempotency_key,
            &request_fingerprint,
        )
        .await
        .map_err(disposition_error)?;
    Ok((StatusCode::CREATED, Json(json!({"data": result.proposal}))))
}

async fn get_disposition_proposal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(proposal_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let identity = identity(&state, &headers).await?;
    if !has_disposition_read_capability(&identity) {
        record_authorization(
            &state,
            &headers,
            &identity,
            "disposition_proposal.read",
            "disposition_proposal",
            Some(proposal_id.to_string()),
            "denied",
            Some("missing_capability"),
        )
        .await?;
        return Err(authorization_denied());
    }
    let mut proposal = state
        .repository
        .get_disposition_proposal(identity.tenant_id, identity.factory_id(), proposal_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    let tenant_read = identity
        .capabilities
        .iter()
        .any(|value| value == "read:disposition_proposals:tenant");
    let self_read = proposal.initiated_by_user_id == identity.user_id
        && identity
            .capabilities
            .iter()
            .any(|value| value == "read:disposition_proposals:self");
    if !tenant_read && !self_read {
        record_authorization(
            &state,
            &headers,
            &identity,
            "disposition_proposal.read",
            "disposition_proposal",
            Some(proposal_id.to_string()),
            "denied",
            Some("resource_scope"),
        )
        .await?;
        return Err(not_found());
    }
    proposal = state
        .repository
        .expire_disposition_proposal(identity.tenant_id, identity.factory_id(), proposal_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    record_authorization(
        &state,
        &headers,
        &identity,
        "disposition_proposal.read",
        "disposition_proposal",
        Some(proposal_id.to_string()),
        "authorized",
        None,
    )
    .await?;
    Ok(Json(json!({"data": proposal})))
}

async fn submit_disposition_proposal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(proposal_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let identity = identity(&state, &headers).await?;
    enforce_capabilities(
        &state,
        &headers,
        &identity,
        &["create:disposition_proposals"],
        "disposition_proposal.submit",
        "disposition_proposal",
        Some(proposal_id.to_string()),
    )
    .await?;
    let proposal = state
        .repository
        .get_disposition_proposal(identity.tenant_id, identity.factory_id(), proposal_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    if proposal.initiated_by_user_id != identity.user_id {
        return Err(not_found());
    }
    state
        .repository
        .expire_disposition_proposal(identity.tenant_id, identity.factory_id(), proposal_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    let expected_version = expected_version(&headers)?;
    let request_fingerprint = serde_json::to_string(&json!({
        "operation": "submit",
        "proposal_id": proposal_id,
        "expected_version": expected_version,
    }))
    .map_err(internal_error)?;
    let result = state
        .repository
        .submit_disposition_proposal(&DispositionTransitionCommand {
            request_id: request_id(&headers),
            tenant_id: identity.tenant_id,
            factory_id: identity.factory_id(),
            actor_user_id: identity.user_id,
            proposal_id,
            expected_version,
            idempotency_key: idempotency_key(&headers)?,
            request_fingerprint,
        })
        .await
        .map_err(disposition_error)?;
    Ok(Json(json!({"data": {"proposal": result.proposal}})))
}

async fn cancel_disposition_proposal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(proposal_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let identity = identity(&state, &headers).await?;
    enforce_capabilities(
        &state,
        &headers,
        &identity,
        &["create:disposition_proposals"],
        "disposition_proposal.cancel",
        "disposition_proposal",
        Some(proposal_id.to_string()),
    )
    .await?;
    let proposal = state
        .repository
        .get_disposition_proposal(identity.tenant_id, identity.factory_id(), proposal_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    if proposal.initiated_by_user_id != identity.user_id {
        return Err(not_found());
    }
    state
        .repository
        .expire_disposition_proposal(identity.tenant_id, identity.factory_id(), proposal_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    let expected_version = expected_version(&headers)?;
    let request_fingerprint = serde_json::to_string(&json!({
        "operation": "cancel",
        "proposal_id": proposal_id,
        "expected_version": expected_version,
    }))
    .map_err(internal_error)?;
    let result = state
        .repository
        .cancel_disposition_proposal(&DispositionTransitionCommand {
            request_id: request_id(&headers),
            tenant_id: identity.tenant_id,
            factory_id: identity.factory_id(),
            actor_user_id: identity.user_id,
            proposal_id,
            expected_version,
            idempotency_key: idempotency_key(&headers)?,
            request_fingerprint,
        })
        .await
        .map_err(disposition_error)?;
    Ok(Json(json!({"data": {"proposal": result.proposal}})))
}

async fn review_disposition_proposal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(proposal_id): Path<Uuid>,
    Json(request): Json<ReviewDispositionProposal>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let identity = identity(&state, &headers).await?;
    enforce_capabilities(
        &state,
        &headers,
        &identity,
        &["review:disposition_proposals"],
        "disposition_proposal.review",
        "disposition_proposal",
        Some(proposal_id.to_string()),
    )
    .await?;
    let proposal = state
        .repository
        .get_disposition_proposal(identity.tenant_id, identity.factory_id(), proposal_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    if proposal.initiated_by_user_id == identity.user_id {
        record_authorization(
            &state,
            &headers,
            &identity,
            "disposition_proposal.review.separation_of_duties",
            "disposition_proposal",
            Some(proposal_id.to_string()),
            "denied",
            Some("initiator_cannot_review"),
        )
        .await?;
        return Err(authorization_denied());
    }
    state
        .repository
        .expire_disposition_proposal(identity.tenant_id, identity.factory_id(), proposal_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    let decision = request
        .decision
        .parse::<ReviewDecision>()
        .map_err(|_| invalid_disposition("unknown review decision"))?;
    let rationale = request
        .rationale
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if rationale.as_ref().is_some_and(|value| value.len() > 2_000) {
        return Err(invalid_disposition(
            "review rationale must not exceed 2000 characters",
        ));
    }
    let expected_version = expected_version(&headers)?;
    let request_fingerprint = serde_json::to_string(&json!({
        "operation": "review",
        "proposal_id": proposal_id,
        "expected_version": expected_version,
        "decision": decision,
        "rationale": rationale,
    }))
    .map_err(internal_error)?;
    let result = state
        .repository
        .review_disposition_proposal(&DispositionReviewCommand {
            transition: DispositionTransitionCommand {
                request_id: request_id(&headers),
                tenant_id: identity.tenant_id,
                factory_id: identity.factory_id(),
                actor_user_id: identity.user_id,
                proposal_id,
                expected_version,
                idempotency_key: idempotency_key(&headers)?,
                request_fingerprint,
            },
            decision,
            rationale,
        })
        .await
        .map_err(disposition_error)?;
    Ok(Json(json!({"data": {
        "proposal": result.proposal,
        "review": result.review,
    }})))
}

async fn create_action_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(proposal_id): Path<Uuid>,
    Json(_request): Json<CreateActionPlan>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let identity = identity(&state, &headers).await?;
    enforce_capabilities(
        &state,
        &headers,
        &identity,
        &["create:action_plans"],
        "action_plan.create",
        "disposition_proposal",
        Some(proposal_id.to_string()),
    )
    .await?;
    let proposal = state
        .repository
        .get_disposition_proposal(identity.tenant_id, identity.factory_id(), proposal_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    if !can_read_disposition_proposal(&identity, proposal.initiated_by_user_id) {
        return Err(not_found());
    }
    if proposal.status != ProposalStatus::ReviewApproved {
        return Err(invalid_action_plan(
            "source proposal must be independently review-approved",
        ));
    }
    let compiled = compile_action_plan(&proposal).map_err(action_plan_error)?;
    let idempotency_key = idempotency_key(&headers)?;
    let request_fingerprint = serde_json::to_string(&json!({
        "proposal_id": proposal_id,
        "proposal_version": proposal.version,
        "plan_type": compiled.plan_type,
        "steps": &compiled.steps,
        "source_evidence_state": compiled.source_evidence_state,
        "compiler_policy_version": compiled.compiler_policy_version,
    }))
    .map_err(internal_error)?;
    let compiled_at = Utc::now();
    let result = state
        .repository
        .create_action_plan_draft(
            &ActionPlanDraft {
                id: Uuid::new_v4(),
                tenant_id: identity.tenant_id,
                factory_id: identity.factory_id(),
                created_by_user_id: identity.user_id,
                source_proposal_id: proposal_id,
                source_proposal_version: proposal.version,
                plan_type: compiled.plan_type,
                steps: compiled.steps,
                source_evidence_state: compiled.source_evidence_state,
                compiler_policy_version: compiled.compiler_policy_version.to_owned(),
                compiled_at,
                summary: compiled.summary,
                expires_at: Utc::now() + Duration::hours(24),
            },
            &idempotency_key,
            &request_fingerprint,
        )
        .await
        .map_err(action_plan_error)?;
    Ok((StatusCode::CREATED, Json(json!({"data": result.plan}))))
}

async fn get_action_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(plan_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let identity = identity(&state, &headers).await?;
    if !has_action_plan_read_capability(&identity) {
        record_authorization(
            &state,
            &headers,
            &identity,
            "action_plan.read",
            "action_plan",
            Some(plan_id.to_string()),
            "denied",
            Some("missing_capability"),
        )
        .await?;
        return Err(authorization_denied());
    }
    let plan = state
        .repository
        .get_action_plan(identity.tenant_id, identity.factory_id(), plan_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    let tenant_read = identity
        .capabilities
        .iter()
        .any(|value| value == "read:action_plans:tenant");
    let self_read = plan.created_by_user_id == identity.user_id
        && identity
            .capabilities
            .iter()
            .any(|value| value == "read:action_plans:self");
    if !tenant_read && !self_read {
        record_authorization(
            &state,
            &headers,
            &identity,
            "action_plan.read",
            "action_plan",
            Some(plan_id.to_string()),
            "denied",
            Some("resource_scope"),
        )
        .await?;
        return Err(not_found());
    }
    record_authorization(
        &state,
        &headers,
        &identity,
        "action_plan.read",
        "action_plan",
        Some(plan_id.to_string()),
        "authorized",
        None,
    )
    .await?;
    Ok(Json(json!({"data": plan})))
}

async fn validate_action_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(plan_id): Path<Uuid>,
    Json(_request): Json<CreateActionPlan>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let identity = identity(&state, &headers).await?;
    enforce_capabilities(
        &state,
        &headers,
        &identity,
        &["validate:action_plans"],
        "action_plan.validate",
        "action_plan",
        Some(plan_id.to_string()),
    )
    .await?;
    let plan = state
        .repository
        .get_action_plan(identity.tenant_id, identity.factory_id(), plan_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    if plan.created_by_user_id != identity.user_id {
        return Err(not_found());
    }
    let expected_version = expected_version(&headers)?;
    let request_fingerprint = serde_json::to_string(&json!({
        "operation": "validate",
        "plan_id": plan_id,
        "expected_version": expected_version,
        "validation_policy_version": ACTION_PLAN_VALIDATION_POLICY_VERSION,
    }))
    .map_err(internal_error)?;
    let result = state
        .repository
        .validate_action_plan(&ActionPlanValidationCommand {
            request_id: request_id(&headers),
            tenant_id: identity.tenant_id,
            factory_id: identity.factory_id(),
            actor_user_id: identity.user_id,
            plan_id,
            expected_version,
            idempotency_key: idempotency_key(&headers)?,
            request_fingerprint,
        })
        .await
        .map_err(action_plan_error)?;
    Ok(Json(json!({"data": {
        "plan": result.plan,
        "snapshot": result.snapshot,
    }})))
}

fn summary_tool_call(
    call_id: &str,
    tool_name: &str,
    line_id: Uuid,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
) -> ModelToolCall {
    ModelToolCall {
        call_id: call_id.to_owned(),
        tool_name: tool_name.to_owned(),
        arguments: json!({
            "line_id": line_id,
            "start_at": start_at,
            "end_at": end_at
        }),
    }
}

async fn identity(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthenticatedPrincipal, ApiError> {
    let principal = match state.identity_service.authenticate(headers).await {
        Ok(principal) => principal,
        Err(error) => {
            if let Err(audit_error) = record_authentication_denial(state, headers, error).await {
                tracing::error!(
                    code = audit_error.code,
                    "authentication denial audit failed"
                );
            }
            return Err(auth_error(error));
        }
    };
    if !state
        .rate_limiter
        .allow(&format!("{}:{}", principal.tenant_id, principal.user_id))
    {
        record_authorization(
            state,
            headers,
            &principal,
            "protected_request.rate_limit",
            "http_request",
            None,
            "denied",
            Some("rate_limit"),
        )
        .await?;
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_exceeded",
            "request rate limit exceeded",
        ));
    }
    Ok(principal)
}

fn auth_error(error: AuthError) -> ApiError {
    match error {
        AuthError::Unauthenticated => ApiError::new(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "valid authentication is required",
        ),
        AuthError::Forbidden => ApiError::new(
            StatusCode::FORBIDDEN,
            "authorization_denied",
            "the authenticated identity has no access to the requested scope",
        ),
        AuthError::Unavailable => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "authentication_unavailable",
            "authentication is temporarily unavailable",
        ),
    }
}

fn has_agent_run_read_capability(principal: &AuthenticatedPrincipal) -> bool {
    principal.capabilities.iter().any(|capability| {
        matches!(
            capability.as_str(),
            "read:agent_runs:self" | "read:agent_runs:tenant"
        )
    })
}

fn has_disposition_read_capability(principal: &AuthenticatedPrincipal) -> bool {
    principal.capabilities.iter().any(|capability| {
        matches!(
            capability.as_str(),
            "read:disposition_proposals:self" | "read:disposition_proposals:tenant"
        )
    })
}

fn can_read_disposition_proposal(
    principal: &AuthenticatedPrincipal,
    initiated_by_user_id: Uuid,
) -> bool {
    principal
        .capabilities
        .iter()
        .any(|value| value == "read:disposition_proposals:tenant")
        || (initiated_by_user_id == principal.user_id
            && principal
                .capabilities
                .iter()
                .any(|value| value == "read:disposition_proposals:self"))
}

fn has_action_plan_read_capability(principal: &AuthenticatedPrincipal) -> bool {
    principal.capabilities.iter().any(|value| {
        matches!(
            value.as_str(),
            "read:action_plans:self" | "read:action_plans:tenant"
        )
    })
}

fn can_read_agent_run(principal: &AuthenticatedPrincipal, run_user_id: &str) -> bool {
    if principal
        .capabilities
        .iter()
        .any(|capability| capability == "read:agent_runs:tenant")
    {
        return true;
    }
    let owns_run = run_user_id == principal.user_id.to_string()
        || (principal.auth_method == auth::AuthMethod::DevHeaders
            && run_user_id == principal.subject);
    owns_run
        && principal
            .capabilities
            .iter()
            .any(|capability| capability == "read:agent_runs:self")
}

fn missing_any_capability(principal: &AuthenticatedPrincipal, required: &[&str]) -> bool {
    required.iter().any(|required| {
        !principal
            .capabilities
            .iter()
            .any(|candidate| candidate == required)
    })
}

#[allow(clippy::too_many_arguments)]
async fn enforce_capabilities(
    state: &AppState,
    headers: &HeaderMap,
    principal: &AuthenticatedPrincipal,
    required: &[&str],
    action: &str,
    resource_type: &str,
    resource_id: Option<String>,
) -> Result<(), ApiError> {
    if missing_any_capability(principal, required) {
        record_authorization(
            state,
            headers,
            principal,
            action,
            resource_type,
            resource_id,
            "denied",
            Some("missing_capability"),
        )
        .await?;
        Err(authorization_denied())
    } else {
        record_authorization(
            state,
            headers,
            principal,
            action,
            resource_type,
            resource_id,
            "authorized",
            None,
        )
        .await
    }
}

#[allow(clippy::too_many_arguments)]
async fn record_authorization(
    state: &AppState,
    headers: &HeaderMap,
    principal: &AuthenticatedPrincipal,
    action: &str,
    resource_type: &str,
    resource_id: Option<String>,
    outcome: &str,
    denial_category: Option<&str>,
) -> Result<(), ApiError> {
    let request_id = request_id(headers);
    let auth_method = match principal.auth_method {
        auth::AuthMethod::DevHeaders => "dev_headers",
        auth::AuthMethod::Oidc => "oidc",
    };
    state
        .repository
        .record_authorization_decision(&AuthorizationDecisionAudit {
            id: Uuid::new_v4(),
            request_id,
            user_id: Some(principal.user_id),
            external_identity_id: Some(principal.external_identity_id),
            tenant_id: Some(principal.tenant_id),
            factory_id: Some(principal.factory_id()),
            auth_method: auth_method.to_owned(),
            issuer: principal.issuer.clone(),
            roles: principal.roles.clone(),
            capabilities: principal.capabilities.clone(),
            action: action.to_owned(),
            resource_type: resource_type.to_owned(),
            resource_id,
            outcome: outcome.to_owned(),
            denial_category: denial_category.map(ToOwned::to_owned),
            duration_ms: 0,
        })
        .await
        .map_err(internal_error)
}

async fn record_authentication_denial(
    state: &AppState,
    headers: &HeaderMap,
    error: AuthError,
) -> Result<(), ApiError> {
    let request_id = request_id(headers);
    let denial_category = match error {
        AuthError::Unauthenticated => "authentication_invalid",
        AuthError::Forbidden => "local_identity_denied",
        AuthError::Unavailable => "authentication_unavailable",
    };
    state
        .repository
        .record_authorization_decision(&AuthorizationDecisionAudit {
            id: Uuid::new_v4(),
            request_id,
            user_id: None,
            external_identity_id: None,
            tenant_id: None,
            factory_id: None,
            auth_method: state.identity_service.provider_name().to_owned(),
            issuer: "unverified".to_owned(),
            roles: Vec::new(),
            capabilities: Vec::new(),
            action: "protected_request.authenticate".to_owned(),
            resource_type: "http_request".to_owned(),
            resource_id: None,
            outcome: "denied".to_owned(),
            denial_category: Some(denial_category.to_owned()),
            duration_ms: 0,
        })
        .await
        .map_err(internal_error)
}

fn authorization_denied() -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "authorization_denied",
        "the authenticated identity is not authorized for this operation",
    )
}

fn idempotency_key(headers: &HeaderMap) -> Result<String, ApiError> {
    let values = headers.get_all("idempotency-key");
    let mut values = values.iter();
    let value = values
        .next()
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .ok_or_else(|| {
            invalid_disposition(
                "one Idempotency-Key header containing 1 to 128 characters is required",
            )
        })?;
    if values.next().is_some() || !value.is_ascii() {
        return Err(invalid_disposition(
            "exactly one ASCII Idempotency-Key header is required",
        ));
    }
    Ok(value.to_owned())
}

fn expected_version(headers: &HeaderMap) -> Result<i64, ApiError> {
    let values = headers.get_all("if-match");
    let mut values = values.iter();
    let value = values
        .next()
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .map(|value| value.trim_matches('"'))
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            invalid_disposition(
                "one If-Match header containing a positive proposal version is required",
            )
        })?;
    if values.next().is_some() {
        return Err(invalid_disposition(
            "exactly one If-Match proposal version is required",
        ));
    }
    Ok(value)
}

fn request_id(headers: &HeaderMap) -> Uuid {
    headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .unwrap_or_else(Uuid::new_v4)
}

fn invalid_disposition(message: impl Into<String>) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_disposition_proposal",
        message,
    )
}

fn disposition_error(error: ApplicationError) -> ApiError {
    match error {
        ApplicationError::Conflict(_) => ApiError::new(
            StatusCode::CONFLICT,
            "proposal_conflict",
            "the idempotency key or proposal version conflicts with persisted state",
        ),
        ApplicationError::Precondition(_) => invalid_disposition(
            "the disposition proposal does not satisfy transition or evidence preconditions",
        ),
        other => internal_error(other),
    }
}

fn invalid_action_plan(message: impl Into<String>) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_action_plan",
        message,
    )
}

fn action_plan_error(error: ApplicationError) -> ApiError {
    match error {
        ApplicationError::Conflict(_) => ApiError::new(
            StatusCode::CONFLICT,
            "action_plan_conflict",
            "the idempotency key conflicts with persisted action-plan state",
        ),
        ApplicationError::Precondition(_) => {
            invalid_action_plan("the action plan does not satisfy validation preconditions")
        }
        ApplicationError::Drift(_) => ApiError::new(
            StatusCode::CONFLICT,
            "action_plan_drifted",
            "trusted manufacturing state differs from the validated action-plan snapshot",
        ),
        other => internal_error(other),
    }
}

fn validate_window(query: &SummaryQuery) -> Result<(), ApiError> {
    let duration = query.end_at - query.start_at;
    if duration <= Duration::zero() || duration > Duration::days(MAX_WINDOW_DAYS) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_time_window",
            "time window must be positive and at most 31 days",
        ));
    }
    Ok(())
}

fn not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "resource_not_found",
        "resource does not exist in the authorized tenant/factory scope",
    )
}

fn internal_error(error: impl std::fmt::Display) -> ApiError {
    tracing::error!(%error, "internal request failure");
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "the request could not be completed",
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::body::{Body, to_bytes};
    use http::{Request, StatusCode};
    use llm_provider::ScriptedMockProvider;
    use mes_infra::PostgresMesRepository;
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    use super::{AppState, AuthError, FixedWindowRateLimiter, LlmDataPolicy, auth_error, router};

    #[tokio::test]
    async fn health_endpoint_does_not_require_identity_or_database() {
        let pool = PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(50))
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("lazy URL must parse");
        let response = router(AppState::development(std::sync::Arc::new(
            PostgresMesRepository::from_pool(pool),
        )))
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .expect("request must be valid"),
        )
        .await
        .expect("health request must complete");

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().contains_key("x-request-id"));
        let body = to_bytes(response.into_body(), 1024)
            .await
            .expect("health body must be readable");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).expect("body must be JSON"),
            serde_json::json!({"status": "ok", "service": "mes-api"})
        );
    }

    #[tokio::test]
    async fn protected_endpoint_fails_closed_with_bearer_challenge() {
        const TOKEN_MARKER: &str = "m7-secret-bearer-marker";
        let pool = PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(50))
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("lazy URL must parse");
        let response = router(AppState::development(std::sync::Arc::new(
            PostgresMesRepository::from_pool(pool),
        )))
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/work-orders/{}", uuid::Uuid::new_v4()))
                .header("authorization", format!("Bearer {TOKEN_MARKER}"))
                .body(Body::empty())
                .expect("request must be valid"),
        )
        .await
        .expect("protected request must complete");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response
                .headers()
                .get("www-authenticate")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer realm=\"agentic-mes\"")
        );
        let body = to_bytes(response.into_body(), 4096)
            .await
            .expect("authentication error body must be readable");
        let body = String::from_utf8(body.to_vec()).expect("authentication error must be UTF-8");
        assert!(!body.contains(TOKEN_MARKER));
        assert!(body.contains("authentication_required"));
    }

    #[test]
    fn fixed_window_rate_limit_fails_closed_at_the_configured_bound() {
        let limiter = FixedWindowRateLimiter::new(1, Duration::from_secs(60));
        assert!(limiter.allow("tenant:user"));
        assert!(!limiter.allow("tenant:user"));
        assert!(limiter.allow("tenant:other-user"));
    }

    #[test]
    fn authentication_failures_map_to_the_frozen_http_statuses() {
        assert_eq!(
            auth_error(AuthError::Unauthenticated).status,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            auth_error(AuthError::Forbidden).status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            auth_error(AuthError::Unavailable).status,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn external_provider_requires_explicit_data_sharing_approval() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("lazy URL must parse");
        let result =
            AppState::development(std::sync::Arc::new(PostgresMesRepository::from_pool(pool)))
                .with_http_provider(
                    std::sync::Arc::new(ScriptedMockProvider::new(Vec::new(), "unused")),
                    LlmDataPolicy::LocalOnly,
                );
        assert!(result.is_err());
    }
}
