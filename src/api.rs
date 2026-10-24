//! OpenAI-compatible HTTP: model discovery and streaming chat completions.
//!
//! Requests run on a dedicated thread that owns a current-thread tokio
//! runtime, because GPUI runs on smol while reqwest needs a tokio reactor.
//! Results cross back over an async channel, so the UI never blocks.

use futures_lite::StreamExt as _;
use std::error::Error as StdError;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::store::{ApiFormat, ModelInfo, Proxy, ReasoningEffort, Role};

/// Events emitted while a chat completion streams.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// A content delta, in arrival order.
    Delta(String),
    /// A reasoning / thinking delta, in arrival order.
    Thinking(String),
    /// The stream ended normally.
    Done,
    /// The stream ended with a user-visible error.
    Error(String),
}

/// Normalizes a user-entered base URL.
///
/// Trailing slashes are dropped, and a bare host gets `/v1` appended, which is
/// what every OpenAI-compatible vendor expects.
pub fn normalize_base(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/').to_string();
    let after_scheme = trimmed.split_once("://").map_or(trimmed.as_str(), |(_, rest)| rest);
    if !after_scheme.contains('/') {
        format!("{trimmed}/v1")
    } else {
        trimmed
    }
}

/// The conversation in Chat Completions shape.
///
/// System turns are lifted out by [`system_text`] and re-inserted once as the
/// leading message, so they must not appear here a second time.
fn chat_messages(messages: &[(Role, String)]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .filter(|(role, _)| *role != Role::System)
        .map(|(role, content)| {
            serde_json::json!({
                "role": role.wire(),
                "content": content,
            })
        })
        .collect()
}

/// The conversation in Responses shape: an item list with typed messages.
fn responses_input(messages: &[(Role, String)]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .filter(|(role, _)| *role != Role::System)
        .map(|(role, content)| {
            let kind = if *role == Role::Assistant {
                "output_text"
            } else {
                "input_text"
            };
            serde_json::json!({
                "role": role.wire(),
                "content": [{ "type": kind, "text": content }],
            })
        })
        .collect()
}

/// The conversation in Messages shape: system turns are lifted out (they are a
/// top-level field), the rest keep their role and content.
fn anthropic_messages(messages: &[(Role, String)]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .filter(|(role, _)| *role != Role::System)
        .map(|(role, content)| {
            serde_json::json!({
                "role": role.wire(),
                "content": content,
            })
        })
        .collect()
}

fn system_text(system_prompt: &str, messages: &[(Role, String)]) -> String {
    let mut parts = Vec::new();
    let configured = system_prompt.trim();
    if !configured.is_empty() {
        parts.push(configured.to_string());
    }
    parts.extend(
        messages
            .iter()
            .filter(|(role, _)| *role == Role::System)
            .map(|(_, content)| content.clone()),
    );
    parts.join("\n\n")
}

/// The budget field Chat Completions understands for a model.
///
/// OpenAI's reasoning lines (`o1` / `o3` / `o4` / `gpt-5`) reject `max_tokens`
/// and require `max_completion_tokens`; the wider compatible ecosystem still
/// uses `max_tokens`, so only those families are switched.
fn completion_budget_field(model: &str) -> &'static str {
    let id = model.rsplit('/').next().unwrap_or(model).to_ascii_lowercase();
    let reasoning = ["o1", "o3", "o4", "gpt-5"].iter().any(|family| {
        id == *family || id.starts_with(&format!("{family}-")) || id.starts_with(&format!("{family}."))
    });
    if reasoning {
        "max_completion_tokens"
    } else {
        "max_tokens"
    }
}

