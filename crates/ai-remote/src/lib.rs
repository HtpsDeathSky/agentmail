use async_trait::async_trait;
use mail_core::{AiAnalysisInput, AiInsightPayload, AiSettings};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AiRemoteError {
    #[error("ai settings are disabled")]
    Disabled,
    #[error("ai settings are incomplete: {0}")]
    InvalidSettings(String),
    #[error("remote ai request failed: {0}")]
    Request(String),
    #[error("remote ai response parse failed: {0}")]
    Parse(String),
}

pub type AiRemoteResult<T> = Result<T, AiRemoteError>;

#[async_trait]
pub trait AiProvider: Send + Sync {
    async fn analyze_mail(
        &self,
        settings: &AiSettings,
        input: &AiAnalysisInput,
    ) -> AiRemoteResult<AiInsightPayload>;
}

#[derive(Clone)]
pub struct MockAiProvider {
    payload: Option<AiInsightPayload>,
    error: Option<AiRemoteError>,
}

impl MockAiProvider {
    pub fn new(payload: AiInsightPayload) -> Self {
        Self {
            payload: Some(payload),
            error: None,
        }
    }

    pub fn error(error: AiRemoteError) -> Self {
        Self {
            payload: None,
            error: Some(error),
        }
    }

    pub fn request_error(message: impl Into<String>) -> Self {
        Self::error(AiRemoteError::Request(message.into()))
    }
}

#[async_trait]
impl AiProvider for MockAiProvider {
    async fn analyze_mail(
        &self,
        _settings: &AiSettings,
        _input: &AiAnalysisInput,
    ) -> AiRemoteResult<AiInsightPayload> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }

        self.payload
            .clone()
            .ok_or_else(|| AiRemoteError::Request("mock provider has no configured payload".into()))
    }
}

#[derive(Clone)]
pub struct OpenAiCompatibleProvider {
    client: reqwest::Client,
}

impl OpenAiCompatibleProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

impl Default for OpenAiCompatibleProvider {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("default reqwest client should build"),
        }
    }
}

#[async_trait]
impl AiProvider for OpenAiCompatibleProvider {
    async fn analyze_mail(
        &self,
        settings: &AiSettings,
        input: &AiAnalysisInput,
    ) -> AiRemoteResult<AiInsightPayload> {
        validate_settings(settings)?;

        let base_url = settings.base_url.trim_end_matches('/');
        let url = format!("{base_url}/chat/completions");
        let user_prompt = build_user_prompt(input)?;
        let request = ChatCompletionRequest {
            model: settings.model.trim(),
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: SYSTEM_PROMPT,
                },
                ChatMessage {
                    role: "user",
                    content: &user_prompt,
                },
            ],
            response_format: json!({ "type": "json_object" }),
        };

        let response = self
            .client
            .post(url)
            .bearer_auth(settings.api_key.trim())
            .json(&request)
            .send()
            .await
            .map_err(|err| AiRemoteError::Request(redact_api_key(err.to_string(), settings)))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| AiRemoteError::Request(redact_api_key(err.to_string(), settings)))?;

        if !status.is_success() {
            let sanitized_body = sanitize_error_body(body, settings);
            return Err(AiRemoteError::Request(format!(
                "status {status}: {sanitized_body}"
            )));
        }

        let parsed: ChatCompletionResponse = serde_json::from_str(&body)
            .map_err(|err| AiRemoteError::Parse(redact_api_key(err.to_string(), settings)))?;
        let content = parsed
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_deref())
            .ok_or_else(|| AiRemoteError::Parse("missing choices[0].message.content".into()))?;

        parse_model_json(content).map_err(|err| match err {
            AiRemoteError::Parse(message) => {
                AiRemoteError::Parse(redact_api_key(message, settings))
            }
            other => other,
        })
    }
}

