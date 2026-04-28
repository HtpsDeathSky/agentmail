use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use uuid::Uuid;

pub type Timestamp = String;

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn now_rfc3339() -> Timestamp {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub fn rfc3339_from_unix_timestamp(timestamp: i64) -> Option<Timestamp> {
    OffsetDateTime::from_unix_timestamp(timestamp)
        .ok()?
        .format(&Rfc3339)
        .ok()
}

pub fn now_plus_seconds_rfc3339(seconds: i64) -> Timestamp {
    let timestamp = OffsetDateTime::now_utc().unix_timestamp() + seconds.max(0);
    rfc3339_from_unix_timestamp(timestamp).unwrap_or_else(now_rfc3339)
}

pub fn timestamp_is_future(timestamp: &str) -> bool {
    OffsetDateTime::parse(timestamp, &Rfc3339)
        .map(|value| value > OffsetDateTime::now_utc())
        .unwrap_or(false)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailAccount {
    pub id: String,
    pub display_name: String,
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_tls: bool,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_tls: bool,
    pub sync_enabled: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailFolder {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub path: String,
    pub role: FolderRole,
    pub unread_count: u32,
    pub total_count: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FolderRole {
    Inbox,
    Sent,
    Archive,
    Trash,
    Drafts,
    Junk,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MessageFlags {
    pub is_read: bool,
    pub is_starred: bool,
    pub is_answered: bool,
    pub is_forwarded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttachmentRef {
    pub id: String,
    pub message_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub local_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailMessage {
    pub id: String,
    pub account_id: String,
    pub folder_id: String,
    pub uid: Option<String>,
    pub message_id_header: Option<String>,
    pub subject: String,
    pub sender: String,
    pub recipients: Vec<String>,
    pub cc: Vec<String>,
    pub received_at: Timestamp,
    pub body_preview: String,
    pub body: Option<String>,
    pub attachments: Vec<AttachmentRef>,
    pub flags: MessageFlags,
    pub size_bytes: Option<i64>,
    pub deleted_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncStateKind {
    Idle,
    Syncing,
    Watching,
    Backoff,
    Error,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncState {
    pub account_id: String,
    pub folder_id: Option<String>,
    pub state: SyncStateKind,
    pub last_uid: Option<String>,
    pub last_synced_at: Option<Timestamp>,
    pub error_message: Option<String>,
    pub backoff_until: Option<Timestamp>,
    pub failure_count: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MailActionKind {
    MarkRead,
    MarkUnread,
    Star,
    Unstar,
    Move,
    Archive,
    Delete,
    PermanentDelete,
    Send,
    Forward,
    BatchDelete,
    BatchMove,
}

impl MailActionKind {
    pub fn requires_confirmation(self) -> bool {
        matches!(
            self,
            Self::PermanentDelete
                | Self::Send
                | Self::Forward
                | Self::BatchDelete
                | Self::BatchMove
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailActionRequest {
    pub action: MailActionKind,
    pub account_id: String,
    pub message_ids: Vec<String>,
    pub target_folder_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MailActionResultKind {
    Executed,
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailActionResult {
    pub kind: MailActionResultKind,
    pub pending_action_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PendingActionStatus {
    Pending,
    Accepted,
    Rejected,
    Executed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingMailAction {
    pub id: String,
    pub account_id: String,
    pub action: MailActionKind,
    pub message_ids: Vec<String>,
    pub target_folder_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_message_id: Option<String>,
    pub draft: Option<SendMessageDraft>,
    pub status: PendingActionStatus,
    pub error_message: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteMailAction {
    pub action: MailActionKind,
    pub source_folder: MailFolder,
    pub target_folder: Option<MailFolder>,
    pub uids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionAuditStatus {
    Queued,
    Accepted,
    Rejected,
    Executed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailActionAudit {
    pub id: String,
    pub account_id: String,
    pub action: MailActionKind,
    pub message_ids: Vec<String>,
    pub status: ActionAuditStatus,
    pub error_message: Option<String>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageQuery {
    pub account_id: Option<String>,
    pub folder_id: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

impl Default for MessageQuery {
    fn default() -> Self {
        Self {
            account_id: None,
            folder_id: None,
            limit: 100,
            offset: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SendMessageDraft {
    pub account_id: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id_header: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SendMessageResult {
    pub message_id: String,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionSettings {
    pub account_id: Option<String>,
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_tls: bool,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_tls: bool,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionTestResult {
    pub imap_ok: bool,
    pub smtp_ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageFetchRequest {
    pub last_uid: Option<String>,
    pub limit: u32,
}

impl Default for MessageFetchRequest {
    fn default() -> Self {
        Self {
            last_uid: None,
            limit: 100,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FolderWatchOutcome {
    Changed,
    Timeout,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiPriority {
    Low,
    Normal,
    High,
    Urgent,
}

impl Default for AiPriority {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiSettings {
    pub id: String,
    pub provider_name: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub enabled: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl std::fmt::Debug for AiSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiSettings")
            .field("id", &self.id)
            .field("provider_name", &self.provider_name)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &"***")
            .field("enabled", &self.enabled)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiSettingsView {
    pub provider_name: String,
    pub base_url: String,
    pub model: String,
    pub enabled: bool,
    pub api_key_mask: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SaveAiSettingsRequest {
    pub provider_name: String,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub enabled: bool,
}

impl std::fmt::Debug for SaveAiSettingsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveAiSettingsRequest")
            .field("provider_name", &self.provider_name)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .field("enabled", &self.enabled)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_ai_settings_request_debug_redacts_api_key() {
        let request = SaveAiSettingsRequest {
            provider_name: "openai-compatible".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            model: "mail-model".to_string(),
            api_key: Some("sk-plaintext-secret".to_string()),
            enabled: true,
        };

        let debug = format!("{request:?}");

        assert!(!debug.contains("sk-plaintext-secret"));
        assert!(debug.contains("***"));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiAnalysisInput {
    pub message_id: String,
    pub subject: String,
    pub sender: String,
    pub recipients: Vec<String>,
    pub cc: Vec<String>,
    pub received_at: Timestamp,
    pub body_preview: String,
    pub body: Option<String>,
    pub attachments: Vec<AttachmentRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiInsightPayload {
    pub summary: String,
    pub category: String,
    pub priority: AiPriority,
    pub todos: Vec<String>,
    pub reply_draft: String,
    #[serde(default, skip_deserializing)]
    pub raw_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiInsight {
    pub id: String,
    pub message_id: String,
    pub provider_name: String,
    pub model: String,
    pub summary: String,
    pub category: String,
    pub priority: AiPriority,
    pub todos: Vec<String>,
    pub reply_draft: String,
    pub raw_json: String,
    pub created_at: Timestamp,
}
