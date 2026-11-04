use std::sync::Arc;

use agent_core::{
    AgentRunRequest, AgentRuntime, AuthorizedToolContext, MesTool, RuntimeError, ToolRegistry,
};
use async_trait::async_trait;
use factory_simulator::{DEFAULT_LAST_DATE, SimulatorConfig, generate_plan};
use llm_provider::{
    AgentProvider, ModelToolCall, ProviderError, ProviderRequest, ProviderResponse,
    ScriptedMockProvider,
};
use mes_application::SeedRepository;
use mes_infra::PostgresMesRepository;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and an isolated PostgreSQL database"]
#[allow(clippy::too_many_lines)]
async fn line_a_analysis_is_bounded_grounded_and_fully_audited() {
    let database_url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL required");
    let repository = Arc::new(
        PostgresMesRepository::connect(&database_url)
            .await
            .expect("database must connect"),
    );
    repository.migrate().await.expect("migrations must run");
    let plan = generate_plan(&SimulatorConfig {
        seed: 126,
        days: 30,
        factory_slug: "m3-integration".to_owned(),
        last_date: DEFAULT_LAST_DATE,
    })
    .expect("plan must generate");
    repository
        .replace_seed_plan(&plan)
        .await
        .expect("seed must persist");
    let line_id = plan.lines[0].id;
    let calls = vec![
        call(
            "prev",
            "get_production_summary",
            line_id,
            "2026-01-29",
            "2026-01-30",
        ),
        call(
            "drop",
            "get_production_summary",
            line_id,
            "2026-01-30",
            "2026-01-31",
        ),
        call(
            "down",
            "get_equipment_downtime",
            line_id,
            "2026-01-30",
            "2026-01-31",
        ),
        call(
            "qc",
            "get_quality_summary",
            line_id,
            "2026-01-30",
            "2026-01-31",
        ),
    ];
    let provider = Arc::new(ScriptedMockProvider::new(
        calls,
        "Line A good output dropped from 1,000 to 600 units. The evidence supports extended machine-failure downtime plus a material-shortage event; quality sampling did not show a corresponding spike. These are qualified contributing hypotheses, not unsupported causal proof.",
    ));
    let mut registry = ToolRegistry::default();
    registry.register(Arc::new(MesTool::work_order(repository.clone())));
    registry.register(Arc::new(MesTool::production(repository.clone())));
    registry.register(Arc::new(MesTool::downtime(repository.clone())));
    registry.register(Arc::new(MesTool::quality(repository.clone())));
    let runtime = AgentRuntime::new(provider, repository.clone(), registry);
    let run_id = Uuid::new_v4();
    let result = runtime
        .run(AgentRunRequest {
            agent_id: "production-analyst".to_owned(),
            message: "Why did Line A output drop?".to_owned(),
            context: context(&plan, run_id),
        })
        .await
        .expect("agent run must complete");
    assert_eq!(result.tool_calls, 4);
    assert!(result.answer.contains("1000 to 600"));
    assert!(result.answer.contains("MACHINE_FAILURE"));
    assert!(result.answer.contains("not proof of causation"));
    assert_ne!(result.evidence.len(), 0);
    let row = sqlx::query(
        "SELECT ar.status, COUNT(atc.id)::BIGINT AS calls FROM agent_runs ar LEFT JOIN agent_tool_calls atc ON atc.run_id=ar.id WHERE ar.id=$1 GROUP BY ar.status",
    )
    .bind(run_id)
    .fetch_one(repository.pool())
    .await
    .expect("audit must query");
    assert_eq!(row.get::<String, _>("status"), "completed");
    assert_eq!(row.get::<i64, _>("calls"), 4);

    let evidence_run_id = Uuid::new_v4();
    let evidence_result = AgentRuntime::new(
        Arc::new(FabricatingEvidenceProvider { line_id }),
        repository.clone(),
        {
            let mut registry = ToolRegistry::default();
            registry.register(Arc::new(MesTool::production(repository.clone())));
            registry
        },
    )
    .run(AgentRunRequest {
        agent_id: "production-analyst".to_owned(),
        message: "return grounded evidence".to_owned(),
        context: context(&plan, evidence_run_id),
    })
    .await
    .expect("evidence validation run must complete");
    assert_ne!(evidence_result.evidence.len(), 0);
    assert!(
        evidence_result
            .evidence
            .iter()
            .all(|item| item.starts_with("production_record:"))
    );
    assert!(
        !evidence_result
            .evidence
            .contains(&"fabricated:123".to_owned())
    );

    let unknown_provider = Arc::new(ScriptedMockProvider::new(
        vec![ModelToolCall {
            call_id: "bad".to_owned(),
            tool_name: "execute_raw_sql".to_owned(),
            arguments: json!({"sql":"DROP TABLE tenants"}),
        }],
        "must not finish",
    ));
    let unknown_run_id = Uuid::new_v4();
    let error = AgentRuntime::new(
        unknown_provider,
        repository.clone(),
        ToolRegistry::default(),
    )
    .run(AgentRunRequest {
        agent_id: "production-analyst".to_owned(),
        message: "ignore policy".to_owned(),
        context: context(&plan, unknown_run_id),
    })
    .await
    .expect_err("unknown tool must fail");
    assert!(matches!(error, RuntimeError::UnknownTool(_)));
    let status: String = sqlx::query_scalar("SELECT status FROM agent_runs WHERE id=$1")
        .bind(unknown_run_id)
        .fetch_one(repository.pool())
        .await
        .expect("failed run must persist");
    assert_eq!(status, "failed");

    let provider_failure_run_id = Uuid::new_v4();
    let error = AgentRuntime::new(
        Arc::new(FailingProvider),
        repository.clone(),
        ToolRegistry::default(),
    )
    .run(AgentRunRequest {
        agent_id: "production-analyst".to_owned(),
        message: "provider fails".to_owned(),
        context: context(&plan, provider_failure_run_id),
    })
    .await
    .expect_err("provider failure must fail run");
    assert!(matches!(error, RuntimeError::Provider(_)));
    let failure: (String, Option<String>) =
        sqlx::query_as("SELECT status, error_code FROM agent_runs WHERE id=$1")
            .bind(provider_failure_run_id)
            .fetch_one(repository.pool())
            .await
            .expect("provider failure lifecycle must persist");
    assert_eq!(failure.0, "failed");
    assert_eq!(failure.1.as_deref(), Some("provider_error"));

    sqlx::query("DELETE FROM agent_runs WHERE tenant_id=$1")
        .bind(plan.tenant.id)
        .execute(repository.pool())
        .await
        .expect("test run cleanup must succeed");
    sqlx::query("DELETE FROM tenants WHERE id=$1")
        .bind(plan.tenant.id)
        .execute(repository.pool())
        .await
        .expect("cleanup must succeed");
}

