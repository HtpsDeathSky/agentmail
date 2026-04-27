#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::sync::Arc;

use app_api::{
    AccountConfigView, AddAccountRequest, AppApi, SaveAccountConfigRequest, SyncSummary,
    TestConnectionRequest,
};
use mail_core::{
    AiInsight, AiSettingsView, ConnectionTestResult, MailAccount, MailActionAudit,
    MailActionRequest, MailActionResult, MailFolder, MailMessage, MessageQuery, PendingMailAction,
    SaveAiSettingsRequest, SendMessageDraft, SyncState,
};
use tauri::{Manager, State};

struct ApiState {
    api: Arc<AppApi>,
}

#[tauri::command]
async fn add_account(
    state: State<'_, ApiState>,
    request: AddAccountRequest,
) -> Result<MailAccount, String> {
    state.api.add_account(request).await.map_err(to_error)
}

#[tauri::command]
async fn test_account_connection(
    state: State<'_, ApiState>,
    request: TestConnectionRequest,
) -> Result<ConnectionTestResult, String> {
    state
        .api
        .test_account_connection(request)
        .await
        .map_err(to_error)
}

#[tauri::command]
fn list_accounts(state: State<'_, ApiState>) -> Result<Vec<MailAccount>, String> {
    state.api.list_accounts().map_err(to_error)
}

#[tauri::command]
fn get_account_config(
    state: State<'_, ApiState>,
    account_id: String,
) -> Result<AccountConfigView, String> {
    state.api.get_account_config(account_id).map_err(to_error)
}

#[tauri::command]
async fn save_account_config(
    state: State<'_, ApiState>,
    request: SaveAccountConfigRequest,
) -> Result<MailAccount, String> {
    state
        .api
        .save_account_config(request)
        .await
        .map_err(to_error)
}

#[tauri::command]
async fn sync_account(
    state: State<'_, ApiState>,
    account_id: String,
) -> Result<SyncSummary, String> {
    state.api.sync_account(account_id).await.map_err(to_error)
}

#[tauri::command]
fn get_sync_status(
    state: State<'_, ApiState>,
    account_id: String,
) -> Result<Vec<SyncState>, String> {
    state.api.get_sync_status(account_id).map_err(to_error)
}

#[tauri::command]
fn list_folders(state: State<'_, ApiState>, account_id: String) -> Result<Vec<MailFolder>, String> {
    state.api.list_folders(account_id).map_err(to_error)
}

#[tauri::command]
fn list_messages(
    state: State<'_, ApiState>,
    query: MessageQuery,
) -> Result<Vec<MailMessage>, String> {
    state.api.list_messages(query).map_err(to_error)
}

#[tauri::command]
fn get_message(state: State<'_, ApiState>, message_id: String) -> Result<MailMessage, String> {
    state.api.get_message(message_id).map_err(to_error)
}

#[tauri::command]
fn search_messages(
    state: State<'_, ApiState>,
    term: String,
    limit: Option<u32>,
) -> Result<Vec<MailMessage>, String> {
    state.api.search_messages(term, limit).map_err(to_error)
}

#[tauri::command]
async fn execute_mail_action(
    state: State<'_, ApiState>,
    request: MailActionRequest,
) -> Result<MailActionResult, String> {
    state
        .api
        .execute_mail_action(request)
        .await
        .map_err(to_error)
}

#[tauri::command]
async fn send_message(
    state: State<'_, ApiState>,
    draft: SendMessageDraft,
) -> Result<String, String> {
    state.api.send_message(draft).await.map_err(to_error)
}

#[tauri::command]
fn get_audit_log(
    state: State<'_, ApiState>,
    limit: Option<u32>,
) -> Result<Vec<MailActionAudit>, String> {
    state.api.get_audit_log(limit).map_err(to_error)
}

#[tauri::command]
fn list_pending_actions(
    state: State<'_, ApiState>,
    account_id: Option<String>,
) -> Result<Vec<PendingMailAction>, String> {
    state.api.list_pending_actions(account_id).map_err(to_error)
}

#[tauri::command]
async fn confirm_action(
    state: State<'_, ApiState>,
    action_id: String,
) -> Result<MailActionResult, String> {
    state.api.confirm_action(action_id).await.map_err(to_error)
}

#[tauri::command]
fn reject_action(state: State<'_, ApiState>, action_id: String) -> Result<(), String> {
    state.api.reject_action(action_id).map_err(to_error)
}

#[tauri::command]
fn get_ai_settings(state: State<'_, ApiState>) -> Result<Option<AiSettingsView>, String> {
    state.api.get_ai_settings().map_err(to_error)
}

#[tauri::command]
fn save_ai_settings(
    state: State<'_, ApiState>,
    request: SaveAiSettingsRequest,
) -> Result<AiSettingsView, String> {
    state.api.save_ai_settings(request).map_err(to_error)
}

#[tauri::command]
fn clear_ai_settings(state: State<'_, ApiState>) -> Result<(), String> {
    state.api.clear_ai_settings().map_err(to_error)
}

#[tauri::command]
async fn run_ai_analysis(
    state: State<'_, ApiState>,
    message_id: String,
) -> Result<AiInsight, String> {
    state
        .api
        .run_ai_analysis(message_id)
        .await
        .map_err(to_error)
}

#[tauri::command]
fn list_ai_insights(
    state: State<'_, ApiState>,
    message_id: String,
) -> Result<Vec<AiInsight>, String> {
    state.api.list_ai_insights(message_id).map_err(to_error)
}

fn to_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|err| format!("failed to resolve app data directory: {err}"))?;
            std::fs::create_dir_all(&data_dir)
                .map_err(|err| format!("failed to create app data directory: {err}"))?;
            let db_path = data_dir.join("agentmail.db");
            let api = AppApi::new_default(db_path)
                .map_err(|err| format!("failed to initialize backend: {err}"))?;
            let api = Arc::new(api);
            app.manage(ApiState {
                api: Arc::clone(&api),
            });
            tauri::async_runtime::spawn(async move {
                if let Ok(accounts) = api.list_accounts() {
                    for account in accounts.into_iter().filter(|account| account.sync_enabled) {
                        let _ = api.sync_account(account.id).await;
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            add_account,
            test_account_connection,
            list_accounts,
            get_account_config,
            save_account_config,
            sync_account,
            get_sync_status,
            list_folders,
            list_messages,
            get_message,
            search_messages,
            execute_mail_action,
            send_message,
            get_audit_log,
            list_pending_actions,
            confirm_action,
            reject_action,
            get_ai_settings,
            save_ai_settings,
            clear_ai_settings,
            run_ai_analysis,
            list_ai_insights,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run AgentMail");
}
