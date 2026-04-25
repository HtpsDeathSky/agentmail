use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use ai_remote::{AiProvider, OpenAiCompatibleProvider};
use mail_core::{
    new_id, now_plus_seconds_rfc3339, now_rfc3339, timestamp_is_future, ActionAuditStatus,
    AiAnalysisInput, AiInsight, AiSettings, AiSettingsView, ConnectionSettings,
    ConnectionTestResult, FolderRole, MailAccount, MailActionAudit, MailActionKind,
    MailActionRequest, MailActionResult, MailActionResultKind, MailFolder, MailMessage,
    MessageFetchRequest, MessageQuery, PendingActionStatus, PendingMailAction, RemoteMailAction,
    SaveAiSettingsRequest, SendMessageDraft, SyncState, SyncStateKind,
};
use mail_protocol::{LiveMailProtocol, MailProtocol, ProtocolError};
use mail_store::{MailStore, MessageFlagPatch, StoreError};
use parking_lot::Mutex;
use secret_store::{mail_password_target, PlatformSecretStore, SecretError, SecretStore};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Secret(#[from] SecretError),
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
    secrets: Arc<dyn SecretStore>,
    protocol: Arc<dyn MailProtocol>,
    ai_provider: Arc<dyn AiProvider>,
    sync_locks: Arc<Mutex<HashSet<String>>>,
}

impl AppApi {
    pub fn new(
        store: MailStore,
        secrets: Arc<dyn SecretStore>,
        protocol: Arc<dyn MailProtocol>,
    ) -> Self {
        Self::new_with_ai_provider(
            store,
            secrets,
            protocol,
            Arc::new(OpenAiCompatibleProvider::default()),
        )
    }

