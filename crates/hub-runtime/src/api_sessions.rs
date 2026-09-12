use crate::api_worker::{ApiAgent, CommandApprovalRequest, CommandAuthorizer};
use anyhow::{anyhow, bail, Context, Result};
use hub_core::{HubThreadId, ModelUsageRecord, ProjectId, TaskId, ThreadMapping};
use hub_db::{
    ProviderMessageRecord, ProviderSegmentRecord, ProviderTurnOutcomeRecord, Store,
    ToolApprovalRecord, TurnTargetRecord,
};
use hub_events::EventBus;
use protocol_types::{
    composer::{CollaborationMode, TurnOptions},
    providers::{
        ContentBlock, ModelTarget, SessionModelPolicy, TargetResolution, TranscriptMessage,
        TranscriptRole,
    },
    AgentEvent, EventDetails, Message, ProviderThread, ProviderTurn, ThreadSnapshot,
};
use provider_api::ProviderRegistry;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex},
};
use tokio::{
    sync::{Mutex, RwLock},
    task::JoinHandle,
};

pub struct CommandApprovals {
    store: Arc<StdMutex<Store>>,
    bus: EventBus,
    waiters: Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
}

impl CommandApprovals {
    pub fn new(store: Arc<StdMutex<Store>>, bus: EventBus) -> Self {
        Self {
            store,
            bus,
            waiters: Mutex::new(HashMap::new()),
        }
    }

    pub async fn decide(&self, approval_id: &str, approve: bool) -> Result<()> {
        let sender = self
            .waiters
            .lock()
            .await
            .remove(approval_id)
            .context("approval is not attached to a live waiting turn")?;
        self.store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .decide_tool_approval(approval_id, approve)?;
        sender
            .send(approve)
            .map_err(|_| anyhow!("approval turn stopped before the decision was delivered"))
    }
}