/// Builds the URL and JSON body of one chat request.
///
/// Kept pure so the three request shapes can be pinned by tests: the fields a
/// vendor accepts (`max_tokens` vs `max_output_tokens`, a `system` message vs
/// the Responses `instructions`, the Anthropic top-level `system`) differ, and
/// a silently dropped system prompt is hard to notice at runtime.
#[allow(clippy::too_many_arguments)]
fn build_request(
    api: ApiFormat,
    base_url: &str,
    model: &str,
    messages: &[(Role, String)],
    max_tokens: u32,
    system_prompt: &str,
    temperature: Option<f64>,
    reasoning: Option<ReasoningEffort>,
) -> (String, serde_json::Value) {
    let base = normalize_base(base_url);
    let system = system_text(system_prompt, messages);
    let (url, mut body) = match api {
        ApiFormat::OpenAiCompletions => {
            let mut conversation = Vec::with_capacity(messages.len() + 1);
            if !system.is_empty() {
                conversation.push(serde_json::json!({
                    "role": Role::System.wire(),
                    "content": system,
                }));
            }
            conversation.extend(chat_messages(messages));
            let mut body = serde_json::json!({
                "model": model,
                "stream": true,
                "messages": conversation,
            });
            body[completion_budget_field(model)] = serde_json::json!(max_tokens);
            if let Some(effort) = reasoning {
                body["reasoning_effort"] = serde_json::json!(effort.wire());
            }
            (format!("{base}/chat/completions"), body)
        }
        ApiFormat::OpenAiResponses => {
            let mut body = serde_json::json!({
                "model": model,
                "stream": true,
                // Responses names the completion budget differently and takes
                // the system prompt as a top-level instruction.
                "max_output_tokens": max_tokens,
                "input": responses_input(messages),
            });
            if !system.is_empty() {
                body["instructions"] = serde_json::Value::String(system);
            }
            if let Some(effort) = reasoning {
                body["reasoning"] = serde_json::json!({ "effort": effort.wire() });
            }
            (format!("{base}/responses"), body)
        }
        ApiFormat::AnthropicMessages => {
            let mut body = serde_json::json!({
                "model": model,
                "stream": true,
                // The Messages API has no `system` role in `messages` and
                // requires a token budget.
                "max_tokens": max_tokens,
                "messages": anthropic_messages(messages),
            });
            if !system.is_empty() {
                body["system"] = serde_json::Value::String(system);
            }
            if let Some(effort) = reasoning {
                // Extended thinking is Anthropic's equivalent of an effort
                // setting; the budget must leave room under max_tokens.
                let budget = effort.thinking_budget();
                body["thinking"] = serde_json::json!({
                    "type": "enabled",
                    "budget_tokens": budget,
                });
                body["max_tokens"] = serde_json::json!(max_tokens.max(budget + 1024));
            }
            (format!("{base}/messages"), body)
        }
    };
    if let Some(temperature) = temperature {
        // The Messages API accepts 0–1 only; the settings field allows 0–2 for
        // OpenAI-compatible endpoints, so clamp per shape rather than fail.
        let temperature = match api {
            ApiFormat::AnthropicMessages => temperature.clamp(0.0, 1.0),
            _ => temperature,
        };
        // Extended thinking requires the provider's default sampling; sending
        // a temperature alongside it is rejected by Claude.
        if !(api == ApiFormat::AnthropicMessages && reasoning.is_some()) {
            body["temperature"] = serde_json::json!(temperature);
        }
    }
    (url, body)
}

/// Everything one streaming completion needs.
pub struct ChatRequest {
    pub base_url: String,
    pub api_key: String,
    pub api: ApiFormat,
    pub model: String,
    pub messages: Vec<(Role, String)>,
    pub max_tokens: u32,
    /// Global prompt, placed ahead of the conversation in each API's own way.
    pub system_prompt: String,
    /// `None` leaves the provider's default sampling temperature.
    pub temperature: Option<f64>,
    /// Requested reasoning effort; `None` leaves it to the model.
    pub reasoning: Option<ReasoningEffort>,
    pub proxy: Proxy,
    pub cancel: Arc<AtomicBool>,
}

/// The text a stream delta carries, per API shape.
fn delta_text(api: ApiFormat, value: &serde_json::Value) -> Option<String> {
    let text = match api {
        ApiFormat::OpenAiCompletions => value
            .pointer("/choices/0/delta/content")
            .and_then(|text| text.as_str()),
        ApiFormat::OpenAiResponses => match value.get("type").and_then(|kind| kind.as_str()) {
            // OpenAI emits one of these per chunk, plus a final `...done`.
            Some("response.output_text.delta") => {
                value.get("delta").and_then(|text| text.as_str())
            }
            _ => None,
        },
        ApiFormat::AnthropicMessages => match value.get("type").and_then(|kind| kind.as_str()) {
            Some("content_block_delta") => value.pointer("/delta/text").and_then(|text| text.as_str()),
            _ => None,
        },
    }?;
    (!text.is_empty()).then(|| text.to_string())
}

