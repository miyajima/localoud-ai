use anyhow::{Context, Result};
use protocol_types::{
    providers::{
        ContentBlock, InferenceRequest, InferenceTool, InferenceUsage, ModelTarget,
        TranscriptMessage, TranscriptRole,
    },
    AgentTool, ToolCall,
};
use provider_api::InferenceTransport;
use serde_json::{json, Value};
use std::{
    collections::{HashSet, VecDeque},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::process::Command;

const MAX_TOOL_CALLS: usize = 32;
const MAX_FILE_BYTES: u64 = 1_000_000;
const MAX_LISTED_FILES: usize = 2_000;
const MAX_SEARCH_MATCHES: usize = 200;
const MAX_COMMAND_OUTPUT: usize = 200_000;

#[derive(Debug, Clone)]
pub struct CommandApprovalRequest {
    pub thread_id: String,
    pub turn_id: String,
    pub digest: String,
    pub executable: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub additional_permissions: Vec<String>,
}

#[async_trait::async_trait]
pub trait CommandAuthorizer: Send + Sync {
    async fn authorize(&self, request: CommandApprovalRequest) -> Result<()>;
}

pub struct ApiAgent {
    pub root: PathBuf,
    pub logical_thread_id: String,
    pub logical_turn_id: String,
    pub target: ModelTarget,
    pub transport: Arc<dyn InferenceTransport>,
    pub external_tools: Vec<InferenceTool>,
    pub external_handler: Option<Arc<dyn AgentTool>>,
    pub command_authorizer: Option<Arc<dyn CommandAuthorizer>>,
    pub allow_writes: bool,
    pub allow_commands: bool,
}

#[derive(Debug)]
pub struct ApiAgentResult {
    pub text: String,
    pub transcript: Vec<TranscriptMessage>,
    pub usage: InferenceUsage,
    pub stop_reason: String,
    pub refused: bool,
    pub failure_kind: Option<provider_api::ProviderFailureKind>,
    pub failure: Option<String>,
    pub changed_paths: Vec<PathBuf>,
    pub tool_activity: Vec<Value>,
}

impl ApiAgent {
    pub async fn run(&self, system: String, prompt: String) -> Result<ApiAgentResult> {
        self.run_with_history(
            system,
            vec![TranscriptMessage {
                role: TranscriptRole::User,
                content: vec![ContentBlock::Text { text: prompt }],
                provider_state: None,
            }],
        )
        .await
    }

    pub async fn run_with_history(
        &self,
        system: String,
        mut transcript: Vec<TranscriptMessage>,
    ) -> Result<ApiAgentResult> {
        let root = self
            .root
            .canonicalize()
            .context("worker root does not exist")?;
        anyhow::ensure!(root.is_dir(), "worker root is not a directory");
        anyhow::ensure!(!transcript.is_empty(), "agent transcript is empty");
        let mut usage = InferenceUsage::default();
        let mut changed = HashSet::new();
        let mut tool_activity = Vec::new();
        let tools = workspace_tools(self.allow_writes, self.allow_commands)
            .into_iter()
            .chain(self.external_tools.clone())
            .collect::<Vec<_>>();

        for _ in 0..MAX_TOOL_CALLS {
            let response = self
                .transport
                .infer(InferenceRequest {
                    target: self.target.clone(),
                    system: Some(system.clone()),
                    messages: transcript.clone(),
                    tools: tools.clone(),
                    max_output_tokens: 16_384,
                })
                .await;
            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    let mut changed_paths = changed.into_iter().collect::<Vec<_>>();
                    changed_paths.sort();
                    return Ok(ApiAgentResult {
                        text: String::new(),
                        transcript,
                        usage,
                        stop_reason: "provider_error".into(),
                        refused: false,
                        failure_kind: Some(provider_api::failure_kind(&error)),
                        failure: Some(
                            hub_policy::redact(&format!("{error:#}"))
                                .chars()
                                .take(2_000)
                                .collect(),
                        ),
                        changed_paths,
                        tool_activity,
                    });
                }
            };
            usage.input_tokens = usage
                .input_tokens
                .saturating_add(response.usage.input_tokens);
            usage.cached_input_tokens = usage
                .cached_input_tokens
                .saturating_add(response.usage.cached_input_tokens);
            usage.output_tokens = usage
                .output_tokens
                .saturating_add(response.usage.output_tokens);
            let stop_reason = response.stop_reason.chars().take(200).collect::<String>();
            let refusal_stop = matches!(
                stop_reason.to_ascii_lowercase().as_str(),
                "refusal" | "refused" | "content_filter" | "safety"
            );
            let mut response_content = response.content;
            if refusal_stop
                && !response_content
                    .iter()
                    .any(|block| matches!(block, ContentBlock::Refusal { .. }))
            {
                response_content.push(ContentBlock::Refusal {
                    category: Some(stop_reason.clone()),
                    text: None,
                });
            }
            let assistant = TranscriptMessage {
                role: TranscriptRole::Assistant,
                content: response_content,
                provider_state: response.provider_state,
            };
            let calls = assistant
                .content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::ToolUse {
                        id,
                        name,
                        arguments,
                    } => Some((id.clone(), name.clone(), arguments.clone())),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let final_text = assistant
                .content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            let refused = assistant
                .content
                .iter()
                .any(|block| matches!(block, ContentBlock::Refusal { .. }));
            let refusal_text = assistant
                .content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Refusal { category, text } => Some(match (category, text) {
                        (Some(category), Some(text)) => format!("{category}: {text}"),
                        (Some(category), None) => category.clone(),
                        (None, Some(text)) => text.clone(),
                        (None, None) => "provider refused the request".into(),
                    }),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            transcript.push(assistant);
            if refused {
                let mut changed_paths = changed.into_iter().collect::<Vec<_>>();
                changed_paths.sort();
                return Ok(ApiAgentResult {
                    text: if refusal_text.trim().is_empty() {
                        "Provider refused the request.".into()
                    } else {
                        refusal_text
                    },
                    transcript,
                    usage,
                    stop_reason,
                    refused: true,
                    failure_kind: Some(provider_api::ProviderFailureKind::Refusal),
                    failure: Some("provider refused the request".into()),
                    changed_paths,
                    tool_activity,
                });
            }
            if calls.is_empty() {
                anyhow::ensure!(
                    !final_text.trim().is_empty(),
                    "provider completed without text or tool calls"
                );
                let mut changed_paths = changed.into_iter().collect::<Vec<_>>();
                changed_paths.sort();
                return Ok(ApiAgentResult {
                    text: final_text,
                    transcript,
                    usage,
                    stop_reason,
                    refused: false,
                    failure_kind: None,
                    failure: None,
                    changed_paths,
                    tool_activity,
                });
            }
            let mut results = Vec::new();
            for (id, name, arguments) in calls {
                let result = self
                    .call_tool(&root, &name, arguments.clone(), &mut changed)
                    .await;
                tool_activity.push(json!({
                    "name": name,
                    "arguments": arguments,
                    "success": result.is_ok(),
                    "output": match &result {
                        Ok(text) => text.chars().take(8_000).collect::<String>(),
                        Err(error) => format!("{error:#}"),
                    },
                }));
                results.push(ContentBlock::ToolResult {
                    tool_use_id: id,
                    text: match &result {
                        Ok(text) => text.clone(),
                        Err(error) => format!("{error:#}"),
                    },
                    is_error: result.is_err(),
                });
            }
            transcript.push(TranscriptMessage {
                role: TranscriptRole::Tool,
                content: results,
                provider_state: None,
            });
        }
        let mut changed_paths = changed.into_iter().collect::<Vec<_>>();
        changed_paths.sort();
        Ok(ApiAgentResult {
            text: String::new(),
            transcript,
            usage,
            stop_reason: "tool_round_limit".into(),
            refused: false,
            failure_kind: Some(provider_api::ProviderFailureKind::Unknown),
            failure: Some(format!(
                "provider exceeded the maximum of {MAX_TOOL_CALLS} tool rounds"
            )),
            changed_paths,
            tool_activity,
        })
    }

    async fn call_tool(
        &self,
        root: &Path,
        name: &str,
        arguments: Value,
        changed: &mut HashSet<PathBuf>,
    ) -> Result<String> {
        match name {
            "workspace_list" => list_files(root, &arguments),
            "workspace_read" => read_file(root, &arguments),
            "workspace_search" => search_files(root, &arguments),
            "workspace_replace" if self.allow_writes => replace_text(root, &arguments, changed),
            "workspace_command" if self.allow_commands => self.run_command(root, &arguments).await,
            _ => {
                let handler = self
                    .external_handler
                    .as_ref()
                    .context("provider requested an unknown tool")?;
                anyhow::ensure!(
                    self.external_tools.iter().any(|tool| tool.name == name),
                    "provider requested an unknown tool"
                );
                let result = handler
                    .call(ToolCall {
                        name: name.into(),
                        arguments,
                        thread_id: self.logical_thread_id.clone(),
                        turn_id: self.logical_turn_id.clone(),
                    })
                    .await?;
                anyhow::ensure!(result.success, "external tool failed: {}", result.text);
                Ok(result.text)
            }
        }
    }

    async fn run_command(&self, root: &Path, arguments: &Value) -> Result<String> {
        let argv = arguments["argv"]
            .as_array()
            .context("argv must be an array")?;
        anyhow::ensure!(
            !argv.is_empty() && argv.len() <= 64,
            "argv must contain 1-64 items"
        );
        let argv = argv
            .iter()
            .map(|value| value.as_str().context("argv items must be strings"))
            .collect::<Result<Vec<_>>>()?;
        anyhow::ensure!(
            argv.iter()
                .all(|value| !value.contains('\0') && value.len() <= 4_096),
            "command argument is invalid"
        );
        let relative_cwd = arguments["cwd"].as_str().unwrap_or(".");
        let cwd = checked_path(root, relative_cwd, true)?;
        anyhow::ensure!(cwd.is_dir(), "command cwd is not a directory");
        if let Err(policy_error) = validate_command(&argv) {
            anyhow::ensure!(
                approval_eligible(&argv),
                "command is never eligible for API-worker approval: {policy_error}"
            );
            let authorizer = self
                .command_authorizer
                .as_ref()
                .context("command requires explicit approval")?;
            let request_body = json!({
                "executable": argv[0],
                "argv": argv,
                "cwd": relative_cwd,
                "additional_permissions": ["command outside the built-in executable allowlist"]
            });
            let digest = hub_policy::scope_digest(
                "api-command-approval-v1",
                &serde_json::to_string(&request_body)?,
            );
            authorizer
                .authorize(CommandApprovalRequest {
                    thread_id: self.logical_thread_id.clone(),
                    turn_id: self.logical_turn_id.clone(),
                    digest,
                    executable: argv[0].into(),
                    argv: argv.iter().map(|value| (*value).to_owned()).collect(),
                    cwd: relative_cwd.into(),
                    additional_permissions: vec![
                        "command outside the built-in executable allowlist".into(),
                    ],
                })
                .await?;
        }
        execute_sandboxed_command(root, &cwd, &argv).await
    }
}