#[async_trait::async_trait]
impl CommandAuthorizer for CommandApprovals {
    async fn authorize(&self, request: CommandApprovalRequest) -> Result<()> {
        let thread_id = HubThreadId(request.thread_id.parse()?);
        let approval_id = TaskId::default().to_string();
        let request_json = serde_json::json!({
            "executable": request.executable,
            "argv": request.argv,
            "cwd": request.cwd,
            "additional_permissions": request.additional_permissions,
        });
        let provider_thread_id = {
            let store = self
                .store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?;
            let mut mapping = store
                .threads()?
                .into_iter()
                .find(|mapping| mapping.id == thread_id)
                .context("approval thread is missing")?;
            store.create_tool_approval(&ToolApprovalRecord {
                id: approval_id.clone(),
                thread_id,
                turn_id: request.turn_id.clone(),
                digest: request.digest.clone(),
                request: request_json.clone(),
                status: "pending".into(),
            })?;
            mapping.status = "awaiting_approval".into();
            let provider_thread_id = mapping.provider_thread_id.clone();
            store.save_thread(&mapping)?;
            provider_thread_id
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.waiters
            .lock()
            .await
            .insert(approval_id.clone(), sender);
        let publication = self.bus.publish(AgentEvent {
            thread_id: Some(provider_thread_id.clone()),
            turn_id: Some(request.turn_id.clone()),
            item_id: Some(approval_id.clone()),
            kind: "approval_required".into(),
            text: serde_json::to_string(&request_json)?,
            details: Some(EventDetails::Approval {
                approval_id: approval_id.clone(),
                digest: request.digest.clone(),
                executable: request_json["executable"].as_str().unwrap_or("").into(),
                argv: request_json["argv"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|value| value.as_str().map(Into::into))
                    .collect(),
                cwd: request_json["cwd"].as_str().unwrap_or(".").into(),
                additional_permissions: request_json["additional_permissions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|value| value.as_str().map(Into::into))
                    .collect(),
            }),
        });
        if let Err(error) = publication {
            self.waiters.lock().await.remove(&approval_id);
            if let Ok(store) = self.store.lock() {
                let _ = store.decide_tool_approval(&approval_id, false);
            }
            return Err(error);
        }
        let approved = receiver
            .await
            .map_err(|_| anyhow!("approval decision channel closed"))?;
        if !approved {
            bail!("user denied the one-shot command approval");
        }
        let consumed = self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .consume_tool_approval(&approval_id, thread_id, &request.turn_id, &request.digest)?;
        anyhow::ensure!(
            consumed == request_json,
            "approval request changed before use"
        );
        if let Ok(store) = self.store.lock() {
            if let Ok(Some(mut mapping)) = store
                .threads()
                .map(|threads| threads.into_iter().find(|mapping| mapping.id == thread_id))
            {
                mapping.status = "running".into();
                let _ = store.save_thread(&mapping);
            }
        }
        Ok(())
    }
}

pub struct ApiSessions {
    store: Arc<StdMutex<Store>>,
    bus: EventBus,
    providers: Arc<RwLock<ProviderRegistry>>,
    running: Mutex<HashMap<HubThreadId, JoinHandle<()>>>,
}

impl ApiSessions {
    pub fn new(
        store: Arc<StdMutex<Store>>,
        bus: EventBus,
        providers: Arc<RwLock<ProviderRegistry>>,
    ) -> Self {
        Self {
            store,
            bus,
            providers,
            running: Mutex::new(HashMap::new()),
        }
    }

    fn mapping(&self, id: HubThreadId) -> Result<ThreadMapping> {
        self.store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .threads()?
            .into_iter()
            .find(|mapping| mapping.id == id && mapping.provider.starts_with("api:"))
            .context("unknown API session")
    }

    fn ensure_transcript_migrated(&self, id: HubThreadId) -> Result<()> {
        let store = self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?;
        if !store.provider_messages(id)?.is_empty() {
            return Ok(());
        }
        let key = format!("api_transcript:{id}");
        let Some(body) = store.setting(&key)? else {
            return Ok(());
        };
        let transcript: Vec<TranscriptMessage> = serde_json::from_str(&body)?;
        for message in transcript {
            store.append_provider_message(&ProviderMessageRecord {
                id: TaskId::default().to_string(),
                thread_id: id,
                segment_id: None,
                provider_turn_id: None,
                role: message.role,
                content: message.content,
                provider_state: None,
            })?;
        }
        store.remove_setting(&key)?;
        Ok(())
    }

    fn transcript(&self, id: HubThreadId) -> Result<Vec<TranscriptMessage>> {
        self.ensure_transcript_migrated(id)?;
        Ok(self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .provider_messages(id)?
            .into_iter()
            .map(|message| TranscriptMessage {
                role: message.role,
                content: message.content,
                provider_state: None,
            })
            .collect())
    }

    fn transcript_for_segment(
        &self,
        id: HubThreadId,
        segment_id: &str,
    ) -> Result<Vec<TranscriptMessage>> {
        self.ensure_transcript_migrated(id)?;
        Ok(self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .provider_messages(id)?
            .into_iter()
            .map(|message| TranscriptMessage {
                role: message.role,
                content: message.content,
                provider_state: (message.segment_id.as_deref() == Some(segment_id))
                    .then_some(message.provider_state)
                    .flatten(),
            })
            .collect())
    }

    pub fn normalized_transcript(&self, id: HubThreadId) -> Result<Vec<TranscriptMessage>> {
        self.transcript(id)
    }

    pub async fn create(
        &self,
        project: ProjectId,
        title: String,
        target: ModelTarget,
    ) -> Result<ThreadMapping> {
        target.validate()?;
        let profile = self
            .providers
            .read()
            .await
            .profile(&target.profile_id)
            .cloned()
            .context("API provider profile is unavailable")?;
        anyhow::ensure!(profile.enabled, "API provider profile is disabled");
        let mapping = ThreadMapping {
            id: HubThreadId::default(),
            project_id: project,
            provider: format!("api:{}", profile.id),
            provider_thread_id: format!("api-session:{}", TaskId::default()),
            title,
            status: "idle".into(),
        };
        let store = self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?;
        anyhow::ensure!(
            store.projects()?.iter().any(|item| item.id == project),
            "unregistered project"
        );
        store.save_thread(&mapping)?;
        store.set_session_model_policy(
            mapping.id,
            &SessionModelPolicy {
                default_target: target,
                reviewer_target: None,
                allow_turn_override: true,
            },
        )?;
        Ok(mapping)
    }

    pub async fn set_target(&self, id: HubThreadId, target: ModelTarget) -> Result<()> {
        target.validate()?;
        anyhow::ensure!(
            !self.running.lock().await.contains_key(&id),
            "cannot change model during an active turn"
        );
        let mapping = self.mapping(id)?;
        let profile = self
            .providers
            .read()
            .await
            .profile(&target.profile_id)
            .cloned()
            .context("API provider profile is unavailable")?;
        anyhow::ensure!(profile.enabled, "API provider profile is disabled");
        let mut policy = self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .session_model_policy(mapping.id)?
            .context("API session model policy is missing")?;
        anyhow::ensure!(
            policy.allow_turn_override,
            "this session does not allow model changes"
        );
        policy.default_target = target;
        self.store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .set_session_model_policy(mapping.id, &policy)
    }

    pub async fn read(&self, id: HubThreadId) -> Result<ThreadSnapshot> {
        let mapping = self.mapping(id)?;
        let transcript = self.transcript(id)?;
        let active_turn = if self.running.lock().await.contains_key(&id) {
            self.store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?
                .setting(&format!("api_active_turn:{id}"))?
                .map(|turn_id| ProviderTurn {
                    thread_id: mapping.provider_thread_id.clone(),
                    id: turn_id,
                    status: "inProgress".into(),
                })
        } else {
            None
        };
        Ok(ThreadSnapshot {
            thread: ProviderThread {
                id: mapping.provider_thread_id,
            },
            active_turn,
            messages: visible_messages(&transcript),
        })
    }

    pub async fn resume(&self, id: HubThreadId) -> Result<ThreadSnapshot> {
        let mut mapping = self.mapping(id)?;
        if matches!(
            mapping.status.as_str(),
            "running" | "inProgress" | "dispatching"
        ) && !self.running.lock().await.contains_key(&id)
        {
            mapping.status = "reconciliation_required".into();
            self.store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?
                .save_thread(&mapping)?;
            bail!("API turn outcome is unknown after restart; it was not replayed")
        }
        self.read(id).await
    }

    pub async fn start(
        self: &Arc<Self>,
        id: HubThreadId,
        text: String,
        options: TurnOptions,
    ) -> Result<ProviderTurn> {
        options.validate()?;
        anyhow::ensure!(!text.trim().is_empty(), "task input is empty");
        let mut running = self.running.lock().await;
        anyhow::ensure!(
            !running.contains_key(&id),
            "session already has an active turn"
        );
        let mut mapping = self.mapping(id)?;
        let root = self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .thread_root(id)?;
        let policy = self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .session_model_policy(id)?
            .context("API session model policy is missing")?;
        let target = policy.default_target;
        let (profile, transport) = {
            let registry = self.providers.read().await;
            let profile = registry
                .profile(&target.profile_id)
                .cloned()
                .context("API provider profile is unavailable")?;
            (profile, registry.transport(&target.profile_id)?)
        };
        let turn_id = self
            .store
            .lock()
            .map_err(|_| anyhow!("database lock poisoned"))?
            .record_turn_intent(id)?;
        let segment = {
            let store = self
                .store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?;
            match store.active_provider_segment(id)? {
                Some(segment)
                    if segment.target == target && segment.profile_revision == profile.revision =>
                {
                    segment
                }
                _ => {
                    let segment = ProviderSegmentRecord {
                        id: TaskId::default().to_string(),
                        thread_id: id,
                        target: target.clone(),
                        profile_revision: profile.revision,
                        provider_thread_id: None,
                        ended: false,
                    };
                    store.start_provider_segment(&segment)?;
                    segment
                }
            }
        };
        let user_message = TranscriptMessage {
            role: TranscriptRole::User,
            content: vec![ContentBlock::Text { text }],
            provider_state: None,
        };
        let mut transcript = self.transcript_for_segment(id, &segment.id)?;
        transcript.push(user_message.clone());
        {
            let store = self
                .store
                .lock()
                .map_err(|_| anyhow!("database lock poisoned"))?;
            store.append_provider_message(&ProviderMessageRecord {
                id: TaskId::default().to_string(),
                thread_id: id,
                segment_id: Some(segment.id.clone()),
                provider_turn_id: Some(turn_id.clone()),
                role: user_message.role,
                content: user_message.content,
                provider_state: None,
            })?;
            store.set_setting(&format!("api_active_turn:{id}"), &turn_id)?;
            store.record_turn_target(&TurnTargetRecord {
                turn_id: turn_id.clone(),
                thread_id: id,
                segment_id: segment.id.clone(),
                target: target.clone(),
                profile_revision: profile.revision,
                resolved_from: TargetResolution::SessionOverride,
            })?;
            store.resolve_turn_intent(&turn_id, Some(&turn_id), "inProgress")?;
            mapping.status = "running".into();
            store.save_thread(&mapping)?;
        }
        let turn = ProviderTurn {
            thread_id: mapping.provider_thread_id.clone(),
            id: turn_id.clone(),
            status: "inProgress".into(),
        };
        self.bus.publish(AgentEvent {
            details: None,
            thread_id: Some(mapping.provider_thread_id.clone()),
            turn_id: Some(turn_id.clone()),
            item_id: None,
            kind: "turn_started".into(),
            text: "inProgress".into(),
        })?;
        let manager = self.clone();
        let provider_thread_id = mapping.provider_thread_id;
        let segment_id = segment.id;
        let mode = options.mode.unwrap_or(CollaborationMode::Default);
        let (release, start) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _ = start.await;
            let system = match mode {
                CollaborationMode::Plan => "You are a read-only planning assistant. Inspect the workspace with list, read, and search tools. Do not modify files or run commands. Produce a concrete implementation plan and identify verification conditions.",
                CollaborationMode::Default => "You are a read-only workspace assistant in this session. Inspect files with list, read, and search tools when needed. Do not modify files or run commands; implementation runs in a dedicated worktree worker.",
            }.to_owned();
            let result = ApiAgent {
                root,
                logical_thread_id: id.to_string(),
                logical_turn_id: turn_id.clone(),
                target: target.clone(),
                transport,
                external_tools: Vec::new(),
                external_handler: None,
                command_authorizer: None,
                allow_writes: false,
                allow_commands: false,
            }
            .run_with_history(system, transcript)
            .await;
            manager
                .finish(
                    id,
                    &provider_thread_id,
                    &turn_id,
                    &segment_id,
                    &target,
                    result,
                )
                .await;
        });
        running.insert(id, handle);
        let _ = release.send(());
        Ok(turn)
    }

