use crate::AppState;
use protocol_types::CodingAgentProvider;
use provider_spark::{LocalServiceConfig, SparkProvider};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub fn ensure_builtin_profiles(
    store: &hub_db::Store,
    local: &LocalServiceConfig,
) -> Result<(), String> {
    use protocol_types::providers::{ProviderLocality, ProviderProfile, ProviderProtocol};
    let desired_protocol = match local.protocol {
        provider_spark::LocalProtocol::Dedicated => ProviderProtocol::LocalDedicated,
        provider_spark::LocalProtocol::OpenAiChat => ProviderProtocol::OpenAiChat,
    };
    let current = store.provider_profile("spark").map_err(|e| e.to_string())?;
    let desired = ProviderProfile {
        id: "spark".into(),
        name: local.display_name.clone(),
        protocol: desired_protocol,
        base_url: Some(local.endpoint.clone()),
        locality: ProviderLocality::Local,
        credential_env: None,
        max_concurrency: 1,
        enabled: true,
        revision: current.as_ref().map_or(1, |profile| profile.revision + 1),
    };
    let unchanged = current.as_ref().is_some_and(|profile| {
        profile.name == desired.name
            && profile.protocol == desired.protocol
            && profile.base_url == desired.base_url
            && profile.locality == desired.locality
            && profile.credential_env == desired.credential_env
            && profile.max_concurrency == desired.max_concurrency
            && profile.enabled == desired.enabled
    });
    if !unchanged {
        store
            .save_provider_profile(&desired)
            .map_err(|e| e.to_string())?;
    }
    migrate_legacy_session_targets(store, local)
}

