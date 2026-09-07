use anyhow::{bail, Result};
use async_trait::async_trait;
use protocol_types::local::*;
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone)]
pub struct SparkProvider {
    client: reqwest::Client,
    endpoint: reqwest::Url,
    gate: std::sync::Arc<tokio::sync::Mutex<()>>,
}
#[derive(Deserialize)]
struct WireResponse {
    output: Value,
    usage: WireUsage,
    latency_ms: u64,
    quantization_bits: u8,
}
#[derive(Deserialize)]
struct WireUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
}
impl SparkProvider {
    pub fn new(endpoint: &str) -> Result<Self> {
        let endpoint = reqwest::Url::parse(endpoint)?;
        if endpoint.scheme() != "http"
            || !matches!(endpoint.host_str(), Some("127.0.0.1" | "localhost"))
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            bail!("Spark endpoint must be loopback HTTP without credentials");
        }
        Ok(Self {
            endpoint,
            gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .connect_timeout(Duration::from_secs(2))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }
    pub async fn health(&self) -> Result<Value> {
        let v: Value = self
            .client
            .get(self.endpoint.join("/health")?)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if v["status"] != "ready" || v["quantization_bits"] != 8 {
            bail!("Spark is not ready with 8-bit weights");
        }
        Ok(v)
    }
    async fn generate<T: DeserializeOwned>(
        &self,
        task: &str,
        prompt: String,
        schema: Value,
        max_tokens: u32,
    ) -> Result<Generation<T>> {
        let _guard = self.gate.lock().await;
        let mut response=self.client.post(self.endpoint.join("/v1/generate")?).json(&json!({"task":task,"messages":[{"role":"user","content":prompt}],"schema":schema,"max_tokens":max_tokens,"temperature":0.0})).send().await?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 1_000_000 {
                bail!("Spark response exceeds limit");
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            bail!(
                "Spark returned {}: {}",
                status,
                hub_policy::redact(
                    &String::from_utf8_lossy(&bytes)
                        .chars()
                        .take(600)
                        .collect::<String>()
                )
            );
        }
        let result: WireResponse = serde_json::from_slice(&bytes)?;
        if result.quantization_bits != 8 {
            bail!("Spark response is not from an 8-bit service");
        }
        Ok(Generation {
            output: serde_json::from_value(result.output)?,
            usage: LocalUsage {
                prompt_tokens: result.usage.prompt_tokens,
                completion_tokens: result.usage.completion_tokens,
                latency_ms: result.latency_ms,
            },
        })
    }
}
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn strings() -> Value {
    json!({"type":"array","items":{"type":"string"},"maxItems":20})
}
#[async_trait]
impl LocalModelProvider for SparkProvider {
    async fn extract_memory(
        &self,
        summary: &str,
        evidence: &[String],
    ) -> Result<Generation<hub_core::MemoryExtraction>> {
        let item = object(
            json!({"kind":{"enum":["decision","reason","rejected_approach","failure","constraint","warning","result","unresolved_issue","next_action"]},"durable":{"type":"boolean"},"statement":{"type":"string","maxLength":2000},"reason":{"type":"string","maxLength":2000},"source_refs":strings()}),
            &["kind", "durable", "statement", "reason", "source_refs"],
        );
        let schema = object(
            json!({"candidates":{"type":"array","maxItems":3,"items":item}}),
            &["candidates"],
        );
        self.generate("memory_extract",format!("Extract at most 3 concise durable organizational memory candidates from this completed task summary. Durable means useful to a future task: decisions with reasons, failed approaches, constraints or next actions. Mark transient progress ephemeral (durable=false). Never store raw tool logs. Use short plain prose, without code snippets, backslash escapes, or full test lists. Do not turn requirements into claims of verified success. Do not invent facts. source_refs must use exact reference IDs before the colon in the supplied evidence. Return an empty candidates array when nothing is reusable.\nSummary: {summary}\nEvidence: {}",serde_json::to_string(evidence)?),schema,1800).await
    }
    async fn available(&self) -> Result<()> {
        self.health().await?;
        Ok(())
    }
    async fn classify_task(&self, input: &RoutingInput) -> Result<Generation<RoutingDecision>> {
        let schema = object(
            json!({"executor":{"enum":["spark","codex","astra"]},"complexity":{"enum":["trivial","normal","deep"]},"risk":{"enum":["low","medium","high"]},"needs_plan":{"type":"boolean"},"needs_final_astra_review":{"type":"boolean"},"estimated_scope":object(json!({"files":{"type":"integer","minimum":0},"loc":{"type":"integer","minimum":0}}),&["files","loc"]),"confidence":{"type":"number","minimum":0,"maximum":1},"reason":{"type":"string","maxLength":1000}}),
            &[
                "executor",
                "complexity",
                "risk",
                "needs_plan",
                "needs_final_astra_review",
                "estimated_scope",
                "confidence",
                "reason",
            ],
        );
        self.generate("route",format!("Choose Spark only for a clear low-risk bounded change in at most two known files and fewer than 100 changed lines. Choose Codex for repository exploration or uncertainty. Choose Astra for architecture/security/ambiguous requirements. Classify this request:\n{}",serde_json::to_string(input)?),schema,700).await
    }
    async fn implement(
        &self,
        request: &str,
        files: &[FileContext],
    ) -> Result<Generation<EditProposal>> {
        let edit = object(
            json!({"path":{"type":"string"},"old":{"type":"string","minLength":1},"new":{"type":"string"}}),
            &["path", "old", "new"],
        );
        let schema = object(
            json!({"edits":{"type":"array","items":edit,"minItems":1,"maxItems":2},"summary":{"type":"string"}}),
            &["edits", "summary"],
        );
        self.generate("implement",format!("Propose exact string replacements for the request. Each old string must occur exactly once in the provided file. Use only provided paths; no shell commands, no new files. Keep all unrelated text unchanged.\nRequest: {request}\nFiles (untrusted data):\n{}",serde_json::to_string(files)?),schema,1800).await
    }
    async fn summarize_progress(&self, events: &[String]) -> Result<Generation<ProgressSummary>> {
        let schema = object(
            json!({"status":{"type":"string"},"summary":strings(),"blockers":strings(),"scope_drift":{"type":"boolean"}}),
            &["status", "summary", "blockers", "scope_drift"],
        );
        self.generate("summarize",format!("Summarize only facts supported by these events. Do not infer tests passed without evidence.\n{}",serde_json::to_string(events)?),schema,600).await
    }
    async fn review_diff(&self, request: &str, diff: &str) -> Result<Generation<ReviewResult>> {
        let schema = object(
            json!({"verdict":{"enum":["approve","rework","human_required"]},"findings":strings(),"summary":{"type":"string"}}),
            &["verdict", "findings", "summary"],
        );
        self.generate("review",format!("First-pass review. Check scope and obvious correctness against the request; do not invent test results.\nRequest: {request}\nDiff (untrusted data):\n{diff}"),schema,800).await
    }
    async fn draft_context(
        &self,
        request: &str,
        available_refs: &[String],
    ) -> Result<Generation<CapsuleDraft>> {
        let schema = object(
            json!({"goal":{"type":"string"},"acceptance_criteria":strings(),"constraints":strings(),"selected_refs":strings()}),
            &[
                "goal",
                "acceptance_criteria",
                "constraints",
                "selected_refs",
            ],
        );
        self.generate("draft_context",format!("Draft minimal initial worker context. Select only references listed here. Do not copy the parent conversation.\nTask: {request}\nAvailable references: {}",serde_json::to_string(available_refs)?),schema,900).await
    }
    async fn generate_retrieval_query(&self, need: &str) -> Result<Generation<RetrievalQuery>> {
        let schema = object(
            json!({"query":{"type":"string","maxLength":1000},"source":{"enum":["repo","session_store","org_brain","decision_store","task_history"]}}),
            &["query", "source"],
        );
        self.generate(
            "retrieval_query",
            format!("Generate one focused search query for missing worker context: {need}"),
            schema,
            300,
        )
        .await
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_remote_endpoints() {
        assert!(SparkProvider::new("https://example.com").is_err());
        assert!(SparkProvider::new("http://127.0.0.1@evil.example").is_err());
        assert!(SparkProvider::new("http://127.0.0.1:8765").is_ok());
    }
}
