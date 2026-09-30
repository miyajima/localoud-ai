fn allow_edits(s: &ReadMcp, id: &str) {
    let mut config: Value = serde_json::from_str(
        &s.store
            .lock()
            .unwrap()
            .setting("read_mcp_settings")
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    config["write_project_ids"] = json!([id]);
    s.store
        .lock()
        .unwrap()
        .set_setting("read_mcp_settings", &config.to_string())
        .unwrap();
}
async fn edit_rpc(s: Arc<ReadMcp>, name: &str, args: Value) -> Value {
    let response = rpc(State(s), modern_headers("tools/call", Some(name)), Json(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}}))).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), RESPONSE_BYTES)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
fn payload(v: &Value) -> Value {
    assert!(v.get("error").is_none(), "{v}");
    assert_eq!(v["result"]["isError"], false, "{v}");
    serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}
fn failed(v: &Value) {
    assert!(
        v.get("error").is_some() || v["result"]["isError"] == true,
        "{v}"
    );
}
#[tokio::test]
async fn mcp_edit_creates_and_replaces_exact_text_with_receipts() {
    use std::os::unix::fs::PermissionsExt;
    let (d, s, id) = fixture();
    allow_edits(&s, &id);
    let root = d.path().join("project");
    std::fs::create_dir(root.join("src")).unwrap();
    let created = payload(
        &edit_rpc(
            s.clone(),
            "file_create",
            json!({"project_id":id,"path":"src/hello.txt","content":"日本語\r\nhello\r\n"}),
        )
        .await,
    );
    assert_eq!(
        std::fs::read(root.join("src/hello.txt")).unwrap(),
        "日本語\r\nhello\r\n".as_bytes()
    );
    assert_eq!(
        created["sha256"],
        format!("{:x}", Sha256::digest("日本語\r\nhello\r\n".as_bytes()))
    );
    assert_eq!(created["bytes"], 18);
    std::fs::set_permissions(
        root.join("src/hello.txt"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let edited = payload(&edit_rpc(s.clone(), "file_edit", json!({"project_id":id,"path":"src/hello.txt","expected_sha256":created["sha256"],"old_text":"hello","new_text":"world"})).await);
    assert_eq!(edited["previous_sha256"], created["sha256"]);
    assert_eq!(
        std::fs::read_to_string(root.join("src/hello.txt")).unwrap(),
        "日本語\r\nworld\r\n"
    );
    assert_eq!(
        std::fs::metadata(root.join("src/hello.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    let read = payload(
        &edit_rpc(
            s,
            "file_read",
            json!({"project_id":id,"path":"src/hello.txt"}),
        )
        .await,
    );
    assert_eq!(read["sha256"], edited["sha256"]);
    assert!(created["operation_id"].as_str().is_some());
}
#[tokio::test]
async fn mcp_edit_requires_explicit_project_permission_and_source_scope() {
    let (d, s, id) = fixture();
    let args = json!({"project_id":id,"path":"hello.txt","content":"hello"});
    failed(&edit_rpc(s.clone(), "file_create", args.clone()).await);
    assert!(!d.path().join("project/hello.txt").exists());
    allow_edits(&s, &id);
    let mut wrong_project = args.clone();
    wrong_project["project_id"] = json!("unpublished");
    failed(&edit_rpc(s.clone(), "file_create", wrong_project).await);
    let mut task = args.clone();
    task["task_id"] = json!(hub_core::TaskId::default().to_string());
    failed(&edit_rpc(s.clone(), "file_create", task).await);
    let mut step = args;
    step["step_key"] = json!("worker");
    failed(&edit_rpc(s, "file_create", step).await);
    assert!(!d.path().join("project/hello.txt").exists());
}
#[tokio::test]
async fn mcp_edit_rejects_conflicts_ambiguous_replacements_and_overwrites() {
    let (d, s, id) = fixture();
    allow_edits(&s, &id);
    let root = d.path().join("project");
    std::fs::write(root.join("hello.txt"), "same same\n").unwrap();
    let hash = format!("{:x}", Sha256::digest(b"same same\n"));
    for (expected, old) in [
        ("0".repeat(64), "same same"),
        (hash.clone(), "same"),
        (hash.clone(), "absent"),
        (hash.clone(), ""),
    ] {
        failed(&edit_rpc(s.clone(), "file_edit", json!({"project_id":id,"path":"hello.txt","expected_sha256":expected,"old_text":old,"new_text":"changed"})).await);
        assert_eq!(
            std::fs::read_to_string(root.join("hello.txt")).unwrap(),
            "same same\n"
        );
    }
    failed(
        &edit_rpc(
            s.clone(),
            "file_create",
            json!({"project_id":id,"path":"hello.txt","content":"overwritten"}),
        )
        .await,
    );
    failed(
        &edit_rpc(
            s.clone(),
            "file_edit",
            json!({"project_id":id,"path":"hello.txt","old_text":"same same","new_text":"changed"}),
        )
        .await,
    );
    failed(
        &edit_rpc(
            s,
            "file_create",
            json!({"project_id":id,"path":"missing/hello.txt","content":"hello"}),
        )
        .await,
    );
    assert!(!root.join("missing").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("hello.txt")).unwrap(),
        "same same\n"
    );
}
#[tokio::test]
async fn mcp_edit_denies_escapes_links_sensitive_paths_and_non_text() {
    let (d, s, id) = fixture();
    allow_edits(&s, &id);
    let root = d.path().join("project");
    std::fs::write(d.path().join("outside.txt"), "OUTSIDE").unwrap();
    std::os::unix::fs::symlink(d.path(), root.join("link")).unwrap();
    std::os::unix::fs::symlink(d.path().join("outside.txt"), root.join("leaf.txt")).unwrap();
    std::fs::hard_link(d.path().join("outside.txt"), root.join("hard.txt")).unwrap();
    std::fs::write(root.join("binary.txt"), [0xff, 0]).unwrap();
    std::fs::create_dir(root.join("folder")).unwrap();
    for path in [
        "../outside.txt",
        "/tmp/outside.txt",
        ".env",
        ".git/config",
        "secrets.txt",
        "link/new.txt",
        "leaf.txt",
        "hard.txt",
        "binary.txt",
        "folder",
        "AGENTS.md",
        "CLAUDE.md",
    ] {
        failed(
            &edit_rpc(
                s.clone(),
                "file_create",
                json!({"project_id":id,"path":path,"content":"unsafe"}),
            )
            .await,
        );
        failed(&edit_rpc(s.clone(), "file_edit", json!({"project_id":id,"path":path,"expected_sha256":format!("{:x}",Sha256::digest(b"OUTSIDE")),"old_text":"OUTSIDE","new_text":"unsafe"})).await);
    }
    for text in ["nul\0text".to_owned(), "a".repeat(32 * 1024 + 1)] {
        failed(
            &edit_rpc(
                s.clone(),
                "file_create",
                json!({"project_id":id,"path":"bounded.txt","content":text}),
            )
            .await,
        );
    }
    failed(&edit_rpc(s.clone(), "file_create", json!({"project_id":id,"path":"new.txt","content":"TOKEN=sk-1234567890abcdef1234567890abcdef"})).await);
    assert!(!root.join("new.txt").exists());
    assert!(!root.join("bounded.txt").exists());
    assert!(!d.path().join("new.txt").exists());
    assert_eq!(
        std::fs::read_to_string(d.path().join("outside.txt")).unwrap(),
        "OUTSIDE"
    );
    assert!(std::fs::read_dir(&root).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".localoud-edit-")));
}
#[tokio::test]
async fn mcp_edit_serializes_updates_and_requires_authentication() {
    let (d, s, id) = fixture();
    allow_edits(&s, &id);
    std::fs::write(d.path().join("project/hello.txt"), "original").unwrap();
    let args = json!({"project_id":id,"path":"hello.txt","expected_sha256":format!("{:x}",Sha256::digest(b"original")),"old_text":"original","new_text":"updated"});
    let denied = rpc(State(s.clone()), HeaderMap::new(), Json(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"file_edit","arguments":args}}))).await;
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    let (a, b) = tokio::join!(
        edit_rpc(s.clone(), "file_edit", args.clone()),
        edit_rpc(s.clone(), "file_edit", args)
    );
    assert_eq!(
        [a, b]
            .iter()
            .filter(|v| v["result"]["isError"] == false)
            .count(),
        1
    );
    assert_eq!(
        std::fs::read_to_string(d.path().join("project/hello.txt")).unwrap(),
        "updated"
    );
}
#[test]
fn mcp_edit_config_defaults_to_read_only_and_rejects_unpublished_permissions() {
    let (_d, s, id) = fixture();
    assert!(s.config().unwrap().write_project_ids.is_empty());
    let invalid = serde_json::from_value::<McpConfig>(json!({"enabled":true,"project_ids":[id],"write_project_ids":["unpublished"],"connection":{"kind":"unconfigured"}})).unwrap();
    assert!(s.validate_config(&invalid).is_err());
    let duplicate = serde_json::from_value::<McpConfig>(json!({"enabled":true,"project_ids":[id],"write_project_ids":[id,id],"connection":{"kind":"unconfigured"}})).unwrap();
    assert!(s.validate_config(&duplicate).is_err());
    let valid = serde_json::from_value::<McpConfig>(json!({"enabled":true,"project_ids":[id],"write_project_ids":[id],"connection":{"kind":"unconfigured"}})).unwrap();
    assert!(s.validate_config(&valid).is_ok());
}

