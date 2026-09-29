use crate::db::DbPool;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{error, info};

use crate::domain::accounts::AccountId;

use super::protocol::{error_codes, JsonRpcRequest, JsonRpcResponse};
use super::service::McpService;

/// Runs the MCP server in STDIO mode for local CLI subprocesses (Claude Desktop, Cursor, etc.).
pub async fn run_stdio(
    services: &crate::application::AppServices,
    pool: &DbPool,
    owner_id: &AccountId,
) -> anyhow::Result<()> {
    eprintln!(
        "[MCP] Starting matrix-bot MCP server in STDIO mode for owner '{}'...",
        owner_id
    );

    let service = McpService::new(services, pool, None, owner_id);

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = reader.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(parsed) => parsed,
            Err(err) => {
                let err_resp = JsonRpcResponse::error(
                    serde_json::Value::Null,
                    error_codes::PARSE_ERROR,
                    format!("Parse error: {}", err),
                );
                let json_str = serde_json::to_string(&err_resp)?;
                stdout.write_all(json_str.as_bytes()).await?;
                stdout.write_all(b"\n").await?;
                stdout.flush().await?;
                continue;
            }
        };

        if let Some(resp) = service.handle_request(req).await {
            let json_str = match serde_json::to_string(&resp) {
                Ok(s) => s,
                Err(err) => {
                    error!("Failed to serialize MCP response: {}", err);
                    continue;
                }
            };

            stdout.write_all(json_str.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }

    info!("MCP STDIO stream closed");
    Ok(())
}