    async fn finish(
        &self,
        id: HubThreadId,
        provider_thread_id: &str,
        turn_id: &str,
        segment_id: &str,
        target: &ModelTarget,
        result: Result<crate::api_worker::ApiAgentResult>,
    ) {
        match result {
            Ok(result) => {
                let refused = result.refused;
                let failed = result.failure_kind.is_some();
                let terminal_status = if failed { "failed" } else { "completed" };
                if let Ok(store) = self.store.lock() {
                    let persisted = store.provider_messages(id).map(|messages| messages.len());
                    if let Ok(persisted) = persisted {
                        for message in result.transcript.into_iter().skip(persisted) {
                            let _ = store.append_provider_message(&ProviderMessageRecord {
                                id: TaskId::default().to_string(),
                                thread_id: id,
                                segment_id: Some(segment_id.into()),
                                provider_turn_id: Some(turn_id.into()),
                                role: message.role,
                                content: message.content,
                                provider_state: message.provider_state,
                            });
                        }
                    }
                    let _ = store.remove_setting(&format!("api_active_turn:{id}"));
                    let _ = store.record_usage(&ModelUsageRecord {
                        id: TaskId::default().to_string(),
                        provider: target.profile_id.clone(),
                        model: target.model_id.clone(),
                        task_id: None,
                        turn_id: Some(turn_id.into()),
                        prompt_tokens: Some(result.usage.input_tokens),
                        cached_tokens: Some(result.usage.cached_input_tokens),
                        completion_tokens: Some(result.usage.output_tokens),
                        estimated_cost: None,
                        latency_ms: None,
                    });
                    let _ = store.record_provider_turn_outcome(&ProviderTurnOutcomeRecord {
                        turn_id: turn_id.into(),
                        thread_id: id,
                        status: if refused {
                            "refused"
                        } else if failed {
                            "failed"
                        } else {
                            "completed"
                        }
                        .into(),
                        stop_reason: Some(result.stop_reason.clone()),
                        failure_kind: result.failure_kind.map(|kind| kind.as_str().to_owned()),
                        detail: result
                            .failure
                            .clone()
                            .or_else(|| refused.then(|| result.text.chars().take(2_000).collect())),
                    });
                    let _ = store.resolve_turn_intent(turn_id, Some(turn_id), terminal_status);
                    if let Ok(Some(mut mapping)) = store
                        .threads()
                        .map(|threads| threads.into_iter().find(|mapping| mapping.id == id))
                    {
                        mapping.status = terminal_status.into();
                        let _ = store.save_thread(&mapping);
                    }
                }
                let _ = self.bus.publish(AgentEvent {
                    details: Some(EventDetails::Usage {
                        model: Some(target.model_id.clone()),
                        last_input_tokens: result.usage.input_tokens,
                        last_cached_tokens: result.usage.cached_input_tokens,
                        last_output_tokens: result.usage.output_tokens,
                        total_input_tokens: result.usage.input_tokens,
                        total_cached_tokens: result.usage.cached_input_tokens,
                        total_output_tokens: result.usage.output_tokens,
                    }),
                    thread_id: Some(provider_thread_id.into()),
                    turn_id: Some(turn_id.into()),
                    item_id: None,
                    kind: "usage".into(),
                    text: "usage".into(),
                });
                let _ = self.bus.publish(AgentEvent {
                    details: None,
                    thread_id: Some(provider_thread_id.into()),
                    turn_id: Some(turn_id.into()),
                    item_id: None,
                    kind: if failed {
                        "error".into()
                    } else {
                        "message_completed".into()
                    },
                    text: if refused {
                        format!(
                            "provider refused the request (stop reason: {}): {}",
                            result.stop_reason, result.text
                        )
                    } else if let Some(failure) = result.failure {
                        failure
                    } else {
                        result.text
                    },
                });
                let _ = self.bus.publish(AgentEvent {
                    details: None,
                    thread_id: Some(provider_thread_id.into()),
                    turn_id: Some(turn_id.into()),
                    item_id: None,
                    kind: "turn_completed".into(),
                    text: terminal_status.into(),
                });
            }
            Err(error) => {
                if let Ok(store) = self.store.lock() {
                    let _ = store.remove_setting(&format!("api_active_turn:{id}"));
                    let _ = store.resolve_turn_intent(turn_id, Some(turn_id), "failed");
                    let _ = store.record_provider_turn_outcome(&ProviderTurnOutcomeRecord {
                        turn_id: turn_id.into(),
                        thread_id: id,
                        status: "failed".into(),
                        stop_reason: None,
                        failure_kind: Some(provider_api::failure_kind(&error).as_str().into()),
                        detail: Some(
                            hub_policy::redact(&format!("{error:#}"))
                                .chars()
                                .take(2_000)
                                .collect(),
                        ),
                    });
                    if let Ok(Some(mut mapping)) = store
                        .threads()
                        .map(|threads| threads.into_iter().find(|mapping| mapping.id == id))
                    {
                        mapping.status = "failed".into();
                        let _ = store.save_thread(&mapping);
                    }
                }
                let _ = self.bus.publish(AgentEvent {
                    details: None,
                    thread_id: Some(provider_thread_id.into()),
                    turn_id: Some(turn_id.into()),
                    item_id: None,
                    kind: "error".into(),
                    text: format!("{error:#}"),
                });
                let _ = self.bus.publish(AgentEvent {
                    details: None,
                    thread_id: Some(provider_thread_id.into()),
                    turn_id: Some(turn_id.into()),
                    item_id: None,
                    kind: "turn_completed".into(),
                    text: "failed".into(),
                });
            }
        }
        self.running.lock().await.remove(&id);
    }

