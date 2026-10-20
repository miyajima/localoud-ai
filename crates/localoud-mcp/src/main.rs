//! `localoud-mcp` binary: stdio by default, optional loopback HTTP.

use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use localoud_mcp::{McpFileConfig, ProjectEntry, RpcSession, Server};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    net::IpAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn usage() -> String {
    "localoud-mcp [--config PATH] [--project-root PATH [--allow-write] [--allow-exec]] [--http ADDR [--insecure-no-token]] [--stdio]\n\n\
     stdio (default): JSON-RPC over stdin/stdout for Codex / Claude Code.\n\
     --http 127.0.0.1:8792: serve Streamable HTTP on loopback instead of stdio.\n\
     --http ADDR --stdio: serve both transports concurrently.\n\
     HTTP auth: set LOCALOUD_MCP_TOKEN; clients send `Authorization: Bearer <token>`.\n\
     --insecure-no-token: allow the HTTP listener without a token (loopback only, not recommended).\n\
     --allow-exec: opt the quick-mode project into host commands with the server user's OS permissions (not sandboxed).\n\
     Flags accept both `--flag value` and `--flag=value` forms.\n\
     Config JSON: {\"projects\": [{\"id\": \"...\", \"name\": \"...\", \"root\": \"/abs/path\", \"write_enabled\": true, \"execution_enabled\": false}]}"
        .to_owned()
}

fn load_config(args: &[String]) -> Result<McpFileConfig, String> {
    let mut config_path: Option<String> = None;
    let mut project_root: Option<String> = None;
    let mut allow_write = false;
    let mut allow_exec = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => {
                i += 1;
                config_path = Some(args.get(i).ok_or("--config needs a value")?.clone());
            }
            "--project-root" => {
                i += 1;
                project_root = Some(args.get(i).ok_or("--project-root needs a value")?.clone());
            }
            "--allow-write" => allow_write = true,
            "--allow-exec" => allow_exec = true,
            other => return Err(format!("unknown argument: {other}\n{}", usage())),
        }
        i += 1;
    }
    if allow_exec && project_root.is_none() {
        return Err("--allow-exec only applies to --project-root; set execution_enabled in the config file instead".into());
    }
    if allow_write && project_root.is_none() {
        return Err(format!(
            "--allow-write only applies to --project-root; set \"write_enabled\" in the config file instead\n{}",
            usage()
        ));
    }
    if let Some(root) = project_root {
        return Ok(McpFileConfig {
            projects: vec![ProjectEntry {
                id: "local".into(),
                name: PathBuf::from(&root)
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "project".into()),
                root: PathBuf::from(root),
                write_enabled: allow_write,
                execution_enabled: allow_exec,
            }],
        });
    }
    let path = config_path
        .or_else(|| {
            std::env::var("LOCALOUD_MCP_CONFIG")
                .ok()
                .or_else(default_config_path)
        })
        .ok_or_else(usage)?;
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("cannot read config {path}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("invalid config {path}: {e}"))
}

fn default_config_path() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .map(|h| format!("{h}/.config/localoud/mcp-projects.json"))
}

async fn stdio_loop(server: Server) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let stdin = tokio::io::stdin();
    let mut lines = tokio::io::BufReader::new(stdin).lines();
    let stdout = Arc::new(tokio::sync::Mutex::new(tokio::io::stdout()));
    let session = RpcSession::new(server);
    let mut requests = tokio::task::JoinSet::new();
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                let err = json!({"jsonrpc": "2.0", "id": null,
                    "error": {"code": -32700, "message": "Parse error"}});
                let mut stdout = stdout.lock().await;
                stdout.write_all(format!("{}\n", err).as_bytes()).await.ok();
                stdout.flush().await.ok();
                continue;
            }
        };
        while requests.try_join_next().is_some() {}
        if requests.len() >= 128 && value["method"] != "notifications/cancelled" {
            let response = json!({"jsonrpc":"2.0", "id":value.get("id").cloned().unwrap_or(Value::Null), "error":{"code":-32600,"message":"Too many active requests"}});
            let mut output = stdout.lock().await;
            output
                .write_all(format!("{response}\n").as_bytes())
                .await
                .ok();
            output.flush().await.ok();
            continue;
        }
        let response = session.handle(value, None);
        let stdout = stdout.clone();
        requests.spawn(async move {
            if let Some(response) = response.await {
                let mut output = stdout.lock().await;
                output
                    .write_all(format!("{response}\n").as_bytes())
                    .await
                    .ok();
                output.flush().await.ok();
            }
        });
    }
    while requests.join_next().await.is_some() {}
}

/// `Host` / `Origin` authorities must be loopback (DNS-rebinding defense).
fn authority_is_loopback(value: &str, port: u16) -> bool {
    let (host, value_port) = if let Some(rest) = value.strip_prefix('[') {
        match rest.split_once(']') {
            Some((host, rest)) => (host, rest.strip_prefix(':')),
            None => return false,
        }
    } else {
        match value.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (value, None),
        }
    };
    if value_port.is_some_and(|p| p != port.to_string()) {
        return false;
    }
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn local_request(headers: &HeaderMap, port: u16) -> bool {
    let host_ok = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|value| authority_is_loopback(value, port));
    let origin_ok = headers.get("origin").is_none_or(|v| {
        v.to_str()
            .ok()
            .and_then(|value| value.strip_prefix("http://"))
            .is_some_and(|value| authority_is_loopback(value, port))
    });
    host_ok && origin_ok
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0
}

