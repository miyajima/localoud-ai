use serde_json::{json, Value};
use std::{
    collections::HashMap,
    error::Error,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct HttpResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: Value,
}

fn post_json(
    address: SocketAddr,
    token: Option<&str>,
    origin: Option<&str>,
    session_id: Option<&str>,
    body: Value,
) -> std::io::Result<HttpResponse> {
    let body = body.to_string();
    let mut request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {address}\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(token) = token {
        request.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    if let Some(origin) = origin {
        request.push_str(&format!("Origin: {origin}\r\n"));
    }
    if let Some(session_id) = session_id {
        request.push_str(&format!("Mcp-Session-Id: {session_id}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(&body);

    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(request.as_bytes())?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;

    let separator = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| std::io::Error::other("HTTP response has no header terminator"))?;
    let header_text = String::from_utf8_lossy(&response[..separator]);
    let mut lines = header_text.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| std::io::Error::other("HTTP response has no status code"))?;
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let body = &response[separator + 4..];
    let body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(body).map_err(std::io::Error::other)?
    };
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

struct ServerGuard(Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct ProcessGroupGuard(libc::pid_t);

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if self.0 > 0 {
            unsafe {
                libc::kill(-self.0, libc::SIGKILL);
            }
        }
    }
}

fn wait_for_server(child: &mut Child, address: SocketAddr) -> std::io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(std::io::Error::other(format!(
                "MCP server exited before accepting requests: {status}"
            )));
        }
        if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "MCP server did not start listening",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_file(path: &std::path::Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.is_file() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    path.is_file()
}

fn process_exists(pid: libc::pid_t) -> bool {
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn wait_for_process_exit(pid: libc::pid_t, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !process_exists(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    !process_exists(pid)
}

#[test]
fn http_command_cancellation_is_authenticated_and_session_scoped() -> Result<(), Box<dyn Error>> {
    let project = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let reserved = TcpListener::bind("127.0.0.1:0")?;
    let address = reserved.local_addr()?;
    drop(reserved);

    let token = "localoud-http-test-token";
    let child = Command::new(env!("CARGO_BIN_EXE_localoud-mcp"))
        .args([
            "--project-root",
            project.path().to_str().ok_or("project path is not UTF-8")?,
            "--allow-exec",
            "--http",
            &address.to_string(),
        ])
        .env("HOME", home.path())
        .env("LOCALOUD_MCP_TOKEN", token)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut server = ServerGuard(child);
    wait_for_server(&mut server.0, address)?;

    let unauthenticated = post_json(
        address,
        None,
        None,
        None,
        json!({"jsonrpc":"2.0", "id":1, "method":"ping"}),
    )?;
    assert_eq!(unauthenticated.status, 401);

    let foreign_origin = post_json(
        address,
        Some(token),
        Some("http://foreign.example"),
        None,
        json!({"jsonrpc":"2.0", "id":2, "method":"ping"}),
    )?;
    assert_eq!(foreign_origin.status, 403);

    let initialize = |id| {
        post_json(
            address,
            Some(token),
            None,
            None,
            json!({"jsonrpc":"2.0", "id":id, "method":"initialize", "params":{"protocolVersion":"2026-07-28"}}),
        )
    };
    let first = initialize(3)?;
    assert_eq!(first.status, 200);
    let first_session = first
        .headers
        .get("mcp-session-id")
        .expect("initialize response must include mcp-session-id")
        .clone();
    let second = initialize(4)?;
    assert_eq!(second.status, 200);
    let second_session = second
        .headers
        .get("mcp-session-id")
        .expect("initialize response must include mcp-session-id")
        .clone();
    assert_ne!(first_session, second_session);

    let unknown_session = post_json(
        address,
        Some(token),
        None,
        Some("unknown-session-id"),
        json!({"jsonrpc":"2.0", "id":5, "method":"ping"}),
    )?;
    assert_eq!(unknown_session.status, 404);

    let pid_file = project.path().join("command-pids.txt");
    let finished_file = project.path().join("command-finished.txt");
    let command = json!({
        "jsonrpc":"2.0",
        "id":1,
        "method":"tools/call",
        "params":{"name":"command_run", "arguments":{
            "project_id":"local",
            "program":"python3",
            "args":[
                "-c",
                "import os,pathlib,subprocess,sys,time; child=subprocess.Popen(['/bin/sleep','1.2']); pathlib.Path(sys.argv[1]).write_text(f'{os.getpgrp()} {child.pid}'); time.sleep(1.2); pathlib.Path(sys.argv[2]).write_text('finished')",
                pid_file.to_str().ok_or("pid path is not UTF-8")?,
                finished_file.to_str().ok_or("finished path is not UTF-8")?
            ],
            "timeout_ms":5000
        }}
    });
    let request_session = first_session.clone();
    let request_token = token.to_owned();
    let request_thread = thread::spawn(move || {
        post_json(
            address,
            Some(&request_token),
            None,
            Some(&request_session),
            command,
        )
    });

    if !wait_for_file(&pid_file, Duration::from_secs(1)) {
        let response = request_thread
            .join()
            .expect("command request thread panicked")?;
        panic!("command did not start; response was {}", response.body);
    }
    let pids = std::fs::read_to_string(&pid_file)?;
    let mut pids = pids.split_whitespace();
    let process_group: libc::pid_t = pids
        .next()
        .ok_or("command process group was not recorded")?
        .parse()?;
    let child_pid: libc::pid_t = pids
        .next()
        .ok_or("command child pid was not recorded")?
        .parse()?;
    assert!(process_group > 0 && child_pid > 0);
    let _process_cleanup = ProcessGroupGuard(process_group);
    assert!(process_exists(process_group));
    assert!(process_exists(child_pid));

    let foreign_cancel = post_json(
        address,
        Some(token),
        None,
        Some(&second_session),
        json!({"jsonrpc":"2.0", "method":"notifications/cancelled", "params":{"requestId":1}}),
    )?;
    assert_eq!(foreign_cancel.status, 202);
    assert!(
        process_exists(process_group),
        "another session cancelled the request"
    );
    assert!(
        process_exists(child_pid),
        "another session cancelled the child process"
    );

    let cancel = post_json(
        address,
        Some(token),
        None,
        Some(&first_session),
        json!({"jsonrpc":"2.0", "method":"notifications/cancelled", "params":{"requestId":1}}),
    )?;
    assert_eq!(cancel.status, 202);

    let original_response = request_thread
        .join()
        .expect("command request thread panicked")?;
    assert_eq!(original_response.status, 200);
    assert_eq!(original_response.body["id"], 1);
    assert_eq!(original_response.body["error"]["code"], -32800);
    assert!(wait_for_process_exit(
        process_group,
        Duration::from_millis(300)
    ));
    assert!(wait_for_process_exit(child_pid, Duration::from_millis(300)));
    assert!(
        !finished_file.exists(),
        "cancelled command wrote its completion marker"
    );

    Ok(())
}
