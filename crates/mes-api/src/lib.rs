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
    http::{HeaderMap, HeaderName, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Duration, Utc};
use llm_provider::{AgentProvider, ModelToolCall, ScriptedMockProvider};
use mes_application::{AgentAuditRepository, MesReadRepository};
use mes_infra::PostgresMesRepository;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use uuid::Uuid;

const MAX_WINDOW_DAYS: i64 = 31;
const MAX_EVIDENCE_ROWS: i64 = 500;

#[derive(Clone)]
pub struct AppState {
    pub repository: Arc<PostgresMesRepository>,
    pub development_identity_enabled: bool,
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
        Self {
            repository,
            development_identity_enabled: true,
            rate_limiter: Arc::new(FixedWindowRateLimiter::new(120, StdDuration::from_secs(60))),
            provider: ProviderSelection::Mock,
            data_policy: LlmDataPolicy::LocalOnly,
            runtime_limits: AgentRuntimeLimits::default(),
        }
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

#[derive(Debug)]
#[allow(clippy::struct_field_names)]
struct DevIdentity {
    tenant_id: Uuid,
    factory_id: Uuid,
    user_id: String,
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
        (
            self.status,
            Json(json!({
                "error": {
                    "code": self.code,
                    "message": self.message,
                    "request_id": self.request_id,
                }
            })),
        )
            .into_response()
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
    Ok(Json(json!({
        "status": "ready",
        "database": "ready",
        "provider": {
            "type": state.provider.name(),
            "status": "configured"
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
    let identity = identity(&state, &headers)?;
    let result = state
        .repository
        .get_work_order(identity.tenant_id, identity.factory_id, id)
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
    let identity = identity(&state, &headers)?;
    let result = state
        .repository
        .production_summary(
            identity.tenant_id,
            identity.factory_id,
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
    let identity = identity(&state, &headers)?;
    let result = state
        .repository
        .downtime_summary(
            identity.tenant_id,
            identity.factory_id,
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
    let identity = identity(&state, &headers)?;
    let result = state
        .repository
        .quality_summary(
            identity.tenant_id,
            identity.factory_id,
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
    let identity = identity(&state, &headers)?;
    if request.factory_id != identity.factory_id {
        return Err(not_found());
    }
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
                user_id: identity.user_id,
                tenant_id: identity.tenant_id,
                permitted_factory_ids: vec![identity.factory_id],
                run_id,
                capabilities: vec!["read:mes".to_owned()],
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
    let identity = identity(&state, &headers)?;
    let run = state
        .repository
        .get_run(identity.tenant_id, run_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    Ok(Json(json!({"data": run})))
}

async fn get_agent_tool_calls(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let identity = identity(&state, &headers)?;
    state
        .repository
        .get_run(identity.tenant_id, run_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    let calls = state
        .repository
        .get_tool_calls(identity.tenant_id, run_id)
        .await
        .map_err(internal_error)?;
    Ok(Json(json!({"data": calls})))
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

fn identity(state: &AppState, headers: &HeaderMap) -> Result<DevIdentity, ApiError> {
    if !state.development_identity_enabled {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "development identity is disabled",
        ));
    }
    let tenant_id = parse_header_uuid(headers, "x-dev-tenant-id")?;
    let factory_id = parse_header_uuid(headers, "x-dev-factory-id")?;
    let user_id = headers
        .get("x-dev-user-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("dev-user")
        .trim();
    if user_id.is_empty() || user_id.len() > 128 {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_development_identity",
            "x-dev-user-id must contain 1 to 128 characters",
        ));
    }
    if !state.rate_limiter.allow(&format!("{tenant_id}:{user_id}")) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_exceeded",
            "request rate limit exceeded",
        ));
    }
    Ok(DevIdentity {
        tenant_id,
        factory_id,
        user_id: user_id.to_owned(),
    })
}

fn parse_header_uuid(headers: &HeaderMap, name: &'static str) -> Result<Uuid, ApiError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "invalid_development_identity",
                format!("missing or invalid {name}"),
            )
        })
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

    use super::{AppState, FixedWindowRateLimiter, LlmDataPolicy, router};

    #[tokio::test]
    async fn health_endpoint_does_not_require_identity_or_database() {
        let pool = PgPoolOptions::new()
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

    #[test]
    fn fixed_window_rate_limit_fails_closed_at_the_configured_bound() {
        let limiter = FixedWindowRateLimiter::new(1, Duration::from_secs(60));
        assert!(limiter.allow("tenant:user"));
        assert!(!limiter.allow("tenant:user"));
        assert!(limiter.allow("tenant:other-user"));
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
