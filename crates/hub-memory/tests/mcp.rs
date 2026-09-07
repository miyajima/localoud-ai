use anyhow::Result;
use hub_core::*;
use hub_memory::*;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[tokio::test]
async fn stateless_mcp_persists_durable_rationale_across_client_sessions() -> Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}/mcp", listener.local_addr()?);
    let server = tokio::spawn(async move {
        let mut saved = Value::Null;
        for index in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = vec![];
            let end;
            loop {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(i) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    end = i + 4;
                    break;
                }
            }
            let headers = String::from_utf8(bytes[..end].to_vec())
                .unwrap()
                .to_lowercase();
            assert!(headers.contains("cf-access-client-id: fixture-id"));
            assert!(headers.contains("cf-access-client-secret: fixture-secret"));
            let len: usize = headers
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            while bytes.len() < end + len {
                let mut b = [0; 4096];
                let n = socket.read(&mut b).await.unwrap();
                bytes.extend_from_slice(&b[..n]);
            }
            let req: Value = serde_json::from_slice(&bytes[end..end + len]).unwrap();
            assert_eq!(req["method"], "tools/call");
            assert_eq!(req["params"]["arguments"]["tenant_id"], "fixture");
            let result = if index == 0 {
                assert_eq!(req["params"]["name"], "orgbrain_memories_upsert");
                saved = req["params"]["arguments"]["items"][0].clone();
                assert_eq!(saved["project_id"], "mapped-project");
                json!({"upserted":1})
            } else if index == 1 {
                assert_eq!(req["params"]["name"], "orgbrain_memories_search");
                json!({"results":[{"id":"memory-1","content_preview":saved["content"]}]})
            } else {
                assert_eq!(req["params"]["name"], "orgbrain_decision_memories_search");
                json!({"results":[{"id":"decision-1","decision":"Use injection","rationale":"Avoid shared global state"}]})
            };
            let body=json!({"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":result.to_string()}]}}).to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
        }
    });
    let config = OrgBrainConfig {
        endpoint,
        tenant: "fixture".into(),
        remote_project_id: "mapped-project".into(),
        auto_store: true,
    };
    let item = DurableMemoryItem {
        external_key: "stable-key".into(),
        project_id: ProjectId::default(),
        task_id: TaskId::default(),
        candidate: MemoryCandidate {
            kind: MemoryKind::RejectedApproach,
            durable: true,
            statement: "Avoid mutable global caches".into(),
            reason: "Prior tests exposed cross-request leakage".into(),
            source_refs: vec!["test-17".into()],
        },
    };
    let first = OrgBrain::new(config.clone(), "fixture-id".into(), "fixture-secret".into())?;
    first.store(vec![item]).await?;
    drop(first);
    let second = OrgBrain::new(config, "fixture-id".into(), "fixture-secret".into())?;
    let found = second
        .search(MemorySearchRequest {
            query: "cache".into(),
            limit: 8,
        })
        .await?;
    assert!(found[0].text.contains("cross-request leakage"));
    assert_eq!(found[0].source_refs, ["orgbrain:memory-1"]);
    assert!(found
        .iter()
        .any(|m| m.text.contains("Avoid shared global state")));
    server.await?;
    Ok(())
}
