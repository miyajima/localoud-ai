use provider_codex::CodexProvider;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let binary = std::env::var("CODEX_BIN").unwrap_or_else(|_| "codex".into());
    let provider = CodexProvider::spawn(&binary).await?;
    println!("initialize/initialized completed; no model turn dispatched");
    provider.shutdown().await
}
