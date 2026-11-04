use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use llm_provider::{
    AgentProvider, HttpProvider, HttpProviderConfig, ProviderRequest, ProviderResponse,
    ResilientProvider,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;

#[tokio::test]
async fn maps_tool_calls_and_retries_transient_failure() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/provider", post(retrying_handler))
        .with_state(calls.clone());
    let endpoint = spawn(app).await;
    let provider =
        HttpProvider::new(HttpProviderConfig::for_test(endpoint)).expect("provider must build");
    let response = provider
        .next_turn(request())
        .await
        .expect("second attempt must succeed");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    match response {
        ProviderResponse::ToolCalls(tool_calls) => {
            assert_eq!(tool_calls.len(), 1);
            assert_eq!(tool_calls[0].tool_name, "get_quality_summary");
            assert_eq!(tool_calls[0].arguments["line_id"], "line-from-provider");
        }
        ProviderResponse::Final { .. } => panic!("expected tool calls"),
    }
}

#[tokio::test]
async fn rejects_ambiguous_provider_response() {
    let app = Router::new().route(
        "/provider",
        post(|| async {
            Json(json!({
                "answer":"unsafe ambiguity",
                "tool_calls":[{
                    "call_id":"1",
                    "tool_name":"get_work_order",
                    "arguments":{"work_order_id":"x"}
                }]
            }))
        }),
    );
    let provider = HttpProvider::new(HttpProviderConfig::for_test(spawn(app).await))
        .expect("provider must build");
    assert!(provider.next_turn(request()).await.is_err());
}

#[tokio::test]
async fn circuit_breaker_opens_after_bounded_consecutive_failures() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed_calls = calls.clone();
    let app = Router::new().route(
        "/provider",
        post(move || {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                (StatusCode::BAD_REQUEST, Json(json!({"error":"failed"})))
            }
        }),
    );
    let inner = Arc::new(
        HttpProvider::new(HttpProviderConfig::for_test(spawn(app).await))
            .expect("provider must build"),
    );
    let provider = ResilientProvider::new(inner, 1, 2, Duration::from_secs(60))
        .expect("resilience wrapper must build");
    assert!(provider.next_turn(request()).await.is_err());
    assert!(provider.next_turn(request()).await.is_err());
    let third = provider
        .next_turn(request())
        .await
        .expect_err("open circuit must reject without another provider request");
    assert!(third.to_string().contains("circuit breaker is open"));
    assert_eq!(observed_calls.load(Ordering::SeqCst), 2);
}

async fn retrying_handler(
    State(calls): State<Arc<AtomicUsize>>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    assert_eq!(body["model"], "test-model");
    assert!(
        body["system"]
            .as_str()
            .is_some_and(|text| text.contains("Do not invent"))
    );
    if calls.fetch_add(1, Ordering::SeqCst) == 0 {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"retry"})),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "tool_calls":[{
                "call_id":"call-1",
                "tool_name":"get_quality_summary",
                "arguments":{"line_id":"line-from-provider"}
            }]
        })),
    )
}

async fn spawn(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener must bind");
    let address = listener.local_addr().expect("address must exist");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("test server must run");
    });
    format!("http://{address}/provider")
}

fn request() -> ProviderRequest {
    ProviderRequest {
        message: "question".to_owned(),
        tool_schemas: vec![json!({"name":"get_quality_summary"})],
        observations: Vec::new(),
    }
}