fn workspace_tools(allow_writes: bool, allow_commands: bool) -> Vec<InferenceTool> {
    let mut tools = vec![
        InferenceTool {
            name: "workspace_list".into(),
            description: "List bounded workspace-relative file paths. Symlinks and generated directories are omitted.".into(),
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"}},"additionalProperties":false}),
        },
        InferenceTool {
            name: "workspace_read".into(),
            description: "Read one UTF-8 text file inside the workspace.".into(),
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}),
        },
        InferenceTool {
            name: "workspace_search".into(),
            description: "Search a literal string in bounded UTF-8 workspace files.".into(),
            input_schema: json!({"type":"object","properties":{"query":{"type":"string"},"path":{"type":"string"}},"required":["query"],"additionalProperties":false}),
        },
    ];
    if allow_writes {
        tools.push(InferenceTool {
            name: "workspace_replace".into(),
            description: "Atomically replace one exact, unique text occurrence in a workspace file. Read first and keep edits minimal.".into(),
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"},"old":{"type":"string"},"new":{"type":"string"}},"required":["path","old","new"],"additionalProperties":false}),
        });
    }
    if allow_commands {
        tools.push(InferenceTool {
            name: "workspace_command".into(),
            description: "Run an allowlisted build, test, formatter, or read-only git command with argv (never a shell) in the network-disabled workspace sandbox.".into(),
            input_schema: json!({"type":"object","properties":{"argv":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":64},"cwd":{"type":"string"}},"required":["argv"],"additionalProperties":false}),
        });
    }
    tools
}

fn arg_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str> {
    arguments[key]
        .as_str()
        .filter(|value| !value.is_empty())
        .with_context(|| format!("{key} must be a non-empty string"))
}

fn checked_path(root: &Path, relative: &str, existing: bool) -> Result<PathBuf> {
    let path = Path::new(relative);
    anyhow::ensure!(
        !path.is_absolute() && !relative.contains('\0'),
        "workspace path must be relative"
    );
    anyhow::ensure!(
        path.components().all(|part| matches!(
            part,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )),
        "workspace path cannot escape the root"
    );
    let joined = root.join(path);
    if existing {
        let canonical = joined
            .canonicalize()
            .context("workspace path does not exist")?;
        anyhow::ensure!(
            canonical.starts_with(root),
            "workspace path escaped through a symlink"
        );
        Ok(canonical)
    } else {
        let parent = joined.parent().context("workspace path has no parent")?;
        let canonical_parent = parent
            .canonicalize()
            .context("workspace parent does not exist")?;
        anyhow::ensure!(
            canonical_parent.starts_with(root),
            "workspace path escaped through a symlink"
        );
        Ok(joined)
    }
}

fn skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | ".next" | "dist" | "build"
    )
}

