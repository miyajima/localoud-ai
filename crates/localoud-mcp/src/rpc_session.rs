//! Request cancellation is scoped to one stdio connection / HTTP MCP session.
use crate::{handle_message_with, Server};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio::sync::oneshot;

type Pending = Arc<Mutex<HashMap<String, Option<oneshot::Sender<()>>>>>;
type Response = Pin<Box<dyn Future<Output = Option<Value>> + Send>>;

#[derive(Clone)]
pub struct RpcSession {
    server: Server,
    pending: Pending,
}

struct PendingRequest {
    requests: Pending,
    key: String,
}

impl Drop for PendingRequest {
    fn drop(&mut self) {
        if let Ok(mut requests) = self.requests.lock() {
            requests.remove(&self.key);
        }
    }
}

impl RpcSession {
    pub fn new(server: Server) -> Self {
        Self {
            server,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register synchronously so cancellation cannot overtake a received
    /// command whose response task has not been polled yet.
    pub fn handle(&self, v: Value, protocol_version: Option<&str>) -> Response {
        if v["jsonrpc"] == "2.0"
            && v.get("id").is_none()
            && v["method"] == "notifications/cancelled"
        {
            let id = &v["params"]["requestId"];
            if id.is_string() || id.is_number() {
                if let Ok(mut pending) = self.pending.lock() {
                    // Retain the occupied id until its future drops so an old
                    // guard cannot remove a newly reused request id.
                    if let Some(sender) = pending.get_mut(&id.to_string()).and_then(Option::take) {
                        let _ = sender.send(());
                    }
                }
            }
            return Box::pin(async { None });
        }
        let server = self.server.clone();
        let version = protocol_version.map(str::to_owned);
        let id = v.get("id").cloned().unwrap_or(Value::Null);
        if v["method"] != "tools/call"
            || v["params"]["name"] != "command_run"
            || v.get("id").is_none()
        {
            return Box::pin(
                async move { handle_message_with(&server, v, version.as_deref()).await },
            );
        }
        let key = id.to_string();
        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = self.pending.lock().expect("pending requests mutex");
            if pending.contains_key(&key) || pending.len() >= 128 {
                return Box::pin(async move {
                    Some(json!({"jsonrpc":"2.0", "id":id,
                    "error":{"code":-32600, "message":"Duplicate or excessive active command requests"}}))
                });
            }
            pending.insert(key.clone(), Some(sender));
        }
        let guard = PendingRequest {
            requests: self.pending.clone(),
            key,
        };
        Box::pin(async move {
            let _guard = guard;
            tokio::select! {
                biased;
                _ = receiver => Some(json!({"jsonrpc":"2.0", "id":id,
                    "error":{"code":-32800, "message":"Command request cancelled; inspect side effects before retrying"}})),
                response = handle_message_with(&server, v, version.as_deref()) => response,
            }
        })
    }
}