/// Reasoning / thinking text a stream delta carries, per API shape.
///
/// DeepSeek and GLM use `reasoning_content` on the Chat Completions delta;
/// Anthropic streams `thinking` inside a `content_block_delta`.
fn thinking_text(api: ApiFormat, value: &serde_json::Value) -> Option<String> {
    let text = match api {
        ApiFormat::OpenAiCompletions => value
            .pointer("/choices/0/delta/reasoning_content")
            .or_else(|| value.pointer("/choices/0/delta/reasoning"))
            .and_then(|text| text.as_str()),
        ApiFormat::OpenAiResponses => match value.get("type").and_then(|kind| kind.as_str()) {
            Some("response.reasoning_summary_text.delta")
            | Some("response.reasoning_text.delta") => {
                value.get("delta").and_then(|text| text.as_str())
            }
            _ => None,
        },
        ApiFormat::AnthropicMessages => match value.get("type").and_then(|kind| kind.as_str()) {
            Some("content_block_delta") => {
                let is_thinking = value
                    .pointer("/delta/type")
                    .and_then(|kind| kind.as_str())
                    == Some("thinking_delta");
                is_thinking
                    .then(|| {
                        value
                            .pointer("/delta/thinking")
                            .and_then(|text| text.as_str())
                    })
                    .flatten()
            }
            _ => None,
        },
    }?;
    (!text.is_empty()).then(|| text.to_string())
}

/// Whether an event closes the stream (beyond the `[DONE]` sentinel).
fn is_final_event(api: ApiFormat, value: &serde_json::Value) -> bool {
    let kind = value.get("type").and_then(|kind| kind.as_str());
    match api {
        ApiFormat::OpenAiCompletions => false,
        ApiFormat::OpenAiResponses => matches!(kind, Some("response.completed") | Some("response.failed")),
        ApiFormat::AnthropicMessages => matches!(kind, Some("message_stop")),
    }
}

/// Runs a fallible request on a worker thread with its own tokio runtime.
fn spawn_runtime<T: Send + 'static>(
    future: impl Future<Output = Result<T, String>> + Send + 'static,
) -> async_channel::Receiver<Result<T, String>> {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(future),
            Err(err) => Err(format!("无法启动网络运行时: {err}")),
        };
        let _ = tx.send_blocking(result);
    });
    rx
}

fn client(proxy: &Proxy, timeout: Option<Duration>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(15));
    builder = match proxy {
        Proxy::System => builder,
        Proxy::None => builder.no_proxy(),
        Proxy::Custom { url } => {
            let url = url.trim();
            if url.is_empty() {
                return Err("自定义代理地址为空".into());
            }
            let proxy = reqwest::Proxy::all(url)
                .map_err(|err| format!("代理地址无效（{url}）: {err}"))?;
            builder.proxy(proxy)
        }
    };
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    builder.build().map_err(|err| format!("无法创建 HTTP 客户端: {err}"))
}

/// Flattens a request error into one line, source chain included.
///
/// reqwest reports "error sending request" for every transport failure, which
/// tells the user nothing; the cause (DNS, TLS, proxy, timeout) is one or more
/// `source()` hops down.
fn describe(err: &reqwest::Error) -> String {
    let mut text = err.to_string();
    let mut cause = StdError::source(err);
    while let Some(error) = cause {
        text.push_str(&format!(" → {error}"));
        cause = error.source();
    }
    text
}

fn truncate(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit).collect();
    format!("{head}…")
}

/// Applies a provider's authentication scheme to a request.
fn authorize(
    request: reqwest::RequestBuilder,
    api: ApiFormat,
    api_key: &str,
) -> reqwest::RequestBuilder {
    if api_key.is_empty() {
        return request;
    }
    match api {
        // Anthropic authenticates with a key header and an API version, and
        // rejects requests that carry a bearer token instead.
        ApiFormat::AnthropicMessages => request
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION),
        _ => request.bearer_auth(api_key),
    }
}

