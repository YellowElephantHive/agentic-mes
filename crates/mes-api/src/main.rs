use std::{env, net::SocketAddr, time::Duration};

use agent_core::AgentRuntimeLimits;
use llm_provider::{
    HttpProvider, HttpProviderConfig, OllamaProvider, OllamaProviderConfig, ResilientProvider,
};
use mes_api::{AppState, LlmDataPolicy, router};
use mes_infra::PostgresMesRepository;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8080";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("mes_api=info")),
        )
        .init();

    let bind_addr = env::var("APP_BIND_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned());
    let app_env = env::var("APP_ENV").unwrap_or_else(|_| "development".to_owned());
    if app_env != "development" {
        return Err("production authentication is not implemented; refusing unsafe startup".into());
    }
    let database_url = env::var("DATABASE_URL")?;
    let repository = PostgresMesRepository::connect(&database_url).await?;
    repository.migrate().await?;
    let socket_addr: SocketAddr = bind_addr.parse()?;
    let listener = TcpListener::bind(socket_addr).await?;
    let rate_limit_requests = env::var("APP_RATE_LIMIT_REQUESTS_PER_MINUTE")
        .unwrap_or_else(|_| "120".to_owned())
        .parse::<usize>()?;
    if rate_limit_requests == 0 {
        return Err("APP_RATE_LIMIT_REQUESTS_PER_MINUTE must be positive".into());
    }
    let provider_name = env::var("AGENT_PROVIDER").unwrap_or_else(|_| "mock".to_owned());
    let data_policy_name = env::var("LLM_DATA_POLICY").unwrap_or_else(|_| "local_only".to_owned());
    let data_policy = match data_policy_name.as_str() {
        "local_only" => LlmDataPolicy::LocalOnly,
        "external_approved" => LlmDataPolicy::ExternalApproved,
        _ => return Err("LLM_DATA_POLICY must be local_only or external_approved".into()),
    };
    let state = AppState::development(std::sync::Arc::new(repository))
        .with_rate_limit(rate_limit_requests, Duration::from_secs(60))
        .with_runtime_limits(AgentRuntimeLimits {
            provider_timeout: Duration::from_millis(positive_env_u64(
                "AGENT_PROVIDER_TIMEOUT_MS",
                10_000,
            )?),
            overall_timeout: Duration::from_millis(positive_env_u64(
                "AGENT_OVERALL_TIMEOUT_MS",
                45_000,
            )?),
            tool_timeout: Duration::from_millis(positive_env_u64("AGENT_TOOL_TIMEOUT_MS", 5_000)?),
            ..AgentRuntimeLimits::default()
        });
    let provider_concurrency =
        usize::try_from(positive_env_u64("AGENT_PROVIDER_MAX_CONCURRENCY", 4)?)?;
    let provider_failure_threshold =
        u32::try_from(positive_env_u64("AGENT_PROVIDER_FAILURE_THRESHOLD", 3)?)?;
    let provider_cooldown = Duration::from_millis(positive_env_u64(
        "AGENT_PROVIDER_CIRCUIT_COOLDOWN_MS",
        30_000,
    )?);
    let state = match provider_name.as_str() {
        "mock" => state,
        "ollama" => state.with_ollama_provider(std::sync::Arc::new(ResilientProvider::new(
            std::sync::Arc::new(OllamaProvider::new(OllamaProviderConfig::from_env()?)?),
            provider_concurrency,
            provider_failure_threshold,
            provider_cooldown,
        )?)),
        "http" => state.with_http_provider(
            std::sync::Arc::new(ResilientProvider::new(
                std::sync::Arc::new(HttpProvider::new(HttpProviderConfig::from_env()?)?),
                provider_concurrency,
                provider_failure_threshold,
                provider_cooldown,
            )?),
            data_policy,
        )?,
        _ => return Err("AGENT_PROVIDER must be mock, ollama, or http".into()),
    };

    info!(address = %socket_addr, provider = %provider_name, policy = %data_policy_name, "MES API listening");
    axum::serve(listener, router(state)).await?;
    Ok(())
}

fn positive_env_u64(name: &str, default: u64) -> Result<u64, Box<dyn std::error::Error>> {
    let value = env::var(name)
        .unwrap_or_else(|_| default.to_string())
        .parse::<u64>()?;
    if value == 0 {
        return Err(format!("{name} must be positive").into());
    }
    Ok(value)
}