impl Clone for AiRemoteError {
    fn clone(&self) -> Self {
        match self {
            Self::Disabled => Self::Disabled,
            Self::InvalidSettings(message) => Self::InvalidSettings(message.clone()),
            Self::Request(message) => Self::Request(message.clone()),
            Self::Parse(message) => Self::Parse(message.clone()),
        }
    }
}

const SYSTEM_PROMPT: &str = r#"You analyze email for a local mail client.
Return only a valid JSON object with these fields:
summary: string
category: string
priority: one of "low", "normal", "high", "urgent"
todos: array of strings
reply_draft: string
Do not include markdown, prose, or extra fields."#;

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    response_format: serde_json::Value,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    content: Option<String>,
}

fn validate_settings(settings: &AiSettings) -> AiRemoteResult<()> {
    if !settings.enabled {
        return Err(AiRemoteError::Disabled);
    }

    let mut missing = Vec::new();
    if settings.base_url.trim().is_empty() {
        missing.push("base_url");
    }
    if settings.model.trim().is_empty() {
        missing.push("model");
    }
    if settings.api_key.trim().is_empty() {
        missing.push("api_key");
    }

    if missing.is_empty() {
        validate_remote_base_url(&settings.base_url)
    } else {
        Err(AiRemoteError::InvalidSettings(missing.join(", ")))
    }
}

pub fn validate_remote_base_url(base_url: &str) -> AiRemoteResult<()> {
    let parsed = Url::parse(base_url.trim()).map_err(|_| {
        AiRemoteError::InvalidSettings("base_url must be a valid https URL".to_string())
    })?;

    if parsed.scheme() != "https" {
        return Err(AiRemoteError::InvalidSettings(
            "base_url must use https".to_string(),
        ));
    }

    if parsed.host_str().is_none() {
        return Err(AiRemoteError::InvalidSettings(
            "base_url must include a host".to_string(),
        ));
    }

    Ok(())
}

fn build_user_prompt(input: &AiAnalysisInput) -> AiRemoteResult<String> {
    serde_json::to_string_pretty(&json!({
        "subject": input.subject,
        "sender": input.sender,
        "recipients": input.recipients,
        "cc": input.cc,
        "received_at": input.received_at,
        "body_preview": input.body_preview,
        "body": input.body,
        "attachments": input.attachments.iter().map(|attachment| {
            json!({
                "id": attachment.id,
                "filename": attachment.filename,
                "mime_type": attachment.mime_type,
                "size_bytes": attachment.size_bytes,
            })
        }).collect::<Vec<_>>(),
    }))
    .map_err(|err| AiRemoteError::Parse(err.to_string()))
}

fn parse_model_json(content: &str) -> AiRemoteResult<AiInsightPayload> {
    let mut payload: AiInsightPayload =
        serde_json::from_str(content).map_err(|err| AiRemoteError::Parse(err.to_string()))?;
    payload.raw_json = content.to_string();
    Ok(payload)
}

fn redact_api_key(message: String, settings: &AiSettings) -> String {
    let api_key = settings.api_key.trim();
    if api_key.is_empty() {
        message
    } else {
        message.replace(api_key, "[REDACTED]")
    }
}

const ERROR_BODY_MAX_CHARS: usize = 512;
const TRUNCATED_ERROR_SUFFIX: &str = "... [truncated]";