fn walk_files(root: &Path, start: &Path) -> Result<Vec<PathBuf>> {
    let mut queue = VecDeque::from([start.to_path_buf()]);
    let mut files = Vec::new();
    while let Some(dir) = queue.pop_front() {
        let mut entries = std::fs::read_dir(&dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                if !skip_dir(&entry.file_name().to_string_lossy()) {
                    queue.push_back(path);
                }
            } else if kind.is_file() {
                files.push(path);
                if files.len() >= MAX_LISTED_FILES {
                    return Ok(files);
                }
            }
        }
    }
    files.retain(|path| path.starts_with(root));
    Ok(files)
}

fn list_files(root: &Path, arguments: &Value) -> Result<String> {
    let root = root
        .canonicalize()
        .context("workspace root does not exist")?;
    let relative = arguments["path"].as_str().unwrap_or(".");
    let start = checked_path(&root, relative, true)?;
    anyhow::ensure!(start.is_dir(), "list path is not a directory");
    Ok(walk_files(&root, &start)?
        .into_iter()
        .filter_map(|path| {
            path.strip_prefix(&root)
                .ok()
                .map(|path| path.to_string_lossy().into_owned())
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn read_file(root: &Path, arguments: &Value) -> Result<String> {
    let root = root
        .canonicalize()
        .context("workspace root does not exist")?;
    let path = checked_path(&root, arg_str(arguments, "path")?, true)?;
    anyhow::ensure!(path.is_file(), "read path is not a file");
    anyhow::ensure!(
        path.metadata()?.len() <= MAX_FILE_BYTES,
        "file exceeds read limit"
    );
    std::fs::read_to_string(path).context("file is not valid UTF-8 text")
}

fn search_files(root: &Path, arguments: &Value) -> Result<String> {
    let root = root
        .canonicalize()
        .context("workspace root does not exist")?;
    let query = arg_str(arguments, "query")?;
    anyhow::ensure!(query.len() <= 2_000, "search query is too long");
    let relative = arguments["path"].as_str().unwrap_or(".");
    let start = checked_path(&root, relative, true)?;
    let files = if start.is_file() {
        vec![start]
    } else {
        walk_files(&root, &start)?
    };
    let mut matches = Vec::new();
    for path in files {
        if path.metadata()?.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (line, value) in text.lines().enumerate() {
            if value.contains(query) {
                matches.push(format!(
                    "{}:{}:{}",
                    path.strip_prefix(&root)?.display(),
                    line + 1,
                    value.chars().take(500).collect::<String>()
                ));
                if matches.len() >= MAX_SEARCH_MATCHES {
                    return Ok(matches.join("\n"));
                }
            }
        }
    }
    Ok(matches.join("\n"))
}

fn replace_text(root: &Path, arguments: &Value, changed: &mut HashSet<PathBuf>) -> Result<String> {
    let root = root
        .canonicalize()
        .context("workspace root does not exist")?;
    let relative = arg_str(arguments, "path")?;
    let path = checked_path(&root, relative, true)?;
    anyhow::ensure!(path.is_file(), "replace path is not a file");
    let old = arg_str(arguments, "old")?;
    let new = arguments["new"].as_str().context("new must be a string")?;
    let current = std::fs::read_to_string(&path).context("file is not valid UTF-8 text")?;
    let count = current.matches(old).count();
    anyhow::ensure!(
        count == 1,
        "old text must occur exactly once; found {count}"
    );
    let next = current.replacen(old, new, 1);
    anyhow::ensure!(
        next.len() <= MAX_FILE_BYTES as usize,
        "edited file exceeds size limit"
    );
    let permissions = path.metadata()?.permissions();
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().context("workspace file has no parent")?)?;
    temp.as_file().set_permissions(permissions)?;
    temp.write_all(next.as_bytes())?;
    temp.as_file_mut().sync_all()?;
    temp.persist(&path)
        .map_err(|error| anyhow::anyhow!(error.error))?;
    changed.insert(path.strip_prefix(&root)?.to_path_buf());
    Ok(format!("updated {relative}"))
}

async fn execute_sandboxed_command(root: &Path, cwd: &Path, argv: &[&str]) -> Result<String> {
    let temporary = tempfile::tempdir().context("failed to create worker temporary directory")?;
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(600),
        sandboxed_command(root, cwd, temporary.path(), argv),
    )
    .await
    .context("worker command exceeded 10 minutes")??;
    let mut text = Vec::new();
    text.extend_from_slice(&output.stdout);
    text.extend_from_slice(&output.stderr);
    if text.len() > MAX_COMMAND_OUTPUT {
        text.truncate(MAX_COMMAND_OUTPUT);
        text.extend_from_slice(b"\n[output truncated]");
    }
    let rendered = String::from_utf8_lossy(&text);
    anyhow::ensure!(
        output.status.success(),
        "command exited with {}:\n{}",
        output.status,
        hub_policy::redact(&rendered)
    );
    Ok(hub_policy::redact(&rendered))
}

fn validate_command(argv: &[&str]) -> Result<()> {
    let executable_path = Path::new(argv[0]);
    anyhow::ensure!(
        executable_path.components().count() == 1,
        "built-in commands must use a bare executable name"
    );
    let executable = executable_path
        .file_name()
        .and_then(|value| value.to_str())
        .context("command executable is invalid")?;
    anyhow::ensure!(
        matches!(
            executable,
            "cargo"
                | "npm"
                | "pnpm"
                | "bun"
                | "deno"
                | "node"
                | "python3"
                | "ruby"
                | "bundle"
                | "go"
                | "git"
                | "swift"
                | "xcodebuild"
                | "make"
                | "cmake"
        ),
        "command requires explicit approval: {executable}"
    );
    if executable == "git" {
        let subcommand = argv.get(1).copied().unwrap_or("");
        anyhow::ensure!(
            matches!(subcommand, "diff" | "status" | "show" | "log" | "rev-parse"),
            "mutating git commands are not available to API workers"
        );
    }
    if matches!(executable, "npm" | "pnpm" | "bun") {
        anyhow::ensure!(
            argv.get(1)
                .is_some_and(|value| matches!(*value, "test" | "run" | "exec")),
            "package installation requires explicit approval"
        );
    }
    Ok(())
}

fn approval_eligible(argv: &[&str]) -> bool {
    let executable = Path::new(argv[0])
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if matches!(
        executable,
        "sh" | "bash" | "zsh" | "fish" | "dash" | "osascript" | "sudo" | "env"
    ) {
        return false;
    }
    if executable == "git" {
        return matches!(
            argv.get(1).copied().unwrap_or(""),
            "diff" | "status" | "show" | "log" | "rev-parse"
        );
    }
    if matches!(executable, "npm" | "pnpm" | "bun") {
        return argv
            .get(1)
            .is_some_and(|value| matches!(*value, "test" | "run" | "exec"));
    }
    !executable.is_empty()
}

async fn sandboxed_command(
    root: &Path,
    cwd: &Path,
    temporary: &Path,
    argv: &[&str],
) -> Result<std::process::Output> {
    #[cfg(target_os = "macos")]
    {
        let root_rule = escape_sandbox_literal(&root.to_string_lossy());
        let temp = escape_sandbox_literal(&temporary.to_string_lossy());
        let profile = format!(
            "(version 1)\n(deny default)\n(import \"system.sb\")\n(allow process*)\n(allow file-read* (subpath \"{root_rule}\") (subpath \"{temp}\") (subpath \"/System\") (subpath \"/usr\") (subpath \"/bin\") (subpath \"/sbin\") (subpath \"/Library\") (subpath \"/opt/homebrew\") (subpath \"{}\") (subpath \"{}\"))\n(allow file-write* (subpath \"{root_rule}\") (subpath \"{temp}\"))\n(deny network*)",
            escape_sandbox_literal(&format!("{}/.cargo", std::env::var("HOME").unwrap_or_default())),
            escape_sandbox_literal(&format!("{}/.rustup", std::env::var("HOME").unwrap_or_default())),
        );
        let mut command = Command::new("/usr/bin/sandbox-exec");
        command.arg("-p").arg(profile).arg(argv[0]).args(&argv[1..]);
        prepare_command(&mut command, cwd, temporary);
        command
            .output()
            .await
            .context("failed to run sandboxed command")
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (root, cwd, temporary, argv);
        anyhow::bail!("API worker command sandbox is implemented only for macOS")
    }
}

fn prepare_command(command: &mut Command, cwd: &Path, temporary: &Path) {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut path = vec![
        "/opt/homebrew/bin".to_owned(),
        "/usr/local/bin".to_owned(),
        "/usr/bin".to_owned(),
        "/bin".to_owned(),
        "/usr/sbin".to_owned(),
        "/sbin".to_owned(),
    ];
    if !home.is_empty() {
        path.push(format!("{home}/.cargo/bin"));
    }
    command
        .current_dir(cwd)
        .env_clear()
        .env("PATH", path.join(":"))
        .env("HOME", home)
        .env("TMPDIR", temporary)
        .env("LANG", "en_US.UTF-8")
        .kill_on_drop(true);
}

#[cfg(target_os = "macos")]
fn escape_sandbox_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol_types::providers::{
        InferenceResponse, ProviderLocality, ProviderModel, ProviderProfile, ProviderProtocol,
    };

    struct RefusingTransport {
        profile: ProviderProfile,
    }

    struct FailingTransport {
        profile: ProviderProfile,
    }

    #[async_trait::async_trait]
    impl InferenceTransport for RefusingTransport {
        fn profile(&self) -> &ProviderProfile {
            &self.profile
        }

        async fn models(&self) -> Result<Vec<ProviderModel>> {
            Ok(Vec::new())
        }

        async fn infer(&self, _request: InferenceRequest) -> Result<InferenceResponse> {
            Ok(InferenceResponse {
                content: vec![ContentBlock::Refusal {
                    category: Some("policy".into()),
                    text: Some("cannot comply".into()),
                }],
                usage: InferenceUsage {
                    input_tokens: 7,
                    cached_input_tokens: 2,
                    output_tokens: 3,
                },
                stop_reason: "refusal".into(),
                provider_state: None,
            })
        }
    }

    #[async_trait::async_trait]
    impl InferenceTransport for FailingTransport {
        fn profile(&self) -> &ProviderProfile {
            &self.profile
        }

        async fn models(&self) -> Result<Vec<ProviderModel>> {
            Ok(Vec::new())
        }

        async fn infer(&self, _request: InferenceRequest) -> Result<InferenceResponse> {
            anyhow::bail!("fixture transport failure")
        }
    }

    fn fixture_profile() -> ProviderProfile {
        ProviderProfile {
            id: "fixture".into(),
            name: "Fixture".into(),
            protocol: ProviderProtocol::OpenAiChat,
            base_url: Some("http://127.0.0.1:9/v1/".into()),
            locality: ProviderLocality::Local,
            credential_env: None,
            max_concurrency: 1,
            enabled: true,
            revision: 1,
        }
    }

    #[test]
    fn workspace_paths_and_exact_replacements_fail_closed() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        std::fs::write(dir.path().join("a.txt"), "one two one")?;
        std::fs::write(outside.path().join("secret.txt"), "outside")?;
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), dir.path().join("escape"))?;
            std::os::unix::fs::symlink(
                outside.path().join("secret.txt"),
                dir.path().join("a.txt.localoud.tmp"),
            )?;
        }
        assert!(read_file(dir.path(), &json!({"path":"../a.txt"})).is_err());
        assert!(read_file(dir.path(), &json!({"path":"/etc/passwd"})).is_err());
        #[cfg(unix)]
        assert!(read_file(dir.path(), &json!({"path":"escape/secret.txt"})).is_err());
        let mut changed = HashSet::new();
        assert!(replace_text(
            dir.path(),
            &json!({"path":"a.txt","old":"one","new":"three"}),
            &mut changed
        )
        .is_err());
        replace_text(
            dir.path(),
            &json!({"path":"a.txt","old":"two","new":"three"}),
            &mut changed,
        )?;
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt"))?,
            "one three one"
        );
        assert_eq!(
            std::fs::read_to_string(outside.path().join("secret.txt"))?,
            "outside"
        );
        Ok(())
    }

    #[test]
    fn command_policy_rejects_shells_installers_and_mutating_git() {
        assert!(validate_command(&["sh", "-c", "echo unsafe"]).is_err());
        assert!(validate_command(&["./cargo", "test"]).is_err());
        assert!(validate_command(&["/tmp/cargo", "test"]).is_err());
        assert!(validate_command(&["npm", "install"]).is_err());
        assert!(validate_command(&["git", "reset", "--hard"]).is_err());
        assert!(validate_command(&["cargo", "test"]).is_ok());
        assert!(validate_command(&["git", "diff", "--check"]).is_ok());
        assert!(!approval_eligible(&["sh", "-c", "echo unsafe"]));
        assert!(!approval_eligible(&["git", "reset", "--hard"]));
        assert!(approval_eligible(&["project-check", "--strict"]));
    }

    #[tokio::test]
    async fn command_environment_drops_provider_credentials_and_uses_dedicated_temp() -> Result<()>
    {
        let root = tempfile::tempdir()?;
        let temporary = tempfile::tempdir()?;
        let mut command = Command::new("/usr/bin/env");
        command.env("ANTHROPIC_API_KEY", "must-not-leak");
        command.env("PATH", root.path());
        prepare_command(&mut command, root.path(), temporary.path());
        let output = command.output().await?;
        let environment = String::from_utf8(output.stdout)?;
        assert!(!environment.contains("ANTHROPIC_API_KEY"));
        assert!(!environment.contains(&format!("PATH={}", root.path().display())));
        assert!(environment.contains(&format!("TMPDIR={}", temporary.path().display())));
        Ok(())
    }

    #[tokio::test]
    async fn refusal_returns_a_persistable_failed_outcome() -> Result<()> {
        let root = tempfile::tempdir()?;
        let profile = fixture_profile();
        let result = ApiAgent {
            root: root.path().into(),
            logical_thread_id: "thread".into(),
            logical_turn_id: "turn".into(),
            target: ModelTarget {
                profile_id: profile.id.clone(),
                model_id: "fixture-model".into(),
                effort: None,
            },
            transport: Arc::new(RefusingTransport { profile }),
            external_tools: Vec::new(),
            external_handler: None,
            command_authorizer: None,
            allow_writes: false,
            allow_commands: false,
        }
        .run("system".into(), "request".into())
        .await?;

        assert!(result.refused);
        assert_eq!(
            result.failure_kind,
            Some(provider_api::ProviderFailureKind::Refusal)
        );
        assert_eq!(result.stop_reason, "refusal");
        assert_eq!(result.text, "policy: cannot comply");
        assert_eq!(result.usage.input_tokens, 7);
        assert!(matches!(
            result.transcript.last().unwrap().content.as_slice(),
            [ContentBlock::Refusal { .. }]
        ));
        Ok(())
    }

    #[tokio::test]
    async fn transport_failure_keeps_the_unsent_history_for_durable_diagnosis() -> Result<()> {
        let root = tempfile::tempdir()?;
        let profile = fixture_profile();
        let result = ApiAgent {
            root: root.path().into(),
            logical_thread_id: "thread".into(),
            logical_turn_id: "turn".into(),
            target: ModelTarget {
                profile_id: profile.id.clone(),
                model_id: "fixture-model".into(),
                effort: None,
            },
            transport: Arc::new(FailingTransport { profile }),
            external_tools: Vec::new(),
            external_handler: None,
            command_authorizer: None,
            allow_writes: false,
            allow_commands: false,
        }
        .run("system".into(), "request".into())
        .await?;

        assert!(!result.refused);
        assert_eq!(
            result.failure_kind,
            Some(provider_api::ProviderFailureKind::Unknown)
        );
        assert_eq!(result.transcript.len(), 1);
        assert!(result
            .failure
            .unwrap()
            .contains("fixture transport failure"));
        Ok(())
    }
}
