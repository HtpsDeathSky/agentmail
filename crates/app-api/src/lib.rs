use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use ai_remote::{validate_remote_base_url, AiProvider, OpenAiCompatibleProvider};
use mail_core::{
    new_id, now_plus_seconds_rfc3339, now_rfc3339, timestamp_is_future, ActionAuditStatus,
    AiAnalysisInput, AiInsight, AiSettings, AiSettingsView, ConnectionSettings,
    ConnectionTestResult, FolderRole, FolderWatchOutcome, MailAccount, MailActionAudit,
    MailActionKind, MailActionRequest, MailActionResult, MailActionResultKind, MailFolder,
    MailMessage, MessageFetchRequest, MessageQuery, PendingActionStatus, PendingMailAction,
    RemoteMailAction, SaveAiSettingsRequest, SendMessageDraft, SyncState, SyncStateKind,
};
use mail_protocol::{validate_mailbox_address, LiveMailProtocol, MailProtocol, ProtocolError};
use mail_store::{MailStore, MessageFlagPatch, StoreError};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error(transparent)]
    AiRemote(#[from] ai_remote::AiRemoteError),
    #[error("blocked action requires explicit confirmation: {0:?}")]
    ConfirmationRequired(MailActionKind),
    #[error("sync already running for account: {0}")]
    SyncAlreadyRunning(String),
    #[error("sync is in backoff until {backoff_until}: {account_id}")]
    SyncBackoff {
        account_id: String,
        backoff_until: String,
    },
    #[error("invalid request: {0}")]
    InvalidRequest(String),
}

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Clone)]
pub struct AppApi {
    store: MailStore,
    protocol: Arc<dyn MailProtocol>,
    ai_provider: Arc<dyn AiProvider>,
    sync_locks: Arc<Mutex<HashSet<String>>>,
}

impl AppApi {
    pub fn new(store: MailStore, protocol: Arc<dyn MailProtocol>) -> Self {
        Self::new_with_ai_provider(
            store,
            protocol,
            Arc::new(OpenAiCompatibleProvider::default()),
        )
    }

