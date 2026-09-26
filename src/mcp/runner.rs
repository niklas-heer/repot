//! Bounded capture of the installed CLI; subprocess output never reaches MCP stdout.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use rmcp::RoleServer;
use rmcp::model::CallToolResult;
use rmcp::service::RequestContext;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt};

const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;

pub fn error(message: &str) -> CallToolResult {
    CallToolResult::structured_error(json!({
        "exit_code": null, "review_required": false, "data": null,
        "output": "", "error": message,
    }))
}

async fn capture(reader: impl AsyncRead + Unpin) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    reader
        .take(OUTPUT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .await?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > OUTPUT_LIMIT {
        return Err(std::io::Error::other("command output limit exceeded"));
    }
    String::from_utf8(bytes).map_err(|_| std::io::Error::other("command output is not UTF-8"))
}

pub async fn execute(
    executable: &Path,
    directory: &Path,
    manifest: Option<&Path>,
    args: Vec<String>,
    context: &RequestContext<RoleServer>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> CallToolResult {
    let mut command = tokio::process::Command::new(executable);
    if let Some(manifest) = manifest {
        command.arg("--manifest").arg(manifest);
    }
    command
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env_remove("REPOT_CD_FILE")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("NO_COLOR", "1");
    let Ok(mut child) = command.spawn() else {
        return error("cannot start repot command in the requested directory");
    };
    let Some(stdout) = child.stdout.take() else {
        return error("cannot capture repot output");
    };
    let Some(stderr) = child.stderr.take() else {
        return error("cannot capture repot diagnostics");
    };
    let pid = child.id();
    let result = {
        let operation = async { tokio::try_join!(capture(stdout), capture(stderr), child.wait()) };
        tokio::pin!(operation);
        let result = tokio::select! {
            biased;
            () = context.ct.cancelled() => None,
            _ = shutdown.changed() => None,
            () = tokio::time::sleep(Duration::from_hours(1)) => None,
            output = &mut operation => Some(output),
        };
        if result.is_none() {
            // SIGTERM asks the CLI to stop Git/forge process groups and unwind
            // staged operations. Keep reading until cleanup finishes.
            terminate(pid);
            let _ = tokio::time::timeout(Duration::from_secs(5), &mut operation).await;
        }
        result
    };
    let (stdout, stderr, status) = match result {
        Some(Ok(output)) => output,
        Some(Err(_)) => {
            terminate(pid);
            let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
            return error("cannot capture command output; inspect current state before retrying");
        }
        None => {
            return error("command cancelled or timed out; inspect current state before retrying");
        }
    };
    let data = serde_json::from_str::<Value>(&stdout).ok();
    let result = json!({
        "exit_code": status.code(),
        "review_required": status.code() == Some(3),
        "output": if data.is_some() { "" } else { stdout.trim_end() },
        "data": data,
        "diagnostics": stderr.trim_end(),
    });
    if status.success() || status.code() == Some(3) {
        CallToolResult::structured(result)
    } else {
        CallToolResult::structured_error(result)
    }
}

fn terminate(pid: Option<u32>) {
    if let Some(pid) = pid.and_then(|pid| i32::try_from(pid).ok()) {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGTERM,
        );
    }
}