fn migrate_legacy_session_targets(
    store: &hub_db::Store,
    local: &LocalServiceConfig,
) -> Result<(), String> {
    use protocol_types::providers::{ModelTarget, SessionModelPolicy};
    for thread in store.threads().map_err(|error| error.to_string())? {
        if !matches!(thread.provider.as_str(), "codex" | "spark") {
            continue;
        }
        let profile_id = thread.provider.clone();
        let model = store
            .setting(&format!("thread_model:{}", thread.id))
            .map_err(|error| error.to_string())?
            .or_else(|| (profile_id == "spark").then(|| local.model_id.clone()));
        let Some(model) = model else {
            continue;
        };
        let target = store
            .session_model_policy(thread.id)
            .map_err(|error| error.to_string())?
            .map(|policy| policy.default_target)
            .unwrap_or(ModelTarget {
                profile_id: profile_id.clone(),
                model_id: model,
                effort: store
                    .setting(&format!("thread_reasoning:{}", thread.id))
                    .map_err(|error| error.to_string())?,
            });
        if store
            .session_model_policy(thread.id)
            .map_err(|error| error.to_string())?
            .is_none()
        {
            store
                .set_session_model_policy(
                    thread.id,
                    &SessionModelPolicy {
                        default_target: target.clone(),
                        reviewer_target: None,
                        allow_turn_override: profile_id == "codex",
                    },
                )
                .map_err(|error| error.to_string())?;
        }
        let active = store
            .active_provider_segment(thread.id)
            .map_err(|error| error.to_string())?;
        if active
            .as_ref()
            .is_none_or(|segment| segment.target.model_id.is_empty())
        {
            let revision = store
                .provider_profile(&profile_id)
                .map_err(|error| error.to_string())?
                .map_or(1, |profile| profile.revision);
            store
                .start_provider_segment(&hub_db::ProviderSegmentRecord {
                    id: hub_core::TaskId::default().to_string(),
                    thread_id: thread.id,
                    target,
                    profile_revision: revision,
                    provider_thread_id: Some(thread.provider_thread_id),
                    ended: false,
                })
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[derive(Serialize)]
pub struct ProviderProfileStatus {
    #[serde(flatten)]
    pub profile: protocol_types::providers::ProviderProfile,
    pub credential_present: bool,
}

#[tauri::command]
pub fn provider_profiles(
    state: tauri::State<AppState>,
) -> Result<Vec<ProviderProfileStatus>, String> {
    Ok(state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .provider_profiles()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|profile| ProviderProfileStatus {
            credential_present: profile.credential_present(),
            profile,
        })
        .collect())
}

#[tauri::command]
pub async fn set_provider_profile(
    profile: protocol_types::providers::ProviderProfile,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<protocol_types::providers::ProviderModel>, String> {
    if matches!(profile.id.as_str(), "codex" | "spark") {
        return Err("組み込み接続は専用の設定欄から変更してください。".into());
    }
    profile.validate().map_err(|e| e.to_string())?;
    let expected_revision = state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .provider_profile(&profile.id)
        .map_err(|e| e.to_string())?
        .map_or(1, |saved| saved.revision.saturating_add(1));
    if profile.revision != expected_revision {
        return Err(
            "provider profileが別の操作で更新されました。設定を再読み込みしてください。".into(),
        );
    }
    let models = if profile.enabled {
        let transport =
            provider_api::ApiTransport::new(profile.clone()).map_err(|e| e.to_string())?;
        provider_api::InferenceTransport::models(&transport)
            .await
            .map_err(|e| format!("接続確認に失敗しました。保存していません: {e:#}"))?
    } else {
        Vec::new()
    };
    {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let current_revision = store
            .provider_profile(&profile.id)
            .map_err(|e| e.to_string())?
            .map_or(0, |saved| saved.revision);
        if current_revision.saturating_add(1) != profile.revision {
            return Err(
                "接続確認中にprovider profileが更新されました。保存せず再読み込みします。".into(),
            );
        }
        store
            .save_provider_profile(&profile)
            .map_err(|e| e.to_string())?;
    }
    state
        .providers
        .write()
        .await
        .insert(profile)
        .map_err(|e| e.to_string())?;
    Ok(models)
}

fn requires_tool_canary(profile: &protocol_types::providers::ProviderProfile) -> bool {
    use protocol_types::providers::ProviderProtocol;
    matches!(
        profile.protocol,
        ProviderProtocol::OpenAiChat | ProviderProtocol::OpenAiResponses
    ) && profile
        .base_url
        .as_deref()
        .is_none_or(|url| url.trim_end_matches('/') != "https://api.openai.com/v1")
}

fn tool_canary_key(profile_id: &str, revision: u64, model_id: &str) -> String {
    format!("provider_tool_canary:{profile_id}:{revision}:{model_id}")
}

fn tool_canary_passed(
    store: &hub_db::Store,
    profile: &protocol_types::providers::ProviderProfile,
    model_id: &str,
) -> Result<bool, String> {
    Ok(!requires_tool_canary(profile)
        || store
            .setting(&tool_canary_key(&profile.id, profile.revision, model_id))
            .map_err(|e| e.to_string())?
            .as_deref()
            == Some("pass"))
}

#[derive(Serialize)]
pub struct ToolCanaryResult {
    pub supported: bool,
    pub profile_id: String,
    pub model_id: String,
}

/// Explicit, potentially billable capability probe. Profile save itself remains GET-only.
#[tauri::command]
pub async fn run_provider_tool_canary(
    profile_id: String,
    model_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<ToolCanaryResult, String> {
    use protocol_types::providers::{
        ContentBlock, InferenceRequest, InferenceTool, ModelTarget, TranscriptMessage,
        TranscriptRole,
    };
    let profile = state
        .providers
        .read()
        .await
        .profile(&profile_id)
        .cloned()
        .ok_or("provider profileがありません。")?;
    if !requires_tool_canary(&profile) {
        return Err("この接続はprotocolからtool機能を判定できます。canaryは不要です。".into());
    }
    let transport = state
        .providers
        .read()
        .await
        .transport(&profile_id)
        .map_err(|e| e.to_string())?;
    if !transport
        .models()
        .await
        .map_err(|e| format!("モデル一覧を確認できません: {e:#}"))?
        .iter()
        .any(|model| model.id == model_id)
    {
        return Err("指定モデルは現在のproviderモデル一覧にありません。".into());
    }
    let response = transport
        .infer(InferenceRequest {
            target: ModelTarget {
                profile_id: profile_id.clone(),
                model_id: model_id.clone(),
                effort: None,
            },
            system: Some("Capability probe. Call the supplied tool exactly once and emit no prose.".into()),
            messages: vec![TranscriptMessage {
                role: TranscriptRole::User,
                content: vec![ContentBlock::Text {
                    text: "Call localoud_capability_probe with an empty object.".into(),
                }],
                provider_state: None,
            }],
            tools: vec![InferenceTool {
                name: "localoud_capability_probe".into(),
                description: "Confirm native function/tool calling support.".into(),
                input_schema: serde_json::json!({"type":"object","properties":{},"additionalProperties":false}),
            }],
            max_output_tokens: 64,
        })
        .await
        .map_err(|e| format!("tool canaryに失敗しました。能力は保存していません: {e:#}"))?;
    let supported = response.content.iter().any(|block| {
        matches!(block, ContentBlock::ToolUse { name, .. } if name == "localoud_capability_probe")
    });
    if !supported {
        return Err(
            "モデルが指定したtool callを返しませんでした。フルworkerには使用しません。".into(),
        );
    }
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .set_setting(
            &tool_canary_key(&profile.id, profile.revision, &model_id),
            "pass",
        )
        .map_err(|e| e.to_string())?;
    Ok(ToolCanaryResult {
        supported,
        profile_id,
        model_id,
    })
}

pub fn read_local(store: &hub_db::Store) -> Result<LocalServiceConfig, String> {
    store
        .setting("local_model_settings")
        .map_err(|e| e.to_string())?
        .map(|v| serde_json::from_str(&v).map_err(|e| e.to_string()))
        .unwrap_or_else(|| Ok(LocalServiceConfig::default()))
}
#[tauri::command]
pub fn local_model_settings(state: tauri::State<AppState>) -> Result<LocalServiceConfig, String> {
    read_local(&*state.store.lock().map_err(|e| e.to_string())?)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalSettingsInput {
    pub endpoint: String,
    pub model_id: String,
    pub display_name: String,
}
#[tauri::command]
pub async fn set_local_model_settings(
    config: LocalSettingsInput,
    state: tauri::State<'_, AppState>,
) -> Result<LocalServiceConfig, String> {
    // Discover loaded quantization, retaining strict identity checks at runtime.
    let config =
        SparkProvider::discover_config(config.endpoint, config.model_id, config.display_name)
            .await
            .map_err(|e| format!("接続確認に失敗しました。保存していません: {e:#}"))?;
    state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .set_setting(
            "local_model_settings",
            &serde_json::to_string(&config).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    ensure_builtin_profiles(&*state.store.lock().map_err(|e| e.to_string())?, &config)?;
    Ok(config)
}
#[derive(Serialize)]
pub struct Choice {
    pub key: String,
    pub label: String,
    pub model: String,
    pub profile_id: String,
    pub protocol: protocol_types::providers::ProviderProtocol,
    pub local: bool,
    pub reasoning: Vec<String>,
    pub tools: bool,
    pub default_reasoning: Option<String>,
    pub is_default: bool,
}
#[derive(Serialize)]
pub struct Catalog {
    pub models: Vec<Choice>,
    pub warnings: Vec<String>,
}
#[tauri::command]
pub async fn available_models(state: tauri::State<'_, AppState>) -> Result<Catalog, String> {
    catalog(&state).await
}
pub async fn catalog(state: &AppState) -> Result<Catalog, String> {
    let mut result = Catalog {
        models: vec![],
        warnings: vec![],
    };
    match state.local.provider.available().await {
        Ok(()) => result.models.push(Choice {
            key: "spark".into(),
            label: state.local_config.display_name.clone(),
            model: state.local_config.model_id.clone(),
            profile_id: "spark".into(),
            protocol: match state.local_config.protocol {
                provider_spark::LocalProtocol::Dedicated => {
                    protocol_types::providers::ProviderProtocol::LocalDedicated
                }
                provider_spark::LocalProtocol::OpenAiChat => {
                    protocol_types::providers::ProviderProtocol::OpenAiChat
                }
            },
            local: true,
            reasoning: vec![],
            tools: false,
            default_reasoning: None,
            is_default: false,
        }),
        Err(e) => result.warnings.push(format!("ローカルモデル: {e:#}")),
    }
    match state.sessions().await {
        Ok(_) => {
            let provider = state
                .connection
                .lock()
                .await
                .as_ref()
                .ok_or("接続がありません")?
                .provider
                .clone();
            match provider.model_catalog().await {
                Ok(models) => {
                    for m in models {
                        if m["hidden"].as_bool() == Some(true) {
                            continue;
                        }
                        if let Some(id) = m["model"].as_str().or(m["id"].as_str()) {
                            result.models.push(Choice {
                                key: format!("codex:{id}"),
                                label: m["displayName"].as_str().unwrap_or(id).into(),
                                model: id.into(),
                                profile_id: "codex".into(),
                                protocol:
                                    protocol_types::providers::ProviderProtocol::CodexAppServer,
                                local: false,
                                reasoning: m["supportedReasoningEfforts"]
                                    .as_array()
                                    .map(|v| {
                                        v.iter()
                                            .filter_map(|e| {
                                                e["reasoningEffort"].as_str().map(str::to_owned)
                                            })
                                            .collect()
                                    })
                                    .unwrap_or_default(),
                                tools: true,
                                default_reasoning: m["defaultReasoningEffort"]
                                    .as_str()
                                    .map(str::to_owned),
                                is_default: m["isDefault"].as_bool() == Some(true),
                            });
                        }
                    }
                }
                Err(e) => result.warnings.push(format!("モデル一覧: {e:#}")),
            }
        }
        Err(e) => result.warnings.push(format!("Codex 接続: {e}")),
    }
    let profiles = state.providers.read().await.profiles();
    for profile in profiles.into_iter().filter(|profile| {
        profile.enabled
            && !matches!(profile.id.as_str(), "codex" | "spark")
            && !matches!(
                profile.protocol,
                protocol_types::providers::ProviderProtocol::CodexAppServer
                    | protocol_types::providers::ProviderProtocol::LocalDedicated
            )
    }) {
        let transport = match state.providers.read().await.transport(&profile.id) {
            Ok(transport) => transport,
            Err(e) => {
                result.warnings.push(format!("{}: {e:#}", profile.name));
                continue;
            }
        };
        match transport.models().await {
            Ok(models) => {
                for model in models {
                    let tools = if requires_tool_canary(&profile) {
                        tool_canary_passed(
                            &*state.store.lock().map_err(|e| e.to_string())?,
                            &profile,
                            &model.id,
                        )?
                    } else {
                        model.capabilities.tools
                    };
                    result.models.push(Choice {
                        key: format!("api:{}:{}", profile.id, model.id),
                        label: format!("{} / {}", profile.name, model.display_name),
                        model: model.id,
                        profile_id: profile.id.clone(),
                        protocol: profile.protocol,
                        local: profile.locality
                            == protocol_types::providers::ProviderLocality::Local,
                        reasoning: model.capabilities.efforts,
                        tools,
                        default_reasoning: None,
                        is_default: false,
                    });
                }
            }
            Err(e) => result.warnings.push(format!("{}: {e:#}", profile.name)),
        }
    }
    Ok(result)
}
#[tauri::command]
pub fn thread_models(state: tauri::State<AppState>) -> Result<BTreeMap<String, String>, String> {
    let s = state.store.lock().map_err(|e| e.to_string())?;
    let mut models = BTreeMap::new();
    for t in s.threads().map_err(|e| e.to_string())? {
        if let Some(policy) = s.session_model_policy(t.id).map_err(|e| e.to_string())? {
            models.insert(t.id.to_string(), policy.default_target.model_id);
            continue;
        }
        if let Some(model) = s
            .setting(&format!("thread_model:{}", t.id))
            .map_err(|e| e.to_string())?
        {
            models.insert(t.id.to_string(), model);
        }
    }
    Ok(models)
}
#[tauri::command]
pub fn thread_reasoning(state: tauri::State<AppState>) -> Result<BTreeMap<String, String>, String> {
    let s = state.store.lock().map_err(|e| e.to_string())?;
    let mut result = BTreeMap::new();
    for thread in s.threads().map_err(|e| e.to_string())? {
        if let Some(effort) = s
            .session_model_policy(thread.id)
            .map_err(|e| e.to_string())?
            .and_then(|policy| policy.default_target.effort)
        {
            result.insert(thread.id.to_string(), effort);
            continue;
        }
        if let Some(effort) = s
            .setting(&format!("thread_reasoning:{}", thread.id))
            .map_err(|e| e.to_string())?
        {
            result.insert(thread.id.to_string(), effort);
        }
    }
    Ok(result)
}

#[tauri::command]
pub fn thread_model_targets(
    state: tauri::State<AppState>,
) -> Result<BTreeMap<String, hub_router::automatic::ModelTarget>, String> {
    use hub_router::automatic::{ModelProvider, ModelTarget};
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let mut result = BTreeMap::new();
    for thread in store.threads().map_err(|e| e.to_string())? {
        if let Some(policy) = store
            .session_model_policy(thread.id)
            .map_err(|e| e.to_string())?
        {
            result.insert(
                thread.id.to_string(),
                ModelTarget {
                    provider: match policy.default_target.profile_id.as_str() {
                        "spark" => ModelProvider::Local,
                        "codex" => ModelProvider::Codex,
                        _ => ModelProvider::Api,
                    },
                    profile_id: Some(policy.default_target.profile_id),
                    model: policy.default_target.model_id,
                    reasoning: policy.default_target.effort,
                },
            );
            continue;
        }
        if let Some(model) = store
            .setting(&format!("thread_model:{}", thread.id))
            .map_err(|e| e.to_string())?
        {
            let (provider, profile_id) = if thread.provider == "spark" {
                (ModelProvider::Local, "spark")
            } else {
                (ModelProvider::Codex, "codex")
            };
            result.insert(
                thread.id.to_string(),
                ModelTarget {
                    provider,
                    profile_id: Some(profile_id.into()),
                    model,
                    reasoning: store
                        .setting(&format!("thread_reasoning:{}", thread.id))
                        .map_err(|e| e.to_string())?,
                },
            );
        }
    }
    Ok(result)
}

#[tauri::command]
pub async fn set_thread_model_target(
    thread_id: String,
    target: hub_router::automatic::ModelTarget,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    use hub_router::automatic::ModelProvider;
    use protocol_types::providers::{
        ContentBlock, ModelTarget as ApiTarget, SessionModelPolicy, TranscriptRole,
    };
    target.validate().map_err(|e| e.to_string())?;
    validate_target(&target, &state).await?;
    let id = crate::parse_thread(thread_id)?;
    let mapping = state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .threads()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|thread| thread.id == id)
        .ok_or("タスクが見つかりません。")?;
    if matches!(
        mapping.status.as_str(),
        "running" | "inProgress" | "dispatching"
    ) {
        return Err("実行中ターンのモデルは変更できません。".into());
    }
    if mapping.provider == "spark" {
        return Err(
            "bounded localタスクはモデルを変更できません。新しいタスクを作成してください。".into(),
        );
    }
    let api_target = ApiTarget {
        profile_id: target
            .profile_id
            .clone()
            .ok_or("provider profileがありません。")?,
        model_id: target.model.clone(),
        effort: target.reasoning.clone(),
    };
    let current_is_api = mapping.provider.starts_with("api:");
    match (current_is_api, target.provider) {
        (true, ModelProvider::Api) => {
            state
                .api_sessions
                .set_target(id, api_target.clone())
                .await
                .map_err(|e| e.to_string())?;
            state
                .store
                .lock()
                .map_err(|e| e.to_string())?
                .switch_thread_provider(
                    id,
                    &format!("api:{}", api_target.profile_id),
                    &mapping.provider_thread_id,
                )
                .map_err(|e| e.to_string())?;
        }
        (false, ModelProvider::Codex) => {
            let store = state.store.lock().map_err(|e| e.to_string())?;
            store
                .set_setting(&format!("thread_model:{id}"), &target.model)
                .map_err(|e| e.to_string())?;
            if let Some(effort) = &target.reasoning {
                store
                    .set_setting(&format!("thread_reasoning:{id}"), effort)
                    .map_err(|e| e.to_string())?;
            } else {
                store
                    .remove_setting(&format!("thread_reasoning:{id}"))
                    .map_err(|e| e.to_string())?;
            }
            let reviewer_target = store
                .session_model_policy(id)
                .map_err(|e| e.to_string())?
                .and_then(|policy| policy.reviewer_target);
            store
                .set_session_model_policy(
                    id,
                    &SessionModelPolicy {
                        default_target: api_target,
                        reviewer_target,
                        allow_turn_override: true,
                    },
                )
                .map_err(|e| e.to_string())?;
        }
        (false, ModelProvider::Api) => {
            let snapshot = state
                .sessions()
                .await?
                .resume(id)
                .await
                .map_err(|e| format!("{e:#}"))?;
            if snapshot.active_turn.is_some() {
                return Err("実行中ターンのモデルは変更できません。".into());
            }
            let store = state.store.lock().map_err(|e| e.to_string())?;
            if store
                .provider_messages(id)
                .map_err(|e| e.to_string())?
                .is_empty()
            {
                for message in snapshot.messages {
                    store
                        .append_provider_message(&hub_db::ProviderMessageRecord {
                            id: hub_core::TaskId::default().to_string(),
                            thread_id: id,
                            segment_id: None,
                            provider_turn_id: None,
                            role: if message.role == "assistant" {
                                TranscriptRole::Assistant
                            } else {
                                TranscriptRole::User
                            },
                            content: vec![ContentBlock::Text { text: message.text }],
                            provider_state: None,
                        })
                        .map_err(|e| e.to_string())?;
                }
            }
            let reviewer_target = store
                .session_model_policy(id)
                .map_err(|e| e.to_string())?
                .and_then(|policy| policy.reviewer_target);
            store
                .set_session_model_policy(
                    id,
                    &SessionModelPolicy {
                        default_target: api_target.clone(),
                        reviewer_target,
                        allow_turn_override: true,
                    },
                )
                .map_err(|e| e.to_string())?;
            store
                .switch_thread_provider(
                    id,
                    &format!("api:{}", api_target.profile_id),
                    &format!("api-session:{id}"),
                )
                .map_err(|e| e.to_string())?;
        }
        (true, ModelProvider::Codex) => {
            let snapshot = state
                .api_sessions
                .read(id)
                .await
                .map_err(|e| e.to_string())?;
            if snapshot.active_turn.is_some() {
                return Err("実行中ターンのモデルは変更できません。".into());
            }
            let root = state
                .store
                .lock()
                .map_err(|e| e.to_string())?
                .thread_root(id)
                .map_err(|e| e.to_string())?;
            let provider = codex_provider(&state).await?;
            let remote = provider
                .start_thread_with_model(root, &target.model)
                .await
                .map_err(|e| format!("{e:#}"))?;
            let replay = serde_json::to_string(
                &state
                    .api_sessions
                    .normalized_transcript(id)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            let store = state.store.lock().map_err(|e| e.to_string())?;
            store
                .switch_thread_provider(id, "codex", &remote.id)
                .map_err(|e| e.to_string())?;
            store
                .set_setting(&format!("codex_context_replay:{id}"), &replay)
                .map_err(|e| e.to_string())?;
            store
                .set_setting(&format!("thread_model:{id}"), &target.model)
                .map_err(|e| e.to_string())?;
            if let Some(effort) = &target.reasoning {
                store
                    .set_setting(&format!("thread_reasoning:{id}"), effort)
                    .map_err(|e| e.to_string())?;
            }
            let reviewer_target = store
                .session_model_policy(id)
                .map_err(|e| e.to_string())?
                .and_then(|policy| policy.reviewer_target);
            store
                .set_session_model_policy(
                    id,
                    &SessionModelPolicy {
                        default_target: api_target.clone(),
                        reviewer_target,
                        allow_turn_override: true,
                    },
                )
                .map_err(|e| e.to_string())?;
            store
                .start_provider_segment(&hub_db::ProviderSegmentRecord {
                    id: hub_core::TaskId::default().to_string(),
                    thread_id: id,
                    target: api_target,
                    profile_revision: 1,
                    provider_thread_id: Some(remote.id),
                    ended: false,
                })
                .map_err(|e| e.to_string())?;
        }
        (_, ModelProvider::Local) => {
            return Err("既存セッションをbounded localモデルへ切り替えることはできません。".into())
        }
    }
    Ok(())
}
pub async fn codex_provider(
    state: &AppState,
) -> Result<std::sync::Arc<provider_codex::CodexProvider>, String> {
    state.sessions().await?;
    Ok(state
        .connection
        .lock()
        .await
        .as_ref()
        .ok_or("接続がありません")?
        .provider
        .clone())
}
pub async fn validate_target(
    target: &hub_router::automatic::ModelTarget,
    state: &AppState,
) -> Result<(), String> {
    use hub_router::automatic::ModelProvider;
    target.validate().map_err(|e| e.to_string())?;
    match target.provider {
        ModelProvider::Local => {
            if target.model != state.local_config.model_id {
                return Err("設定したローカルモデルと、現在のモデルが一致しません。Auto 設定を更新してください。".into());
            }
            state
                .local
                .provider
                .available()
                .await
                .map_err(|e| format!("{e:#}"))
        }
        ModelProvider::Codex => codex_provider(state)
            .await?
            .validate_model_reasoning(&target.model, target.reasoning.as_deref())
            .await
            .map_err(|e| format!("{e:#}")),
        ModelProvider::Api => {
            let profile_id = target
                .profile_id
                .as_deref()
                .ok_or("APIモデルのprovider profileがありません。")?;
            let transport = state
                .providers
                .read()
                .await
                .transport(profile_id)
                .map_err(|e| e.to_string())?;
            let models = transport.models().await.map_err(|e| format!("{e:#}"))?;
            let model = models
                .into_iter()
                .find(|model| model.id == target.model)
                .ok_or_else(|| format!("{} はproviderのモデル一覧にありません。", target.model))?;
            if target.reasoning.as_ref().is_some_and(|effort| {
                !model
                    .capabilities
                    .efforts
                    .iter()
                    .any(|value| value == effort)
            }) {
                return Err(format!(
                    "{} は指定したreasoning/effortに対応していません。",
                    target.model
                ));
            }
            Ok(())
        }
    }
}

pub async fn validate_worker_target(
    target: &hub_router::automatic::ModelTarget,
    state: &AppState,
) -> Result<(), String> {
    validate_target(target, state).await?;
    if target.provider != hub_router::automatic::ModelProvider::Api {
        return Ok(());
    }
    let profile_id = target
        .profile_id
        .as_deref()
        .ok_or("APIモデルのprovider profileがありません。")?;
    let profile = state
        .providers
        .read()
        .await
        .profile(profile_id)
        .cloned()
        .ok_or("API provider profileがありません。")?;
    if tool_canary_passed(
        &*state.store.lock().map_err(|e| e.to_string())?,
        &profile,
        &target.model,
    )? {
        Ok(())
    } else {
        Err("この汎用OpenAI互換モデルはtool canary未合格です。接続設定から明示的に確認してください。".into())
    }
}

pub async fn api_infer(
    target: &hub_router::automatic::ModelTarget,
    system: Option<String>,
    prompt: String,
    tools: Vec<protocol_types::providers::InferenceTool>,
    max_output_tokens: u32,
    state: &AppState,
) -> Result<protocol_types::providers::InferenceResponse, String> {
    use protocol_types::providers::{
        ContentBlock, InferenceRequest, ModelTarget as ApiTarget, TranscriptMessage, TranscriptRole,
    };
    if target.provider != hub_router::automatic::ModelProvider::Api {
        return Err("API provider targetが必要です。".into());
    }
    let profile_id = target
        .profile_id
        .clone()
        .ok_or("APIモデルのprovider profileがありません。")?;
    let request = InferenceRequest {
        target: ApiTarget {
            profile_id,
            model_id: target.model.clone(),
            effort: target.reasoning.clone(),
        },
        system,
        messages: vec![TranscriptMessage {
            role: TranscriptRole::User,
            content: vec![ContentBlock::Text { text: prompt }],
            provider_state: None,
        }],
        tools,
        max_output_tokens,
    };
    let transport = state
        .providers
        .read()
        .await
        .transport(&request.target.profile_id)
        .map_err(|e| e.to_string())?;
    transport.infer(request).await.map_err(|e| format!("{e:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol_types::providers::{ProviderLocality, ProviderProfile, ProviderProtocol};

    fn generic_profile(revision: u64) -> ProviderProfile {
        ProviderProfile {
            id: "generic".into(),
            name: "Generic".into(),
            protocol: ProviderProtocol::OpenAiChat,
            base_url: Some("http://127.0.0.1:8765/v1".into()),
            locality: ProviderLocality::Local,
            credential_env: None,
            max_concurrency: 1,
            enabled: true,
            revision,
        }
    }

    #[test]
    fn generic_openai_tool_canary_is_bound_to_profile_revision_and_model() {
        let directory = tempfile::tempdir().unwrap();
        let store = hub_db::Store::open(&directory.path().join("state.db")).unwrap();
        let profile = generic_profile(3);
        assert!(requires_tool_canary(&profile));
        assert!(!tool_canary_passed(&store, &profile, "model-a").unwrap());
        store
            .set_setting(&tool_canary_key("generic", 3, "model-a"), "pass")
            .unwrap();
        assert!(tool_canary_passed(&store, &profile, "model-a").unwrap());
        assert!(!tool_canary_passed(&store, &profile, "model-b").unwrap());
        assert!(!tool_canary_passed(&store, &generic_profile(4), "model-a").unwrap());

        let mut official = profile;
        official.base_url = Some("https://api.openai.com/v1/".into());
        official.locality = ProviderLocality::Cloud;
        official.credential_env = Some("OPENAI_API_KEY".into());
        assert!(!requires_tool_canary(&official));
        assert!(tool_canary_passed(&store, &official, "model-a").unwrap());
    }

    #[test]
    fn builtin_profiles_migrate_legacy_codex_and_spark_session_targets_once() {
        let directory = tempfile::tempdir().unwrap();
        assert!(std::process::Command::new("git")
            .arg("init")
            .arg(directory.path())
            .output()
            .unwrap()
            .status
            .success());
        let store = hub_db::Store::open(&directory.path().join("state.db")).unwrap();
        let project = store.register_project(directory.path()).unwrap().id;
        let codex = hub_core::ThreadMapping {
            id: hub_core::HubThreadId::default(),
            project_id: project,
            provider: "codex".into(),
            provider_thread_id: "legacy-codex".into(),
            title: "Codex".into(),
            status: "idle".into(),
        };
        let spark = hub_core::ThreadMapping {
            id: hub_core::HubThreadId::default(),
            project_id: project,
            provider: "spark".into(),
            provider_thread_id: "legacy-spark".into(),
            title: "Spark".into(),
            status: "idle".into(),
        };
        store.save_thread(&codex).unwrap();
        store.save_thread(&spark).unwrap();
        store
            .set_setting(&format!("thread_model:{}", codex.id), "codex-model")
            .unwrap();
        store
            .set_setting(&format!("thread_reasoning:{}", codex.id), "high")
            .unwrap();
        let local = LocalServiceConfig::default();

        ensure_builtin_profiles(&store, &local).unwrap();
        let codex_policy = store.session_model_policy(codex.id).unwrap().unwrap();
        assert_eq!(codex_policy.default_target.model_id, "codex-model");
        assert_eq!(codex_policy.default_target.effort.as_deref(), Some("high"));
        assert_eq!(
            store
                .session_model_policy(spark.id)
                .unwrap()
                .unwrap()
                .default_target
                .model_id,
            local.model_id
        );
        let spark_segment = store.active_provider_segment(spark.id).unwrap().unwrap();
        ensure_builtin_profiles(&store, &local).unwrap();
        assert_eq!(
            store.active_provider_segment(spark.id).unwrap().unwrap().id,
            spark_segment.id
        );
    }
}
