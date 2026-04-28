use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};

use app_api::{ApiError, AppApi};
use mail_core::FolderWatchOutcome;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub const MAIL_SYNC_EVENT: &str = "agentmail-mail-sync";
const WATCHER_RETRY_DELAY: Duration = Duration::from_secs(30);

#[derive(Clone, Default)]
pub struct WatcherRegistry {
    active: Arc<Mutex<HashSet<String>>>,
}

#[derive(Clone, Serialize)]
pub struct MailSyncEventPayload {
    pub account_id: String,
    pub folder_id: Option<String>,
    pub reason: &'static str,
    pub message: Option<String>,
}

fn to_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn watcher_key(account_id: &str, folder_id: &str) -> String {
    format!("{account_id}\0{folder_id}")
}

fn register_watcher(registry: &WatcherRegistry, account_id: &str, folder_id: &str) -> bool {
    let key = watcher_key(account_id, folder_id);
    match registry.active.lock() {
        Ok(mut active) => active.insert(key),
        Err(_) => false,
    }
}

fn remove_watcher(registry: &WatcherRegistry, account_id: &str, folder_id: &str) {
    let key = watcher_key(account_id, folder_id);
    if let Ok(mut active) = registry.active.lock() {
        active.remove(&key);
    }
}

pub fn start_account_watchers_for_api(
    api: Arc<AppApi>,
    registry: WatcherRegistry,
    app: AppHandle,
    account_id: String,
) -> Result<(), String> {
    if !api.is_account_sync_enabled(&account_id).map_err(to_error)? {
        return Ok(());
    }

    let folders = api.list_folders(account_id.clone()).map_err(to_error)?;
    for folder in folders {
        start_folder_watcher(
            Arc::clone(&api),
            registry.clone(),
            app.clone(),
            account_id.clone(),
            folder.id,
        );
    }
    Ok(())
}

fn start_folder_watcher(
    api: Arc<AppApi>,
    registry: WatcherRegistry,
    app: AppHandle,
    account_id: String,
    folder_id: String,
) {
    if !register_watcher(&registry, &account_id, &folder_id) {
        return;
    }

    tauri::async_runtime::spawn(async move {
        loop {
            match api.is_account_sync_enabled(&account_id) {
                Ok(true) => {}
                Ok(false) | Err(_) => {
                    remove_watcher(&registry, &account_id, &folder_id);
                    return;
                }
            }

            match api
                .watch_folder_until_change(account_id.clone(), folder_id.clone())
                .await
            {
                Ok(FolderWatchOutcome::Changed) => {
                    match api.is_account_sync_enabled(&account_id) {
                        Ok(true) => {}
                        Ok(false) | Err(_) => {
                            remove_watcher(&registry, &account_id, &folder_id);
                            return;
                        }
                    }

                    sync_folder_after_change(&api, &app, &account_id, &folder_id).await;
                }
                Ok(FolderWatchOutcome::Timeout) => {}
                Err(_) => {
                    tokio::time::sleep(WATCHER_RETRY_DELAY).await;
                    continue;
                }
            }
        }
    });
}

async fn sync_folder_after_change(
    api: &AppApi,
    app: &AppHandle,
    account_id: &str,
    folder_id: &str,
) {
    loop {
        match api.is_account_sync_enabled(account_id) {
            Ok(true) => {}
            Ok(false) | Err(_) => return,
        }

        match api
            .sync_folder(account_id.to_string(), folder_id.to_string())
            .await
        {
            Ok(summary) => {
                emit_mail_sync_event(
                    app,
                    MailSyncEventPayload {
                        account_id: summary.account_id,
                        folder_id: Some(folder_id.to_string()),
                        reason: "watch_changed",
                        message: Some(format!(
                            "{} folders / {} messages",
                            summary.folders, summary.messages
                        )),
                    },
                );
                return;
            }
            Err(
                ApiError::SyncAlreadyRunning(_)
                | ApiError::SyncBackoff { .. }
                | ApiError::Protocol(_)
                | ApiError::Store(_),
            ) => {
                tokio::time::sleep(WATCHER_RETRY_DELAY).await;
            }
            Err(
                ApiError::InvalidRequest(_)
                | ApiError::ConfirmationRequired(_)
                | ApiError::AiRemote(_),
            ) => return,
        }
    }
}

pub fn emit_mail_sync_event(app: &AppHandle, payload: MailSyncEventPayload) {
    let _ = app.emit(MAIL_SYNC_EVENT, payload);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_watcher_accepts_each_folder_once() {
        let registry = WatcherRegistry::default();

        assert!(register_watcher(&registry, "acct", "inbox"));
        assert!(!register_watcher(&registry, "acct", "inbox"));
        assert!(register_watcher(&registry, "acct", "trash"));
    }

    #[test]
    fn remove_watcher_allows_restart() {
        let registry = WatcherRegistry::default();

        assert!(register_watcher(&registry, "acct", "inbox"));
        remove_watcher(&registry, "acct", "inbox");

        assert!(register_watcher(&registry, "acct", "inbox"));
    }
}