const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Fetches the provider's model catalog from `{base_url}/models`.
///
/// Picks up context / completion limits when the vendor reports them
/// (OpenRouter-style `context_length`, Anthropic `max_output_tokens`, nested
/// `top_provider.*`, and a few common aliases).
pub fn fetch_models(
    base_url: String,
    api_key: String,
    api: ApiFormat,
    proxy: Proxy,
) -> async_channel::Receiver<Result<Vec<ModelInfo>, String>> {
    spawn_runtime(async move {
        let client = client(&proxy, Some(Duration::from_secs(30)))?;
        let url = format!("{}/models", normalize_base(&base_url));
        let request = authorize(client.get(&url), api, &api_key);
        let response = request
            .send()
            .await
            .map_err(|err| format!("请求 {url} 失败: {}", describe(&err)))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| format!("读取响应失败: {err}"))?;
        if !status.is_success() {
            return Err(format!("HTTP {status}: {}", truncate(&body, 300)));
        }
        let value: serde_json::Value = serde_json::from_str(&body)
            .map_err(|err| format!("响应不是合法 JSON: {err}"))?;
        let mut models = Vec::new();
        if let Some(items) = value.get("data").and_then(|data| data.as_array()) {
            for item in items {
                let id = item
                    .as_str()
                    .or_else(|| item.get("id").and_then(|id| id.as_str()));
                let Some(id) = id else { continue };
                models.push(ModelInfo::with_limits(
                    id,
                    first_u32(item, &["max_tokens", "max_output_tokens", "max_completion_tokens"])
                        .or_else(|| {
                            item.pointer("/top_provider/max_completion_tokens")
                                .and_then(|v| v.as_u64())
                                .map(|v| v as u32)
                        }),
                    first_u32(item, &["context_length", "context_window"])
                        .or_else(|| {
                            item.pointer("/top_provider/context_length")
                                .and_then(|v| v.as_u64())
                                .map(|v| v as u32)
                        }),
                ));
            }
        }
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models.dedup_by(|a, b| a.id == b.id);
        if models.is_empty() {
            Err(format!("接口没有返回模型: {}", truncate(&body, 200)))
        } else {
            Ok(models)
        }
    })
}

fn first_u32(value: &serde_json::Value, keys: &[&str]) -> Option<u32> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|v| v.as_u64()))
        .map(|v| v as u32)
}

/// How long a streaming response may stay silent before it is treated as dead.
///
/// Without this a stalled connection leaves the reply "streaming" forever:
/// the request has no total timeout because a long answer is legitimate.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// The `data:` payloads of one SSE event block.
fn sse_data(event: &str) -> Vec<&str> {
    event
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter(|data| !data.is_empty())
        .collect()
}