    pub async fn interrupt(&self, id: HubThreadId) -> Result<()> {
        let mapping = self.mapping(id)?;
        let handle = self
            .running
            .lock()
            .await
            .remove(&id)
            .context("no active API turn")?;
        handle.abort();
        if let Ok(store) = self.store.lock() {
            let active_turn = store
                .setting(&format!("api_active_turn:{id}"))
                .ok()
                .flatten();
            let _ = store.remove_setting(&format!("api_active_turn:{id}"));
            if let Some(turn_id) = active_turn {
                let _ = store.resolve_turn_intent(&turn_id, Some(&turn_id), "interrupted");
                let _ = store.record_provider_turn_outcome(&ProviderTurnOutcomeRecord {
                    turn_id,
                    thread_id: id,
                    status: "interrupted".into(),
                    stop_reason: None,
                    failure_kind: None,
                    detail: None,
                });
            }
            if let Ok(Some(mut mapping)) = store
                .threads()
                .map(|threads| threads.into_iter().find(|mapping| mapping.id == id))
            {
                mapping.status = "interrupted".into();
                let _ = store.save_thread(&mapping);
            }
        }
        self.bus.publish(AgentEvent {
            details: None,
            thread_id: Some(mapping.provider_thread_id),
            turn_id: None,
            item_id: None,
            kind: "turn_completed".into(),
            text: "interrupted".into(),
        })?;
        Ok(())
    }
}

