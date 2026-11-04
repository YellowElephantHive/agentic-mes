use std::{env, sync::Arc};

use agent_core::{AgentRunRequest, AgentRuntime, AuthorizedToolContext, MesTool, ToolRegistry};
use chrono::{DateTime, Utc};
use factory_simulator::{DEFAULT_LAST_DATE, SimulatorConfig, generate_plan};
use llm_provider::{ModelToolCall, ScriptedMockProvider};
use mes_application::{MesReadRepository, SeedRepository};
use mes_infra::PostgresMesRepository;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database_url = env::var("DATABASE_URL")?;
    let repository = Arc::new(PostgresMesRepository::connect(&database_url).await?);
    repository.migrate().await?;

    let plan = generate_plan(&SimulatorConfig {
        seed: 42,
        days: 30,
        factory_slug: "demo".to_owned(),
        last_date: DEFAULT_LAST_DATE,
    })?;
    let seed = repository.replace_seed_plan(&plan).await?;
    let line_id = plan.lines[0].id;
    let current_start = timestamp("2026-01-30T00:00:00Z")?;
    let current_end = timestamp("2026-01-31T00:00:00Z")?;
    let production = repository
        .production_summary(
            plan.tenant.id,
            plan.factory.id,
            line_id,
            current_start,
            current_end,
            500,
        )
        .await?
        .ok_or("Line A production summary was not found")?;
    if production.good_qty != 600 {
        return Err(format!(
            "unexpected Line A output: expected 600, got {}",
            production.good_qty
        )
        .into());
    }

    let provider = Arc::new(ScriptedMockProvider::new(
        vec![
            summary_call(
                "baseline",
                "get_production_summary",
                line_id,
                "2026-01-29T00:00:00Z",
                "2026-01-30T00:00:00Z",
            ),
            summary_call(
                "current",
                "get_production_summary",
                line_id,
                "2026-01-30T00:00:00Z",
                "2026-01-31T00:00:00Z",
            ),
            summary_call(
                "downtime",
                "get_equipment_downtime",
                line_id,
                "2026-01-30T00:00:00Z",
                "2026-01-31T00:00:00Z",
            ),
            summary_call(
                "quality",
                "get_quality_summary",
                line_id,
                "2026-01-30T00:00:00Z",
                "2026-01-31T00:00:00Z",
            ),
        ],
        "The deterministic walkthrough did not receive enough source observations.",
    ));
    let mut registry = ToolRegistry::default();
    registry.register(Arc::new(MesTool::work_order(repository.clone())));
    registry.register(Arc::new(MesTool::production(repository.clone())));
    registry.register(Arc::new(MesTool::downtime(repository.clone())));
    registry.register(Arc::new(MesTool::quality(repository.clone())));

    let run_id = Uuid::new_v4();
    let question = "Why did Line A output drop on 2026-01-30?";
    let result = AgentRuntime::new(provider, repository.clone(), registry)
        .run(AgentRunRequest {
            agent_id: "production-analyst".to_owned(),
            message: question.to_owned(),
            context: AuthorizedToolContext {
                user_id: "m5-walkthrough".to_owned(),
                tenant_id: plan.tenant.id,
                permitted_factory_ids: vec![plan.factory.id],
                run_id,
                capabilities: vec!["read:mes".to_owned()],
            },
        })
        .await?;

    let audit = sqlx::query(
        "SELECT ar.status, ar.error_code, COUNT(atc.id)::BIGINT AS tool_calls \
         FROM agent_runs ar LEFT JOIN agent_tool_calls atc ON atc.run_id=ar.id \
         WHERE ar.id=$1 AND ar.tenant_id=$2 GROUP BY ar.status, ar.error_code",
    )
    .bind(run_id)
    .bind(plan.tenant.id)
    .fetch_one(repository.pool())
    .await?;
    let status: String = audit.get("status");
    let tool_calls: i64 = audit.get("tool_calls");
    if status != "completed" || tool_calls != 4 || result.tool_calls != 4 {
        return Err("agent run or audit trail did not meet the M5 acceptance check".into());
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": "PASS",
            "seed": seed,
            "identity": {
                "tenant_id": plan.tenant.id,
                "factory_id": plan.factory.id,
                "line_id": line_id
            },
            "mes_query": {
                "window": [current_start, current_end],
                "good_qty": production.good_qty,
                "evidence_records": production.records.len()
            },
            "agent": {
                "question": question,
                "run_id": result.run_id,
                "answer": result.answer,
                "evidence": result.evidence,
                "tool_calls": result.tool_calls
            },
            "audit": {
                "status": status,
                "error_code": audit.get::<Option<String>, _>("error_code"),
                "persisted_tool_calls": tool_calls
            }
        }))?
    );
    Ok(())
}

fn summary_call(
    call_id: &str,
    tool_name: &str,
    line_id: Uuid,
    start_at: &str,
    end_at: &str,
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

fn timestamp(value: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    value.parse()
}
