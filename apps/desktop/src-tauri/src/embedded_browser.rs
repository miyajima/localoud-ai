//! User-operated browser with a completion-metadata observer for local usage counts.
use serde::Deserialize;
use tauri::{Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl};

const LABEL: &str = "chatgpt-browser";
// Stable, app-specific WKWebsiteDataStore; never imports another browser's cookies.
const DATA_STORE: [u8; 16] = *b"LocaloudChatGPT1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    visible: bool,
    viewport_height: f64,
}

fn valid_bounds(b: &BrowserBounds) -> bool {
    [b.x, b.y, b.width, b.height, b.viewport_height].iter().all(|n| n.is_finite())
        && b.x >= 0.0
        && b.y >= 0.0
        && b.width >= 1.0
        && b.height >= 1.0
        && b.viewport_height >= 1.0
        && b.width <= 20000.0
        && b.height <= 20000.0
}

#[tauri::command]
pub async fn browser_layout(app: tauri::AppHandle, mut bounds: BrowserBounds) -> Result<(), String> {
    if !valid_bounds(&bounds) {
        return Err("invalid browser bounds".into());
    }
    // macOS includes the title-bar inset in the native parent height, but not
    // in the main WKWebView's DOM viewport. Measure that difference instead of
    // hard-coding title-bar pixels (which change in fullscreen).
    #[cfg(target_os = "macos")]
    {
        let window = app.get_window("main").ok_or("main window missing")?;
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let height = window.inner_size().map_err(|e| e.to_string())?.to_logical::<f64>(scale).height;
        bounds.y += (height - bounds.viewport_height).max(0.0);
    }
    let view = match app.get_webview(LABEL) {
        Some(view) => view,
        None if !bounds.visible => return Ok(()),
        None => {
            let popup_app = app.clone();
            let usage_app = app.clone();
            let builder = tauri::webview::WebviewBuilder::new(
                LABEL,
                WebviewUrl::External("https://chatgpt.com/".parse().map_err(|e| format!("{e}"))?),
            )
            .data_store_identifier(DATA_STORE)
            .initialization_script(include_str!("chatgpt-usage-observer.js"))
            .on_navigation(move |url| {
                if url.scheme()=="localoud-usage" {
                    if let Some(event)=usage_event(url) { let _=usage_app.emit_to("main","chatgpt-turn-completed",event); }
                    return false;
                }
                url.scheme()=="https"
            })
            .on_new_window(move |url, features| {
                if url.scheme() != "https" && url.as_str() != "about:blank" {
                    return tauri::webview::NewWindowResponse::Deny;
                }
                let label = format!("chatgpt-popup-{}", hub_core::TaskId::default());
                let result =
                    tauri::WebviewWindowBuilder::new(&popup_app, label, WebviewUrl::External(url))
                        .title("ChatGPT — Browser")
                        .data_store_identifier(DATA_STORE)
                        .window_features(features)
                        .on_navigation(|url| {
                            url.scheme() == "https" || url.as_str() == "about:blank"
                        })
                        .build();
                match result {
                    Ok(window) => tauri::webview::NewWindowResponse::Create { window },
                    Err(_) => tauri::webview::NewWindowResponse::Deny,
                }
            });
            app.get_window("main")
                .ok_or("main window missing")?
                .add_child(
                    builder,
                    LogicalPosition::new(bounds.x, bounds.y),
                    LogicalSize::new(bounds.width, bounds.height),
                )
                .map_err(|e| e.to_string())?
        }
    };
    if bounds.visible {
        view.set_bounds(tauri::Rect {
            position: LogicalPosition::new(bounds.x, bounds.y).into(),
            size: LogicalSize::new(bounds.width, bounds.height).into(),
        })
        .map_err(|e| e.to_string())?;
        view.show().map_err(|e| e.to_string())
    } else {
        view.hide().map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn browser_reload(app: tauri::AppHandle) -> Result<(), String> {
    app.get_webview(LABEL)
        .ok_or("ブラウザを開いてください")?
        .reload()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_bounds_are_finite_and_positive() {
        let mut b = BrowserBounds {
            x: 0.0,
            y: 48.0,
            width: 500.0,
            height: 700.0,
            visible: true,
            viewport_height: 850.0,
        };
        assert!(valid_bounds(&b));
        b.width = f64::NAN;
        assert!(!valid_bounds(&b));
    }
}

#[derive(Clone, serde::Serialize)]
struct UsageEvent { id: String, model: String, duration: u64 }
fn usage_event(url: &tauri::Url) -> Option<UsageEvent> {
    if url.host_str()!=Some("completed") { return None; }
    let fields: std::collections::HashMap<_,_>=url.query_pairs().collect();
    let id=fields.get("id")?.to_string();
    let model=fields.get("model")?.to_string();
    let duration=fields.get("duration")?.parse::<u64>().ok()?;
    if id.is_empty() || id.len()>128 || !id.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-'||b==b'_') || !matches!(model.as_str(),"astra"|"sol"|"unknown") || duration>86_400_000 { return None; }
    Some(UsageEvent{id,model,duration})
}
