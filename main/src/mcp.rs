//! Model Context Protocol integration.
//!
//! Cntx Code ships two built-in MCP servers: Context7, surfaced as built-in
//! documentation search, and Headroom, surfaced as built-in token saving.
//! Users can also register custom MCP servers from YAML or the `cntx mcp`
//! commands. Servers run as local stdio subprocesses and are spawned on
//! demand, so they never slow down ordinary prompts.

use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use owo_colors::OwoColorize;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::config::{AppConfig, McpServerConfig};

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const MCP_REQUEST_TIMEOUT_SECS: u64 = 30;

#[derive(Clone, Debug, Serialize)]
pub struct McpTool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: Value,
}

/// A live stdio JSON-RPC connection to a single MCP server.
pub struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    server_name: String,
}

impl McpClient {
    /// Spawn an MCP server subprocess and open a JSON-RPC channel over stdio.
    pub fn spawn(server: &McpServerConfig) -> Result<Self> {
        let mut command = Command::new(&server.command);
        command.args(&server.args);
        for (key, value) in &server.env {
            command.env(key, value);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = command.spawn().with_context(|| {
            format!(
                "failed to spawn MCP server `{}` (command `{}`). Install the server runtime \
                     (Node.js for Context7 via npx, or `pip install headroom-ai[mcp]` for \
                     Headroom) and retry.",
                server.name, server.command
            )
        })?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            server_name: server.name.clone(),
        })
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&request)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;

        loop {
            let mut buffer = String::new();
            let read = tokio::time::timeout(
                Duration::from_secs(MCP_REQUEST_TIMEOUT_SECS),
                self.stdout.read_line(&mut buffer),
            )
            .await
            .map_err(|_| anyhow!("MCP server `{}` timed out", self.server_name))??;
            if read == 0 {
                return Err(anyhow!(
                    "MCP server `{}` closed its output stream",
                    self.server_name
                ));
            }
            let trimmed = buffer.trim();
            if trimmed.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(trimmed).with_context(|| {
                format!("MCP server `{}` sent non-JSON output", self.server_name)
            })?;
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = value.get("error") {
                    return Err(anyhow!(
                        "MCP server `{}` error: {}",
                        self.server_name,
                        error
                    ));
                }
                return Ok(value.get("result").cloned().unwrap_or(Value::Null));
            }
            // Notifications and unrelated responses are ignored.
        }
    }

    pub async fn initialize(&mut self) -> Result<Value> {
        self.request(
            "initialize",
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "cntx-code",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )
        .await
    }

    pub async fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        let result = self.request("tools/list", json!({})).await?;
        let tools = result
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("MCP server `{}` returned no tools array", self.server_name))?;
        let mut parsed = Vec::with_capacity(tools.len());
        for tool in tools {
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("MCP tool missing name"))?
                .to_string();
            let description = tool
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let input_schema = tool
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({}));
            parsed.push(McpTool {
                name,
                description,
                input_schema,
            });
        }
        Ok(parsed)
    }

    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value> {
        self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )
        .await
    }

    pub async fn shutdown(mut self) {
        let _ = self.request("shutdown", json!({})).await;
        let _ = self.child.kill().await;
    }
}

/// Iterate configured servers, skipping disabled ones.
pub fn enabled_servers(config: &AppConfig) -> Vec<&McpServerConfig> {
    config
        .mcp
        .servers
        .values()
        .filter(|server| server.enabled)
        .collect()
}

pub fn find_server<'a>(config: &'a AppConfig, name: &str) -> Option<&'a McpServerConfig> {
    config.mcp.servers.get(name)
}

/// Print a static listing of all configured MCP servers (no subprocess spawn).
pub fn print_servers(config: &AppConfig) {
    for server in config.mcp.servers.values() {
        let marker = if server.enabled { "*" } else { " " };
        let built_in = if server.built_in { " (built-in)" } else { "" };
        println!(
            "{} {} {}{}",
            marker,
            server.name.bold(),
            server.command,
            built_in
        );
        if let Some(description) = server.description.as_ref() {
            println!("      {description}");
        }
        if !server.args.is_empty() {
            println!("      args: {}", server.args.join(" "));
        }
    }
    println!("\n* = enabled. Spawned on demand; never on every prompt.");
}

/// Connect to a server, list its tools, print them, and shut down.
pub async fn print_tools(config: &AppConfig, name: &str) -> Result<()> {
    let server = find_server(config, name)
        .ok_or_else(|| anyhow!("MCP server `{name}` is not configured"))?
        .clone();
    if !server.enabled {
        return Err(anyhow!(
            "MCP server `{name}` is disabled; enable it with `cntx mcp enable {name}`"
        ));
    }

    let mut client = McpClient::spawn(&server)?;
    let _initialized = client.initialize().await?;
    let tools = client.list_tools().await?;
    client.shutdown().await;

    println!("{}", name.bold());
    for tool in tools {
        println!(
            "  {} - {}",
            tool.name.bold(),
            tool.description.unwrap_or_default()
        );
    }
    Ok(())
}

/// Default env hints for the built-in servers, used to document setup.
pub fn builtin_env_hints() -> BTreeMap<String, String> {
    let mut hints = BTreeMap::new();
    hints.insert(
        "context7".to_string(),
        "Optional CONTEXT7_API_KEY env var raises rate limits; otherwise anonymous use is allowed."
            .to_string(),
    );
    hints.insert(
        "headroom".to_string(),
        "Install with `pip install headroom-ai[mcp]`; no key required for local compression."
            .to_string(),
    );
    hints
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn builtins_are_present_after_default() {
        let config = AppConfig::default();
        assert!(config.mcp.servers.contains_key("context7"));
        assert!(config.mcp.servers.contains_key("headroom"));
        assert!(config.mcp.servers["context7"].built_in);
        assert!(config.mcp.servers["headroom"].built_in);
    }

    #[test]
    fn enabled_servers_filters_disabled() {
        let mut config = AppConfig::default();
        config.mcp.servers.get_mut("headroom").unwrap().enabled = false;
        let enabled = enabled_servers(&config);
        let names: Vec<&str> = enabled.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"context7"));
        assert!(!names.contains(&"headroom"));
    }
}
