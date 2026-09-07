use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use hub_core::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

#[async_trait]
pub trait DurableMemoryProvider: Send + Sync {
    fn persistence_enabled(&self) -> bool {
        true
    }
    async fn search(&self, req: MemorySearchRequest) -> Result<Vec<MemoryItem>>;
    async fn store(&self, items: Vec<DurableMemoryItem>) -> Result<()>;
}
/// Local classification gate. A model cannot create new evidence references.
pub fn durable_items(
    project: ProjectId,
    task: TaskId,
    extraction: MemoryExtraction,
    approved_refs: &[String],
) -> Result<Vec<DurableMemoryItem>> {
    if extraction.candidates.len() > 8 {
        bail!("too many memory candidates");
    }
    let allowed: HashSet<_> = approved_refs.iter().collect();
    let mut result = vec![];
    for (index, c) in extraction.candidates.into_iter().enumerate() {
        if !c.durable {
            continue;
        }
        if c.statement.trim().is_empty()
            || c.reason.trim().is_empty()
            || c.statement.len() > 6000
            || c.reason.len() > 6000
            || c.source_refs.is_empty()
            || c.source_refs.iter().any(|r| !allowed.contains(r))
        {
            bail!("durable memory lacks bounded statement, rationale or known provenance");
        }
        let text = serde_json::to_string(&c)?;
        if hub_policy::redact(&text) != text {
            bail!("memory contains secret-like text");
        }
        result.push(DurableMemoryItem {
            external_key: format!("astra-hub:{task}:{index}"),
            project_id: project,
            task_id: task,
            candidate: c,
        });
    }
    Ok(result)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrgBrainConfig {
    pub endpoint: String,
    pub tenant: String,
    pub remote_project_id: String,
    pub auto_store: bool,
}
pub struct OrgBrain {
    client: reqwest::Client,
    config: OrgBrainConfig,
    client_id: String,
    client_secret: String,
}
impl OrgBrain {
    pub fn new(config: OrgBrainConfig, client_id: String, client_secret: String) -> Result<Self> {
        let url = reqwest::Url::parse(&config.endpoint)?;
        let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
        if (url.scheme() != "https" && !(url.scheme() == "http" && local))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!(
                "OrgBrain requires HTTPS or a loopback fixture endpoint, without URL credentials"
            );
        }
        if config.tenant.is_empty()
            || config.remote_project_id.is_empty()
            || client_id.is_empty()
            || client_secret.is_empty()
        {
            bail!("OrgBrain requires tenant, mapped project ID and CF Access credentials");
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(12))
                .build()?,
            config,
            client_id,
            client_secret,
        })
    }
    async fn tool(&self, name: &str, args: Value) -> Result<Value> {
        let mut response=self.client.post(&self.config.endpoint).header("CF-Access-Client-Id",&self.client_id).header("CF-Access-Client-Secret",&self.client_secret).header("x-orgbrain-tenant",&self.config.tenant).header("MCP-Protocol-Version","2026-07-28").header("Accept","application/json, text/event-stream").json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}})).send().await?.error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 1024 * 1024 {
                bail!("OrgBrain response exceeds 1 MB");
            }
            bytes.extend_from_slice(&chunk);
        }
        let wire = String::from_utf8(bytes)?;
        let value: Value = if wire.trim_start().starts_with('{') {
            serde_json::from_str(&wire)?
        } else {
            let data = wire
                .lines()
                .filter_map(|l| l.strip_prefix("data:"))
                .collect::<Vec<_>>()
                .join("\n");
            serde_json::from_str(data.trim()).context("invalid MCP event response")?
        };
        if value.get("error").is_some() {
            bail!("OrgBrain MCP returned an error; no automatic retry");
        }
        let result = value.get("result").context("missing MCP result")?;
        if result["isError"] == true {
            bail!("OrgBrain tool failed; no automatic retry");
        }
        if let Some(v) = result.get("structuredContent") {
            return Ok(v.clone());
        }
        let texts = result["content"]
            .as_array()
            .context("missing MCP content")?
            .iter()
            .filter_map(|c| c["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        serde_json::from_str(&texts).context("OrgBrain tool did not return JSON content")
    }
}
#[async_trait]
impl DurableMemoryProvider for OrgBrain {
    fn persistence_enabled(&self) -> bool {
        self.config.auto_store
    }
    async fn search(&self, req: MemorySearchRequest) -> Result<Vec<MemoryItem>> {
        if req.query.trim().is_empty() || req.query.len() > 500 || !(1..=50).contains(&req.limit) {
            bail!("invalid memory search request");
        }
        let v=self.tool("orgbrain_memories_search",json!({"tenant_id":self.config.tenant,"project_id":self.config.remote_project_id,"q":req.query,"limit":req.limit,"rewrite_query":false,"search_mode":"memories"})).await?;
        let mut items = v["results"]
            .as_array()
            .context("missing memory results")?
            .iter()
            .map(|r| {
                let id = r["id"].as_str().context("missing memory ID")?.to_owned();
                let text = r["content_preview"]
                    .as_str()
                    .or(r["content"].as_str())
                    .or(r["summary"].as_str())
                    .context("missing memory content")?;
                Ok(MemoryItem {
                    id: id.clone(),
                    text: hub_policy::redact(text),
                    source_refs: vec![format!("orgbrain:{id}")],
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let decisions=self.tool("orgbrain_decision_memories_search",json!({"tenant_id":self.config.tenant,"project_id":self.config.remote_project_id,"q":req.query,"limit":req.limit})).await?;
        for r in decisions["results"]
            .as_array()
            .context("missing decision results")?
        {
            let id = r["id"].as_str().context("missing decision ID")?.to_owned();
            if !items.iter().any(|i| i.id == id) {
                items.push(MemoryItem {
                    id: id.clone(),
                    text: hub_policy::redact(&serde_json::to_string(r)?),
                    source_refs: vec![format!("orgbrain-decision:{id}")],
                });
            }
        }
        items.truncate(req.limit as usize);
        Ok(items)
    }
    async fn store(&self, items: Vec<DurableMemoryItem>) -> Result<()> {
        if !self.config.auto_store {
            bail!("OrgBrain automatic persistence is disabled");
        }
        if items.is_empty() {
            return Ok(());
        }
        if items.len() > 8 || items.iter().any(|i| !i.candidate.durable) {
            bail!("only bounded durable candidates can be persisted");
        }
        let mut groups: std::collections::HashMap<TaskId, Vec<DurableMemoryItem>> =
            std::collections::HashMap::new();
        for item in items {
            groups.entry(item.task_id).or_default().push(item);
        }
        let mut payload = vec![];
        for (task, items) in groups {
            let content = serde_json::to_string(&json!({"task_id":task,"candidates":items}))?;
            if hub_policy::redact(&content) != content {
                bail!("memory contains secret-like content");
            }
            let summary = items
                .iter()
                .map(|i| i.candidate.statement.as_str())
                .collect::<Vec<_>>()
                .join("; ")
                .chars()
                .take(900)
                .collect::<String>();
            payload.push(json!({"external_key":format!("astra-hub:{task}:candidates"),"content":content,"summary":summary,"tags":["astra-hub","durable-candidates"],"project_id":self.config.remote_project_id}));
        }
        self.tool(
            "orgbrain_memories_upsert",
            json!({"tenant_id":self.config.tenant,"source":"astra-hub","items":payload}),
        )
        .await?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ephemeral_discarded_and_unknown_provenance_rejected() {
        let mut c = MemoryCandidate {
            kind: MemoryKind::NextAction,
            durable: false,
            statement: "temporary progress".into(),
            reason: "ephemeral".into(),
            source_refs: vec![],
        };
        assert!(durable_items(
            ProjectId::default(),
            TaskId::default(),
            MemoryExtraction {
                candidates: vec![c.clone()]
            },
            &[]
        )
        .unwrap()
        .is_empty());
        c.durable = true;
        c.source_refs = vec!["invented".into()];
        assert!(durable_items(
            ProjectId::default(),
            TaskId::default(),
            MemoryExtraction {
                candidates: vec![c]
            },
            &["known".into()]
        )
        .is_err());
    }
}
