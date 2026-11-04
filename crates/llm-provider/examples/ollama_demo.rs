use llm_provider::{
    AgentProvider, OllamaProvider, OllamaProviderConfig, ProviderRequest, ProviderResponse,
};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = OllamaProvider::new(OllamaProviderConfig::from_env()?)?;
    let response = provider
        .next_turn(ProviderRequest {
            message: "Use get_work_order for work order 11111111-1111-1111-1111-111111111111."
                .to_owned(),
            tool_schemas: vec![json!({
                "name":"get_work_order",
                "parameters":{
                    "type":"object",
                    "additionalProperties":false,
                    "required":["work_order_id"],
                    "properties":{"work_order_id":{"type":"string","format":"uuid"}}
                }
            })],
            observations: Vec::new(),
        })
        .await?;
    match response {
        ProviderResponse::ToolCalls(calls) => {
            let call = calls
                .first()
                .ok_or("Ollama returned an empty tool-call list")?;
            if call.tool_name != "get_work_order"
                || call.arguments["work_order_id"] != "11111111-1111-1111-1111-111111111111"
            {
                return Err("Ollama tool mapping did not match the requested operation".into());
            }
            println!(
                "{}",
                serde_json::json!({
                    "status":"pass",
                    "tool_name":call.tool_name,
                    "arguments":call.arguments
                })
            );
        }
        ProviderResponse::Final { answer, .. } => {
            return Err(format!("expected tool call, received final answer: {answer}").into());
        }
    }
    Ok(())
}
