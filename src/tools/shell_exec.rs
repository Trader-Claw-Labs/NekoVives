use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use crate::security::SecurityPolicy;

const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Run a shell command and return stdout + stderr.
pub struct ShellExecTool {
    security: Arc<SecurityPolicy>,
    /// Working directory for command execution (defaults to workspace dir)
    workspace_dir: PathBuf,
}

impl ShellExecTool {
    pub fn new(security: Arc<SecurityPolicy>, workspace_dir: PathBuf) -> Self {
        Self { security, workspace_dir }
    }
}

#[async_trait]
impl Tool for ShellExecTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return the output (stdout + stderr). \
        Use for running scripts, checking system state, or any terminal operation. \
        Commands run in the workspace directory by default."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory for the command (optional, defaults to workspace)"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 30, max: 120)",
                    "default": 30
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'command' parameter"))?;

        // Security check — give a clear message when ReadOnly blocks all shell access
        if self.security.autonomy == crate::security::AutonomyLevel::ReadOnly {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(
                    "Shell execution requires autonomy level AutoEdit or higher. \
                    Current: ReadOnly. Change in Settings → Config → [autonomy] level = \"autoedit\"."
                        .into(),
                ),
            });
        }
        if !self.security.is_command_allowed(command) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Command '{command}' is not in the allowed_commands list. \
                    Add it to [autonomy] allowed_commands in config.toml, or use a different command."
                )),
            });
        }

        // Resolve working directory
        let cwd = args
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.workspace_dir.clone());

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .min(120);

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir(&cwd)
                .output(),
        )
        .await;

        match output {
            Err(_) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Command timed out after {timeout_secs}s")),
            }),
            Ok(Err(e)) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to spawn process: {e}")),
            }),
            Ok(Ok(out)) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                let success = out.status.success();

                let mut combined = stdout.trim_end().to_string();
                if !stderr.trim().is_empty() {
                    if !combined.is_empty() {
                        combined.push('\n');
                    }
                    combined.push_str("[stderr]\n");
                    combined.push_str(stderr.trim_end());
                }

                if combined.is_empty() {
                    combined = format!(
                        "(exit {})",
                        out.status.code().unwrap_or(-1)
                    );
                }

                Ok(ToolResult {
                    success,
                    output: combined,
                    error: if success {
                        None
                    } else {
                        Some(format!(
                            "Command exited with status {}",
                            out.status.code().unwrap_or(-1)
                        ))
                    },
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_tool() -> (TempDir, ShellExecTool) {
        let tmp = TempDir::new().unwrap();
        let mut policy = SecurityPolicy::default();
        policy.autonomy = crate::security::AutonomyLevel::AutoEdit;
        let tool = ShellExecTool::new(Arc::new(policy), tmp.path().to_path_buf());
        (tmp, tool)
    }

    #[tokio::test]
    async fn echo_works() {
        let (_tmp, tool) = make_tool();
        let result = tool.execute(json!({ "command": "echo hello" })).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("hello"));
    }

    #[tokio::test]
    async fn failing_command() {
        let (_tmp, tool) = make_tool();
        let result = tool.execute(json!({ "command": "exit 1" })).await.unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn missing_command_param() {
        let (_tmp, tool) = make_tool();
        assert!(tool.execute(json!({})).await.is_err());
    }
}