#[tokio::test]
async fn mcp_edit_refuses_redacted_readonly_and_oversized_files_and_overlapping_matches() {
    use std::os::unix::fs::PermissionsExt;
    let (d, s, id) = fixture();
    allow_edits(&s, &id);
    let path = d.path().join("project/hello.txt");
    for text in [
        "aaa".to_owned(),
        "TOKEN=sk-1234567890abcdef1234567890abcdef\nhello".to_owned(),
        "x".repeat(FILE_BYTES as usize + 1),
    ] {
        std::fs::write(&path, &text).unwrap();
        let (old, new) = if text == "aaa" {
            ("aa", "b")
        } else if text.starts_with("TOKEN=") {
            ("hello", "changed")
        } else {
            ("x", "b")
        };
        failed(&edit_rpc(s.clone(),"file_edit",json!({"project_id":id,"path":"hello.txt","expected_sha256":format!("{:x}",Sha256::digest(text.as_bytes())),"old_text":old,"new_text":new})).await);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
    let at_limit = format!("hello{}", "x".repeat(FILE_BYTES as usize - 5));
    std::fs::write(&path, &at_limit).unwrap();
    failed(&edit_rpc(s.clone(),"file_edit",json!({"project_id":id,"path":"hello.txt","expected_sha256":format!("{:x}",Sha256::digest(at_limit.as_bytes())),"old_text":"hello","new_text":"larger"})).await);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), at_limit);
    std::fs::write(&path, "hello").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
    failed(&edit_rpc(s,"file_edit",json!({"project_id":id,"path":"hello.txt","expected_sha256":format!("{:x}",Sha256::digest(b"hello")),"old_text":"hello","new_text":"changed"})).await);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
}

