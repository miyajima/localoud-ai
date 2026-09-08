use crate::AppState;
use provider_spark::{LocalServiceConfig, SparkProvider};
use serde::Serialize;
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
#[tauri::command]
pub async fn set_local_model_settings(
    config: LocalServiceConfig,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    // Validate the actual loaded identity before saving; changes take effect on restart.
    SparkProvider::configured(config.clone())
        .map_err(|e| e.to_string())?
        .health()
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
        .map_err(|e| e.to_string())
}
#[derive(Serialize)]
pub struct Choice {
    key: String,
    label: String,
    model: String,
    local: bool,
}
#[derive(Serialize)]
pub struct Catalog {
    models: Vec<Choice>,
    warnings: Vec<String>,
}
#[tauri::command]
pub async fn available_models(state: tauri::State<'_, AppState>) -> Result<Catalog, String> {
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