/// Starts a streaming chat completion. Set `cancel` to stop early.
pub fn stream_chat(request: ChatRequest) -> async_channel::Receiver<StreamEvent> {
    let ChatRequest {
        base_url,
        api_key,
        api,
        model,
        messages,
        max_tokens,
        system_prompt,
        temperature,
        reasoning,
        proxy,
        cancel,
    } = request;
    let (tx, rx) = async_channel::unbounded();
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                let _ = tx.send_blocking(StreamEvent::Error(format!(
                    "无法启动网络运行时: {err}"
                )));
                return;
            }
        };
        runtime.block_on(async move {
            let send = |event: StreamEvent| {
                let tx = tx.clone();
                async move {
                    let _ = tx.send(event).await;
                }
            };

            let client = match client(&proxy, None) {
                Ok(client) => client,
                Err(err) => {
                    send(StreamEvent::Error(err)).await;
                    return;
                }
            };
            let (url, body) = build_request(
                api,
                &base_url,
                &model,
                &messages,
                max_tokens,
                &system_prompt,
                temperature,
                reasoning,
            );
            let request = authorize(client.post(&url).json(&body), api, &api_key);
            // Waiting for response headers is part of "the stream is silent":
            // a server that accepts the socket and never answers must not keep
            // the composer running forever.
            let response = match tokio::time::timeout(STREAM_IDLE_TIMEOUT, request.send()).await {
                Ok(Ok(response)) => response,
                Ok(Err(err)) => {
                    send(StreamEvent::Error(format!(
                        "请求 {url} 失败: {}",
                        describe(&err)
                    )))
                    .await;
                    return;
                }
                Err(_) => {
                    send(StreamEvent::Error(format!(
                        "连接超时：{} 秒内没有收到响应。",
                        STREAM_IDLE_TIMEOUT.as_secs()
                    )))
                    .await;
                    return;
                }
            };
            let status = response.status();
            if !status.is_success() {
                // An error body is small, but a broken server can stall here
                // too; don't let it outlive the same budget.
                let detail = match tokio::time::timeout(Duration::from_secs(10), response.text()).await
                {
                    Ok(Ok(detail)) => detail,
                    _ => String::new(),
                };
                send(StreamEvent::Error(format!(
                    "HTTP {status}: {}",
                    truncate(&detail, 300)
                )))
                .await;
                return;
            }

            let mut stream = Box::pin(response.bytes_stream());
            let mut buffer = String::new();
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let next = match tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next()).await {
                    Ok(next) => next,
                    Err(_) => {
                        send(StreamEvent::Error(format!(
                            "连接超时：{} 秒没有收到新数据。",
                            STREAM_IDLE_TIMEOUT.as_secs()
                        )))
                        .await;
                        return;
                    }
                };
                let Some(chunk) = next else { break };
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(err) => {
                        send(StreamEvent::Error(format!("连接中断: {}", describe(&err)))).await;
                        return;
                    }
                };
                buffer.push_str(&String::from_utf8_lossy(&chunk).replace('\r', ""));

                while let Some(index) = buffer.find("\n\n") {
                    let event: String = buffer.drain(..index + 2).collect();
                    for data in sse_data(&event) {
                        if data == "[DONE]" {
                            send(StreamEvent::Done).await;
                            return;
                        }
                        let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
                            continue;
                        };
                        // Errors are shaped the same way in all three APIs: a
                        // message nested under `error`.
                        if let Some(message) = value
                            .pointer("/error/message")
                            .or_else(|| value.pointer("/error/type"))
                            .and_then(|message| message.as_str())
                        {
                            send(StreamEvent::Error(message.to_string())).await;
                            return;
                        }
                        if let Some(content) = delta_text(api, &value) {
                            send(StreamEvent::Delta(content)).await;
                        }
                        if let Some(thinking) = thinking_text(api, &value) {
                            send(StreamEvent::Thinking(thinking)).await;
                        }
                        if is_final_event(api, &value) {
                            send(StreamEvent::Done).await;
                            return;
                        }
                    }
                }
            }
            send(StreamEvent::Done).await;
        });
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_for(api: ApiFormat, system_prompt: &str, temperature: Option<f64>) -> serde_json::Value {
        let messages = vec![(Role::User, "你好".to_string())];
        let (_, body) = build_request(
            api,
            "http://127.0.0.1:18080",
            "mock-mini",
            &messages,
            4096,
            system_prompt,
            temperature,
            None,
        );
        body
    }

    #[test]
    fn base_url_gets_v1_only_when_bare() {
        assert_eq!(
            normalize_base("https://api.deepseek.com"),
            "https://api.deepseek.com/v1"
        );
        assert_eq!(
            normalize_base("https://api.deepseek.com/"),
            "https://api.deepseek.com/v1"
        );
        assert_eq!(
            normalize_base("http://127.0.0.1:18080/v1/"),
            "http://127.0.0.1:18080/v1"
        );
        assert_eq!(
            normalize_base(" http://host/openai/v1 "),
            "http://host/openai/v1"
        );
    }

    /// The documented output budget must actually reach the wire. It used to be
    /// computed for every provider and then only sent to Anthropic.
    #[test]
    fn chat_completions_body_carries_system_budget_and_temperature() {
        let body = body_for(ApiFormat::OpenAiCompletions, "简短回答。", Some(0.2));
        assert_eq!(body["model"], "mock-mini");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 4096);
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "简短回答。");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "你好");
    }

    #[test]
    fn openai_reasoning_models_use_max_completion_tokens() {
        let messages = vec![(Role::User, "你好".to_string())];
        let (_, body) = build_request(
            ApiFormat::OpenAiCompletions,
            "http://host/v1",
            "o3-mini",
            &messages,
            1024,
            "",
            None,
            None,
        );
        assert_eq!(body["max_completion_tokens"], 1024);
        assert!(body.get("max_tokens").is_none(), "{body}");
    }

    #[test]
    fn responses_body_uses_instructions_and_output_budget() {
        let body = body_for(ApiFormat::OpenAiResponses, "简短回答。", Some(0.2));
        assert_eq!(body["model"], "mock-mini");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_output_tokens"], 4096);
        assert_eq!(body["instructions"], "简短回答。");
        assert_eq!(body["temperature"], 0.2);
        assert!(body.get("max_tokens").is_none());
        let input = body["input"].as_array().expect("input array");
        assert!(input.iter().all(|item| item["role"] != "system"));
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["text"], "你好");
    }

    #[test]
    fn anthropic_body_lifts_system_out_of_messages() {
        let body = body_for(ApiFormat::AnthropicMessages, "简短回答。", Some(0.2));
        assert_eq!(body["model"], "mock-mini");
        assert_eq!(body["stream"], true);
        assert_eq!(body["system"], "简短回答。");
        assert_eq!(body["max_tokens"], 4096);
        assert_eq!(body["temperature"], 0.2);
        let messages = body["messages"].as_array().expect("messages array");
        assert!(messages.iter().all(|item| item["role"] != "system"));
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "你好");
    }

    /// The Messages API accepts 0–1 only; the settings field allows 0–2 for the
    /// other shapes, so the wire value must be clamped, not rejected.
    #[test]
    fn anthropic_temperature_is_clamped_to_one() {
        let body = body_for(ApiFormat::AnthropicMessages, "", Some(1.7));
        assert_eq!(body["temperature"], 1.0);
        let body = body_for(ApiFormat::OpenAiCompletions, "", Some(1.7));
        assert_eq!(body["temperature"], 1.7);
    }

    /// Reasoning effort maps to each shape's own vocabulary; Claude uses an
    /// extended-thinking budget and must drop a custom temperature.
    #[test]
    fn reasoning_effort_maps_per_shape() {
        let messages = vec![(Role::User, "你好".to_string())];
        for api in ApiFormat::ALL {
            let (_, body) = build_request(
                api,
                "http://host/v1",
                "mock-mini",
                &messages,
                4096,
                "",
                Some(0.2),
                Some(ReasoningEffort::High),
            );
            match api {
                ApiFormat::OpenAiCompletions => {
                    assert_eq!(body["reasoning_effort"], "high", "{body}");
                    assert_eq!(body["temperature"], 0.2, "{body}");
                }
                ApiFormat::OpenAiResponses => {
                    assert_eq!(body["reasoning"]["effort"], "high", "{body}");
                    assert_eq!(body["temperature"], 0.2, "{body}");
                }
                ApiFormat::AnthropicMessages => {
                    assert_eq!(body["thinking"]["type"], "enabled", "{body}");
                    assert_eq!(body["thinking"]["budget_tokens"], 8192, "{body}");
                    assert_eq!(body["max_tokens"], 9216, "{body}");
                    assert!(body.get("temperature").is_none(), "{body}");
                }
            }
        }
    }

    /// An unset system prompt or temperature must produce no field at all, not
    /// an empty string or a zero temperature that would change the sampling.
    #[test]
    fn empty_system_prompt_and_no_temperature_stay_absent() {
        for api in ApiFormat::ALL {
            let body = body_for(api, "  \n ", None);
            assert!(body.get("temperature").is_none(), "{api:?}");
            assert!(body.get("instructions").is_none(), "{api:?}");
            assert!(body.get("system").is_none(), "{api:?}");
            assert!(body.get("reasoning").is_none(), "{api:?}");
            assert!(body.get("reasoning_effort").is_none(), "{api:?}");
            assert!(body.get("thinking").is_none(), "{api:?}");
            if let Some(messages) = body.get("messages").and_then(|value| value.as_array()) {
                assert!(
                    messages.iter().all(|message| message["role"] != "system"),
                    "{api:?}"
                );
            }
        }
    }

    #[test]
    fn system_messages_are_merged_once_ahead_of_the_conversation() {
        let messages = vec![
            (Role::System, "旧设定".to_string()),
            (Role::User, "你好".to_string()),
        ];
        for api in ApiFormat::ALL {
            let (_, body) = build_request(
                api,
                "http://host/v1",
                "mock-mini",
                &messages,
                4096,
                "新设定",
                None,
                None,
            );
            let merged = "新设定\n\n旧设定";
            match api {
                ApiFormat::OpenAiCompletions => {
                    let messages = body["messages"].as_array().expect("messages array");
                    assert_eq!(
                        messages
                            .iter()
                            .filter(|message| message["role"] == "system")
                            .count(),
                        1,
                        "{body}"
                    );
                    assert_eq!(messages[0]["content"], merged);
                    assert_eq!(messages[1]["role"], "user");
                }
                ApiFormat::OpenAiResponses => {
                    assert_eq!(body["instructions"], merged, "{body}");
                    assert_eq!(body["input"][0]["role"], "user");
                }
                ApiFormat::AnthropicMessages => {
                    assert_eq!(body["system"], merged, "{body}");
                    assert_eq!(body["messages"][0]["role"], "user");
                }
            }
        }
    }

    #[test]
    fn completion_deltas_carry_content_and_reasoning() {
        let value = serde_json::json!({
            "choices": [{ "delta": { "content": "答", "reasoning_content": "想" } }]
        });
        assert_eq!(
            delta_text(ApiFormat::OpenAiCompletions, &value).as_deref(),
            Some("答")
        );
        assert_eq!(
            thinking_text(ApiFormat::OpenAiCompletions, &value).as_deref(),
            Some("想")
        );
    }

    #[test]
    fn responses_deltas_are_typed_events() {
        let content = serde_json::json!({
            "type": "response.output_text.delta", "delta": "答"
        });
        let thinking = serde_json::json!({
            "type": "response.reasoning_summary_text.delta", "delta": "想"
        });
        assert_eq!(
            delta_text(ApiFormat::OpenAiResponses, &content).as_deref(),
            Some("答")
        );
        assert_eq!(
            thinking_text(ApiFormat::OpenAiResponses, &thinking).as_deref(),
            Some("想")
        );
    }

    #[test]
    fn anthropic_deltas_separate_text_from_thinking() {
        let text = serde_json::json!({
            "type": "content_block_delta",
            "delta": { "type": "text_delta", "text": "答" }
        });
        let thinking = serde_json::json!({
            "type": "content_block_delta",
            "delta": { "type": "thinking_delta", "thinking": "想" }
        });
        assert_eq!(
            delta_text(ApiFormat::AnthropicMessages, &text).as_deref(),
            Some("答")
        );
        assert!(thinking_text(ApiFormat::AnthropicMessages, &text).is_none());
        assert_eq!(
            thinking_text(ApiFormat::AnthropicMessages, &thinking).as_deref(),
            Some("想")
        );
    }

    #[test]
    fn stream_end_events_are_shape_specific() {
        assert!(!is_final_event(
            ApiFormat::OpenAiCompletions,
            &serde_json::json!({ "choices": [] })
        ));
        assert!(is_final_event(
            ApiFormat::OpenAiResponses,
            &serde_json::json!({ "type": "response.completed" })
        ));
        assert!(is_final_event(
            ApiFormat::AnthropicMessages,
            &serde_json::json!({ "type": "message_stop" })
        ));
    }

    #[test]
    fn sse_data_reads_every_data_line_in_a_block() {
        let event = "event: message\ndata: {\"a\":1}\r\ndata: [DONE]\n\n";
        assert_eq!(sse_data(event), vec!["{\"a\":1}", "[DONE]"]);
        assert!(sse_data("event: ping\n\n").is_empty());
    }
}