async fn http_rpc(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Json(v): Json<Value>,
) -> Response {
    if !local_request(&headers, state.port) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Some(expected) = &state.token {
        let ok = headers
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|h| {
                constant_time_eq(h.as_bytes(), format!("Bearer {expected}").as_bytes())
            });
        if !ok {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    if !headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/json"))
        && v.get("method").is_some()
    {
        // Be lenient: stdio-style clients probing HTTP without Accept still work
        // when they POST JSON. Only enforce when an Accept header is present.
        if headers.contains_key("accept") {
            return StatusCode::NOT_ACCEPTABLE.into_response();
        }
    }
    let protocol_version = headers
        .get("mcp-protocol-version")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut new_session_id = None;
    let session = {
        let mut sessions = state.sessions.lock().expect("HTTP sessions mutex");
        sessions.retain(|_, (_, last_seen)| last_seen.elapsed() < Duration::from_secs(900));
        if v["method"] == "initialize" {
            if sessions.len() >= 128 {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            let id = uuid::Uuid::new_v4().to_string();
            let session = RpcSession::new(state.server.clone());
            sessions.insert(id.clone(), (session.clone(), Instant::now()));
            new_session_id = Some(id);
            session
        } else if let Some(id) = headers.get("mcp-session-id") {
            let Some((session, last_seen)) = id.to_str().ok().and_then(|id| sessions.get_mut(id))
            else {
                return StatusCode::NOT_FOUND.into_response();
            };
            *last_seen = Instant::now();
            session.clone()
        } else {
            // Stateless probes remain compatible. Clients use the session ID
            // returned by initialize to cancel their own active commands.
            RpcSession::new(state.server.clone())
        }
    };
    let response = session.handle(v, protocol_version.as_deref());
    let mut response = match response.await {
        Some(resp) => Json(resp).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    };
    if let Some(id) = new_session_id {
        response
            .headers_mut()
            .insert("mcp-session-id", id.parse().expect("UUID header"));
    }
    response
}

struct HttpState {
    server: Server,
    token: Option<String>,
    port: u16,
    sessions: Mutex<HashMap<String, (RpcSession, Instant)>>,
}

#[tokio::main]
async fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", usage());
        return;
    }
    // Normalize `--flag=value` into `--flag value` for every flag.
    let mut args: Vec<String> = Vec::with_capacity(raw.len());
    for a in raw {
        match a.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => {
                args.push(flag.to_owned());
                args.push(value.to_owned());
            }
            _ => args.push(a),
        }
    }
    let mut http_addr: Option<String> = None;
    let mut stdio_flag = false;
    let mut insecure_no_token = false;
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--http" => match args.get(i + 1) {
                Some(value) => {
                    http_addr = Some(value.clone());
                    i += 1;
                }
                None => {
                    eprintln!("--http needs an address, e.g. --http 127.0.0.1:8792");
                    std::process::exit(2);
                }
            },
            "--stdio" => stdio_flag = true,
            "--insecure-no-token" => insecure_no_token = true,
            other => rest.push(other.to_owned()),
        }
        i += 1;
    }
    // stdio is the default transport; --http replaces it unless --stdio is also
    // given, so a closed stdin cannot kill an HTTP-only server.
    let want_stdio = http_addr.is_none() || stdio_flag;
    let config = match load_config(&rest) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let execution_enabled = config.projects.iter().any(|p| p.execution_enabled);
    let server = match Server::new(config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("invalid MCP configuration: {e}");
            std::process::exit(1);
        }
    };
    let Some(addr) = http_addr else {
        stdio_loop(server).await;
        return;
    };
    let token = std::env::var("LOCALOUD_MCP_TOKEN")
        .ok()
        .filter(|t| !t.is_empty());
    if token.is_none() && execution_enabled {
        eprintln!("HTTP command execution requires LOCALOUD_MCP_TOKEN, including with --insecure-no-token");
        std::process::exit(2);
    }
    if token.is_none() && !insecure_no_token {
        eprintln!(
            "refusing HTTP without LOCALOUD_MCP_TOKEN; set it or pass --insecure-no-token for loopback-only use"
        );
        std::process::exit(2);
    }
    let addrs: Vec<std::net::SocketAddr> = match tokio::net::lookup_host(&addr).await {
        Ok(iter) => iter.collect(),
        Err(e) => {
            eprintln!("cannot resolve {addr}: {e}");
            std::process::exit(2);
        }
    };
    if addrs.is_empty() || addrs.iter().any(|a| !a.ip().is_loopback()) {
        eprintln!("refusing non-loopback HTTP bind: {addr}");
        std::process::exit(2);
    }
    let listener = match tokio::net::TcpListener::bind(addrs.as_slice()).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    let port = match listener.local_addr() {
        Ok(local) => local.port(),
        Err(e) => {
            eprintln!("cannot inspect {addr}: {e}");
            std::process::exit(1);
        }
    };
    let state = Arc::new(HttpState {
        server: server.clone(),
        token,
        port,
        sessions: Mutex::new(HashMap::new()),
    });
    let app = Router::new()
        .route("/mcp", post(http_rpc))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state);
    eprintln!("localoud-mcp serving http://{addr}/mcp");
    if want_stdio {
        eprintln!("localoud-mcp also serving stdio");
        let serve = tokio::spawn(async move { axum::serve(listener, app).await });
        stdio_loop(server).await;
        serve.abort();
    } else if let Err(e) = axum::serve(listener, app).await {
        eprintln!("http server error: {e}");
        std::process::exit(1);
    }
}