fn visible_messages(transcript: &[TranscriptMessage]) -> Vec<Message> {
    transcript
        .iter()
        .filter_map(|message| {
            let text = message
                .content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    ContentBlock::Refusal { text, category } => text
                        .as_deref()
                        .or(category.as_deref())
                        .or(Some("Provider refused the request.")),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            (!text.is_empty()).then(|| Message {
                role: match message.role {
                    TranscriptRole::Assistant => "assistant",
                    TranscriptRole::System => "system",
                    _ => "user",
                }
                .into(),
                text,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol_types::providers::InferenceUsage;

    #[tokio::test]
    async fn command_approval_is_durable_and_consumed_once() -> Result<()> {
        let directory = tempfile::tempdir()?;
        anyhow::ensure!(
            std::process::Command::new("git")
                .arg("init")
                .arg(directory.path())
                .output()?
                .status
                .success(),
            "git init failed"
        );
        let store = Arc::new(StdMutex::new(Store::open(
            &directory.path().join("approvals.db"),
        )?));
        let project = store.lock().unwrap().register_project(directory.path())?.id;
        let thread = ThreadMapping {
            id: HubThreadId::default(),
            project_id: project,
            provider: "api:fixture".into(),
            provider_thread_id: "api-fixture".into(),
            title: "fixture".into(),
            status: "running".into(),
        };
        {
            let store = store.lock().unwrap();
            store.save_thread(&thread)?;
        }
        let bus = EventBus::new(store.clone());
        let mut events = bus.subscribe();
        let approvals = Arc::new(CommandApprovals::new(store.clone(), bus));
        let request = CommandApprovalRequest {
            thread_id: thread.id.to_string(),
            turn_id: "turn-1".into(),
            digest: "a".repeat(64),
            executable: "fixture-check".into(),
            argv: vec!["fixture-check".into(), "--strict".into()],
            cwd: ".".into(),
            additional_permissions: vec!["custom executable".into()],
        };
        let running = {
            let approvals = approvals.clone();
            tokio::spawn(async move { approvals.authorize(request).await })
        };
        let event = events.recv().await?;
        assert_eq!(event.event.kind, "approval_required");
        let approval_id = event.event.item_id.context("missing approval ID")?;
        assert_eq!(
            store
                .lock()
                .unwrap()
                .tool_approval(&approval_id)?
                .unwrap()
                .status,
            "pending"
        );
        approvals.decide(&approval_id, true).await?;
        running.await??;
        assert_eq!(
            store
                .lock()
                .unwrap()
                .tool_approval(&approval_id)?
                .unwrap()
                .status,
            "consumed"
        );
        assert!(approvals.decide(&approval_id, true).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn finished_api_turn_persists_terminal_status_for_restart() -> Result<()> {
        let directory = tempfile::tempdir()?;
        anyhow::ensure!(
            std::process::Command::new("git")
                .arg("init")
                .arg(directory.path())
                .output()?
                .status
                .success(),
            "git init failed"
        );
        let store = Arc::new(StdMutex::new(Store::open(
            &directory.path().join("sessions.db"),
        )?));
        let project = store.lock().unwrap().register_project(directory.path())?.id;
        let thread = ThreadMapping {
            id: HubThreadId::default(),
            project_id: project,
            provider: "api:fixture".into(),
            provider_thread_id: "api-fixture".into(),
            title: "fixture".into(),
            status: "running".into(),
        };
        store.lock().unwrap().save_thread(&thread)?;
        let bus = EventBus::new(store.clone());
        let sessions = ApiSessions::new(
            store.clone(),
            bus,
            Arc::new(RwLock::new(ProviderRegistry::default())),
        );
        let target = ModelTarget {
            profile_id: "fixture".into(),
            model_id: "model".into(),
            effort: None,
        };
        sessions
            .finish(
                thread.id,
                &thread.provider_thread_id,
                "turn-ok",
                "segment",
                &target,
                Ok(crate::api_worker::ApiAgentResult {
                    text: "done".into(),
                    transcript: vec![],
                    usage: Default::default(),
                    stop_reason: "end_turn".into(),
                    refused: false,
                    failure_kind: None,
                    failure: None,
                    changed_paths: vec![],
                    tool_activity: vec![],
                }),
            )
            .await;
        assert_eq!(
            store
                .lock()
                .unwrap()
                .threads()?
                .into_iter()
                .find(|saved| saved.id == thread.id)
                .unwrap()
                .status,
            "completed"
        );
        assert_eq!(
            store
                .lock()
                .unwrap()
                .provider_turn_outcome("turn-ok")?
                .unwrap()
                .stop_reason
                .as_deref(),
            Some("end_turn")
        );

        let mut failed = thread.clone();
        failed.id = HubThreadId::default();
        failed.provider_thread_id = "api-failed".into();
        store.lock().unwrap().save_thread(&failed)?;
        sessions
            .finish(
                failed.id,
                &failed.provider_thread_id,
                "turn-failed",
                "segment",
                &target,
                Err(anyhow!("fixture failure")),
            )
            .await;
        assert_eq!(
            store
                .lock()
                .unwrap()
                .threads()?
                .into_iter()
                .find(|saved| saved.id == failed.id)
                .unwrap()
                .status,
            "failed"
        );
        assert_eq!(
            store
                .lock()
                .unwrap()
                .provider_turn_outcome("turn-failed")?
                .unwrap()
                .failure_kind
                .as_deref(),
            Some("unknown")
        );

        let mut refused = thread.clone();
        refused.id = HubThreadId::default();
        refused.provider_thread_id = "api-refused".into();
        store.lock().unwrap().save_thread(&refused)?;
        sessions
            .finish(
                refused.id,
                &refused.provider_thread_id,
                "turn-refused",
                "segment",
                &target,
                Ok(crate::api_worker::ApiAgentResult {
                    text: "policy: cannot comply".into(),
                    transcript: vec![],
                    usage: InferenceUsage {
                        input_tokens: 4,
                        cached_input_tokens: 0,
                        output_tokens: 1,
                    },
                    stop_reason: "refusal".into(),
                    refused: true,
                    failure_kind: Some(provider_api::ProviderFailureKind::Refusal),
                    failure: Some("provider refused the request".into()),
                    changed_paths: vec![],
                    tool_activity: vec![],
                }),
            )
            .await;
        assert_eq!(
            store
                .lock()
                .unwrap()
                .threads()?
                .into_iter()
                .find(|saved| saved.id == refused.id)
                .unwrap()
                .status,
            "failed"
        );
        assert_eq!(
            store
                .lock()
                .unwrap()
                .provider_turn_outcome("turn-refused")?
                .unwrap()
                .status,
            "refused"
        );
        Ok(())
    }
}
