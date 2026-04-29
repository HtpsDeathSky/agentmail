use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub const MAIL_SYNC_EVENT: &str = "agentmail-mail-sync";

#[derive(Clone, Serialize)]
pub struct MailSyncEventPayload {
    pub account_id: String,
    pub folder_id: Option<String>,
    pub reason: &'static str,
    pub message: Option<String>,
}

pub fn emit_mail_sync_event(app: &AppHandle, payload: MailSyncEventPayload) {
    let _ = app.emit(MAIL_SYNC_EVENT, payload);
}
