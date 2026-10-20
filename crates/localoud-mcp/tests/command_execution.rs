#![cfg(unix)]

use localoud_mcp::{handle_message, Arguments, McpFileConfig, Server};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

struct Fixture {
    dir: tempfile::TempDir,
    server: Server,
}

impl Fixture {
    fn new(execution: Option<bool>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let mut project =
            json!({"id":"p", "name":"fixture", "root":dir.path(), "write_enabled":true});
        if let Some(enabled) = execution {
            project["execution_enabled"] = json!(enabled);
        }
        let config: McpFileConfig = serde_json::from_value(json!({"projects":[project]})).unwrap();
        Self {
            dir,
            server: Server::new(config).unwrap(),
        }
    }

    async fn call(&self, extra: Value) -> Result<Value, String> {
        let mut args = json!({"project_id":"p"});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        self.server
            .call(
                "command_run",
                serde_json::from_value::<Arguments>(args).unwrap(),
            )
            .await
    }
}

#[tokio::test]
async fn legacy_and_disabled_configs_deny_execution_without_side_effects() {
    for enabled in [None, Some(false)] {
        let f = Fixture::new(enabled);
        let result = f
            .call(json!({"program":"/usr/bin/touch", "args":["should-not-exist"]}))
            .await;
        assert!(result.unwrap_err().contains("not enabled"));
        assert!(!f.dir.path().join("should-not-exist").exists());
    }
}

#[tokio::test]
async fn command_uses_project_cwd_literal_argv_and_reports_real_status() {
    let f = Fixture::new(Some(true));
    let result = f.call(json!({"program":"/bin/sh", "args":["-c", "pwd; printf '%s' \"$1\"; printf error >&2; exit 7", "fixture", "$(touch injected); *"], "cwd":"sub"})).await.unwrap();
    let cwd = f.dir.path().canonicalize().unwrap().join("sub");
    assert_eq!(result["exit_code"], 7);
    assert_eq!(result["success"], false);
    assert_eq!(result["timed_out"], false);
    assert_eq!(
        result["stdout"],
        format!("{}\n$(touch injected); *", cwd.display())
    );
    assert_eq!(result["stderr"], "error");
    assert!(!f.dir.path().join("sub/injected").exists());
    let list = f
        .server
        .call("project_list", Arguments::default())
        .await
        .unwrap();
    assert_eq!(list["projects"][0]["command_execution_enabled"], true);
}

#[tokio::test]
async fn execution_opt_in_is_independent_of_editing_and_project_scoped() {
    let f = Fixture::new(None);
    let config = serde_json::from_value(json!({"projects":[
        {"id":"enabled", "name":"exec", "root":f.dir.path(), "execution_enabled":true},
        {"id":"disabled", "name":"read", "root":f.dir.path()}
    ]}))
    .unwrap();
    let server = Server::new(config).unwrap();
    for (id, allowed) in [("enabled", true), ("disabled", false), ("unknown", false)] {
        let result = server
            .call(
                "command_run",
                serde_json::from_value(
                    json!({"project_id":id,"program":"/usr/bin/touch","args":[id]}),
                )
                .unwrap(),
            )
            .await;
        assert_eq!(result.is_ok(), allowed);
        assert_eq!(f.dir.path().join(id).exists(), allowed);
    }
}

#[tokio::test]
async fn invalid_cwd_and_arguments_are_rejected_before_spawn() {
    let f = Fixture::new(Some(true));
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), f.dir.path().join("link")).unwrap();
    for extra in [
        json!({"program":"/usr/bin/touch", "args":["marker"], "cwd":"../"}),
        json!({"program":"/usr/bin/touch", "args":["marker"], "cwd":outside.path()}),
        json!({"program":"/usr/bin/touch", "args":["marker"], "cwd":"link"}),
        json!({"program":"/usr/bin/touch", "args":["marker"], "timeout_ms":0}),
        json!({"program":"/usr/bin/touch", "args":["marker"], "timeout_ms":120001}),
        json!({"program":"/usr/bin/touch", "args":["marker"], "content":"ignored?"}),
        json!({"program":"/usr/bin/touch", "args":["x".repeat(32769)]}),
        json!({"program":"/usr/bin/touch", "args":["bad\u{0}arg"]}),
    ] {
        assert!(f.call(extra).await.is_err());
    }
    assert!(!f.dir.path().join("marker").exists());
    assert!(!outside.path().join("marker").exists());
}

#[tokio::test]
async fn timeout_kills_background_children_and_returns_promptly() {
    let f = Fixture::new(Some(true));
    let start = Instant::now();
    let result = f.call(json!({"program":"/bin/sh", "args":["-c", "(sleep 1; touch survived) & wait"], "timeout_ms":80})).await.unwrap();
    assert_eq!(result["timed_out"], true);
    assert_eq!(result["success"], false);
    assert!(start.elapsed() < Duration::from_secs(2));
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(!f.dir.path().join("survived").exists());
}