    pub fn new_with_ai_provider(
        store: MailStore,
        secrets: Arc<dyn SecretStore>,
        protocol: Arc<dyn MailProtocol>,
        ai_provider: Arc<dyn AiProvider>,
    ) -> Self {
        Self {
            store,
            secrets,
            protocol,
            ai_provider,
            sync_locks: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn new_default(db_path: impl AsRef<Path>) -> ApiResult<Self> {
        Ok(Self::new(
            MailStore::open(db_path)?,
            Arc::new(PlatformSecretStore::default()),
            Arc::new(LiveMailProtocol),
        ))
    }

    pub async fn add_account(&self, request: AddAccountRequest) -> ApiResult<MailAccount> {
        request.validate()?;

        let settings = ConnectionSettings {
            account_id: None,
            email: request.email.clone(),
            imap_host: request.imap_host.clone(),
            imap_port: request.imap_port,
            imap_tls: request.imap_tls,
            smtp_host: request.smtp_host.clone(),
            smtp_port: request.smtp_port,
            smtp_tls: request.smtp_tls,
            password: request.password.clone(),
        };
        let connection_result = self.protocol.test_connection(&settings).await?;
        if !connection_result.imap_ok || !connection_result.smtp_ok {
            return Err(ApiError::InvalidRequest(format!(
                "connection test failed: {}",
                connection_result.message
            )));
        }

        let now = now_rfc3339();
        let account = MailAccount {
            id: new_id(),
            display_name: request.display_name,
            email: request.email,
            imap_host: request.imap_host,
            imap_port: request.imap_port,
            imap_tls: request.imap_tls,
            smtp_host: request.smtp_host,
            smtp_port: request.smtp_port,
            smtp_tls: request.smtp_tls,
            sync_enabled: true,
            created_at: now.clone(),
            updated_at: now,
        };

        self.secrets
            .set_secret(&mail_password_target(&account.id), &request.password)?;
        self.store.save_account(&account)?;
        self.write_audit(
            &account.id,
            MailActionKind::MarkRead,
            Vec::new(),
            ActionAuditStatus::Executed,
            None,
        )?;
        Ok(account)
    }

    pub async fn test_account_connection(
        &self,
        request: TestConnectionRequest,
    ) -> ApiResult<ConnectionTestResult> {
        let settings = if let Some(account_id) = request.account_id {
            let account = self.store.get_account(&account_id)?;
            let password = self
                .secrets
                .get_secret(&mail_password_target(&account.id))?;
            account_to_settings(&account, password)
        } else {
            let manual = request
                .manual
                .ok_or_else(|| ApiError::InvalidRequest("manual settings required".to_string()))?;
            manual.validate()?;
            ConnectionSettings {
                account_id: None,
                email: manual.email,
                imap_host: manual.imap_host,
                imap_port: manual.imap_port,
                imap_tls: manual.imap_tls,
                smtp_host: manual.smtp_host,
                smtp_port: manual.smtp_port,
                smtp_tls: manual.smtp_tls,
                password: manual.password,
            }
        };

        Ok(self.protocol.test_connection(&settings).await?)
    }

    pub fn list_accounts(&self) -> ApiResult<Vec<MailAccount>> {
        Ok(self.store.list_accounts()?)
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
        let normalized_action = confirmation_action_for_request(&request);
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
        let pending = self.queue_pending_action(
            draft.account_id.clone(),
            MailActionKind::Send,
            Vec::new(),
            None,
            Some(draft),
        )?;
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
                self.send_message_now(draft).await.map(|_| ())
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
            MailActionKind::PermanentDelete => Err(ApiError::InvalidRequest(
                "permanent delete is disabled in this release".to_string(),
            )),
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
        let remote_action = self.build_remote_action(&account, request)?;
        let settings = self.connection_settings_for_account(&account)?;

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

    fn build_remote_action(
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

        let mut uids = Vec::with_capacity(messages.len());
        for message in &messages {
            let uid = message.uid.clone().ok_or_else(|| {
                ApiError::InvalidRequest(format!("message {} has no remote UID", message.id))
            })?;
            uids.push(uid);
        }

        let source_folder = self.store.get_folder(&source_folder_id)?;
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
            _ => {
                return Err(ApiError::InvalidRequest(format!(
                    "unsupported confirmed mail action: {:?}",
                    request.action
                )));
            }
        }
        Ok(())
    }

    fn required_role_folder(&self, account_id: &str, role: FolderRole) -> ApiResult<MailFolder> {
        self.store
            .find_folder_by_role(account_id, role)?
            .ok_or_else(|| ApiError::InvalidRequest(format!("{role:?} folder is not available")))
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
        let password = self
            .secrets
            .get_secret(&mail_password_target(&account.id))?;
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

impl AddAccountRequest {
    fn validate(&self) -> ApiResult<()> {
        if self.email.trim().is_empty() {
            return Err(ApiError::InvalidRequest("email is required".to_string()));
        }
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestConnectionRequest {
    pub account_id: Option<String>,
    pub manual: Option<AddAccountRequest>,
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

fn backoff_seconds(failure_count: u32) -> i64 {
    let exponent = failure_count.saturating_sub(1).min(5);
    (60_i64 * 2_i64.pow(exponent)).min(1_800)
}

fn confirmation_action_for_request(request: &MailActionRequest) -> MailActionKind {
    match request.action {
        MailActionKind::Delete if request.message_ids.len() > 1 => MailActionKind::BatchDelete,
        MailActionKind::Move if request.message_ids.len() > 1 => MailActionKind::BatchMove,
        action => action,
    }
}

fn required_trimmed(value: String, field: &str) -> ApiResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::InvalidRequest(format!("{field} is required")));
    }
    Ok(trimmed.to_string())
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
    if chars.len() <= 4 {
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
    use secret_store::MemorySecretStore;

    #[test]
    fn ai_settings_are_saved_and_masked() {
        let api = AppApi::new(
            MailStore::memory().unwrap(),
            Arc::new(MemorySecretStore::default()),
            Arc::new(MockMailProtocol),
        );

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
    fn unicode_ai_settings_key_is_saved_and_masked() {
        let api = AppApi::new(
            MailStore::memory().unwrap(),
            Arc::new(MemorySecretStore::default()),
            Arc::new(MockMailProtocol),
        );

        let saved = api
            .save_ai_settings(SaveAiSettingsRequest {
                provider_name: "openai-compatible".to_string(),
                base_url: "https://api.example.com/v1".to_string(),
                model: "mail-model".to_string(),
                api_key: Some("钥匙abcdef".to_string()),
                enabled: true,
            })
            .unwrap();

        assert_eq!(saved.api_key_mask, Some("钥匙a...cdef".to_string()));

        let loaded = api.get_ai_settings().unwrap().unwrap();
        assert_eq!(loaded.api_key_mask, Some("钥匙a...cdef".to_string()));
    }

    #[tokio::test]
    async fn run_ai_analysis_requires_settings() {
        let api = AppApi::new_with_ai_provider(
            MailStore::memory().unwrap(),
            Arc::new(MemorySecretStore::default()),
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
            Arc::new(MemorySecretStore::default()),
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
            Arc::new(MemorySecretStore::default()),
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
    async fn provider_failure_does_not_store_insight() {
        let api = AppApi::new_with_ai_provider(
            MailStore::memory().unwrap(),
            Arc::new(MemorySecretStore::default()),
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
        let api = AppApi::new(
            MailStore::memory().unwrap(),
            Arc::new(MemorySecretStore::default()),
            Arc::new(MockMailProtocol),
        );
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
        let api = AppApi::new(
            MailStore::memory().unwrap(),
            Arc::new(MemorySecretStore::default()),
            Arc::new(MockMailProtocol),
        );

        let guard = api.acquire_sync_lock("acct").unwrap();
        match api.acquire_sync_lock("acct") {
            Err(ApiError::SyncAlreadyRunning(account_id)) => assert_eq!(account_id, "acct"),
            _ => panic!("expected duplicate sync lock error"),
        }
        drop(guard);
        assert!(api.acquire_sync_lock("acct").is_ok());
    }

    #[tokio::test]
    async fn sync_failure_enters_backoff_and_blocks_next_attempt() {
        let api = AppApi::new(
            MailStore::memory().unwrap(),
            Arc::new(MemorySecretStore::default()),
            Arc::new(FailingFetchProtocol),
        );
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
            Arc::new(MemorySecretStore::default()),
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
    async fn low_risk_action_calls_protocol_then_updates_local_state() {
        let protocol = RecordingProtocol::default();
        let actions = Arc::clone(&protocol.actions);
        let api = AppApi::new(
            MailStore::memory().unwrap(),
            Arc::new(MemorySecretStore::default()),
            Arc::new(protocol),
        );
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
    async fn remote_action_failure_does_not_mutate_local_message() {
        let protocol = RecordingProtocol {
            fail_actions: true,
            ..RecordingProtocol::default()
        };
        let api = AppApi::new(
            MailStore::memory().unwrap(),
            Arc::new(MemorySecretStore::default()),
            Arc::new(protocol),
        );
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
    async fn delete_moves_to_trash_and_clears_local_uid() {
        let protocol = RecordingProtocol::default();
        let actions = Arc::clone(&protocol.actions);
        let api = AppApi::new(
            MailStore::memory().unwrap(),
            Arc::new(MemorySecretStore::default()),
            Arc::new(protocol),
        );
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
    async fn send_message_queues_until_confirmed() {
        let protocol = RecordingProtocol::default();
        let sends = Arc::clone(&protocol.sends);
        let api = AppApi::new(
            MailStore::memory().unwrap(),
            Arc::new(MemorySecretStore::default()),
            Arc::new(protocol),
        );
        let account = add_sample_account(&api).await;

        let pending_id = api
            .send_message(SendMessageDraft {
                account_id: account.id.clone(),
                to: vec!["sec@example.com".to_string()],
                cc: Vec::new(),
                subject: "Confirm send".to_string(),
                body: "body".to_string(),
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
    async fn reject_pending_action_does_not_execute_protocol() {
        let protocol = RecordingProtocol::default();
        let sends = Arc::clone(&protocol.sends);
        let api = AppApi::new(
            MailStore::memory().unwrap(),
            Arc::new(MemorySecretStore::default()),
            Arc::new(protocol),
        );
        let account = add_sample_account(&api).await;

        let pending_id = api
            .send_message(SendMessageDraft {
                account_id: account.id.clone(),
                to: vec!["sec@example.com".to_string()],
                cc: Vec::new(),
                subject: "Reject send".to_string(),
                body: "body".to_string(),
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

    struct RecordingProtocol {
        actions: Arc<Mutex<Vec<RemoteMailAction>>>,
        sends: Arc<Mutex<Vec<SendMessageDraft>>>,
        fail_actions: bool,
    }

    impl Default for RecordingProtocol {
        fn default() -> Self {
            Self {
                actions: Arc::new(Mutex::new(Vec::new())),
                sends: Arc::new(Mutex::new(Vec::new())),
                fail_actions: false,
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