fn sanitize_error_body(body: String, settings: &AiSettings) -> String {
    let redacted = redact_api_key(body, settings);
    let mut chars = redacted.chars();
    let truncated: String = chars.by_ref().take(ERROR_BODY_MAX_CHARS).collect();

    if chars.next().is_some() {
        format!("{truncated}{TRUNCATED_ERROR_SUFFIX}")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::{
        now_rfc3339, AiAnalysisInput, AiInsightPayload, AiPriority, AiSettings, AttachmentRef,
    };

    fn settings() -> AiSettings {
        let now = now_rfc3339();
        AiSettings {
            id: "default".to_string(),
            provider_name: "openai-compatible".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            model: "mail-model".to_string(),
            api_key: "sk-local-test".to_string(),
            enabled: true,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    fn input() -> AiAnalysisInput {
        AiAnalysisInput {
            message_id: "message-1".to_string(),
            subject: "Release train".to_string(),
            sender: "ops@example.com".to_string(),
            recipients: vec!["me@example.com".to_string()],
            cc: Vec::new(),
            received_at: now_rfc3339(),
            body_preview: "Build passed smoke tests.".to_string(),
            body: Some("Build passed smoke tests. Reply before 18:00.".to_string()),
            attachments: vec![AttachmentRef {
                id: "att-1".to_string(),
                message_id: "message-1".to_string(),
                filename: "report.txt".to_string(),
                mime_type: "text/plain".to_string(),
                size_bytes: 512,
                local_path: None,
            }],
        }
    }

    #[tokio::test]
    async fn mock_provider_returns_configured_payload() {
        let provider = MockAiProvider::new(AiInsightPayload {
            summary: "Summary".to_string(),
            category: "ops".to_string(),
            priority: AiPriority::High,
            todos: vec!["Reply".to_string()],
            reply_draft: "Thanks.".to_string(),
            raw_json: "{}".to_string(),
        });

        let result = provider.analyze_mail(&settings(), &input()).await.unwrap();
        assert_eq!(result.summary, "Summary");
        assert_eq!(result.priority, AiPriority::High);
    }

    #[test]
    fn parses_model_json_payload() {
        let parsed = parse_model_json(
            r#"{"summary":"S","category":"ops","priority":"urgent","todos":["A"],"reply_draft":"R"}"#,
        )
        .unwrap();

        assert_eq!(parsed.priority, AiPriority::Urgent);
        assert_eq!(parsed.todos, vec!["A".to_string()]);
        assert_eq!(
            parsed.raw_json,
            r#"{"summary":"S","category":"ops","priority":"urgent","todos":["A"],"reply_draft":"R"}"#
        );
    }

    #[tokio::test]
    async fn disabled_settings_are_rejected_before_request() {
        let mut settings = settings();
        settings.enabled = false;

        let result = OpenAiCompatibleProvider::default()
            .analyze_mail(&settings, &input())
            .await;

        assert!(matches!(result, Err(AiRemoteError::Disabled)));
    }

    #[test]
    fn cleartext_remote_base_url_is_rejected() {
        let mut settings = settings();
        settings.base_url = "http://api.example.com/v1".to_string();

        let result = validate_settings(&settings);

        assert!(
            matches!(result, Err(AiRemoteError::InvalidSettings(message)) if message.contains("https"))
        );
    }

    #[test]
    fn redacts_api_key_from_error_text() {
        let settings = settings();

        let redacted = redact_api_key(
            "upstream echoed sk-local-test in an error".to_string(),
            &settings,
        );

        assert_eq!(redacted, "upstream echoed [REDACTED] in an error");
        assert!(!redacted.contains("sk-local-test"));
    }

    #[test]
    fn sanitizes_long_error_body_by_redacting_key_and_truncating() {
        let settings = settings();
        let body = format!(
            "provider leaked sk-local-test in a long response: {}",
            "x".repeat(700)
        );

        let sanitized = sanitize_error_body(body, &settings);

        assert!(sanitized.len() <= ERROR_BODY_MAX_CHARS + TRUNCATED_ERROR_SUFFIX.len());
        assert!(sanitized.ends_with(TRUNCATED_ERROR_SUFFIX));
        assert!(sanitized.contains("[REDACTED]"));
        assert!(!sanitized.contains("sk-local-test"));
    }

    #[test]
    fn sanitizes_short_error_body_by_redacting_key() {
        let settings = settings();

        let sanitized = sanitize_error_body(
            "provider rejected api key sk-local-test".to_string(),
            &settings,
        );

        assert_eq!(sanitized, "provider rejected api key [REDACTED]");
        assert!(!sanitized.contains("sk-local-test"));
    }
}