    pub fn new_with_ai_provider(
        store: MailStore,
        protocol: Arc<dyn MailProtocol>,
        ai_provider: Arc<dyn AiProvider>,
    ) -> Self {
        Self {
            store,
            protocol,
            ai_provider,
            sync_locks: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn new_default(db_path: impl AsRef<Path>) -> ApiResult<Self> {
        Ok(Self::new(
            MailStore::open(db_path)?,
            Arc::new(LiveMailProtocol),
        ))
    }

    pub async fn add_account(&self, request: AddAccountRequest) -> ApiResult<MailAccount> {
        self.save_account_config(SaveAccountConfigRequest {
            id: None,
            display_name: request.display_name,
            email: request.email,
            password: request.password,
            imap_host: request.imap_host,
            imap_port: request.imap_port,
            imap_tls: request.imap_tls,
            smtp_host: request.smtp_host,
            smtp_port: request.smtp_port,
            smtp_tls: request.smtp_tls,
            sync_enabled: true,
        })
        .await
    }

    pub async fn test_account_connection(
        &self,
        request: TestConnectionRequest,
    ) -> ApiResult<ConnectionTestResult> {
        let settings = if let Some(account_id) = request.account_id {
            let account = self.store.get_account(&account_id)?;
            let password = self.store.get_account_password(&account.id)?;
            account_to_settings(&account, password)
        } else {
            let manual = request
                .manual
                .ok_or_else(|| ApiError::InvalidRequest("manual settings required".to_string()))?;
            manual.validate()?;
            manual.connection_settings(None)
        };

        Ok(self.protocol.test_connection(&settings).await?)
    }

    pub fn list_accounts(&self) -> ApiResult<Vec<MailAccount>> {
        Ok(self.store.list_accounts()?)
    }

    pub fn is_account_sync_enabled(&self, account_id: &str) -> ApiResult<bool> {
        Ok(self.store.get_account(account_id)?.sync_enabled)
    }

    pub fn get_account_config(&self, account_id: String) -> ApiResult<AccountConfigView> {
        let account = self.store.get_account(&account_id)?;
        let password = self.store.get_account_password(&account.id)?;
        Ok(account_config_view(account, password))
    }

    pub async fn save_account_config(
        &self,
        request: SaveAccountConfigRequest,
    ) -> ApiResult<MailAccount> {
        request.validate()?;
        let email = normalize_email_address(&request.email)?;

        let now = now_rfc3339();
        let existing = match request.id.as_deref() {
            Some(id) => Some(self.store.get_account(id)?),
            None => None,
        };
        let account = MailAccount {
            id: existing
                .as_ref()
                .map(|account| account.id.clone())
                .unwrap_or_else(new_id),
            display_name: request.display_name,
            email,
            imap_host: request.imap_host,
            imap_port: request.imap_port,
            imap_tls: request.imap_tls,
            smtp_host: request.smtp_host,
            smtp_port: request.smtp_port,
            smtp_tls: request.smtp_tls,
            sync_enabled: request.sync_enabled,
            created_at: existing
                .as_ref()
                .map(|account| account.created_at.clone())
                .unwrap_or_else(|| now.clone()),
            updated_at: now,
        };

        self.store
            .save_account_with_password(&account, &request.password)?;
        if existing.is_none() {
            self.write_audit(
                &account.id,
                MailActionKind::MarkRead,
                Vec::new(),
                ActionAuditStatus::Executed,
                None,
            )?;
        }
        Ok(account)
    }

    pub async fn sync_account(&self, account_id: String) -> ApiResult<SyncSummary> {
        let _guard = self.acquire_sync_lock(&account_id)?;
        let account = self.store.get_account(&account_id)?;
        let previous_root = self.store.get_sync_state(&account.id, None)?;
        if let Some(backoff_until) = previous_root
            .as_ref()
            .and_then(|state| state.backoff_until.as_deref())
        {
            if matches!(
                previous_root.as_ref().map(|state| state.state),
                Some(SyncStateKind::Backoff)
            ) && timestamp_is_future(backoff_until)
            {
                return Err(ApiError::SyncBackoff {
                    account_id,
                    backoff_until: backoff_until.to_string(),
                });
            }
        }

        self.store.save_sync_state(&SyncState {
            account_id: account.id.clone(),
            folder_id: None,
            state: SyncStateKind::Syncing,
            last_uid: previous_root
                .as_ref()
                .and_then(|state| state.last_uid.clone()),
            last_synced_at: None,
            error_message: None,
            backoff_until: None,
            failure_count: previous_root
                .as_ref()
                .map(|state| state.failure_count)
                .unwrap_or(0),
        })?;

        let result = self.sync_account_inner(&account).await;
        match result {
            Ok(summary) => {
                self.store.save_sync_state(&SyncState {
                    account_id: account.id.clone(),
                    folder_id: None,
                    state: SyncStateKind::Idle,
                    last_uid: summary.last_uid.clone(),
                    last_synced_at: Some(now_rfc3339()),
                    error_message: None,
                    backoff_until: None,
                    failure_count: 0,
                })?;
                self.write_audit(
                    &account.id,
                    MailActionKind::MarkRead,
                    Vec::new(),
                    ActionAuditStatus::Executed,
                    None,
                )?;
                Ok(summary)
            }
            Err(err) => {
                let failure_count = previous_root
                    .as_ref()
                    .map(|state| state.failure_count)
                    .unwrap_or(0)
                    .saturating_add(1);
                self.store.save_sync_state(&SyncState {
                    account_id: account.id.clone(),
                    folder_id: None,
                    state: SyncStateKind::Backoff,
                    last_uid: previous_root.and_then(|state| state.last_uid),
                    last_synced_at: None,
                    error_message: Some(err.to_string()),
                    backoff_until: Some(now_plus_seconds_rfc3339(backoff_seconds(failure_count))),
                    failure_count,
                })?;
                self.write_audit(
                    &account.id,
                    MailActionKind::MarkRead,
                    Vec::new(),
                    ActionAuditStatus::Failed,
                    Some(err.to_string()),
                )?;
                Err(err)
            }
        }
    }

    async fn sync_account_inner(&self, account: &MailAccount) -> ApiResult<SyncSummary> {
        let settings = self.connection_settings_for_account(account)?;
        let folders = self.protocol.fetch_folders(&settings, account).await?;
        let mut message_count = 0_u32;
        let mut last_uid = None;
        let mut attempted_folders = 0_u32;
        let mut successful_folders = 0_u32;
        let mut first_folder_error = None;
        for folder in &folders {
            self.store.save_folder(folder)?;
            if folder.role == FolderRole::Sent {
                self.converge_fallback_sent_placeholders(&account.id, folder)?;
            }

            let previous_state = self.store.get_sync_state(&account.id, Some(&folder.id))?;
            if let Some(backoff_until) = previous_state
                .as_ref()
                .and_then(|state| state.backoff_until.as_deref())
            {
                if matches!(
                    previous_state.as_ref().map(|state| state.state),
                    Some(SyncStateKind::Backoff)
                ) && timestamp_is_future(backoff_until)
                {
                    continue;
                }
            }

            attempted_folders += 1;
            self.store.save_sync_state(&SyncState {
                account_id: account.id.clone(),
                folder_id: Some(folder.id.clone()),
                state: SyncStateKind::Syncing,
                last_uid: previous_state
                    .as_ref()
                    .and_then(|state| state.last_uid.clone()),
                last_synced_at: None,
                error_message: None,
                backoff_until: None,
                failure_count: previous_state
                    .as_ref()
                    .map(|state| state.failure_count)
                    .unwrap_or(0),
            })?;

            let fetch_request = MessageFetchRequest {
                last_uid: previous_state.and_then(|state| state.last_uid),
                limit: 100,
            };
            let messages_result = self
                .protocol
                .fetch_messages(&settings, account, folder, &fetch_request)
                .await;
            let messages = match messages_result {
                Ok(messages) => messages,
                Err(err) => {
                    let failure_count = self
                        .store
                        .get_sync_state(&account.id, Some(&folder.id))?
                        .map(|state| state.failure_count)
                        .unwrap_or(0)
                        .saturating_add(1);
                    self.store.save_sync_state(&SyncState {
                        account_id: account.id.clone(),
                        folder_id: Some(folder.id.clone()),
                        state: SyncStateKind::Backoff,
                        last_uid: fetch_request.last_uid,
                        last_synced_at: None,
                        error_message: Some(err.to_string()),
                        backoff_until: Some(now_plus_seconds_rfc3339(backoff_seconds(
                            failure_count,
                        ))),
                        failure_count,
                    })?;
                    if first_folder_error.is_none() {
                        first_folder_error = Some(ApiError::Protocol(err));
                    }
                    continue;
                }
            };
            let mut folder_last_uid = fetch_request.last_uid.clone();
            for message in messages {
                folder_last_uid = message.uid.clone().or(folder_last_uid);
                self.store.upsert_message(&message)?;
                message_count += 1;
            }
            self.store.refresh_folder_counts(&folder.id)?;
            last_uid = folder_last_uid.clone().or(last_uid);
            successful_folders += 1;

            self.store.save_sync_state(&SyncState {
                account_id: account.id.clone(),
                folder_id: Some(folder.id.clone()),
                state: SyncStateKind::Idle,
                last_uid: folder_last_uid,
                last_synced_at: Some(now_rfc3339()),
                error_message: None,
                backoff_until: None,
                failure_count: 0,
            })?;
        }

        if attempted_folders > 0 && successful_folders == 0 {
            if let Some(err) = first_folder_error {
                return Err(err);
            }
        }

        Ok(SyncSummary {
            account_id: account.id.clone(),
            folders: folders.len() as u32,
            messages: message_count,
            last_uid,
            synced_at: now_rfc3339(),
        })
    }

    pub async fn watch_folder_until_change(
        &self,
        account_id: String,
        folder_id: String,
    ) -> ApiResult<FolderWatchOutcome> {
        let account = self.store.get_account(&account_id)?;
        let folder = self.store.get_folder(&folder_id)?;
        if folder.account_id != account.id {
            return Err(ApiError::InvalidRequest(
                "folder belongs to a different account".to_string(),
            ));
        }

        let settings = self.connection_settings_for_account(&account)?;
        let previous_state = self.store.get_sync_state(&account.id, Some(&folder.id))?;
        self.store.save_sync_state(&SyncState {
            account_id: account.id.clone(),
            folder_id: Some(folder.id.clone()),
            state: SyncStateKind::Watching,
            last_uid: previous_state
                .as_ref()
                .and_then(|state| state.last_uid.clone()),
            last_synced_at: previous_state
                .as_ref()
                .and_then(|state| state.last_synced_at.clone()),
            error_message: None,
            backoff_until: None,
            failure_count: previous_state
                .as_ref()
                .map(|state| state.failure_count)
                .unwrap_or(0),
        })?;

        match self
            .protocol
            .watch_folder_until_change(&settings, &account, &folder)
            .await
        {
            Ok(outcome) => Ok(outcome),
            Err(err) => {
                let current_state = self.store.get_sync_state(&account.id, Some(&folder.id))?;
                let failure_count = current_state
                    .as_ref()
                    .map(|state| state.failure_count)
                    .or_else(|| previous_state.as_ref().map(|state| state.failure_count))
                    .unwrap_or(0)
                    .saturating_add(1);
                self.store.save_sync_state(&SyncState {
                    account_id: account.id,
                    folder_id: Some(folder.id),
                    state: SyncStateKind::Backoff,
                    last_uid: current_state
                        .as_ref()
                        .and_then(|state| state.last_uid.clone())
                        .or_else(|| {
                            previous_state
                                .as_ref()
                                .and_then(|state| state.last_uid.clone())
                        }),
                    last_synced_at: current_state
                        .as_ref()
                        .and_then(|state| state.last_synced_at.clone())
                        .or_else(|| {
                            previous_state
                                .as_ref()
                                .and_then(|state| state.last_synced_at.clone())
                        }),
                    error_message: Some(err.to_string()),
                    backoff_until: Some(now_plus_seconds_rfc3339(backoff_seconds(failure_count))),
                    failure_count,
                })?;
                Err(err.into())
            }
        }
    }

    pub fn get_sync_status(&self, account_id: String) -> ApiResult<Vec<SyncState>> {
        Ok(self.store.get_sync_status(&account_id)?)
    }

    pub fn list_folders(&self, account_id: String) -> ApiResult<Vec<MailFolder>> {
        Ok(self.store.list_folders(&account_id)?)
    }

    pub fn list_messages(&self, query: MessageQuery) -> ApiResult<Vec<MailMessage>> {
        Ok(self.store.list_messages(&query)?)
    }

    pub fn get_message(&self, message_id: String) -> ApiResult<MailMessage> {
        Ok(self.store.get_message(&message_id)?)
    }

    pub fn search_messages(&self, term: String, limit: Option<u32>) -> ApiResult<Vec<MailMessage>> {
        Ok(self.store.search_messages(&term, limit.unwrap_or(100))?)
    }

    pub fn get_ai_settings(&self) -> ApiResult<Option<AiSettingsView>> {
        Ok(self.store.get_ai_settings()?.map(ai_settings_view))
    }

    pub fn save_ai_settings(&self, request: SaveAiSettingsRequest) -> ApiResult<AiSettingsView> {
        let provider_name = required_trimmed(request.provider_name, "provider_name")?;
        let base_url = required_trimmed(request.base_url, "base_url")?;
        validate_remote_base_url(&base_url)
            .map_err(|error| ApiError::InvalidRequest(error.to_string()))?;
        let model = required_trimmed(request.model, "model")?;
        let existing = self.store.get_ai_settings()?;
        let api_key = match request.api_key {
            Some(api_key) => required_trimmed(api_key, "api_key")?,
            None => existing
                .as_ref()
                .map(|settings| settings.api_key.clone())
                .ok_or_else(|| ApiError::InvalidRequest("api_key is required".to_string()))?,
        };
        let now = now_rfc3339();
        let settings = AiSettings {
            id: "default".to_string(),
            provider_name,
            base_url,
            model,
            api_key,
            enabled: request.enabled,
            created_at: existing
                .as_ref()
                .map(|settings| settings.created_at.clone())
                .unwrap_or_else(|| now.clone()),
            updated_at: now,
        };

        self.store.save_ai_settings(&settings)?;
        Ok(ai_settings_view(settings))
    }

    pub fn clear_ai_settings(&self) -> ApiResult<()> {
        self.store.clear_ai_settings()?;
        Ok(())
    }

    pub async fn run_ai_analysis(&self, message_id: String) -> ApiResult<AiInsight> {
        let settings = self
            .store
            .get_ai_settings()?
            .ok_or_else(|| ApiError::InvalidRequest("ai settings are required".to_string()))?;
        if !settings.enabled {
            return Err(ai_remote::AiRemoteError::Disabled.into());
        }

        let message = self.store.get_message(&message_id)?;
        let input = ai_analysis_input(&message);
        let payload = self.ai_provider.analyze_mail(&settings, &input).await?;
        let raw_json = if payload.raw_json.is_empty() {
            serde_json::to_string(&payload).unwrap_or_default()
        } else {
            payload.raw_json.clone()
        };
        let insight = AiInsight {
            id: new_id(),
            message_id: message.id,
            provider_name: settings.provider_name,
            model: settings.model,
            summary: payload.summary,
            category: payload.category,
            priority: payload.priority,
            todos: payload.todos,
            reply_draft: payload.reply_draft,
            raw_json,
            created_at: now_rfc3339(),
        };
        self.store.save_ai_insight(&insight)?;
        Ok(insight)
    }

    pub fn list_ai_insights(&self, message_id: String) -> ApiResult<Vec<AiInsight>> {
        Ok(self.store.list_ai_insights(&message_id)?)
    }

    pub async fn execute_mail_action(
        &self,
        request: MailActionRequest,
    ) -> ApiResult<MailActionResult> {
        let normalized_action = self.confirmation_action_for_request(&request)?;
        if normalized_action.requires_confirmation() {
            let pending = self.queue_pending_action(
                request.account_id,
                normalized_action,
                request.message_ids,
                request.target_folder_id,
                None,
            )?;
            return Ok(MailActionResult {
                kind: MailActionResultKind::Pending,
                pending_action_id: Some(pending.id),
            });
        }

        self.execute_confirmed_mail_action(&request).await?;
        Ok(MailActionResult {
            kind: MailActionResultKind::Executed,
            pending_action_id: None,
        })
    }

    pub async fn send_message(&self, draft: SendMessageDraft) -> ApiResult<String> {
        if draft.to.is_empty() {
            return Err(ApiError::InvalidRequest(
                "recipient list is empty".to_string(),
            ));
        }
        let account = self.store.get_account(&draft.account_id)?;
        let now = now_rfc3339();
        let message_id = new_id();
        let message_id_header = draft
            .message_id_header
            .clone()
            .unwrap_or_else(|| format!("<{message_id}@agentmail.local>"));
        let draft = SendMessageDraft {
            message_id_header: Some(message_id_header.clone()),
            ..draft
        };
        let sent = self.sent_folder_for_local_send(&account)?;
        let pending = PendingMailAction {
            id: new_id(),
            account_id: account.id.clone(),
            action: MailActionKind::Send,
            message_ids: Vec::new(),
            target_folder_id: None,
            local_message_id: Some(message_id.clone()),
            draft: Some(draft.clone()),
            status: PendingActionStatus::Pending,
            error_message: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let audit = MailActionAudit {
            id: new_id(),
            account_id: account.id.clone(),
            action: MailActionKind::Send,
            message_ids: Vec::new(),
            status: ActionAuditStatus::Queued,
            error_message: None,
            created_at: now.clone(),
        };
        let placeholder = MailMessage {
            id: message_id.clone(),
            account_id: account.id.clone(),
            folder_id: sent.id.clone(),
            uid: None,
            message_id_header: Some(message_id_header),
            subject: draft.subject.clone(),
            sender: account.email.clone(),
            recipients: draft.to.clone(),
            cc: draft.cc.clone(),
            received_at: now.clone(),
            body_preview: body_preview_from_body(&draft.body),
            body: Some(draft.body.clone()),
            attachments: Vec::new(),
            flags: mail_core::MessageFlags {
                is_read: true,
                is_starred: false,
                is_answered: true,
                is_forwarded: false,
            },
            size_bytes: None,
            deleted_at: None,
        };
        self.store
            .save_queued_send_with_placeholder(&pending, &audit, &sent, &placeholder)?;
        Ok(pending.id)
    }

    pub fn list_pending_actions(
        &self,
        account_id: Option<String>,
    ) -> ApiResult<Vec<PendingMailAction>> {
        Ok(self.store.list_pending_actions(account_id.as_deref())?)
    }

    pub async fn confirm_action(&self, action_id: String) -> ApiResult<MailActionResult> {
        let pending = self.store.get_pending_action(&action_id)?;
        if pending.status != PendingActionStatus::Pending {
            return Err(ApiError::InvalidRequest(format!(
                "pending action {action_id} is already {:?}",
                pending.status
            )));
        }

        self.store.update_pending_action_status(
            &pending.id,
            PendingActionStatus::Accepted,
            None,
        )?;
        self.write_audit(
            &pending.account_id,
            pending.action,
            pending.message_ids.clone(),
            ActionAuditStatus::Accepted,
            None,
        )?;

        let result = match pending.action {
            MailActionKind::Send => {
                let draft = pending.draft.as_ref().ok_or_else(|| {
                    ApiError::InvalidRequest("send pending action is missing draft".to_string())
                })?;
                let send_result = self.send_message_now(draft).await;
                if send_result.is_ok() {
                    let _ = self
                        .reconcile_sent_placeholders_after_send(&pending.account_id)
                        .await;
                }
                send_result.map(|_| ())
            }
            MailActionKind::BatchDelete => {
                let request = MailActionRequest {
                    action: MailActionKind::Delete,
                    account_id: pending.account_id.clone(),
                    message_ids: pending.message_ids.clone(),
                    target_folder_id: None,
                };
                self.execute_confirmed_mail_action(&request).await
            }
            MailActionKind::BatchMove => {
                let request = MailActionRequest {
                    action: MailActionKind::Move,
                    account_id: pending.account_id.clone(),
                    message_ids: pending.message_ids.clone(),
                    target_folder_id: pending.target_folder_id.clone(),
                };
                self.execute_confirmed_mail_action(&request).await
            }
            MailActionKind::PermanentDelete => {
                let request = MailActionRequest {
                    action: MailActionKind::PermanentDelete,
                    account_id: pending.account_id.clone(),
                    message_ids: pending.message_ids.clone(),
                    target_folder_id: None,
                };
                self.execute_confirmed_mail_action(&request).await
            }
            MailActionKind::Forward => Err(ApiError::InvalidRequest(
                "forward confirmation is not implemented yet".to_string(),
            )),
            _ => {
                let request = MailActionRequest {
                    action: pending.action,
                    account_id: pending.account_id.clone(),
                    message_ids: pending.message_ids.clone(),
                    target_folder_id: pending.target_folder_id.clone(),
                };
                self.execute_confirmed_mail_action(&request).await
            }
        };

        match result {
            Ok(()) => {
                self.store.update_pending_action_status(
                    &pending.id,
                    PendingActionStatus::Executed,
                    None,
                )?;
                self.write_audit(
                    &pending.account_id,
                    pending.action,
                    pending.message_ids,
                    ActionAuditStatus::Executed,
                    None,
                )?;
                Ok(MailActionResult {
                    kind: MailActionResultKind::Executed,
                    pending_action_id: None,
                })
            }
            Err(err) => {
                let error_message = err.to_string();
                self.store.update_pending_action_status(
                    &pending.id,
                    PendingActionStatus::Failed,
                    Some(&error_message),
                )?;
                if pending.action == MailActionKind::Send {
                    self.cleanup_pending_send_placeholder(&pending)?;
                }
                self.write_audit(
                    &pending.account_id,
                    pending.action,
                    pending.message_ids,
                    ActionAuditStatus::Failed,
                    Some(error_message),
                )?;
                Err(err)
            }
        }
    }

    pub fn reject_action(&self, action_id: String) -> ApiResult<()> {
        let pending = self.store.get_pending_action(&action_id)?;
        if pending.status != PendingActionStatus::Pending {
            return Err(ApiError::InvalidRequest(format!(
                "pending action {action_id} is already {:?}",
                pending.status
            )));
        }
        self.store.update_pending_action_status(
            &pending.id,
            PendingActionStatus::Rejected,
            None,
        )?;
        if pending.action == MailActionKind::Send {
            self.cleanup_pending_send_placeholder(&pending)?;
        }
        self.write_audit(
            &pending.account_id,
            pending.action,
            pending.message_ids,
            ActionAuditStatus::Rejected,
            None,
        )?;
        Ok(())
    }

    async fn send_message_now(&self, draft: &SendMessageDraft) -> ApiResult<String> {
        let account = self.store.get_account(&draft.account_id)?;
        let settings = self.connection_settings_for_account(&account)?;
        self.protocol
            .send_message(&settings, draft)
            .await
            .map_err(Into::into)
    }

    async fn reconcile_sent_placeholders_after_send(&self, account_id: &str) -> ApiResult<()> {
        let account = self.store.get_account(account_id)?;
        if !account.sync_enabled {
            return Ok(());
        }
        let settings = self.connection_settings_for_account(&account)?;
        let folders = self.protocol.fetch_folders(&settings, &account).await?;
        for folder in folders
            .into_iter()
            .filter(|folder| folder.role == FolderRole::Sent)
        {
            self.store.save_folder(&folder)?;
            self.converge_fallback_sent_placeholders(&account.id, &folder)?;
            let messages = self
                .protocol
                .fetch_messages(
                    &settings,
                    &account,
                    &folder,
                    &MessageFetchRequest {
                        last_uid: None,
                        limit: 25,
                    },
                )
                .await?;
            for message in messages {
                self.store.upsert_message(&message)?;
            }
            self.store.refresh_folder_counts(&folder.id)?;
        }
        Ok(())
    }

    pub fn get_audit_log(&self, limit: Option<u32>) -> ApiResult<Vec<MailActionAudit>> {
        Ok(self.store.list_audits(limit.unwrap_or(100))?)
    }

    fn queue_pending_action(
        &self,
        account_id: String,
        action: MailActionKind,
        message_ids: Vec<String>,
        target_folder_id: Option<String>,
        draft: Option<SendMessageDraft>,
    ) -> ApiResult<PendingMailAction> {
        if action == MailActionKind::Send && draft.is_none() {
            return Err(ApiError::InvalidRequest(
                "send pending action requires a draft".to_string(),
            ));
        }
        self.store.get_account(&account_id)?;
        let now = now_rfc3339();
        let pending = PendingMailAction {
            id: new_id(),
            account_id,
            action,
            message_ids,
            target_folder_id,
            local_message_id: None,
            draft,
            status: PendingActionStatus::Pending,
            error_message: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.store.save_pending_action(&pending)?;
        self.write_audit(
            &pending.account_id,
            pending.action,
            pending.message_ids.clone(),
            ActionAuditStatus::Queued,
            None,
        )?;
        Ok(pending)
    }

    async fn execute_confirmed_mail_action(&self, request: &MailActionRequest) -> ApiResult<()> {
        if request.message_ids.is_empty() {
            return Err(ApiError::InvalidRequest(
                "message_ids cannot be empty".to_string(),
            ));
        }
        let account = self.store.get_account(&request.account_id)?;
        let remote_action = self.build_remote_action(&account, request).await?;
        let settings = self.connection_settings_for_account(&account)?;

        if request.action == MailActionKind::PermanentDelete && remote_action.uids.is_empty() {
            self.apply_local_action(request, &remote_action)?;
            self.write_audit(
                &request.account_id,
                request.action,
                request.message_ids.clone(),
                ActionAuditStatus::Executed,
                None,
            )?;
            return Ok(());
        }

        let result = self
            .protocol
            .apply_action(&settings, &account, &remote_action)
            .await;
        match result {
            Ok(()) => {
                self.apply_local_action(request, &remote_action)?;
                self.write_audit(
                    &request.account_id,
                    request.action,
                    request.message_ids.clone(),
                    ActionAuditStatus::Executed,
                    None,
                )?;
                Ok(())
            }
            Err(err)
                if request.action == MailActionKind::PermanentDelete
                    && is_missing_remote_message_error(&err) =>
            {
                self.apply_local_action(request, &remote_action)?;
                self.write_audit(
                    &request.account_id,
                    request.action,
                    request.message_ids.clone(),
                    ActionAuditStatus::Executed,
                    Some(err.to_string()),
                )?;
                Ok(())
            }
            Err(err) => {
                self.write_audit(
                    &request.account_id,
                    request.action,
                    request.message_ids.clone(),
                    ActionAuditStatus::Failed,
                    Some(err.to_string()),
                )?;
                Err(err.into())
            }
        }
    }

    async fn build_remote_action(
        &self,
        account: &MailAccount,
        request: &MailActionRequest,
    ) -> ApiResult<RemoteMailAction> {
        let mut messages = Vec::with_capacity(request.message_ids.len());
        for id in &request.message_ids {
            let message = self.store.get_message(id)?;
            if message.account_id != account.id {
                return Err(ApiError::InvalidRequest(format!(
                    "message {id} does not belong to account {}",
                    account.id
                )));
            }
            if message.deleted_at.is_some() {
                return Err(ApiError::InvalidRequest(format!(
                    "message {id} is locally deleted"
                )));
            }
            messages.push(message);
        }

        let source_folder_id = messages
            .first()
            .map(|message| message.folder_id.clone())
            .ok_or_else(|| ApiError::InvalidRequest("message_ids cannot be empty".to_string()))?;
        if messages
            .iter()
            .any(|message| message.folder_id != source_folder_id)
        {
            return Err(ApiError::InvalidRequest(
                "all messages in one action must be in the same folder".to_string(),
            ));
        }

        let source_folder = self.store.get_folder(&source_folder_id)?;
        let uids = if request.action == MailActionKind::PermanentDelete {
            self.resolve_permanent_delete_uids(account, &source_folder, &messages)
                .await?
        } else {
            require_message_uids(&messages)?
        };
        let target_folder = match request.action {
            MailActionKind::Move | MailActionKind::BatchMove => {
                let target_id = request.target_folder_id.as_deref().ok_or_else(|| {
                    ApiError::InvalidRequest("target_folder_id required".to_string())
                })?;
                let folder = self.store.get_folder(target_id)?;
                if folder.account_id != account.id {
                    return Err(ApiError::InvalidRequest(
                        "target folder belongs to a different account".to_string(),
                    ));
                }
                Some(folder)
            }
            MailActionKind::Archive => {
                Some(self.required_role_folder(&account.id, FolderRole::Archive)?)
            }
            MailActionKind::Delete | MailActionKind::BatchDelete => {
                Some(self.required_role_folder(&account.id, FolderRole::Trash)?)
            }
            MailActionKind::PermanentDelete => None,
            _ => None,
        };

        Ok(RemoteMailAction {
            action: request.action,
            source_folder,
            target_folder,
            uids,
        })
    }

    fn apply_local_action(
        &self,
        request: &MailActionRequest,
        remote_action: &RemoteMailAction,
    ) -> ApiResult<()> {
        match request.action {
            MailActionKind::MarkRead => self.store.set_message_flags(
                &request.message_ids,
                MessageFlagPatch {
                    is_read: Some(true),
                    is_starred: None,
                },
            )?,
            MailActionKind::MarkUnread => self.store.set_message_flags(
                &request.message_ids,
                MessageFlagPatch {
                    is_read: Some(false),
                    is_starred: None,
                },
            )?,
            MailActionKind::Star => self.store.set_message_flags(
                &request.message_ids,
                MessageFlagPatch {
                    is_read: None,
                    is_starred: Some(true),
                },
            )?,
            MailActionKind::Unstar => self.store.set_message_flags(
                &request.message_ids,
                MessageFlagPatch {
                    is_read: None,
                    is_starred: Some(false),
                },
            )?,
            MailActionKind::Move
            | MailActionKind::Archive
            | MailActionKind::Delete
            | MailActionKind::BatchMove
            | MailActionKind::BatchDelete => {
                let target = remote_action.target_folder.as_ref().ok_or_else(|| {
                    ApiError::InvalidRequest("target folder required".to_string())
                })?;
                self.store
                    .move_messages_and_clear_uids(&request.message_ids, &target.id)?;
            }
            MailActionKind::PermanentDelete => {
                self.store.soft_delete_messages(&request.message_ids)?
            }
            _ => {
                return Err(ApiError::InvalidRequest(format!(
                    "unsupported confirmed mail action: {:?}",
                    request.action
                )));
            }
        }
        self.store
            .refresh_folder_counts(&remote_action.source_folder.id)?;
        if let Some(target) = remote_action.target_folder.as_ref() {
            if target.id != remote_action.source_folder.id {
                self.store.refresh_folder_counts(&target.id)?;
            }
        }
        Ok(())
    }

    async fn resolve_permanent_delete_uids(
        &self,
        account: &MailAccount,
        source_folder: &MailFolder,
        messages: &[MailMessage],
    ) -> ApiResult<Vec<String>> {
        let settings = self.connection_settings_for_account(account)?;
        let remote_messages = self
            .protocol
            .fetch_messages(
                &settings,
                account,
                source_folder,
                &MessageFetchRequest {
                    last_uid: None,
                    limit: 500,
                },
            )
            .await?;

        let mut uids = Vec::with_capacity(messages.len());
        for message in messages {
            if let Some(uid) = remote_messages
                .iter()
                .find(|candidate| messages_match_for_remote_delete(message, candidate))
                .and_then(|candidate| candidate.uid.clone())
            {
                uids.push(uid);
            }
        }
        Ok(uids)
    }

    fn required_role_folder(&self, account_id: &str, role: FolderRole) -> ApiResult<MailFolder> {
        self.store
            .find_folder_by_role(account_id, role)?
            .ok_or_else(|| ApiError::InvalidRequest(format!("{role:?} folder is not available")))
    }

    fn sent_folder_for_local_send(&self, account: &MailAccount) -> ApiResult<MailFolder> {
        if let Some(folder) = self
            .store
            .find_folder_by_role(&account.id, FolderRole::Sent)?
        {
            return Ok(folder);
        }
        if let Some(folder) = self
            .store
            .list_folders(&account.id)?
            .into_iter()
            .find(|folder| is_sent_folder_path(&folder.path))
        {
            return Ok(MailFolder {
                role: FolderRole::Sent,
                ..folder
            });
        }
        Ok(MailFolder {
            id: format!("{}:sent", account.id),
            account_id: account.id.clone(),
            name: "Sent".to_string(),
            path: "Sent".to_string(),
            role: FolderRole::Sent,
            unread_count: 0,
            total_count: 0,
        })
    }

    fn cleanup_pending_send_placeholder(&self, pending: &PendingMailAction) -> ApiResult<()> {
        let Some(message_id) = pending.local_message_id.as_ref() else {
            return Ok(());
        };
        let expected_message_id = pending
            .draft
            .as_ref()
            .and_then(|draft| draft.message_id_header.as_deref());
        let Some(expected_message_id) = expected_message_id else {
            return Ok(());
        };
        let message = match self.store.get_message(message_id) {
            Ok(message) => message,
            Err(StoreError::NotFound(_)) => return Ok(()),
            Err(err) => return Err(err.into()),
        };
        let folder = match self.store.get_folder(&message.folder_id) {
            Ok(folder) => folder,
            Err(StoreError::NotFound(_)) => return Ok(()),
            Err(err) => return Err(err.into()),
        };
        if message.account_id != pending.account_id
            || message.uid.is_some()
            || folder.role != FolderRole::Sent
            || message.message_id_header.as_deref() != Some(expected_message_id)
        {
            return Ok(());
        }
        self.store
            .soft_delete_messages(std::slice::from_ref(message_id))?;
        self.store.refresh_folder_counts(&message.folder_id)?;
        Ok(())
    }

    fn converge_fallback_sent_placeholders(
        &self,
        account_id: &str,
        real_sent_folder: &MailFolder,
    ) -> ApiResult<()> {
        let fallback_sent_id = format!("{account_id}:sent");
        if real_sent_folder.id == fallback_sent_id {
            return Ok(());
        }
        let moved = self.store.move_uidless_messages_to_folder(
            account_id,
            &fallback_sent_id,
            &real_sent_folder.id,
        )?;
        if moved > 0 {
            self.store.refresh_folder_counts(&fallback_sent_id)?;
            self.store.refresh_folder_counts(&real_sent_folder.id)?;
        }
        Ok(())
    }

    fn confirmation_action_for_request(
        &self,
        request: &MailActionRequest,
    ) -> ApiResult<MailActionKind> {
        match request.action {
            MailActionKind::Delete => {
                if request.message_ids.is_empty() {
                    return Ok(MailActionKind::Delete);
                }
                let first_message = self.store.get_message(&request.message_ids[0])?;
                if first_message.account_id != request.account_id {
                    return Err(ApiError::InvalidRequest(format!(
                        "message {} does not belong to account {}",
                        first_message.id, request.account_id
                    )));
                }
                let source_folder = self.store.get_folder(&first_message.folder_id)?;
                if source_folder.role == FolderRole::Trash {
                    Ok(MailActionKind::PermanentDelete)
                } else if request.message_ids.len() > 1 {
                    Ok(MailActionKind::BatchDelete)
                } else {
                    Ok(MailActionKind::Delete)
                }
            }
            MailActionKind::Move if request.message_ids.len() > 1 => Ok(MailActionKind::BatchMove),
            action => Ok(action),
        }
    }

    fn write_audit(
        &self,
        account_id: &str,
        action: MailActionKind,
        message_ids: Vec<String>,
        status: ActionAuditStatus,
        error_message: Option<String>,
    ) -> ApiResult<()> {
        self.store.write_audit(&MailActionAudit {
            id: new_id(),
            account_id: account_id.to_string(),
            action,
            message_ids,
            status,
            error_message,
            created_at: now_rfc3339(),
        })?;
        Ok(())
    }

    fn connection_settings_for_account(
        &self,
        account: &MailAccount,
    ) -> ApiResult<ConnectionSettings> {
        let password = self.store.get_account_password(&account.id)?;
        if password.is_empty() {
            return Err(ApiError::InvalidRequest(
                "account password is required; save it in configuration".to_string(),
            ));
        }
        Ok(account_to_settings(account, password))
    }

    fn acquire_sync_lock(&self, account_id: &str) -> ApiResult<SyncLockGuard> {
        let key = format!("account:{account_id}");
        let mut locks = self.sync_locks.lock();
        if !locks.insert(key.clone()) {
            return Err(ApiError::SyncAlreadyRunning(account_id.to_string()));
        }
        Ok(SyncLockGuard {
            key,
            locks: Arc::clone(&self.sync_locks),
        })
    }
}

struct SyncLockGuard {
    key: String,
    locks: Arc<Mutex<HashSet<String>>>,
}

impl Drop for SyncLockGuard {
    fn drop(&mut self) {
        self.locks.lock().remove(&self.key);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddAccountRequest {
    pub display_name: String,
    pub email: String,
    pub password: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_tls: bool,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_tls: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountConfigView {
    pub id: String,
    pub display_name: String,
    pub email: String,
    pub password: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_tls: bool,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_tls: bool,
    pub sync_enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveAccountConfigRequest {
    #[serde(default)]
    pub id: Option<String>,
    pub display_name: String,
    pub email: String,
    pub password: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_tls: bool,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_tls: bool,
    #[serde(default = "default_sync_enabled")]
    pub sync_enabled: bool,
}

impl SaveAccountConfigRequest {
    fn validate(&self) -> ApiResult<()> {
        normalize_email_address(&self.email)?;
        if self.password.is_empty() {
            return Err(ApiError::InvalidRequest("password is required".to_string()));
        }
        if self.imap_host.trim().is_empty() || self.smtp_host.trim().is_empty() {
            return Err(ApiError::InvalidRequest(
                "imap_host and smtp_host are required".to_string(),
            ));
        }
        Ok(())
    }

    fn connection_settings(&self, account_id: Option<String>) -> ConnectionSettings {
        ConnectionSettings {
            account_id,
            email: self.email.trim().to_string(),
            imap_host: self.imap_host.clone(),
            imap_port: self.imap_port,
            imap_tls: self.imap_tls,
            smtp_host: self.smtp_host.clone(),
            smtp_port: self.smtp_port,
            smtp_tls: self.smtp_tls,
            password: self.password.clone(),
        }
    }
}

fn default_sync_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestConnectionRequest {
    pub account_id: Option<String>,
    pub manual: Option<SaveAccountConfigRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSummary {
    pub account_id: String,
    pub folders: u32,
    pub messages: u32,
    pub last_uid: Option<String>,
    pub synced_at: String,
}

fn account_to_settings(account: &MailAccount, password: String) -> ConnectionSettings {
    ConnectionSettings {
        account_id: Some(account.id.clone()),
        email: account.email.clone(),
        imap_host: account.imap_host.clone(),
        imap_port: account.imap_port,
        imap_tls: account.imap_tls,
        smtp_host: account.smtp_host.clone(),
        smtp_port: account.smtp_port,
        smtp_tls: account.smtp_tls,
        password,
    }
}

fn account_config_view(account: MailAccount, password: String) -> AccountConfigView {
    AccountConfigView {
        id: account.id,
        display_name: account.display_name,
        email: account.email,
        password,
        imap_host: account.imap_host,
        imap_port: account.imap_port,
        imap_tls: account.imap_tls,
        smtp_host: account.smtp_host,
        smtp_port: account.smtp_port,
        smtp_tls: account.smtp_tls,
        sync_enabled: account.sync_enabled,
        created_at: account.created_at,
        updated_at: account.updated_at,
    }
}

fn backoff_seconds(failure_count: u32) -> i64 {
    let exponent = failure_count.saturating_sub(1).min(5);
    (60_i64 * 2_i64.pow(exponent)).min(1_800)
}

fn required_trimmed(value: String, field: &str) -> ApiResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::InvalidRequest(format!("{field} is required")));
    }
    Ok(trimmed.to_string())
}

fn normalize_email_address(value: &str) -> ApiResult<String> {
    let email = required_trimmed(value.to_string(), "email")?;
    validate_mailbox_address(&email)
        .map_err(|_| ApiError::InvalidRequest("email is invalid".to_string()))?;
    Ok(email)
}

fn require_message_uids(messages: &[MailMessage]) -> ApiResult<Vec<String>> {
    let mut uids = Vec::with_capacity(messages.len());
    for message in messages {
        let uid = message.uid.clone().ok_or_else(|| {
            ApiError::InvalidRequest(format!("message {} has no remote UID", message.id))
        })?;
        uids.push(uid);
    }
    Ok(uids)
}

fn body_preview_from_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "(empty message)".to_string();
    }
    trimmed.chars().take(180).collect()
}

fn is_sent_folder_path(path: &str) -> bool {
    matches!(
        path.to_ascii_lowercase()
            .rsplit(['/', '.'])
            .next()
            .unwrap_or_default(),
        "sent" | "sent mail" | "sent messages" | "sent items"
    )
}

fn messages_match_for_remote_delete(local: &MailMessage, remote: &MailMessage) -> bool {
    local.id == remote.id
        || local
            .message_id_header
            .as_ref()
            .zip(remote.message_id_header.as_ref())
            .map(|(left, right)| left == right)
            .unwrap_or(false)
        || (local.subject == remote.subject
            && local.sender == remote.sender
            && local.received_at == remote.received_at)
}

fn is_missing_remote_message_error(err: &ProtocolError) -> bool {
    match err {
        ProtocolError::Fetch(message) => {
            let lower = message.to_ascii_lowercase();
            lower.contains("not exist") || lower.contains("not found")
        }
        _ => false,
    }
}

fn ai_settings_view(settings: AiSettings) -> AiSettingsView {
    AiSettingsView {
        provider_name: settings.provider_name,
        base_url: settings.base_url,
        model: settings.model,
        enabled: settings.enabled,
        api_key_mask: Some(mask_api_key(&settings.api_key)),
    }
}

fn mask_api_key(api_key: &str) -> String {
    let chars: Vec<char> = api_key.chars().collect();
    if chars.len() <= 8 {
        return "****".to_string();
    }

    let prefix: String = chars.iter().take(3).collect();
    let suffix: String = chars.iter().skip(chars.len() - 4).collect();
    format!("{prefix}...{suffix}")
}

fn ai_analysis_input(message: &MailMessage) -> AiAnalysisInput {
    AiAnalysisInput {
        message_id: message.id.clone(),
        subject: message.subject.clone(),
        sender: message.sender.clone(),
        recipients: message.recipients.clone(),
        cc: message.cc.clone(),
        received_at: message.received_at.clone(),
        body_preview: message.body_preview.clone(),
        body: message.body.clone(),
        attachments: message.attachments.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_remote::{AiRemoteError, MockAiProvider};
    use async_trait::async_trait;
    use mail_core::{AiInsightPayload, AiPriority, FolderRole, SaveAiSettingsRequest};
    use mail_protocol::MockMailProtocol;
    use mail_protocol::ProtocolResult;
    use mail_store::MailStore;

    #[test]
    fn ai_settings_are_saved_and_masked() {
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(MockMailProtocol));

        let saved = api
            .save_ai_settings(SaveAiSettingsRequest {
                provider_name: "openai-compatible".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
                model: "mail-model".to_string(),
                api_key: Some("sk-local-test".to_string()),
                enabled: true,
            })
            .unwrap();

        assert_eq!(saved.api_key_mask, Some("sk-...test".to_string()));

        let loaded = api.get_ai_settings().unwrap().unwrap();
        assert_eq!(loaded.api_key_mask, Some("sk-...test".to_string()));
        assert_ne!(loaded.api_key_mask, Some("sk-local-test".to_string()));
    }

    #[test]
    fn short_ai_settings_key_is_not_reconstructable_from_mask() {
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(MockMailProtocol));

        let saved = api
            .save_ai_settings(SaveAiSettingsRequest {
                provider_name: "openai-compatible".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
                model: "mail-model".to_string(),
                api_key: Some("abcdefg".to_string()),
                enabled: true,
            })
            .unwrap();

        assert_eq!(saved.api_key_mask, Some("****".to_string()));
    }

    #[test]
    fn unicode_ai_settings_key_is_saved_and_masked() {
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(MockMailProtocol));

        let saved = api
            .save_ai_settings(SaveAiSettingsRequest {
                provider_name: "openai-compatible".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
                model: "mail-model".to_string(),
                api_key: Some("钥匙abcdefghi".to_string()),
                enabled: true,
            })
            .unwrap();

        assert_eq!(saved.api_key_mask, Some("钥匙a...fghi".to_string()));

        let loaded = api.get_ai_settings().unwrap().unwrap();
        assert_eq!(loaded.api_key_mask, Some("钥匙a...fghi".to_string()));
    }

    #[test]
    fn save_ai_settings_rejects_cleartext_remote_base_url() {
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(MockMailProtocol));

        let result = api.save_ai_settings(SaveAiSettingsRequest {
            provider_name: "openai-compatible".to_string(),
            base_url: "http://api.example.com/v1".to_string(),
            model: "mail-model".to_string(),
            api_key: Some("sk-local-test".to_string()),
            enabled: true,
        });

        match result {
            Err(ApiError::InvalidRequest(message)) => {
                assert!(message.contains("https"));
            }
            other => panic!("expected https validation error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_ai_analysis_requires_settings() {
        let api = AppApi::new_with_ai_provider(
            MailStore::memory().unwrap(),
            Arc::new(MockMailProtocol),
            Arc::new(MockAiProvider::new(ai_payload())),
        );
        let account = add_sample_account(&api).await;
        api.sync_account(account.id.clone()).await.unwrap();
        let message = first_message(&api, &account.id);

        match api.run_ai_analysis(message.id).await {
            Err(ApiError::InvalidRequest(message)) => {
                assert!(message.contains("ai settings"));
            }
            other => panic!("expected invalid request for missing ai settings, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn disabled_ai_settings_do_not_call_provider_or_store_insight() {
        let api = AppApi::new_with_ai_provider(
            MailStore::memory().unwrap(),
            Arc::new(MockMailProtocol),
            Arc::new(MockAiProvider::new(ai_payload())),
        );
        let account = add_sample_account(&api).await;
        api.sync_account(account.id.clone()).await.unwrap();
        let message = first_message(&api, &account.id);
        api.save_ai_settings(SaveAiSettingsRequest {
            provider_name: "openai-compatible".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            model: "mail-model".to_string(),
            api_key: Some("sk-local-test".to_string()),
            enabled: false,
        })
        .unwrap();

        assert!(matches!(
            api.run_ai_analysis(message.id.clone()).await,
            Err(ApiError::AiRemote(AiRemoteError::Disabled))
        ));
        assert!(api.list_ai_insights(message.id).unwrap().is_empty());
    }

    #[tokio::test]
    async fn run_ai_analysis_stores_provider_result() {
        let api = AppApi::new_with_ai_provider(
            MailStore::memory().unwrap(),
            Arc::new(MockMailProtocol),
            Arc::new(MockAiProvider::new(ai_payload())),
        );
        let account = add_sample_account(&api).await;
        api.sync_account(account.id.clone()).await.unwrap();
        let message = first_message(&api, &account.id);
        save_test_ai_settings(&api);

        let insight = api.run_ai_analysis(message.id.clone()).await.unwrap();
        let stored = api.list_ai_insights(message.id).unwrap();

        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0], insight);
        assert_eq!(stored[0].summary, "Short summary");
        assert_eq!(stored[0].priority, AiPriority::High);
    }

    #[tokio::test]
    async fn run_ai_analysis_sends_selected_message_body_to_provider() {
        let provider = RecordingAiProvider::default();
        let captured = Arc::clone(&provider.inputs);
        let api = AppApi::new_with_ai_provider(
            MailStore::memory().unwrap(),
            Arc::new(MockMailProtocol),
            Arc::new(provider),
        );
        let account = add_sample_account(&api).await;
        api.sync_account(account.id.clone()).await.unwrap();
        let message = first_message(&api, &account.id);
        save_test_ai_settings(&api);

        api.run_ai_analysis(message.id.clone()).await.unwrap();

        let inputs = captured.lock();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].message_id, message.id);
        assert_eq!(inputs[0].subject, message.subject);
        assert_eq!(inputs[0].body, message.body);
    }

    #[tokio::test]
    async fn provider_failure_does_not_store_insight() {
        let api = AppApi::new_with_ai_provider(
            MailStore::memory().unwrap(),
            Arc::new(MockMailProtocol),
            Arc::new(MockAiProvider::error(AiRemoteError::Request(
                "forced failure".to_string(),
            ))),
        );
        let account = add_sample_account(&api).await;
        api.sync_account(account.id.clone()).await.unwrap();
        let message = first_message(&api, &account.id);
        save_test_ai_settings(&api);

        assert!(api.run_ai_analysis(message.id.clone()).await.is_err());
        assert!(api.list_ai_insights(message.id).unwrap().is_empty());
    }

    #[tokio::test]
    async fn account_sync_populates_folders_and_messages() {
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(MockMailProtocol));
        let account = api
            .add_account(AddAccountRequest {
                display_name: "Ops".to_string(),
                email: "ops@example.com".to_string(),
                password: "secret".to_string(),
                imap_host: "imap.example.com".to_string(),
                imap_port: 993,
                imap_tls: true,
                smtp_host: "smtp.example.com".to_string(),
                smtp_port: 465,
                smtp_tls: true,
            })
            .await
            .unwrap();

        let summary = api.sync_account(account.id.clone()).await.unwrap();
        assert_eq!(summary.folders, 5);
        assert_eq!(summary.messages, 7);

        let messages = api
            .list_messages(MessageQuery {
                account_id: Some(account.id.clone()),
                folder_id: None,
                limit: 10,
                offset: 0,
            })
            .unwrap();
        assert_eq!(messages.len(), 7);

        let archive = api
            .store
            .find_folder_by_role(&account.id, FolderRole::Archive)
            .unwrap()
            .unwrap();
        let archive_messages = api
            .list_messages(MessageQuery {
                account_id: Some(account.id.clone()),
                folder_id: Some(archive.id.clone()),
                limit: 10,
                offset: 0,
            })
            .unwrap();
        assert_eq!(archive_messages.len(), 1);

        let states = api.get_sync_status(account.id).unwrap();
        assert!(states.iter().any(
            |state| state.folder_id.as_deref() == Some(archive.id.as_str())
                && state.last_uid.as_deref() == Some("3001")
        ));
    }

