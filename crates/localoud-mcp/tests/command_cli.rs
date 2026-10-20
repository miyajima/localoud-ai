#![cfg(unix)]

use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn cli_requires_separate_execution_flag_and_does_not_inherit_credentials() {
    let dir = tempfile::tempdir().unwrap();
    for enabled in [false, true] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_localoud-mcp"));
        cmd.args([
            "--project-root",
            dir.path().to_str().unwrap(),
            "--allow-write",
        ]);
        if enabled {
            cmd.arg("--allow-exec");
        }
        cmd.env("LOCALOUD_TEST_PRIVATE_VALUE", "fixture-private-value")
            .env("LOCALOUD_MCP_TOKEN", "fixture-http-token")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        let request = json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":"command_run", "arguments":{"project_id":"local", "program":"/usr/bin/env"}}});
        writeln!(child.stdin.take().unwrap(), "{request}").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["result"]["isError"], !enabled);
        if enabled {
            let receipt: Value =
                serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(receipt["exit_code"], 0);
            assert!(!receipt.to_string().contains("fixture-private-value"));
            assert!(!receipt.to_string().contains("fixture-http-token"));
        }
    }
}

#[test]
fn allow_exec_requires_project_root_instead_of_silently_enabling_config_projects() {
    let output = Command::new(env!("CARGO_BIN_EXE_localoud-mcp"))
        .args(["--allow-exec", "--config", "/nonexistent/config.json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("--allow-exec only applies to --project-root"));
}

#[test]
fn http_execution_requires_auth_even_with_insecure_read_mode_requested() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_localoud-mcp"))
        .args([
            "--project-root",
            dir.path().to_str().unwrap(),
            "--allow-exec",
            "--http",
            "127.0.0.1:0",
            "--insecure-no-token",
        ])
        .env_remove("LOCALOUD_MCP_TOKEN")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The test must never hang if an implementation mistakenly starts HTTP.
    let mut child = output;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let exited = loop {
        if child.try_wait().unwrap().is_some() {
            break true;
        }
        if std::time::Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    if !exited {
        child.kill().unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        exited,
        "execution-enabled HTTP must refuse to start without authentication"
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("LOCALOUD_MCP_TOKEN"));
}

#[test]
fn stdio_cancel_notification_stops_the_running_command() {
    use std::io::{BufRead, BufReader};
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_localoud-mcp"))
        .args([
            "--project-root",
            dir.path().to_str().unwrap(),
            "--allow-exec",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let request = json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":"command_run", "arguments":{"project_id":"local", "program":"/bin/sh", "args":["-c", "touch started; sleep 1; touch survived"]}}});
    writeln!(input, "{request}").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !dir.path().join("started").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(dir.path().join("started").exists());
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0", "method":"notifications/cancelled", "params":{"requestId":1}})
    )
    .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut line = String::new();
    BufReader::new(stdout).read_line(&mut line).unwrap();
    drop(input);
    child.wait().unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], 1);
    assert_eq!(response["error"]["code"], -32800);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert!(!dir.path().join("survived").exists());
}
