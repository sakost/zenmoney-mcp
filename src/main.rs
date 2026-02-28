//! ZenMoney MCP server entry point.
//!
//! Reads `ZENMONEY_TOKEN` from the environment, creates a [`ZenMoney`]
//! client backed by [`FileStorage`], performs an initial sync, then
//! serves MCP tools over stdio.

mod params;
mod response;
mod server;

use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;
use zenmoney_rs::storage::FileStorage;
use zenmoney_rs::zen_money::ZenMoney;

use crate::server::ZenMoneyMcpServer;

/// Runs the MCP server.
///
/// # Errors
///
/// Returns an error if the token is missing, the client cannot be built,
/// the initial sync fails, or the stdio transport encounters an error.
async fn run() -> Result<(), Box<dyn core::error::Error>> {
    // Initialise tracing to stderr (stdout is used for MCP stdio transport).
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("starting ZenMoney MCP server");

    // Read token from environment.
    let token: String = std::env::var("ZENMONEY_TOKEN")
        .map_err(|_err| "ZENMONEY_TOKEN environment variable is required")?;

    // Create file storage at default XDG location.
    let storage_dir = FileStorage::default_dir()?;
    let storage = FileStorage::new(storage_dir)?;

    // Build the ZenMoney client.
    let client = ZenMoney::builder().token(token).storage(storage).build()?;

    // Perform initial sync.
    tracing::info!("performing initial sync");
    let _sync_response = client.sync().await?;
    tracing::info!("initial sync complete");

    // Create MCP server and serve over stdio.
    let mcp_server = ZenMoneyMcpServer::new(client);
    let transport = (tokio::io::stdin(), tokio::io::stdout());
    let service = mcp_server.serve(transport).await?;

    tracing::info!("MCP server running on stdio");
    let _quit_reason = service.waiting().await?;

    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        tracing::error!(%err, "fatal error");
        std::process::exit(1);
    }
}