#[tokio::test]
async fn mcp_edit_concurrent_creates_never_replace_an_existing_destination() {
    let (d, s, id) = fixture();
    allow_edits(&s, &id);
    let a = json!({"project_id":id,"path":"new.txt","content":"first\n"});
    let b = json!({"project_id":id,"path":"new.txt","content":"second\n"});
    let (a, b) = tokio::join!(
        edit_rpc(s.clone(), "file_create", a),
        edit_rpc(s, "file_create", b)
    );
    assert_eq!(
        [a, b]
            .iter()
            .filter(|v| v["result"]["isError"] == false)
            .count(),
        1
    );
    assert!(["first\n", "second\n"].contains(
        &std::fs::read_to_string(d.path().join("project/new.txt"))
            .unwrap()
            .as_str()
    ));
}

#[tokio::test]
async fn mcp_edit_http_transport_creates_edits_reads_and_enforces_body_limit() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (d, s, id) = fixture();
    allow_edits(&s, &id);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router(s)).await.unwrap() });
    let send = |name: &'static str, args: Value| async move {
        let body=json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}}).to_string();
        let request=format!("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:8792\r\nAuthorization: Bearer fixture-token\r\nAccept: application/json\r\nContent-Type: application/json\r\nMCP-Protocol-Version: {MODERN_PROTOCOL}\r\nMcp-Method: tools/call\r\nMcp-Name: {name}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        stream
            .take((RESPONSE_BYTES + 4096) as u64)
            .read_to_string(&mut response)
            .await
            .unwrap();
        response
    };
    let response = send(
        "file_create",
        json!({"project_id":id,"path":"web-test.txt","content":"created\n"}),
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let created = payload(
        &serde_json::from_str::<Value>(response.split_once("\r\n\r\n").unwrap().1).unwrap(),
    );
    let response=send("file_edit",json!({"project_id":id,"path":"web-test.txt","expected_sha256":created["sha256"],"old_text":"created","new_text":"edited"})).await;
    let edited = payload(
        &serde_json::from_str::<Value>(response.split_once("\r\n\r\n").unwrap().1).unwrap(),
    );
    assert_eq!(
        std::fs::read(d.path().join("project/web-test.txt")).unwrap(),
        b"edited\n"
    );
    let response = send("file_read", json!({"project_id":id,"path":"web-test.txt"})).await;
    let read = payload(
        &serde_json::from_str::<Value>(response.split_once("\r\n\r\n").unwrap().1).unwrap(),
    );
    assert_eq!(read["sha256"], edited["sha256"]);
    let response = send(
        "file_create",
        json!({"project_id":id,"path":"huge.txt","content":"x".repeat(65*1024)}),
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 413"), "{response}");
    assert!(!d.path().join("project/huge.txt").exists());
    server.abort();
    let _ = server.await;
}
#[tokio::test]
async fn mcp_edit_is_advertised_as_write_and_a2a_stays_read_only() {
    let (d, s, id) = fixture();
    allow_edits(&s, &id);
    let tools = tools_list();
    for name in ["file_create", "file_edit"] {
        let tool = tools
            .iter()
            .find(|t| t["name"] == name)
            .expect("write tool advertised");
        assert_eq!(tool["annotations"]["readOnlyHint"], false);
        assert_eq!(tool["annotations"]["idempotentHint"], false);
    }
    let message=protocol_types::a2a::data_message("write-denied",None,None,json!({"skillId":"localoud-read-evidence","tool":"file_create","arguments":{"project_id":id,"path":"hello.txt","content":"no"}}),protocol_types::a2a::READ_REQUEST_MEDIA_TYPE,vec![]).unwrap();
    let response = a2a_send(
        State(s),
        a2a_headers(),
        Json(protocol_types::a2a::SendMessageRequest {
            tenant: None,
            message,
            configuration: None,
            metadata: Map::new(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(!d.path().join("project/hello.txt").exists());
}