    #[test]
    fn sync_lock_blocks_duplicate_account_sync() {
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(MockMailProtocol));

        let guard = api.acquire_sync_lock("acct").unwrap();
        match api.acquire_sync_lock("acct") {
            Err(ApiError::SyncAlreadyRunning(account_id)) => assert_eq!(account_id, "acct"),
            _ => panic!("expected duplicate sync lock error"),
        }
        drop(guard);
        assert!(api.acquire_sync_lock("acct").is_ok());
    }

    #[tokio::test]
    async fn account_config_round_trips_plaintext_sqlite_password() {
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(MockMailProtocol));

        let account = api
            .save_account_config(SaveAccountConfigRequest {
                id: None,
                display_name: "Ops".to_string(),
                email: "ops@example.com".to_string(),
                password: "sqlite-secret".to_string(),
                imap_host: "imap.example.com".to_string(),
                imap_port: 993,
                imap_tls: true,
                smtp_host: "smtp.example.com".to_string(),
                smtp_port: 465,
                smtp_tls: true,
                sync_enabled: true,
            })
            .await
            .unwrap();

        let saved = api.get_account_config(account.id.clone()).unwrap();
        assert_eq!(saved.password, "sqlite-secret");

        api.save_account_config(SaveAccountConfigRequest {
            id: Some(account.id.clone()),
            display_name: "Ops Mail".to_string(),
            email: "ops@example.com".to_string(),
            password: "updated-secret".to_string(),
            imap_host: "imap.mail.example.com".to_string(),
            imap_port: 993,
            imap_tls: true,
            smtp_host: "smtp.mail.example.com".to_string(),
            smtp_port: 587,
            smtp_tls: true,
            sync_enabled: true,
        })
        .await
        .unwrap();

        let updated = api.get_account_config(account.id).unwrap();
        assert_eq!(updated.display_name, "Ops Mail");
        assert_eq!(updated.smtp_port, 587);
        assert_eq!(updated.password, "updated-secret");
    }

    #[tokio::test]
    async fn save_account_config_does_not_test_connection() {
        let protocol = CountingConnectionProtocol::default();
        let test_calls = Arc::clone(&protocol.test_calls);
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));

        let account = api
            .save_account_config(SaveAccountConfigRequest {
                id: None,
                display_name: "Ops".to_string(),
                email: "ops@example.com".to_string(),
                password: "secret".to_string(),
                imap_host: "imap.invalid.example".to_string(),
                imap_port: 993,
                imap_tls: true,
                smtp_host: "smtp.invalid.example".to_string(),
                smtp_port: 465,
                smtp_tls: true,
                sync_enabled: true,
            })
            .await
            .unwrap();

        assert_eq!(test_calls.lock().len(), 0);
        assert_eq!(
            api.get_account_config(account.id).unwrap().password,
            "secret"
        );
    }

    #[tokio::test]
    async fn manual_test_account_connection_uses_protocol() {
        let protocol = CountingConnectionProtocol::default();
        let test_calls = Arc::clone(&protocol.test_calls);
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));

        let result = api
            .test_account_connection(TestConnectionRequest {
                account_id: None,
                manual: Some(SaveAccountConfigRequest {
                    id: None,
                    display_name: "Ops".to_string(),
                    email: "ops@example.com".to_string(),
                    password: "secret".to_string(),
                    imap_host: "imap.example.com".to_string(),
                    imap_port: 993,
                    imap_tls: true,
                    smtp_host: "smtp.example.com".to_string(),
                    smtp_port: 465,
                    smtp_tls: true,
                    sync_enabled: true,
                }),
            })
            .await
            .unwrap();

        assert!(result.imap_ok);
        assert_eq!(test_calls.lock().len(), 1);
    }

    #[tokio::test]
    async fn watch_folder_changed_returns_changed_and_marks_folder_watching() {
        let protocol = WatchProtocol {
            outcome: Ok(mail_core::FolderWatchOutcome::Changed),
            ..WatchProtocol::default()
        };
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;
        api.sync_account(account.id.clone()).await.unwrap();
        let inbox = api
            .store
            .find_folder_by_role(&account.id, FolderRole::Inbox)
            .unwrap()
            .unwrap();

        let outcome = api
            .watch_folder_until_change(account.id.clone(), inbox.id.clone())
            .await
            .unwrap();

        assert_eq!(outcome, mail_core::FolderWatchOutcome::Changed);
        let state = api
            .store
            .get_sync_state(&account.id, Some(&inbox.id))
            .unwrap()
            .unwrap();
        assert_eq!(state.state, SyncStateKind::Watching);
    }

    #[tokio::test]
    async fn watch_folder_error_enters_folder_backoff() {
        let protocol = WatchProtocol {
            outcome: Err(ProtocolError::Unsupported("idle unavailable".to_string())),
            ..WatchProtocol::default()
        };
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;
        api.sync_account(account.id.clone()).await.unwrap();
        let inbox = api
            .store
            .find_folder_by_role(&account.id, FolderRole::Inbox)
            .unwrap()
            .unwrap();

        assert!(api
            .watch_folder_until_change(account.id.clone(), inbox.id.clone())
            .await
            .is_err());

        let state = api
            .store
            .get_sync_state(&account.id, Some(&inbox.id))
            .unwrap()
            .unwrap();
        assert_eq!(state.state, SyncStateKind::Backoff);
        assert_eq!(state.failure_count, 1);
        assert!(state.backoff_until.is_some());
    }

    #[tokio::test]
    async fn sync_failure_enters_backoff_and_blocks_next_attempt() {
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(FailingFetchProtocol));
        let account = api
            .add_account(AddAccountRequest {
                display_name: "Ops".to_string(),
                email: "ops@example.com".to_string(),
                password: "secret".to_string(),
                imap_host: "imap.example.com".to_string(),
                imap_port: 993,
                imap_tls: true,
                smtp_host: "smtp.example.com".to_string(),
                smtp_port: 465,
                smtp_tls: true,
            })
            .await
            .unwrap();

        assert!(api.sync_account(account.id.clone()).await.is_err());
        let state = api
            .store
            .get_sync_state(&account.id, None)
            .unwrap()
            .unwrap();
        assert_eq!(state.state, SyncStateKind::Backoff);
        assert_eq!(state.failure_count, 1);
        assert!(state.backoff_until.is_some());

        assert!(matches!(
            api.sync_account(account.id.clone()).await.unwrap_err(),
            ApiError::SyncBackoff { .. }
        ));
    }

    #[tokio::test]
    async fn folder_fetch_failure_does_not_block_other_folders() {
        let api = AppApi::new(
            MailStore::memory().unwrap(),
            Arc::new(PartialFailingFetchProtocol),
        );
        let account = add_sample_account(&api).await;

        let summary = api.sync_account(account.id.clone()).await.unwrap();
        assert_eq!(summary.folders, 2);
        assert_eq!(summary.messages, 1);

        let states = api.get_sync_status(account.id.clone()).unwrap();
        let inbox_state = states
            .iter()
            .find(|state| state.folder_id.as_deref() == Some(&format!("{}:inbox", account.id)))
            .unwrap();
        let archive_state = states
            .iter()
            .find(|state| state.folder_id.as_deref() == Some(&format!("{}:archive", account.id)))
            .unwrap();
        assert_eq!(inbox_state.state, SyncStateKind::Idle);
        assert_eq!(archive_state.state, SyncStateKind::Backoff);
        assert_eq!(archive_state.failure_count, 1);

        let messages = api
            .list_messages(MessageQuery {
                account_id: Some(account.id),
                folder_id: None,
                limit: 10,
                offset: 0,
            })
            .unwrap();
        assert_eq!(messages.len(), 1);
    }

    #[tokio::test]
    async fn sync_recomputes_folder_counts_from_stored_messages() {
        let api = AppApi::new(
            MailStore::memory().unwrap(),
            Arc::new(PartialFailingFetchProtocol),
        );
        let account = add_sample_account(&api).await;

        api.sync_account(account.id.clone()).await.unwrap();

        let inbox = api
            .store
            .find_folder_by_role(&account.id, FolderRole::Inbox)
            .unwrap()
            .unwrap();
        assert_eq!(inbox.total_count, 1);
        assert_eq!(inbox.unread_count, 1);
    }

    #[tokio::test]
    async fn low_risk_action_calls_protocol_then_updates_local_state() {
        let protocol = RecordingProtocol::default();
        let actions = Arc::clone(&protocol.actions);
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;
        api.sync_account(account.id.clone()).await.unwrap();
        let message = first_message(&api, &account.id);

        let result = api
            .execute_mail_action(MailActionRequest {
                action: MailActionKind::MarkRead,
                account_id: account.id.clone(),
                message_ids: vec![message.id.clone()],
                target_folder_id: None,
            })
            .await
            .unwrap();

        assert_eq!(result.kind, MailActionResultKind::Executed);
        assert_eq!(actions.lock().len(), 1);
        assert_eq!(actions.lock()[0].action, MailActionKind::MarkRead);
        assert_eq!(actions.lock()[0].uids, vec!["42".to_string()]);
        assert!(api.store.get_message(&message.id).unwrap().flags.is_read);
    }

    #[tokio::test]
    async fn confirmed_action_refreshes_folder_counts() {
        let api = AppApi::new(
            MailStore::memory().unwrap(),
            Arc::new(RecordingProtocol::default()),
        );
        let account = add_sample_account(&api).await;
        api.sync_account(account.id.clone()).await.unwrap();
        let message = first_message(&api, &account.id);
        let inbox_id = message.folder_id.clone();

        api.execute_mail_action(MailActionRequest {
            action: MailActionKind::MarkRead,
            account_id: account.id.clone(),
            message_ids: vec![message.id],
            target_folder_id: None,
        })
        .await
        .unwrap();

        let inbox = api.store.get_folder(&inbox_id).unwrap();
        assert_eq!(inbox.total_count, 1);
        assert_eq!(inbox.unread_count, 0);
    }

    #[tokio::test]
    async fn remote_action_failure_does_not_mutate_local_message() {
        let protocol = RecordingProtocol {
            fail_actions: true,
            ..RecordingProtocol::default()
        };
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;
        api.sync_account(account.id.clone()).await.unwrap();
        let message = first_message(&api, &account.id);

        assert!(api
            .execute_mail_action(MailActionRequest {
                action: MailActionKind::MarkRead,
                account_id: account.id.clone(),
                message_ids: vec![message.id.clone()],
                target_folder_id: None,
            })
            .await
            .is_err());

        assert!(!api.store.get_message(&message.id).unwrap().flags.is_read);
        assert_eq!(
            api.get_audit_log(Some(1)).unwrap()[0].status,
            ActionAuditStatus::Failed
        );
    }

    #[tokio::test]
    async fn invalid_account_email_is_rejected_before_save() {
        let api = AppApi::new(
            MailStore::memory().unwrap(),
            Arc::new(RecordingProtocol::default()),
        );

        let err = api
            .add_account(AddAccountRequest {
                display_name: "Ops".to_string(),
                email: "app test".to_string(),
                password: "secret".to_string(),
                imap_host: "imap.example.com".to_string(),
                imap_port: 993,
                imap_tls: true,
                smtp_host: "smtp.example.com".to_string(),
                smtp_port: 465,
                smtp_tls: true,
            })
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "invalid request: email is invalid");
        assert!(api.list_accounts().unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_moves_to_trash_and_clears_local_uid() {
        let protocol = RecordingProtocol::default();
        let actions = Arc::clone(&protocol.actions);
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;
        api.sync_account(account.id.clone()).await.unwrap();
        let message = first_message(&api, &account.id);

        api.execute_mail_action(MailActionRequest {
            action: MailActionKind::Delete,
            account_id: account.id.clone(),
            message_ids: vec![message.id.clone()],
            target_folder_id: None,
        })
        .await
        .unwrap();

        let updated = api.store.get_message(&message.id).unwrap();
        let trash = api
            .store
            .find_folder_by_role(&account.id, FolderRole::Trash)
            .unwrap()
            .unwrap();
        assert_eq!(updated.folder_id, trash.id);
        assert_eq!(updated.uid, None);
        assert_eq!(
            actions.lock()[0].target_folder.as_ref().unwrap().role,
            FolderRole::Trash
        );
    }

    #[tokio::test]
    async fn delete_to_trash_remote_sync_hydrates_local_placeholder() {
        let protocol = RecordingProtocol::default();
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;
        api.sync_account(account.id.clone()).await.unwrap();
        let message = first_message(&api, &account.id);

        api.execute_mail_action(MailActionRequest {
            action: MailActionKind::Delete,
            account_id: account.id.clone(),
            message_ids: vec![message.id.clone()],
            target_folder_id: None,
        })
        .await
        .unwrap();

        let trash = api
            .store
            .find_folder_by_role(&account.id, FolderRole::Trash)
            .unwrap()
            .unwrap();
        api.store
            .upsert_message(&MailMessage {
                id: format!("{}:{}:900", account.id, trash.id),
                account_id: account.id.clone(),
                folder_id: trash.id.clone(),
                uid: Some("900".to_string()),
                message_id_header: message.message_id_header.clone(),
                subject: message.subject.clone(),
                sender: message.sender.clone(),
                recipients: message.recipients.clone(),
                cc: message.cc.clone(),
                received_at: message.received_at.clone(),
                body_preview: message.body_preview.clone(),
                body: message.body.clone(),
                attachments: Vec::new(),
                flags: message.flags.clone(),
                size_bytes: message.size_bytes,
                deleted_at: None,
            })
            .unwrap();

        let trash_messages = api
            .list_messages(MessageQuery {
                account_id: Some(account.id.clone()),
                folder_id: Some(trash.id),
                limit: 10,
                offset: 0,
            })
            .unwrap();

        assert_eq!(trash_messages.len(), 1);
        assert_eq!(trash_messages[0].id, message.id);
        assert_eq!(trash_messages[0].uid.as_deref(), Some("900"));
        assert_eq!(
            api.store.get_message(&message.id).unwrap().uid.as_deref(),
            Some("900")
        );
    }

    #[tokio::test]
    async fn uid_resync_after_trash_hydration_updates_search_for_preserved_row() {
        let protocol = RecordingProtocol::default();
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;
        api.sync_account(account.id.clone()).await.unwrap();
        let message = first_message(&api, &account.id);

        api.execute_mail_action(MailActionRequest {
            action: MailActionKind::Delete,
            account_id: account.id.clone(),
            message_ids: vec![message.id.clone()],
            target_folder_id: None,
        })
        .await
        .unwrap();

        let trash = api
            .store
            .find_folder_by_role(&account.id, FolderRole::Trash)
            .unwrap()
            .unwrap();
        let mut remote = message.clone();
        remote.id = format!("{}:{}:900", account.id, trash.id);
        remote.folder_id = trash.id.clone();
        remote.uid = Some("900".to_string());
        remote.body_preview = "first trash sync body".to_string();
        remote.body = Some("first trash sync body".to_string());
        api.store.upsert_message(&remote).unwrap();

        remote.id = format!("{}:{}:900:second", account.id, trash.id);
        remote.body_preview = "second trash sync searchable marker".to_string();
        remote.body = Some("second trash sync searchable marker".to_string());
        api.store.upsert_message(&remote).unwrap();

        let updated = api.store.get_message(&message.id).unwrap();
        assert_eq!(updated.uid.as_deref(), Some("900"));
        assert_eq!(
            updated.body.as_deref(),
            Some("second trash sync searchable marker")
        );

        let hits = api.search_messages("second".to_string(), Some(10)).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, message.id);
        assert!(api
            .search_messages("first".to_string(), Some(10))
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn delete_inside_trash_queues_and_confirms_permanent_delete() {
        let protocol = RecordingProtocol::default();
        let actions = Arc::clone(&protocol.actions);
        let trash_refetch_uid = Arc::clone(&protocol.trash_refetch_uid);
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;
        api.sync_account(account.id.clone()).await.unwrap();
        let message = first_message(&api, &account.id);

        api.execute_mail_action(MailActionRequest {
            action: MailActionKind::Delete,
            account_id: account.id.clone(),
            message_ids: vec![message.id.clone()],
            target_folder_id: None,
        })
        .await
        .unwrap();
        assert_eq!(api.store.get_message(&message.id).unwrap().uid, None);
        *trash_refetch_uid.lock() = Some("900".to_string());

        let result = api
            .execute_mail_action(MailActionRequest {
                action: MailActionKind::Delete,
                account_id: account.id.clone(),
                message_ids: vec![message.id.clone()],
                target_folder_id: None,
            })
            .await
            .unwrap();

        assert_eq!(result.kind, MailActionResultKind::Pending);
        let pending_id = result.pending_action_id.unwrap();
        let pending = api.list_pending_actions(Some(account.id.clone())).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, pending_id);
        assert_eq!(pending[0].action, MailActionKind::PermanentDelete);

        api.confirm_action(pending_id).await.unwrap();

        let recorded = actions.lock();
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[1].action, MailActionKind::PermanentDelete);
        assert_eq!(recorded[1].source_folder.role, FolderRole::Trash);
        assert_eq!(recorded[1].target_folder, None);
        assert_eq!(recorded[1].uids, vec!["900".to_string()]);
        drop(recorded);

        assert!(api
            .store
            .get_message(&message.id)
            .unwrap()
            .deleted_at
            .is_some());
    }

    #[tokio::test]
    async fn send_message_queues_until_confirmed() {
        let protocol = RecordingProtocol::default();
        let sends = Arc::clone(&protocol.sends);
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;
        save_sent_folder(&api, &account);

        let pending_id = api
            .send_message(SendMessageDraft {
                account_id: account.id.clone(),
                to: vec!["sec@example.com".to_string()],
                cc: Vec::new(),
                subject: "Confirm send".to_string(),
                body: "body".to_string(),
                message_id_header: None,
            })
            .await
            .unwrap();

        assert!(sends.lock().is_empty());
        let pending = api.list_pending_actions(Some(account.id.clone())).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, pending_id);
        assert_eq!(pending[0].status, PendingActionStatus::Pending);

        api.confirm_action(pending_id).await.unwrap();
        assert_eq!(sends.lock().len(), 1);
        assert!(api
            .list_pending_actions(Some(account.id))
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn send_message_creates_local_sent_placeholder_until_confirmed() {
        let protocol = RecordingProtocol::default();
        let sends = Arc::clone(&protocol.sends);
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;
        let sent = save_sent_folder(&api, &account);

        let pending_id = api
            .send_message(SendMessageDraft {
                account_id: account.id.clone(),
                to: vec!["sec@example.com".to_string()],
                cc: vec!["ops-lead@example.com".to_string()],
                subject: "Confirm send".to_string(),
                body: "body\nwith local visibility".to_string(),
                message_id_header: None,
            })
            .await
            .unwrap();

        assert!(sends.lock().is_empty());
        assert!(!pending_id.is_empty());

        let sent_messages = api
            .list_messages(MessageQuery {
                account_id: Some(account.id.clone()),
                folder_id: Some(sent.id.clone()),
                limit: 10,
                offset: 0,
            })
            .unwrap();

        assert_eq!(sent_messages.len(), 1);
        assert_eq!(sent_messages[0].uid, None);
        assert_eq!(sent_messages[0].sender, account.email);
        assert_eq!(sent_messages[0].recipients, vec!["sec@example.com"]);
        assert_eq!(sent_messages[0].cc, vec!["ops-lead@example.com"]);
        assert_eq!(sent_messages[0].subject, "Confirm send");
        assert_eq!(
            sent_messages[0].body.as_deref(),
            Some("body\nwith local visibility")
        );
        assert!(sent_messages[0]
            .body_preview
            .contains("body\nwith local visibility"));
        assert!(sent_messages[0].flags.is_read);
        assert!(sent_messages[0].flags.is_answered);

        let updated_sent = api.store.get_folder(&sent.id).unwrap();
        assert_eq!(updated_sent.total_count, 1);
        assert_eq!(updated_sent.unread_count, 0);
    }

    #[tokio::test]
    async fn send_message_creates_fallback_sent_folder_for_local_placeholder() {
        let protocol = RecordingProtocol::default();
        let sends = Arc::clone(&protocol.sends);
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;

        let pending_id = api
            .send_message(SendMessageDraft {
                account_id: account.id.clone(),
                to: vec!["sec@example.com".to_string()],
                cc: Vec::new(),
                subject: "Fallback sent".to_string(),
                body: "body".to_string(),
                message_id_header: None,
            })
            .await
            .unwrap();

        assert!(sends.lock().is_empty());
        assert!(!pending_id.is_empty());
        let sent = api
            .store
            .find_folder_by_role(&account.id, FolderRole::Sent)
            .unwrap()
            .unwrap();
        assert_eq!(sent.id, format!("{}:sent", account.id));
        assert_eq!(sent.path, "Sent");
        assert_eq!(sent.total_count, 1);
    }

    #[tokio::test]
    async fn reject_pending_send_soft_deletes_local_sent_placeholder() {
        let protocol = RecordingProtocol::default();
        let sends = Arc::clone(&protocol.sends);
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;
        let sent = save_sent_folder(&api, &account);

        let pending_id = api
            .send_message(SendMessageDraft {
                account_id: account.id.clone(),
                to: vec!["sec@example.com".to_string()],
                cc: Vec::new(),
                subject: "Reject send".to_string(),
                body: "body".to_string(),
                message_id_header: None,
            })
            .await
            .unwrap();

        api.reject_action(pending_id).unwrap();

        assert!(sends.lock().is_empty());
        assert!(api
            .list_messages(MessageQuery {
                account_id: Some(account.id.clone()),
                folder_id: Some(sent.id.clone()),
                limit: 10,
                offset: 0,
            })
            .unwrap()
            .is_empty());
        let updated_sent = api.store.get_folder(&sent.id).unwrap();
        assert_eq!(updated_sent.total_count, 0);
        assert_eq!(updated_sent.unread_count, 0);
    }

    #[tokio::test]
    async fn reject_pending_send_keeps_hydrated_sent_placeholder_visible() {
        let protocol = RecordingProtocol::default();
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;
        let sent = save_sent_folder(&api, &account);

        let pending_id = api
            .send_message(SendMessageDraft {
                account_id: account.id.clone(),
                to: vec!["sec@example.com".to_string()],
                cc: Vec::new(),
                subject: "Reject hydrated send".to_string(),
                body: "body".to_string(),
                message_id_header: None,
            })
            .await
            .unwrap();
        let pending = api.store.get_pending_action(&pending_id).unwrap();
        let mut remote = api
            .store
            .get_message(pending.local_message_id.as_deref().unwrap())
            .unwrap();
        remote.id = format!("{}:{}:777", account.id, sent.id);
        remote.uid = Some("777".to_string());
        api.store.upsert_message(&remote).unwrap();

        api.reject_action(pending_id).unwrap();

        let sent_messages = api
            .list_messages(MessageQuery {
                account_id: Some(account.id.clone()),
                folder_id: Some(sent.id.clone()),
                limit: 10,
                offset: 0,
            })
            .unwrap();
        assert_eq!(sent_messages.len(), 1);
        assert_eq!(sent_messages[0].uid.as_deref(), Some("777"));
        assert_eq!(api.store.get_folder(&sent.id).unwrap().total_count, 1);
    }

    #[tokio::test]
    async fn reject_pending_send_keeps_wrongly_linked_non_sent_message_visible() {
        let protocol = RecordingProtocol::default();
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;
        api.sync_account(account.id.clone()).await.unwrap();
        save_sent_folder(&api, &account);
        let inbox = api
            .store
            .find_folder_by_role(&account.id, FolderRole::Inbox)
            .unwrap()
            .unwrap();

        let pending_id = api
            .send_message(SendMessageDraft {
                account_id: account.id.clone(),
                to: vec!["sec@example.com".to_string()],
                cc: Vec::new(),
                subject: "Wrong link send".to_string(),
                body: "body".to_string(),
                message_id_header: None,
            })
            .await
            .unwrap();
        let pending = api.store.get_pending_action(&pending_id).unwrap();
        let linked_id = pending.local_message_id.unwrap();
        api.store
            .move_messages_and_clear_uids(std::slice::from_ref(&linked_id), &inbox.id)
            .unwrap();
        api.store.refresh_folder_counts(&inbox.id).unwrap();

        api.reject_action(pending_id).unwrap();

        let linked = api.store.get_message(&linked_id).unwrap();
        assert_eq!(linked.folder_id, inbox.id);
        assert!(linked.deleted_at.is_none());
        assert_eq!(api.store.get_folder(&inbox.id).unwrap().total_count, 2);
    }

    #[tokio::test]
    async fn sent_sync_moves_fallback_placeholder_to_real_sent_folder_before_hydration() {
        let sent_message_id = Arc::new(Mutex::new(None));
        let protocol = SentConvergenceProtocol {
            sent_message_id: Arc::clone(&sent_message_id),
        };
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;

        let pending_id = api
            .send_message(SendMessageDraft {
                account_id: account.id.clone(),
                to: vec!["sec@example.com".to_string()],
                cc: Vec::new(),
                subject: "Converge sent".to_string(),
                body: "body".to_string(),
                message_id_header: None,
            })
            .await
            .unwrap();
        let pending = api.store.get_pending_action(&pending_id).unwrap();
        *sent_message_id.lock() = pending.draft.unwrap().message_id_header;

        api.sync_account(account.id.clone()).await.unwrap();

        let real_sent = api
            .store
            .get_folder(&format!("{}:gmail--sent-mail", account.id))
            .unwrap();
        let real_sent_messages = api
            .list_messages(MessageQuery {
                account_id: Some(account.id.clone()),
                folder_id: Some(real_sent.id.clone()),
                limit: 10,
                offset: 0,
            })
            .unwrap();
        assert_eq!(real_sent_messages.len(), 1);
        assert_eq!(real_sent_messages[0].id, pending.local_message_id.unwrap());
        assert_eq!(real_sent_messages[0].uid.as_deref(), Some("501"));
        assert_eq!(
            api.store
                .get_folder(&format!("{}:sent", account.id))
                .unwrap()
                .total_count,
            0
        );
        assert_eq!(real_sent.total_count, 1);
    }

    #[tokio::test]
    async fn confirm_send_reconciles_remote_sent_copy_without_waiting_for_polling() {
        let sent_message_id = Arc::new(Mutex::new(None));
        let protocol = SentConvergenceProtocol {
            sent_message_id: Arc::clone(&sent_message_id),
        };
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;

        let pending_id = api
            .send_message(SendMessageDraft {
                account_id: account.id.clone(),
                to: vec!["sec@example.com".to_string()],
                cc: Vec::new(),
                subject: "Confirm and reconcile".to_string(),
                body: "body".to_string(),
                message_id_header: None,
            })
            .await
            .unwrap();
        let pending = api.store.get_pending_action(&pending_id).unwrap();
        *sent_message_id.lock() = pending.draft.unwrap().message_id_header;

        api.confirm_action(pending_id).await.unwrap();

        let real_sent = api
            .store
            .get_folder(&format!("{}:gmail--sent-mail", account.id))
            .unwrap();
        let real_sent_messages = api
            .list_messages(MessageQuery {
                account_id: Some(account.id.clone()),
                folder_id: Some(real_sent.id.clone()),
                limit: 10,
                offset: 0,
            })
            .unwrap();
        assert_eq!(real_sent_messages.len(), 1);
        assert_eq!(real_sent_messages[0].uid.as_deref(), Some("501"));
        assert_eq!(real_sent_messages[0].subject, "Converge sent");
        assert_eq!(
            api.store
                .get_folder(&format!("{}:sent", account.id))
                .unwrap()
                .total_count,
            0
        );
        assert_eq!(real_sent.total_count, 1);
    }

    #[tokio::test]
    async fn confirm_send_passes_generated_message_id_header_to_protocol() {
        let protocol = RecordingProtocol::default();
        let sends = Arc::clone(&protocol.sends);
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;
        save_sent_folder(&api, &account);

        let pending_id = api
            .send_message(SendMessageDraft {
                account_id: account.id.clone(),
                to: vec!["sec@example.com".to_string()],
                cc: Vec::new(),
                subject: "Confirm send".to_string(),
                body: "body".to_string(),
                message_id_header: None,
            })
            .await
            .unwrap();
        let pending = api.store.get_pending_action(&pending_id).unwrap();
        let expected_message_id = pending.draft.unwrap().message_id_header.unwrap();

        api.confirm_action(pending_id).await.unwrap();

        let sends = sends.lock();
        assert_eq!(sends.len(), 1);
        assert_eq!(
            sends[0].message_id_header.as_deref(),
            Some(expected_message_id.as_str())
        );
    }

    #[tokio::test]
    async fn failed_send_confirmation_soft_deletes_local_sent_placeholder() {
        let protocol = RecordingProtocol {
            fail_sends: true,
            ..RecordingProtocol::default()
        };
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;
        let sent = save_sent_folder(&api, &account);

        let pending_id = api
            .send_message(SendMessageDraft {
                account_id: account.id.clone(),
                to: vec!["sec@example.com".to_string()],
                cc: Vec::new(),
                subject: "Fail send".to_string(),
                body: "body".to_string(),
                message_id_header: None,
            })
            .await
            .unwrap();

        assert!(api.confirm_action(pending_id).await.is_err());

        assert!(api
            .list_messages(MessageQuery {
                account_id: Some(account.id.clone()),
                folder_id: Some(sent.id.clone()),
                limit: 10,
                offset: 0,
            })
            .unwrap()
            .is_empty());
        assert_eq!(api.store.get_folder(&sent.id).unwrap().total_count, 0);
    }

    #[tokio::test]
    async fn reject_pending_action_does_not_execute_protocol() {
        let protocol = RecordingProtocol::default();
        let sends = Arc::clone(&protocol.sends);
        let api = AppApi::new(MailStore::memory().unwrap(), Arc::new(protocol));
        let account = add_sample_account(&api).await;
        save_sent_folder(&api, &account);

        let pending_id = api
            .send_message(SendMessageDraft {
                account_id: account.id.clone(),
                to: vec!["sec@example.com".to_string()],
                cc: Vec::new(),
                subject: "Reject send".to_string(),
                body: "body".to_string(),
                message_id_header: None,
            })
            .await
            .unwrap();

        api.reject_action(pending_id).unwrap();
        assert!(sends.lock().is_empty());
        assert!(api
            .list_pending_actions(Some(account.id))
            .unwrap()
            .is_empty());
    }

    async fn add_sample_account(api: &AppApi) -> MailAccount {
        api.add_account(AddAccountRequest {
            display_name: "Ops".to_string(),
            email: "ops@example.com".to_string(),
            password: "secret".to_string(),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_tls: true,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_tls: true,
        })
        .await
        .unwrap()
    }

    fn save_sent_folder(api: &AppApi, account: &MailAccount) -> MailFolder {
        let folder = MailFolder {
            id: format!("{}:sent", account.id),
            account_id: account.id.clone(),
            name: "Sent".to_string(),
            path: "Sent".to_string(),
            role: FolderRole::Sent,
            unread_count: 0,
            total_count: 0,
        };
        api.store.save_folder(&folder).unwrap();
        folder
    }

    fn first_message(api: &AppApi, account_id: &str) -> MailMessage {
        api.list_messages(MessageQuery {
            account_id: Some(account_id.to_string()),
            folder_id: None,
            limit: 10,
            offset: 0,
        })
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
    }

    fn save_test_ai_settings(api: &AppApi) {
        api.save_ai_settings(SaveAiSettingsRequest {
            provider_name: "openai-compatible".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            model: "mail-model".to_string(),
            api_key: Some("sk-local-test".to_string()),
            enabled: true,
        })
        .unwrap();
    }

    fn ai_payload() -> AiInsightPayload {
        AiInsightPayload {
            summary: "Short summary".to_string(),
            category: "operations".to_string(),
            priority: AiPriority::High,
            todos: vec!["Reply before 18:00".to_string()],
            reply_draft: "Acknowledged.".to_string(),
            raw_json: "{\"summary\":\"Short summary\"}".to_string(),
        }
    }

    #[derive(Default)]
    struct RecordingAiProvider {
        inputs: Arc<Mutex<Vec<AiAnalysisInput>>>,
    }

    #[async_trait]
    impl AiProvider for RecordingAiProvider {
        async fn analyze_mail(
            &self,
            _settings: &AiSettings,
            input: &AiAnalysisInput,
        ) -> Result<AiInsightPayload, AiRemoteError> {
            self.inputs.lock().push(input.clone());
            Ok(ai_payload())
        }
    }

    struct RecordingProtocol {
        actions: Arc<Mutex<Vec<RemoteMailAction>>>,
        sends: Arc<Mutex<Vec<SendMessageDraft>>>,
        fail_actions: bool,
        fail_sends: bool,
        trash_refetch_uid: Arc<Mutex<Option<String>>>,
    }

    impl Default for RecordingProtocol {
        fn default() -> Self {
            Self {
                actions: Arc::new(Mutex::new(Vec::new())),
                sends: Arc::new(Mutex::new(Vec::new())),
                fail_actions: false,
                fail_sends: false,
                trash_refetch_uid: Arc::new(Mutex::new(None)),
            }
        }
    }

    #[derive(Default)]
    struct CountingConnectionProtocol {
        test_calls: Arc<Mutex<Vec<ConnectionSettings>>>,
    }

    #[async_trait]
    impl MailProtocol for CountingConnectionProtocol {
        async fn test_connection(
            &self,
            settings: &ConnectionSettings,
        ) -> ProtocolResult<ConnectionTestResult> {
            self.test_calls.lock().push(settings.clone());
            Ok(ConnectionTestResult {
                imap_ok: true,
                smtp_ok: true,
                message: "ok".to_string(),
            })
        }

        async fn fetch_folders(
            &self,
            _settings: &ConnectionSettings,
            _account: &MailAccount,
        ) -> ProtocolResult<Vec<MailFolder>> {
            Ok(Vec::new())
        }

        async fn fetch_messages(
            &self,
            _settings: &ConnectionSettings,
            _account: &MailAccount,
            _folder: &MailFolder,
            _request: &MessageFetchRequest,
        ) -> ProtocolResult<Vec<MailMessage>> {
            Ok(Vec::new())
        }

        async fn send_message(
            &self,
            _settings: &ConnectionSettings,
            _draft: &SendMessageDraft,
        ) -> ProtocolResult<String> {
            Ok(new_id())
        }

        async fn apply_action(
            &self,
            _settings: &ConnectionSettings,
            _account: &MailAccount,
            _action: &RemoteMailAction,
        ) -> ProtocolResult<()> {
            Ok(())
        }
    }

    struct WatchProtocol {
        outcome: ProtocolResult<mail_core::FolderWatchOutcome>,
    }

    impl Default for WatchProtocol {
        fn default() -> Self {
            Self {
                outcome: Ok(mail_core::FolderWatchOutcome::Timeout),
            }
        }
    }

    struct SentConvergenceProtocol {
        sent_message_id: Arc<Mutex<Option<String>>>,
    }

    #[async_trait]
    impl MailProtocol for SentConvergenceProtocol {
        async fn test_connection(
            &self,
            _settings: &ConnectionSettings,
        ) -> ProtocolResult<ConnectionTestResult> {
            Ok(ConnectionTestResult {
                imap_ok: true,
                smtp_ok: true,
                message: "ok".to_string(),
            })
        }

        async fn fetch_folders(
            &self,
            _settings: &ConnectionSettings,
            account: &MailAccount,
        ) -> ProtocolResult<Vec<MailFolder>> {
            Ok(vec![MailFolder {
                id: format!("{}:gmail--sent-mail", account.id),
                account_id: account.id.clone(),
                name: "Sent Mail".to_string(),
                path: "[Gmail]/Sent Mail".to_string(),
                role: FolderRole::Sent,
                unread_count: 0,
                total_count: 0,
            }])
        }

        async fn fetch_messages(
            &self,
            _settings: &ConnectionSettings,
            account: &MailAccount,
            folder: &MailFolder,
            _request: &MessageFetchRequest,
        ) -> ProtocolResult<Vec<MailMessage>> {
            let Some(message_id_header) = self.sent_message_id.lock().clone() else {
                return Ok(Vec::new());
            };
            Ok(vec![MailMessage {
                id: format!("{}:{}:501", account.id, folder.id),
                account_id: account.id.clone(),
                folder_id: folder.id.clone(),
                uid: Some("501".to_string()),
                message_id_header: Some(message_id_header),
                subject: "Converge sent".to_string(),
                sender: account.email.clone(),
                recipients: vec!["sec@example.com".to_string()],
                cc: Vec::new(),
                received_at: now_rfc3339(),
                body_preview: "body".to_string(),
                body: Some("body".to_string()),
                attachments: Vec::new(),
                flags: mail_core::MessageFlags {
                    is_read: true,
                    is_starred: false,
                    is_answered: true,
                    is_forwarded: false,
                },
                size_bytes: Some(128),
                deleted_at: None,
            }])
        }

        async fn send_message(
            &self,
            _settings: &ConnectionSettings,
            _draft: &SendMessageDraft,
        ) -> ProtocolResult<String> {
            Ok(new_id())
        }

        async fn apply_action(
            &self,
            _settings: &ConnectionSettings,
            _account: &MailAccount,
            _action: &RemoteMailAction,
        ) -> ProtocolResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl MailProtocol for WatchProtocol {
        async fn test_connection(
            &self,
            _settings: &ConnectionSettings,
        ) -> ProtocolResult<ConnectionTestResult> {
            Ok(ConnectionTestResult {
                imap_ok: true,
                smtp_ok: true,
                message: "ok".to_string(),
            })
        }

        async fn fetch_folders(
            &self,
            _settings: &ConnectionSettings,
            account: &MailAccount,
        ) -> ProtocolResult<Vec<MailFolder>> {
            RecordingProtocol::default()
                .fetch_folders(_settings, account)
                .await
        }

        async fn fetch_messages(
            &self,
            _settings: &ConnectionSettings,
            account: &MailAccount,
            folder: &MailFolder,
            request: &MessageFetchRequest,
        ) -> ProtocolResult<Vec<MailMessage>> {
            RecordingProtocol::default()
                .fetch_messages(_settings, account, folder, request)
                .await
        }

        async fn send_message(
            &self,
            _settings: &ConnectionSettings,
            _draft: &SendMessageDraft,
        ) -> ProtocolResult<String> {
            Ok(new_id())
        }

        async fn apply_action(
            &self,
            _settings: &ConnectionSettings,
            _account: &MailAccount,
            _action: &RemoteMailAction,
        ) -> ProtocolResult<()> {
            Ok(())
        }

        async fn watch_folder_until_change(
            &self,
            _settings: &ConnectionSettings,
            _account: &MailAccount,
            _folder: &MailFolder,
        ) -> ProtocolResult<mail_core::FolderWatchOutcome> {
            match &self.outcome {
                Ok(outcome) => Ok(*outcome),
                Err(ProtocolError::Connection(message)) => {
                    Err(ProtocolError::Connection(message.clone()))
                }
                Err(ProtocolError::Authentication(message)) => {
                    Err(ProtocolError::Authentication(message.clone()))
                }
                Err(ProtocolError::Fetch(message)) => Err(ProtocolError::Fetch(message.clone())),
                Err(ProtocolError::Parse(message)) => Err(ProtocolError::Parse(message.clone())),
                Err(ProtocolError::Send(message)) => Err(ProtocolError::Send(message.clone())),
                Err(ProtocolError::Unsupported(message)) => {
                    Err(ProtocolError::Unsupported(message.clone()))
                }
            }
        }
    }

    #[async_trait]
    impl MailProtocol for RecordingProtocol {
        async fn test_connection(
            &self,
            _settings: &ConnectionSettings,
        ) -> ProtocolResult<ConnectionTestResult> {
            Ok(ConnectionTestResult {
                imap_ok: true,
                smtp_ok: true,
                message: "ok".to_string(),
            })
        }

        async fn fetch_folders(
            &self,
            _settings: &ConnectionSettings,
            account: &MailAccount,
        ) -> ProtocolResult<Vec<MailFolder>> {
            Ok(vec![
                MailFolder {
                    id: format!("{}:inbox", account.id),
                    account_id: account.id.clone(),
                    name: "INBOX".to_string(),
                    path: "INBOX".to_string(),
                    role: FolderRole::Inbox,
                    unread_count: 1,
                    total_count: 1,
                },
                MailFolder {
                    id: format!("{}:archive", account.id),
                    account_id: account.id.clone(),
                    name: "Archive".to_string(),
                    path: "Archive".to_string(),
                    role: FolderRole::Archive,
                    unread_count: 0,
                    total_count: 0,
                },
                MailFolder {
                    id: format!("{}:trash", account.id),
                    account_id: account.id.clone(),
                    name: "Trash".to_string(),
                    path: "Trash".to_string(),
                    role: FolderRole::Trash,
                    unread_count: 0,
                    total_count: 0,
                },
            ])
        }

        async fn fetch_messages(
            &self,
            _settings: &ConnectionSettings,
            account: &MailAccount,
            folder: &MailFolder,
            _request: &MessageFetchRequest,
        ) -> ProtocolResult<Vec<MailMessage>> {
            if folder.role == FolderRole::Trash {
                let Some(uid) = self.trash_refetch_uid.lock().clone() else {
                    return Ok(Vec::new());
                };
                return Ok(vec![MailMessage {
                    id: format!("{}:{}:{uid}", account.id, folder.id),
                    account_id: account.id.clone(),
                    folder_id: folder.id.clone(),
                    uid: Some(uid.clone()),
                    message_id_header: Some("<42@example.com>".to_string()),
                    subject: "Action required".to_string(),
                    sender: "sec@example.com".to_string(),
                    recipients: vec![account.email.clone()],
                    cc: Vec::new(),
                    received_at: now_rfc3339(),
                    body_preview: "review".to_string(),
                    body: Some("review".to_string()),
                    attachments: Vec::new(),
                    flags: mail_core::MessageFlags {
                        is_read: false,
                        is_starred: false,
                        is_answered: false,
                        is_forwarded: false,
                    },
                    size_bytes: Some(128),
                    deleted_at: None,
                }]);
            }
            if folder.role != FolderRole::Inbox {
                return Ok(Vec::new());
            }
            Ok(vec![MailMessage {
                id: format!("{}:{}:42", account.id, folder.id),
                account_id: account.id.clone(),
                folder_id: folder.id.clone(),
                uid: Some("42".to_string()),
                message_id_header: Some("<42@example.com>".to_string()),
                subject: "Action required".to_string(),
                sender: "sec@example.com".to_string(),
                recipients: vec![account.email.clone()],
                cc: Vec::new(),
                received_at: now_rfc3339(),
                body_preview: "review".to_string(),
                body: Some("review".to_string()),
                attachments: Vec::new(),
                flags: mail_core::MessageFlags {
                    is_read: false,
                    is_starred: false,
                    is_answered: false,
                    is_forwarded: false,
                },
                size_bytes: Some(128),
                deleted_at: None,
            }])
        }

        async fn send_message(
            &self,
            _settings: &ConnectionSettings,
            draft: &SendMessageDraft,
        ) -> ProtocolResult<String> {
            if self.fail_sends {
                return Err(ProtocolError::Send("forced send failure".to_string()));
            }
            self.sends.lock().push(draft.clone());
            Ok("sent-id".to_string())
        }

        async fn apply_action(
            &self,
            _settings: &ConnectionSettings,
            _account: &MailAccount,
            action: &RemoteMailAction,
        ) -> ProtocolResult<()> {
            if self.fail_actions {
                return Err(ProtocolError::Fetch("forced action failure".to_string()));
            }
            self.actions.lock().push(action.clone());
            Ok(())
        }
    }

    #[derive(Debug)]
    struct FailingFetchProtocol;

    #[async_trait]
    impl MailProtocol for FailingFetchProtocol {
        async fn test_connection(
            &self,
            _settings: &ConnectionSettings,
        ) -> ProtocolResult<ConnectionTestResult> {
            Ok(ConnectionTestResult {
                imap_ok: true,
                smtp_ok: true,
                message: "ok".to_string(),
            })
        }

        async fn fetch_folders(
            &self,
            _settings: &ConnectionSettings,
            account: &MailAccount,
        ) -> ProtocolResult<Vec<MailFolder>> {
            Ok(vec![MailFolder {
                id: format!("{}:inbox", account.id),
                account_id: account.id.clone(),
                name: "INBOX".to_string(),
                path: "INBOX".to_string(),
                role: FolderRole::Inbox,
                unread_count: 0,
                total_count: 0,
            }])
        }

        async fn fetch_messages(
            &self,
            _settings: &ConnectionSettings,
            _account: &MailAccount,
            _folder: &MailFolder,
            _request: &MessageFetchRequest,
        ) -> ProtocolResult<Vec<MailMessage>> {
            Err(ProtocolError::Fetch("forced failure".to_string()))
        }

        async fn send_message(
            &self,
            _settings: &ConnectionSettings,
            _draft: &SendMessageDraft,
        ) -> ProtocolResult<String> {
            Ok(new_id())
        }

        async fn apply_action(
            &self,
            _settings: &ConnectionSettings,
            _account: &MailAccount,
            _action: &RemoteMailAction,
        ) -> ProtocolResult<()> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct PartialFailingFetchProtocol;

    #[async_trait]
    impl MailProtocol for PartialFailingFetchProtocol {
        async fn test_connection(
            &self,
            _settings: &ConnectionSettings,
        ) -> ProtocolResult<ConnectionTestResult> {
            Ok(ConnectionTestResult {
                imap_ok: true,
                smtp_ok: true,
                message: "ok".to_string(),
            })
        }

        async fn fetch_folders(
            &self,
            _settings: &ConnectionSettings,
            account: &MailAccount,
        ) -> ProtocolResult<Vec<MailFolder>> {
            Ok(vec![
                MailFolder {
                    id: format!("{}:inbox", account.id),
                    account_id: account.id.clone(),
                    name: "INBOX".to_string(),
                    path: "INBOX".to_string(),
                    role: FolderRole::Inbox,
                    unread_count: 0,
                    total_count: 0,
                },
                MailFolder {
                    id: format!("{}:archive", account.id),
                    account_id: account.id.clone(),
                    name: "Archive".to_string(),
                    path: "Archive".to_string(),
                    role: FolderRole::Archive,
                    unread_count: 0,
                    total_count: 0,
                },
            ])
        }

        async fn fetch_messages(
            &self,
            _settings: &ConnectionSettings,
            account: &MailAccount,
            folder: &MailFolder,
            _request: &MessageFetchRequest,
        ) -> ProtocolResult<Vec<MailMessage>> {
            if folder.role == FolderRole::Archive {
                return Err(ProtocolError::Fetch("archive failure".to_string()));
            }
            Ok(vec![MailMessage {
                id: format!("{}:{}:10", account.id, folder.id),
                account_id: account.id.clone(),
                folder_id: folder.id.clone(),
                uid: Some("10".to_string()),
                message_id_header: Some("<10@example.com>".to_string()),
                subject: "Inbox ok".to_string(),
                sender: "sec@example.com".to_string(),
                recipients: vec![account.email.clone()],
                cc: Vec::new(),
                received_at: now_rfc3339(),
                body_preview: "ok".to_string(),
                body: Some("ok".to_string()),
                attachments: Vec::new(),
                flags: mail_core::MessageFlags::default(),
                size_bytes: Some(128),
                deleted_at: None,
            }])
        }

        async fn send_message(
            &self,
            _settings: &ConnectionSettings,
            _draft: &SendMessageDraft,
        ) -> ProtocolResult<String> {
            Ok(new_id())
        }

        async fn apply_action(
            &self,
            _settings: &ConnectionSettings,
            _account: &MailAccount,
            _action: &RemoteMailAction,
        ) -> ProtocolResult<()> {
            Ok(())
        }
    }
}
