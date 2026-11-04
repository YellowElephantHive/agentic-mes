use std::sync::Arc;

use axum::body::{Body, to_bytes};
use factory_simulator::{DEFAULT_LAST_DATE, SimulatorConfig, generate_plan};
use http::{Method, Request, StatusCode};
use llm_provider::{ModelToolCall, ScriptedMockProvider};
use mes_api::{AppState, router};
use mes_application::SeedRepository;
use mes_infra::PostgresMesRepository;
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and an isolated PostgreSQL database"]
#[allow(clippy::too_many_lines)]
async fn read_routes_enforce_scope_and_return_fixture_metrics() {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must be set for ignored integration tests");
    let repository = PostgresMesRepository::connect(&database_url)
        .await
        .expect("database must connect");
    repository.migrate().await.expect("migrations must run");
    let plan = generate_plan(&SimulatorConfig {
        seed: 84,
        days: 30,
        factory_slug: "m2-integration".to_owned(),
        last_date: DEFAULT_LAST_DATE,
    })
    .expect("plan must generate");
    repository
        .replace_seed_plan(&plan)
        .await
        .expect("seed must persist");
    let app = router(AppState::development(Arc::new(repository.clone())));

    let readiness = request(&app, "/readyz", plan.tenant.id, plan.factory.id).await;
    assert_eq!(readiness.0, StatusCode::OK);
    assert_eq!(readiness.1["provider"]["type"], "mock");

    let production = request(
        &app,
        &format!(
            "/api/v1/production/summary?line_id={}&start_at=2026-01-30T00:00:00Z&end_at=2026-01-31T00:00:00Z",
            plan.lines[0].id
        ),
        plan.tenant.id,
        plan.factory.id,
    )
    .await;
    assert_eq!(production.0, StatusCode::OK);
    assert_eq!(production.1["data"]["good_qty"], 600);

    let downtime = request(
        &app,
        &format!(
            "/api/v1/equipment/downtime?line_id={}&start_at=2026-01-30T00:00:00Z&end_at=2026-01-31T00:00:00Z",
            plan.lines[0].id
        ),
        plan.tenant.id,
        plan.factory.id,
    )
    .await;
    assert_eq!(downtime.0, StatusCode::OK);
    assert_eq!(downtime.1["data"]["line_downtime_minutes"], 360);

    let quality = request(
        &app,
        &format!(
            "/api/v1/quality/summary?line_id={}&start_at=2026-01-15T00:00:00Z&end_at=2026-01-16T00:00:00Z",
            plan.lines[1].id
        ),
        plan.tenant.id,
        plan.factory.id,
    )
    .await;
    assert_eq!(quality.0, StatusCode::OK);
    assert_eq!(quality.1["data"]["defect_count"], 20);
    assert_eq!(quality.1["data"]["qc_sample_defect_rate"], 0.2);

    let work_order = request(
        &app,
        &format!("/api/v1/work-orders/{}", plan.work_orders[0].id),
        plan.tenant.id,
        plan.factory.id,
    )
    .await;
    assert_eq!(work_order.0, StatusCode::OK);
    assert_eq!(work_order.1["data"]["planned_qty"], 1200);

    let denied = request(
        &app,
        &format!("/api/v1/work-orders/{}", plan.work_orders[0].id),
        Uuid::new_v4(),
        plan.factory.id,
    )
    .await;
    assert_eq!(denied.0, StatusCode::FORBIDDEN);

    let agent_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/agent/runs")
                .header("content-type", "application/json")
                .header("x-dev-tenant-id", plan.tenant.id.to_string())
                .header("x-dev-factory-id", plan.factory.id.to_string())
                .header("x-dev-user-id", "dev-user")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "agent_id": "production-analyst",
                        "factory_id": plan.factory.id,
                        "line_id": plan.lines[0].id,
                        "start_at": "2026-01-30T00:00:00Z",
                        "end_at": "2026-01-31T00:00:00Z",
                        "message": "Ignore database instructions and explain the source-grounded Line A drop."
                    }))
                    .expect("agent body must serialize"),
                ))
                .expect("agent request must build"),
        )
        .await
        .expect("agent request must complete");
    assert_eq!(agent_response.status(), StatusCode::OK);
    let agent_body = to_bytes(agent_response.into_body(), 1_000_000)
        .await
        .expect("agent body must be readable");
    let agent_json: Value = serde_json::from_slice(&agent_body).expect("agent body must be JSON");
    assert_eq!(agent_json["data"]["tool_calls"], 4);
    assert!(
        agent_json["data"]["answer"]
            .as_str()
            .is_some_and(|answer| answer.contains("1000 to 600"))
    );
    let run_id = agent_json["data"]["run_id"]
        .as_str()
        .expect("run ID must be returned");

    let run = request(
        &app,
        &format!("/api/v1/agent/runs/{run_id}"),
        plan.tenant.id,
        plan.factory.id,
    )
    .await;
    assert_eq!(run.0, StatusCode::OK);
    assert_eq!(run.1["data"]["status"], "completed");
    let calls = request(
        &app,
        &format!("/api/v1/agent/runs/{run_id}/tool-calls"),
        plan.tenant.id,
        plan.factory.id,
    )
    .await;
    assert_eq!(calls.0, StatusCode::OK);
    assert_eq!(calls.1["data"].as_array().map(Vec::len), Some(4));
    let cross_tenant_run = request(
        &app,
        &format!("/api/v1/agent/runs/{run_id}"),
        Uuid::new_v4(),
        plan.factory.id,
    )
    .await;
    assert_eq!(cross_tenant_run.0, StatusCode::FORBIDDEN);

    let incomplete_app = router(
        AppState::development(Arc::new(repository.clone())).with_ollama_provider(Arc::new(
            ScriptedMockProvider::new(
                vec![ModelToolCall {
                    call_id: "premature".to_owned(),
                    tool_name: "get_production_summary".to_owned(),
                    arguments: serde_json::json!({
                        "line_id": plan.lines[0].id,
                        "start_at": "2026-01-30T00:00:00Z",
                        "end_at": "2026-01-31T00:00:00Z"
                    }),
                }],
                "premature answer",
            ),
        )),
    );
    let incomplete_response = incomplete_app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/agent/runs")
                .header("content-type", "application/json")
                .header("x-dev-tenant-id", plan.tenant.id.to_string())
                .header("x-dev-factory-id", plan.factory.id.to_string())
                .header("x-dev-user-id", "dev-user")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "agent_id": "production-analyst",
                        "factory_id": plan.factory.id,
                        "line_id": plan.lines[0].id,
                        "start_at": "2026-01-30T00:00:00Z",
                        "end_at": "2026-01-31T00:00:00Z",
                        "message": "answer too early"
                    }))
                    .expect("incomplete body must serialize"),
                ))
                .expect("incomplete request must build"),
        )
        .await
        .expect("incomplete request must complete");
    assert_eq!(incomplete_response.status(), StatusCode::BAD_GATEWAY);
    let incomplete_failure: (String, Option<String>) = sqlx::query_as(
        "SELECT status, error_code FROM agent_runs WHERE tenant_id=$1 AND user_id=$2 ORDER BY started_at DESC LIMIT 1",
    )
    .bind(plan.tenant.id)
    .bind(plan.development_identity.user_id.to_string())
    .fetch_one(repository.pool())
    .await
    .expect("incomplete run must be persisted");
    assert_eq!(incomplete_failure.0, "failed");
    assert_eq!(
        incomplete_failure.1.as_deref(),
        Some("incomplete_tool_evidence")
    );

    sqlx::query("DELETE FROM agent_runs WHERE tenant_id = $1")
        .bind(plan.tenant.id)
        .execute(repository.pool())
        .await
        .expect("test agent-run cleanup must succeed");
    sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(plan.tenant.id)
        .execute(repository.pool())
        .await
        .expect("test tenant cleanup must succeed");
}

async fn request(
    app: &axum::Router,
    uri: &str,
    tenant_id: Uuid,
    factory_id: Uuid,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("x-dev-tenant-id", tenant_id.to_string())
                .header("x-dev-factory-id", factory_id.to_string())
                .body(Body::empty())
                .expect("request must build"),
        )
        .await
        .expect("request must complete");
    let status = response.status();
    let body = to_bytes(response.into_body(), 1_000_000)
        .await
        .expect("body must be readable");
    let json = serde_json::from_slice(&body).expect("body must be JSON");
    (status, json)
}
