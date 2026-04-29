use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};

use app_api::{AppApi, SyncSummary};
use mail_core::MailAccount;
use tauri::AppHandle;

use crate::sync_events::{emit_mail_sync_event, MailSyncEventPayload};

pub const CONSISTENCY_SYNC_INTERVAL: Duration = Duration::from_secs(120);

#[derive(Clone, Default)]
pub struct SyncRunRegistry {
    active: Arc<Mutex<HashSet<String>>>,
}

pub struct SyncRunGuard {
    registry: SyncRunRegistry,
    account_id: String,
}

impl SyncRunRegistry {
    pub fn try_start(&self, account_id: &str) -> Option<SyncRunGuard> {
        let mut active = self.active.lock().ok()?;
        if !active.insert(account_id.to_string()) {
            return None;
        }
        Some(SyncRunGuard {
            registry: self.clone(),
            account_id: account_id.to_string(),
        })
    }

    #[cfg(test)]
    pub fn is_active(&self, account_id: &str) -> bool {
        self.active
            .lock()
            .map(|active| active.contains(account_id))
            .unwrap_or(false)
    }
}

impl Drop for SyncRunGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.registry.active.lock() {
            active.remove(&self.account_id);
        }
    }
}

pub fn sync_enabled_account_ids(accounts: &[MailAccount]) -> Vec<String> {
    accounts
        .iter()
        .filter(|account| account.sync_enabled)
        .map(|account| account.id.clone())
        .collect()
}

pub fn sync_reason_from_request(reason: Option<String>) -> &'static str {
    match reason.as_deref() {
        Some("startup_sync") => "startup_sync",
        Some("account_saved_sync") => "account_saved_sync",
        Some("manual_sync") => "manual_sync",
        Some("foreground_sync") => "foreground_sync",
        Some("interval_sync") => "interval_sync",
        _ => "manual_sync",
    }
}

#[derive(Clone)]
pub struct ConsistencySyncService {
    api: Arc<AppApi>,
    app: AppHandle,
    registry: SyncRunRegistry,
}

impl ConsistencySyncService {
    pub fn new(api: Arc<AppApi>, app: AppHandle) -> Self {
        Self {
            api,
            app,
            registry: SyncRunRegistry::default(),
        }
    }

    pub async fn sync_account_once(
        &self,
        account_id: String,
        reason: &'static str,
    ) -> Result<SyncSummary, String> {
        let _guard = self
            .registry
            .try_start(&account_id)
            .ok_or_else(|| format!("sync already running: {account_id}"))?;
        let summary = self
            .api
            .sync_account(account_id)
            .await
            .map_err(|error| error.to_string())?;
        self.emit_summary(&summary, reason);
        Ok(summary)
    }

    pub async fn sync_account_if_idle(
        &self,
        account_id: String,
        reason: &'static str,
    ) -> Option<Result<SyncSummary, String>> {
        let guard = self.registry.try_start(&account_id)?;
        let result = self
            .api
            .sync_account(account_id)
            .await
            .map_err(|error| error.to_string());
        drop(guard);
        if let Ok(summary) = &result {
            self.emit_summary(summary, reason);
        }
        Some(result)
    }

    pub async fn sync_enabled_accounts(&self, reason: &'static str) {
        let Ok(accounts) = self.api.list_accounts() else {
            return;
        };
        for account_id in sync_enabled_account_ids(&accounts) {
            let _ = self.sync_account_if_idle(account_id, reason).await;
        }
    }

    pub async fn handle_foreground_resume(&self, selected_account_id: Option<String>) {
        if let Some(account_id) = selected_account_id {
            if self
                .api
                .is_account_sync_enabled(&account_id)
                .unwrap_or(false)
            {
                let _ = self
                    .sync_account_if_idle(account_id, "foreground_sync")
                    .await;
            }
            return;
        }
        self.sync_enabled_accounts("foreground_sync").await;
    }

    pub fn start_interval_sync(&self) {
        let service = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(CONSISTENCY_SYNC_INTERVAL).await;
                service.sync_enabled_accounts("interval_sync").await;
            }
        });
    }

    fn emit_summary(&self, summary: &SyncSummary, reason: &'static str) {
        emit_mail_sync_event(
            &self.app,
            MailSyncEventPayload {
                account_id: summary.account_id.clone(),
                folder_id: None,
                reason,
                message: Some(format!(
                    "{} folders / {} messages",
                    summary.folders, summary.messages
                )),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, sync_enabled: bool) -> MailAccount {
        MailAccount {
            id: id.to_string(),
            display_name: id.to_string(),
            email: format!("{id}@example.com"),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_tls: true,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_tls: true,
            sync_enabled,
            created_at: "2026-04-29T00:00:00Z".to_string(),
            updated_at: "2026-04-29T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn sync_run_registry_prevents_overlap_for_same_account() {
        let registry = SyncRunRegistry::default();
        let first = registry.try_start("acct-1");

        assert!(first.is_some());
        assert!(registry.try_start("acct-1").is_none());
        assert!(registry.try_start("acct-2").is_some());
    }

    #[test]
    fn sync_run_guard_releases_account_on_drop() {
        let registry = SyncRunRegistry::default();

        {
            let _guard = registry.try_start("acct-1").expect("first sync starts");
            assert!(registry.is_active("acct-1"));
        }

        assert!(!registry.is_active("acct-1"));
        assert!(registry.try_start("acct-1").is_some());
    }

    #[test]
    fn sync_enabled_account_ids_filters_disabled_accounts() {
        let accounts = vec![account("enabled", true), account("disabled", false)];

        assert_eq!(
            sync_enabled_account_ids(&accounts),
            vec!["enabled".to_string()]
        );
    }

    #[test]
    fn sync_reason_from_request_allows_known_reasons_only() {
        assert_eq!(
            sync_reason_from_request(Some("account_saved_sync".to_string())),
            "account_saved_sync"
        );
        assert_eq!(
            sync_reason_from_request(Some("unexpected".to_string())),
            "manual_sync"
        );
        assert_eq!(sync_reason_from_request(None), "manual_sync");
    }
}
