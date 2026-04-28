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
pub const WATCH_DIAGNOSTIC_EVENT: &str = "agentmail-watch-diagnostic";
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

#[derive(Clone, Serialize)]
pub struct WatchDiagnosticEventPayload {
    pub account_id: String,
    pub folder_id: Option<String>,
    pub stage: &'static str,
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
    emit_watch_diagnostic(
        &app,
        WatchDiagnosticEventPayload {
            account_id: account_id.clone(),
            folder_id: None,
            stage: "watch_start_requested",
            message: None,
        },
    );

    let sync_enabled = match api.is_account_sync_enabled(&account_id) {
        Ok(sync_enabled) => sync_enabled,
        Err(error) => {
            let message = to_error(error);
            emit_watch_diagnostic(
                &app,
                WatchDiagnosticEventPayload {
                    account_id,
                    folder_id: None,
                    stage: "watch_start_sync_check_failed",
                    message: Some(message.clone()),
                },
            );
            return Err(message);
        }
    };

    if !sync_enabled {
        emit_watch_diagnostic(
            &app,
            WatchDiagnosticEventPayload {
                account_id,
                folder_id: None,
                stage: "watch_start_skipped_sync_disabled",
                message: None,
            },
        );
        return Ok(());
    }

    let folders = match api.list_folders(account_id.clone()) {
        Ok(folders) => folders,
        Err(error) => {
            let message = to_error(error);
            emit_watch_diagnostic(
                &app,
                WatchDiagnosticEventPayload {
                    account_id,
                    folder_id: None,
                    stage: "watch_start_list_folders_failed",
                    message: Some(message.clone()),
                },
            );
            return Err(message);
        }
    };
    emit_watch_diagnostic(
        &app,
        WatchDiagnosticEventPayload {
            account_id: account_id.clone(),
            folder_id: None,
            stage: "watch_plan",
            message: Some(format!("{} folders", folders.len())),
        },
    );
    for folder in folders {
        let folder_id = folder.id;
        emit_watch_diagnostic(
            &app,
            WatchDiagnosticEventPayload {
                account_id: account_id.clone(),
                folder_id: Some(folder_id.clone()),
                stage: "watch_spawn_requested",
                message: None,
            },
        );
        start_folder_watcher(
            Arc::clone(&api),
            registry.clone(),
            app.clone(),
            account_id.clone(),
            folder_id,
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
        emit_watch_diagnostic(
            &app,
            WatchDiagnosticEventPayload {
                account_id,
                folder_id: Some(folder_id),
                stage: "watch_already_active",
                message: None,
            },
        );
        return;
    }

    tauri::async_runtime::spawn(async move {
        emit_watch_diagnostic(
            &app,
            WatchDiagnosticEventPayload {
                account_id: account_id.clone(),
                folder_id: Some(folder_id.clone()),
                stage: "watch_loop_started",
                message: None,
            },
        );
        loop {
            match api.is_account_sync_enabled(&account_id) {
                Ok(true) => {}
                Ok(false) => {
                    emit_watch_diagnostic(
                        &app,
                        WatchDiagnosticEventPayload {
                            account_id: account_id.clone(),
                            folder_id: Some(folder_id.clone()),
                            stage: "watch_stop_sync_disabled",
                            message: None,
                        },
                    );
                    remove_watcher(&registry, &account_id, &folder_id);
                    return;
                }
                Err(error) => {
                    emit_watch_diagnostic(
                        &app,
                        WatchDiagnosticEventPayload {
                            account_id: account_id.clone(),
                            folder_id: Some(folder_id.clone()),
                            stage: "watch_stop_sync_check_failed",
                            message: Some(to_error(error)),
                        },
                    );
                    remove_watcher(&registry, &account_id, &folder_id);
                    return;
                }
            }

            emit_watch_diagnostic(
                &app,
                WatchDiagnosticEventPayload {
                    account_id: account_id.clone(),
                    folder_id: Some(folder_id.clone()),
                    stage: "watch_idle_begin",
                    message: None,
                },
            );
            match api
                .watch_folder_until_change(account_id.clone(), folder_id.clone())
                .await
            {
                Ok(FolderWatchOutcome::Changed) => {
                    emit_watch_diagnostic(
                        &app,
                        WatchDiagnosticEventPayload {
                            account_id: account_id.clone(),
                            folder_id: Some(folder_id.clone()),
                            stage: "watch_idle_changed",
                            message: None,
                        },
                    );
                    match api.is_account_sync_enabled(&account_id) {
                        Ok(true) => {}
                        Ok(false) => {
                            emit_watch_diagnostic(
                                &app,
                                WatchDiagnosticEventPayload {
                                    account_id: account_id.clone(),
                                    folder_id: Some(folder_id.clone()),
                                    stage: "watch_skip_sync_disabled",
                                    message: None,
                                },
                            );
                            remove_watcher(&registry, &account_id, &folder_id);
                            return;
                        }
                        Err(error) => {
                            emit_watch_diagnostic(
                                &app,
                                WatchDiagnosticEventPayload {
                                    account_id: account_id.clone(),
                                    folder_id: Some(folder_id.clone()),
                                    stage: "watch_skip_sync_check_failed",
                                    message: Some(to_error(error)),
                                },
                            );
                            remove_watcher(&registry, &account_id, &folder_id);
                            return;
                        }
                    }

                    sync_folder_after_change(&api, &app, &account_id, &folder_id).await;
                }
                Ok(FolderWatchOutcome::Timeout) => {
                    emit_watch_diagnostic(
                        &app,
                        WatchDiagnosticEventPayload {
                            account_id: account_id.clone(),
                            folder_id: Some(folder_id.clone()),
                            stage: "watch_idle_timeout",
                            message: None,
                        },
                    );
                }
                Err(error) => {
                    emit_watch_diagnostic(
                        &app,
                        WatchDiagnosticEventPayload {
                            account_id: account_id.clone(),
                            folder_id: Some(folder_id.clone()),
                            stage: "watch_idle_error",
                            message: Some(to_error(error)),
                        },
                    );
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
            Ok(false) => {
                emit_watch_diagnostic(
                    app,
                    WatchDiagnosticEventPayload {
                        account_id: account_id.to_string(),
                        folder_id: Some(folder_id.to_string()),
                        stage: "watch_sync_stop_disabled",
                        message: None,
                    },
                );
                return;
            }
            Err(error) => {
                emit_watch_diagnostic(
                    app,
                    WatchDiagnosticEventPayload {
                        account_id: account_id.to_string(),
                        folder_id: Some(folder_id.to_string()),
                        stage: "watch_sync_stop_check_failed",
                        message: Some(to_error(error)),
                    },
                );
                return;
            }
        }

        emit_watch_diagnostic(
            app,
            WatchDiagnosticEventPayload {
                account_id: account_id.to_string(),
                folder_id: Some(folder_id.to_string()),
                stage: "watch_sync_begin",
                message: None,
            },
        );
        match api
            .sync_folder(account_id.to_string(), folder_id.to_string())
            .await
        {
            Ok(summary) => {
                let message = format!(
                    "{} folders / {} messages",
                    summary.folders, summary.messages
                );
                emit_watch_diagnostic(
                    app,
                    WatchDiagnosticEventPayload {
                        account_id: summary.account_id.clone(),
                        folder_id: Some(folder_id.to_string()),
                        stage: "watch_sync_complete",
                        message: Some(message.clone()),
                    },
                );
                emit_mail_sync_event(
                    app,
                    MailSyncEventPayload {
                        account_id: summary.account_id,
                        folder_id: Some(folder_id.to_string()),
                        reason: "watch_changed",
                        message: Some(message),
                    },
                );
                return;
            }
            Err(error) => {
                let message = to_error(&error);
                match error {
                    ApiError::SyncAlreadyRunning(_)
                    | ApiError::SyncBackoff { .. }
                    | ApiError::Protocol(_)
                    | ApiError::Store(_) => {
                        emit_watch_diagnostic(
                            app,
                            WatchDiagnosticEventPayload {
                                account_id: account_id.to_string(),
                                folder_id: Some(folder_id.to_string()),
                                stage: "watch_sync_retry",
                                message: Some(message),
                            },
                        );
                        tokio::time::sleep(WATCHER_RETRY_DELAY).await;
                    }
                    ApiError::InvalidRequest(_)
                    | ApiError::ConfirmationRequired(_)
                    | ApiError::AiRemote(_) => {
                        emit_watch_diagnostic(
                            app,
                            WatchDiagnosticEventPayload {
                                account_id: account_id.to_string(),
                                folder_id: Some(folder_id.to_string()),
                                stage: "watch_sync_stop",
                                message: Some(message),
                            },
                        );
                        return;
                    }
                }
            }
        }
    }
}

pub fn emit_mail_sync_event(app: &AppHandle, payload: MailSyncEventPayload) {
    let _ = app.emit(MAIL_SYNC_EVENT, payload);
}

pub fn emit_watch_diagnostic(app: &AppHandle, payload: WatchDiagnosticEventPayload) {
    let _ = app.emit(WATCH_DIAGNOSTIC_EVENT, payload);
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
