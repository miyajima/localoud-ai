use anyhow::{anyhow, Result};
use hub_db::Store;
use protocol_types::AgentEvent;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEvent {
    pub sequence: i64,
    pub event: AgentEvent,
}
#[derive(Clone)]
pub struct EventBus {
    store: Arc<Mutex<Store>>,
    sender: broadcast::Sender<JournalEvent>,
}
impl EventBus {
    pub fn new(store: Arc<Mutex<Store>>) -> Self {
        let (sender, _) = broadcast::channel(4096);
        Self { store, sender }
    }
    pub fn subscribe(&self) -> broadcast::Receiver<JournalEvent> {
        self.sender.subscribe()
    }
    /// Durably record before broadcasting. Delta text is only streamed; final items
    /// are journaled whole so split secret strings never leak across stored chunks.
    pub fn publish(&self, mut event: AgentEvent) -> Result<()> {
        let mut safe = serde_json::to_value(&event)?;
        hub_policy::redact_json(&mut safe);
        event = serde_json::from_value(safe)?;
        let mut stored = event.clone();
        if matches!(stored.kind.as_str(), "message_delta" | "tool_output") {
            stored.text = "[delta omitted from journal; see completed item]".into();
        }
        let mut db = self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?;
        let sequence = db.append_event(
            event.thread_id.as_deref(),
            &event.kind,
            &serde_json::to_string(&stored)?,
        )?;
        if let Some(thread) = &event.thread_id {
            if event.kind == "turn_completed" || event.kind == "turn_started" {
                db.update_provider_status(thread, &event.text, event.turn_id.as_deref())?;
            }
        }
        if let (Some(thread), Some(turn), Some(details)) =
            (&event.thread_id, &event.turn_id, &event.details)
        {
            db.record_codex_usage(thread, turn, details)?;
        }
        drop(db);
        let _ = self.sender.send(JournalEvent { sequence, event });
        Ok(())
    }
}
