use crate::AppState;
use provider_spark::{LocalServiceConfig, SparkProvider};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    Ok(config)
}
#[derive(Serialize)]
pub struct Choice {
    pub key: String,
    pub label: String,
    pub model: String,
    pub local: bool,
    pub reasoning: Vec<String>,
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
            local: true,
            reasoning: vec![],
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
    Ok(result)
}
#[tauri::command]
pub fn thread_models(state: tauri::State<AppState>) -> Result<BTreeMap<String, String>, String> {
    let s = state.store.lock().map_err(|e| e.to_string())?;
    let mut models = BTreeMap::new();
    for t in s.threads().map_err(|e| e.to_string())? {
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
            .setting(&format!("thread_reasoning:{}", thread.id))
            .map_err(|e| e.to_string())?
        {
            result.insert(thread.id.to_string(), effort);
        }
    }
    Ok(result)
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
    }
}
