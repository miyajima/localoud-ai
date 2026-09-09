//! Read-only history of the retired extension transport. No browser or network API.
use crate::AppState;
use hub_core::HubThreadId;
use protocol_types::{Message, ProviderThread, ThreadSnapshot};

pub fn is_thread(state: &AppState, id: HubThreadId) -> Result<bool, String> {
    Ok(state
        .store
        .lock()
        .map_err(|e| e.to_string())?
        .threads()
        .map_err(|e| e.to_string())?
        .iter()
        .any(|t| t.id == id && t.provider == "chatgpt"))
}
pub fn read(state: &AppState, id: HubThreadId) -> Result<ThreadSnapshot, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let thread = store
        .threads()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|t| t.id == id && t.provider == "chatgpt")
        .ok_or("legacy session missing")?;
    let messages = store
        .setting(&format!("chatgpt_messages:{id}"))
        .map_err(|e| e.to_string())?
        .map(|v| serde_json::from_str::<Vec<Message>>(&v))
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    Ok(ThreadSnapshot {
        thread: ProviderThread {
            id: thread.provider_thread_id,
        },
        active_turn: None,
        messages,
    })
}
pub fn retire(store: &hub_db::Store) -> Result<(), String> {
    store
        .remove_setting("chatgpt_bridge_token")
        .map_err(|e| e.to_string())?;
    for mut t in store.threads().map_err(|e| e.to_string())? {
        let old_autonomous = if t.provider == "autonomous" {
            store
                .setting(&format!("autonomous:{}", t.id))
                .map_err(|e| e.to_string())?
                .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok())
                .is_some_and(|v| v.get("manifest").is_none_or(|m| m.is_null()))
        } else {
            false
        };
        if t.provider == "chatgpt" || t.provider == "workflow" || old_autonomous {
            t.status = "legacy_read_only".into();
            store.save_thread(&t).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