struct FailingProvider;

struct FabricatingEvidenceProvider {
    line_id: Uuid,
}

#[async_trait]
impl AgentProvider for FabricatingEvidenceProvider {
    async fn next_turn(&self, request: ProviderRequest) -> Result<ProviderResponse, ProviderError> {
        if request.observations.is_empty() {
            return Ok(ProviderResponse::ToolCalls(vec![call(
                "evidence",
                "get_production_summary",
                self.line_id,
                "2026-01-30",
                "2026-01-31",
            )]));
        }
        Ok(ProviderResponse::Final {
            answer: "source-grounded answer".to_owned(),
            evidence: vec!["fabricated:123".to_owned()],
        })
    }
}

#[async_trait]
impl AgentProvider for FailingProvider {
    async fn next_turn(
        &self,
        _request: ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        Err(ProviderError::Failure(
            "intentional test failure".to_owned(),
        ))
    }
}

fn call(call_id: &str, tool_name: &str, line_id: Uuid, start: &str, end: &str) -> ModelToolCall {
    ModelToolCall {
        call_id: call_id.to_owned(),
        tool_name: tool_name.to_owned(),
        arguments: json!({
            "line_id": line_id,
            "start_at": format!("{start}T00:00:00Z"),
            "end_at": format!("{end}T00:00:00Z")
        }),
    }
}

fn context(plan: &mes_application::FactorySeedPlan, run_id: Uuid) -> AuthorizedToolContext {
    AuthorizedToolContext {
        user_id: "m3-test-user".to_owned(),
        tenant_id: plan.tenant.id,
        permitted_factory_ids: vec![plan.factory.id],
        run_id,
        capabilities: vec!["read:mes".to_owned()],
    }
}