#[tokio::test]
async fn cancellation_also_kills_command_group() {
    let f = Fixture::new(Some(true));
    let server = f.server.clone();
    let handle = tokio::spawn(async move {
        server.call("command_run", serde_json::from_value(json!({"project_id":"p","program":"/bin/sh","args":["-c","touch started; (sleep 1; touch survived) & wait"]})).unwrap()).await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !f.dir.path().join("started").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    handle.abort();
    let _ = handle.await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(!f.dir.path().join("survived").exists());
}

#[tokio::test]
async fn successful_exit_also_cleans_background_commands() {
    let f = Fixture::new(Some(true));
    let result = f
        .call(json!({"program":"/bin/sh", "args":["-c", "(sleep 1; touch survived) & exit 0"]}))
        .await
        .unwrap();
    assert_eq!(result["success"], true);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(!f.dir.path().join("survived").exists());
}

#[tokio::test]
async fn binary_control_output_still_returns_a_bounded_receipt() {
    let f = Fixture::new(Some(true));
    let response = handle_message(&f.server, json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":"command_run", "arguments":{"project_id":"p", "program":"/bin/sh", "args":["-c", "head -c 40000 /dev/zero; head -c 40000 /dev/zero >&2"]}}})).await.unwrap();
    assert_eq!(response["result"]["isError"], false);
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    let receipt: Value = serde_json::from_str(text).unwrap();
    assert_eq!(receipt["exit_code"], 0);
    assert_eq!(receipt["output_truncated"], true);
    assert!(text.len() < 65536);
}

#[tokio::test]
async fn output_is_bounded_redacted_and_keeps_nonzero_status() {
    let f = Fixture::new(Some(true));
    let result = f.call(json!({"program":"/bin/sh", "args":["-c", "printf 'api_key=fixture-only-value\\n'; /usr/bin/yes x | /usr/bin/head -c 100000; /usr/bin/yes y | /usr/bin/head -c 100000 >&2; exit 9"]})).await.unwrap();
    assert_eq!(result["exit_code"], 9);
    assert_eq!(result["output_truncated"], true);
    assert!(result["stdout"].as_str().unwrap().contains("[REDACTED]"));
    assert!(!result.to_string().contains("fixture-only-value"));
    assert!(result.to_string().len() < 65536);
    assert!(result["stdout"].as_str().unwrap().len() < 20000);
    assert!(result["stderr"].as_str().unwrap().len() < 20000);
}

#[tokio::test]
async fn command_tool_is_available_through_jsonrpc_with_execution_annotations() {
    let f = Fixture::new(Some(true));
    let list = localoud_mcp::tools_list();
    let tool = list.iter().find(|t| t["name"] == "command_run").unwrap();
    assert_eq!(tool["annotations"]["readOnlyHint"], false);
    assert_eq!(tool["annotations"]["destructiveHint"], true);
    assert_eq!(tool["annotations"]["openWorldHint"], true);
    let response = handle_message(&f.server, json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":"command_run", "arguments":{"project_id":"p", "program":"/bin/echo", "args":["ok"]}}})).await.unwrap();
    assert_eq!(response["result"]["isError"], false);
    let receipt: Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(receipt["stdout"], "ok\n");
    assert_eq!(receipt["exit_code"], 0);
}

#[tokio::test]
async fn concurrency_limit_is_shared_by_clones_and_released_on_cancellation() {
    let f = Fixture::new(Some(true));
    let mut active = Vec::new();
    for n in 0..4 {
        let server = f.server.clone();
        active.push(tokio::spawn(async move {
            server.call("command_run", serde_json::from_value(json!({"project_id":"p", "program":"/bin/sh", "args":["-c", format!("touch started-{n}; sleep 10")]})).unwrap()).await
        }));
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while !(0..4).all(|n| f.dir.path().join(format!("started-{n}")).exists()) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(f
        .call(json!({"program":"/bin/echo"}))
        .await
        .unwrap_err()
        .contains("Too many active commands"));
    for task in active {
        task.abort();
        let _ = task.await;
    }
    assert_eq!(
        f.call(json!({"program":"/bin/echo","args":["slot released"]}))
            .await
            .unwrap()["success"],
        true
    );
}

#[tokio::test]
async fn session_cancel_before_polling_never_launches_and_ids_can_be_reused_afterward() {
    let f = Fixture::new(Some(true));
    let session = localoud_mcp::RpcSession::new(f.server.clone());
    let request = json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":"command_run", "arguments":{"project_id":"p", "program":"/usr/bin/touch", "args":["should-not-exist"]}}});
    let first = session.handle(request.clone(), None);
    let duplicate = session.handle(request.clone(), None).await.unwrap();
    assert_eq!(duplicate["error"]["code"], -32600);
    assert!(session
        .handle(
            json!({"jsonrpc":"2.0", "method":"notifications/cancelled", "params":{"requestId":1}}),
            None
        )
        .await
        .is_none());
    assert_eq!(first.await.unwrap()["error"]["code"], -32800);
    assert!(!f.dir.path().join("should-not-exist").exists());
    assert_eq!(
        session.handle(request, None).await.unwrap()["result"]["isError"],
        false
    );
    assert!(f.dir.path().join("should-not-exist").exists());
}
