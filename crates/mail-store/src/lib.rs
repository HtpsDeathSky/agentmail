use std::path::Path;
use std::sync::Arc;

use mail_core::{
    now_rfc3339, ActionAuditStatus, AiInsight, AiSettings, AttachmentRef, FolderRole, MailAccount,
    MailActionAudit, MailActionKind, MailFolder, MailMessage, MessageFlags, MessageQuery,
    PendingActionStatus, PendingMailAction, SyncState, SyncStateKind,
};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not found: {0}")]
    NotFound(String),
}

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Clone)]
pub struct MailStore {
    conn: Arc<Mutex<Connection>>,
}

impl MailStore {
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn memory() -> StoreResult<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn migrate(&self) -> StoreResult<()> {
        let mut conn = self.conn.lock();
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;

            CREATE TABLE IF NOT EXISTS accounts (
              id TEXT PRIMARY KEY,
              display_name TEXT NOT NULL,
              email TEXT NOT NULL,
              password TEXT NOT NULL DEFAULT '',
              imap_host TEXT NOT NULL,
              imap_port INTEGER NOT NULL,
              imap_tls INTEGER NOT NULL,
              smtp_host TEXT NOT NULL,
              smtp_port INTEGER NOT NULL,
              smtp_tls INTEGER NOT NULL,
              sync_enabled INTEGER NOT NULL,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS folders (
              id TEXT PRIMARY KEY,
              account_id TEXT NOT NULL,
              name TEXT NOT NULL,
              path TEXT NOT NULL,
              role TEXT NOT NULL,
              unread_count INTEGER NOT NULL DEFAULT 0,
              total_count INTEGER NOT NULL DEFAULT 0,
              UNIQUE(account_id, path),
              FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS messages (
              id TEXT PRIMARY KEY,
              account_id TEXT NOT NULL,
              folder_id TEXT NOT NULL,
              uid TEXT,
              message_id_header TEXT,
              subject TEXT NOT NULL,
              sender TEXT NOT NULL,
              recipients_json TEXT NOT NULL,
              cc_json TEXT NOT NULL,
              received_at TEXT NOT NULL,
              body_preview TEXT NOT NULL,
              body TEXT,
              attachments_json TEXT NOT NULL,
              is_read INTEGER NOT NULL,
              is_starred INTEGER NOT NULL,
              is_answered INTEGER NOT NULL,
              is_forwarded INTEGER NOT NULL,
              size_bytes INTEGER,
              deleted_at TEXT,
              UNIQUE(account_id, folder_id, uid),
              FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE,
              FOREIGN KEY(folder_id) REFERENCES folders(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_messages_account_folder
              ON messages(account_id, folder_id, received_at DESC);
            CREATE INDEX IF NOT EXISTS idx_messages_deleted
              ON messages(deleted_at);

            CREATE TABLE IF NOT EXISTS attachments (
              id TEXT PRIMARY KEY,
              message_id TEXT NOT NULL,
              filename TEXT NOT NULL,
              mime_type TEXT NOT NULL,
              size_bytes INTEGER NOT NULL,
              local_path TEXT,
              FOREIGN KEY(message_id) REFERENCES messages(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS sync_states (
              account_id TEXT NOT NULL,
              folder_id TEXT,
              state TEXT NOT NULL,
              last_uid TEXT,
              last_synced_at TEXT,
              error_message TEXT,
              backoff_until TEXT,
              failure_count INTEGER NOT NULL DEFAULT 0,
              PRIMARY KEY(account_id, folder_id),
              FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS ai_settings (
              id TEXT PRIMARY KEY,
              provider_name TEXT NOT NULL,
              base_url TEXT NOT NULL,
              model TEXT NOT NULL,
              api_key TEXT NOT NULL,
              enabled INTEGER NOT NULL,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS ai_insights (
              id TEXT PRIMARY KEY,
              message_id TEXT NOT NULL,
              kind TEXT NOT NULL,
              payload_json TEXT NOT NULL,
              created_at TEXT NOT NULL,
              FOREIGN KEY(message_id) REFERENCES messages(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_ai_insights_message_kind_created
              ON ai_insights(message_id, kind, created_at DESC);

            CREATE TABLE IF NOT EXISTS ai_audits (
              id TEXT PRIMARY KEY,
              message_id TEXT NOT NULL,
              sensitivity_level TEXT NOT NULL,
              uploaded_fields_json TEXT NOT NULL,
              redaction_applied INTEGER NOT NULL,
              model TEXT NOT NULL,
              request_time TEXT NOT NULL,
              result_id TEXT,
              FOREIGN KEY(message_id) REFERENCES messages(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS action_audits (
              id TEXT PRIMARY KEY,
              account_id TEXT NOT NULL,
              action TEXT NOT NULL,
              message_ids_json TEXT NOT NULL,
              status TEXT NOT NULL,
              error_message TEXT,
              created_at TEXT NOT NULL,
              FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS pending_actions (
              id TEXT PRIMARY KEY,
              account_id TEXT NOT NULL,
              action TEXT NOT NULL,
              message_ids_json TEXT NOT NULL,
              target_folder_id TEXT,
              local_message_id TEXT,
              draft_json TEXT,
              status TEXT NOT NULL,
              error_message TEXT,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_pending_actions_account_status
              ON pending_actions(account_id, status, created_at DESC);

            CREATE VIRTUAL TABLE IF NOT EXISTS message_fts USING fts5(
              message_id UNINDEXED,
              subject,
              sender,
              recipients,
              body,
              summary
            );
            "#,
        )?;
        ensure_ai_insights_message_fk(&mut conn)?;
        conn.execute_batch(
            r#"
            CREATE INDEX IF NOT EXISTS idx_ai_insights_message_kind_created
              ON ai_insights(message_id, kind, created_at DESC);
            "#,
        )?;
        ensure_column(
            &conn,
            "sync_states",
            "failure_count",
            "ALTER TABLE sync_states ADD COLUMN failure_count INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_column(
            &conn,
            "accounts",
            "password",
            "ALTER TABLE accounts ADD COLUMN password TEXT NOT NULL DEFAULT ''",
        )?;
        ensure_column(
            &conn,
            "pending_actions",
            "local_message_id",
            "ALTER TABLE pending_actions ADD COLUMN local_message_id TEXT",
        )?;
        conn.execute(
            "UPDATE sync_states SET folder_id = '' WHERE folder_id IS NULL",
            [],
        )?;
        Ok(())
    }

    pub fn save_account_with_password(
        &self,
        account: &MailAccount,
        password: &str,
    ) -> StoreResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"
            INSERT INTO accounts (
              id, display_name, email, password, imap_host, imap_port, imap_tls,
              smtp_host, smtp_port, smtp_tls, sync_enabled, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(id) DO UPDATE SET
              display_name=excluded.display_name,
              email=excluded.email,
              password=excluded.password,
              imap_host=excluded.imap_host,
              imap_port=excluded.imap_port,
              imap_tls=excluded.imap_tls,
              smtp_host=excluded.smtp_host,
              smtp_port=excluded.smtp_port,
              smtp_tls=excluded.smtp_tls,
              sync_enabled=excluded.sync_enabled,
              updated_at=excluded.updated_at
            "#,
            params![
                account.id,
                account.display_name,
                account.email,
                password,
                account.imap_host,
                account.imap_port,
                account.imap_tls,
                account.smtp_host,
                account.smtp_port,
                account.smtp_tls,
                account.sync_enabled,
                account.created_at,
                account.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn save_account(&self, account: &MailAccount) -> StoreResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"
            INSERT INTO accounts (
              id, display_name, email, imap_host, imap_port, imap_tls,
              smtp_host, smtp_port, smtp_tls, sync_enabled, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            ON CONFLICT(id) DO UPDATE SET
              display_name=excluded.display_name,
              email=excluded.email,
              imap_host=excluded.imap_host,
              imap_port=excluded.imap_port,
              imap_tls=excluded.imap_tls,
              smtp_host=excluded.smtp_host,
              smtp_port=excluded.smtp_port,
              smtp_tls=excluded.smtp_tls,
              sync_enabled=excluded.sync_enabled,
              updated_at=excluded.updated_at
            "#,
            params![
                account.id,
                account.display_name,
                account.email,
                account.imap_host,
                account.imap_port,
                account.imap_tls,
                account.smtp_host,
                account.smtp_port,
                account.smtp_tls,
                account.sync_enabled,
                account.created_at,
                account.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_accounts(&self) -> StoreResult<Vec<MailAccount>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"
            SELECT id, display_name, email, imap_host, imap_port, imap_tls,
                   smtp_host, smtp_port, smtp_tls, sync_enabled, created_at, updated_at
            FROM accounts
            ORDER BY created_at ASC
            "#,
        )?;

        let rows = stmt.query_map([], account_from_row)?;
        collect_rows(rows)
    }

    pub fn get_account_password(&self, id: &str) -> StoreResult<String> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT password FROM accounts WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("account {id}")))
    }

    pub fn get_account(&self, id: &str) -> StoreResult<MailAccount> {
        let conn = self.conn.lock();
        conn.query_row(
            r#"
            SELECT id, display_name, email, imap_host, imap_port, imap_tls,
                   smtp_host, smtp_port, smtp_tls, sync_enabled, created_at, updated_at
            FROM accounts
            WHERE id = ?1
            "#,
            params![id],
            account_from_row,
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("account {id}")))
    }

    pub fn save_folder(&self, folder: &MailFolder) -> StoreResult<()> {
        let conn = self.conn.lock();
        save_folder_on_conn(&conn, folder)?;
        Ok(())
    }

    pub fn list_folders(&self, account_id: &str) -> StoreResult<Vec<MailFolder>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"
            SELECT id, account_id, name, path, role, unread_count, total_count
            FROM folders
            WHERE account_id = ?1
            ORDER BY
              CASE role
                WHEN 'inbox' THEN 0
                WHEN 'sent' THEN 1
                WHEN 'archive' THEN 2
                WHEN 'drafts' THEN 3
                WHEN 'junk' THEN 4
                WHEN 'trash' THEN 5
                ELSE 9
              END,
              name ASC
            "#,
        )?;
        let rows = stmt.query_map(params![account_id], folder_from_row)?;
        collect_rows(rows)
    }

    pub fn get_folder(&self, id: &str) -> StoreResult<MailFolder> {
        let conn = self.conn.lock();
        conn.query_row(
            r#"
            SELECT id, account_id, name, path, role, unread_count, total_count
            FROM folders
            WHERE id = ?1
            "#,
            params![id],
            folder_from_row,
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("folder {id}")))
    }

    pub fn find_folder_by_role(
        &self,
        account_id: &str,
        role: FolderRole,
    ) -> StoreResult<Option<MailFolder>> {
        let conn = self.conn.lock();
        conn.query_row(
            r#"
            SELECT id, account_id, name, path, role, unread_count, total_count
            FROM folders
            WHERE account_id = ?1 AND role = ?2
            ORDER BY name ASC
            LIMIT 1
            "#,
            params![account_id, folder_role_to_str(role)],
            folder_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn refresh_folder_counts(&self, folder_id: &str) -> StoreResult<()> {
        let conn = self.conn.lock();
        refresh_folder_counts_on_conn(&conn, folder_id)?;
        Ok(())
    }

    pub fn upsert_message(&self, message: &MailMessage) -> StoreResult<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        upsert_message_tx(&tx, message)?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_messages(&self, query: &MessageQuery) -> StoreResult<Vec<MailMessage>> {
        let conn = self.conn.lock();
        let account_clause = query.account_id.as_deref().unwrap_or("%");
        let folder_clause = query.folder_id.as_deref().unwrap_or("%");

        let mut stmt = conn.prepare(
            r#"
            SELECT id, account_id, folder_id, uid, message_id_header, subject, sender,
                   recipients_json, cc_json, received_at, body_preview, body, attachments_json,
                   is_read, is_starred, is_answered, is_forwarded, size_bytes, deleted_at
            FROM messages
            WHERE account_id LIKE ?1
              AND folder_id LIKE ?2
              AND deleted_at IS NULL
            ORDER BY received_at DESC
            LIMIT ?3 OFFSET ?4
            "#,
        )?;

        let rows = stmt.query_map(
            params![
                account_clause,
                folder_clause,
                query.limit as i64,
                query.offset as i64,
            ],
            message_from_row,
        )?;
        collect_rows(rows)
    }

    pub fn get_message(&self, id: &str) -> StoreResult<MailMessage> {
        let conn = self.conn.lock();
        conn.query_row(
            r#"
            SELECT id, account_id, folder_id, uid, message_id_header, subject, sender,
                   recipients_json, cc_json, received_at, body_preview, body, attachments_json,
                   is_read, is_starred, is_answered, is_forwarded, size_bytes, deleted_at
            FROM messages
            WHERE id = ?1
            "#,
            params![id],
            message_from_row,
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("message {id}")))
    }

    pub fn search_messages(&self, term: &str, limit: u32) -> StoreResult<Vec<MailMessage>> {
        if term.trim().is_empty() {
            return self.list_messages(&MessageQuery {
                limit,
                ..MessageQuery::default()
            });
        }

        let escaped = term.trim().replace('"', "\"\"");
        let fts_query = format!("\"{escaped}\"");
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"
            SELECT m.id, m.account_id, m.folder_id, m.uid, m.message_id_header,
                   m.subject, m.sender, m.recipients_json, m.cc_json, m.received_at,
                   m.body_preview, m.body, m.attachments_json, m.is_read, m.is_starred,
                   m.is_answered, m.is_forwarded, m.size_bytes, m.deleted_at
            FROM message_fts
            JOIN messages m ON m.id = message_fts.message_id
            WHERE message_fts MATCH ?1
              AND m.deleted_at IS NULL
            ORDER BY bm25(message_fts)
            LIMIT ?2
            "#,
        )?;
        let rows = stmt.query_map(params![fts_query, limit as i64], message_from_row)?;
        collect_rows(rows)
    }

    pub fn save_sync_state(&self, state: &SyncState) -> StoreResult<()> {
        let conn = self.conn.lock();
        let folder_id = state.folder_id.as_deref().unwrap_or("");
        conn.execute(
            r#"
            INSERT INTO sync_states (
              account_id, folder_id, state, last_uid, last_synced_at, error_message, backoff_until, failure_count
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(account_id, folder_id) DO UPDATE SET
              state=excluded.state,
              last_uid=excluded.last_uid,
              last_synced_at=excluded.last_synced_at,
              error_message=excluded.error_message,
              backoff_until=excluded.backoff_until,
              failure_count=excluded.failure_count
            "#,
            params![
                state.account_id,
                folder_id,
                sync_state_to_str(state.state),
                state.last_uid,
                state.last_synced_at,
                state.error_message,
                state.backoff_until,
                state.failure_count,
            ],
        )?;
        Ok(())
    }

    pub fn get_sync_status(&self, account_id: &str) -> StoreResult<Vec<SyncState>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"
            SELECT account_id, folder_id, state, last_uid, last_synced_at, error_message, backoff_until, failure_count
            FROM sync_states
            WHERE account_id = ?1
            ORDER BY folder_id ASC
            "#,
        )?;
        let rows = stmt.query_map(params![account_id], sync_state_from_row)?;
        collect_rows(rows)
    }

    pub fn get_sync_state(
        &self,
        account_id: &str,
        folder_id: Option<&str>,
    ) -> StoreResult<Option<SyncState>> {
        let conn = self.conn.lock();
        let mut stmt = if folder_id.is_some() {
            conn.prepare(
                r#"
                SELECT account_id, folder_id, state, last_uid, last_synced_at, error_message, backoff_until, failure_count
                FROM sync_states
                WHERE account_id = ?1 AND folder_id = ?2
                "#,
            )?
        } else {
            conn.prepare(
                r#"
                SELECT account_id, folder_id, state, last_uid, last_synced_at, error_message, backoff_until, failure_count
                FROM sync_states
                WHERE account_id = ?1 AND folder_id = ''
                "#,
            )?
        };

        if let Some(folder_id) = folder_id {
            stmt.query_row(params![account_id, folder_id], sync_state_from_row)
                .optional()
                .map_err(Into::into)
        } else {
            stmt.query_row(params![account_id], sync_state_from_row)
                .optional()
                .map_err(Into::into)
        }
    }

    pub fn set_message_flags(
        &self,
        message_ids: &[String],
        flags: MessageFlagPatch,
    ) -> StoreResult<()> {
        let conn = self.conn.lock();
        for id in message_ids {
            if let Some(is_read) = flags.is_read {
                conn.execute(
                    "UPDATE messages SET is_read = ?1 WHERE id = ?2",
                    params![is_read, id],
                )?;
            }
            if let Some(is_starred) = flags.is_starred {
                conn.execute(
                    "UPDATE messages SET is_starred = ?1 WHERE id = ?2",
                    params![is_starred, id],
                )?;
            }
        }
        Ok(())
    }

    pub fn move_messages(&self, message_ids: &[String], target_folder_id: &str) -> StoreResult<()> {
        let conn = self.conn.lock();
        for id in message_ids {
            conn.execute(
                "UPDATE messages SET folder_id = ?1 WHERE id = ?2",
                params![target_folder_id, id],
            )?;
        }
        Ok(())
    }

    pub fn move_messages_and_clear_uids(
        &self,
        message_ids: &[String],
        target_folder_id: &str,
    ) -> StoreResult<()> {
        let conn = self.conn.lock();
        for id in message_ids {
            conn.execute(
                "UPDATE messages SET folder_id = ?1, uid = NULL WHERE id = ?2",
                params![target_folder_id, id],
            )?;
        }
        Ok(())
    }

    pub fn soft_delete_messages(&self, message_ids: &[String]) -> StoreResult<()> {
        let deleted_at = now_rfc3339();
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        for id in message_ids {
            tx.execute(
                "UPDATE messages SET deleted_at = ?1 WHERE id = ?2",
                params![deleted_at, id],
            )?;
            tx.execute("DELETE FROM message_fts WHERE message_id = ?1", params![id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn move_uidless_messages_to_folder(
        &self,
        account_id: &str,
        source_folder_id: &str,
        target_folder_id: &str,
    ) -> StoreResult<usize> {
        if source_folder_id == target_folder_id {
            return Ok(0);
        }
        let conn = self.conn.lock();
        let changed = conn.execute(
            r#"
            UPDATE messages
            SET folder_id = ?3
            WHERE account_id = ?1
              AND folder_id = ?2
              AND uid IS NULL
              AND message_id_header IS NOT NULL
              AND deleted_at IS NULL
            "#,
            params![account_id, source_folder_id, target_folder_id],
        )?;
        Ok(changed)
    }

    pub fn write_audit(&self, audit: &MailActionAudit) -> StoreResult<()> {
        let conn = self.conn.lock();
        write_audit_on_conn(&conn, audit)?;
        Ok(())
    }

    pub fn list_audits(&self, limit: u32) -> StoreResult<Vec<MailActionAudit>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"
            SELECT id, account_id, action, message_ids_json, status, error_message, created_at
            FROM action_audits
            ORDER BY created_at DESC
            LIMIT ?1
            "#,
        )?;
        let rows = stmt.query_map(params![limit as i64], audit_from_row)?;
        collect_rows(rows)
    }

    pub fn save_pending_action(&self, action: &PendingMailAction) -> StoreResult<()> {
        let conn = self.conn.lock();
        save_pending_action_on_conn(&conn, action)?;
        Ok(())
    }

    pub fn save_queued_send_with_placeholder(
        &self,
        pending: &PendingMailAction,
        audit: &MailActionAudit,
        sent_folder: &MailFolder,
        placeholder: &MailMessage,
    ) -> StoreResult<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        save_folder_on_conn(&tx, sent_folder)?;
        save_pending_action_on_conn(&tx, pending)?;
        write_audit_on_conn(&tx, audit)?;
        upsert_message_tx(&tx, placeholder)?;
        refresh_folder_counts_on_conn(&tx, &sent_folder.id)?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_pending_action(&self, id: &str) -> StoreResult<PendingMailAction> {
        let conn = self.conn.lock();
        conn.query_row(
            r#"
            SELECT id, account_id, action, message_ids_json, target_folder_id, local_message_id, draft_json,
                   status, error_message, created_at, updated_at
            FROM pending_actions
            WHERE id = ?1
            "#,
            params![id],
            pending_action_from_row,
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("pending action {id}")))
    }

    pub fn list_pending_actions(
        &self,
        account_id: Option<&str>,
    ) -> StoreResult<Vec<PendingMailAction>> {
        let conn = self.conn.lock();
        if let Some(account_id) = account_id {
            let mut stmt = conn.prepare(
                r#"
                SELECT id, account_id, action, message_ids_json, target_folder_id, local_message_id, draft_json,
                       status, error_message, created_at, updated_at
                FROM pending_actions
                WHERE account_id = ?1 AND status = 'pending'
                ORDER BY created_at DESC
                "#,
            )?;
            let rows = stmt.query_map(params![account_id], pending_action_from_row)?;
            collect_rows(rows)
        } else {
            let mut stmt = conn.prepare(
                r#"
                SELECT id, account_id, action, message_ids_json, target_folder_id, local_message_id, draft_json,
                       status, error_message, created_at, updated_at
                FROM pending_actions
                WHERE status = 'pending'
                ORDER BY created_at DESC
                "#,
            )?;
            let rows = stmt.query_map([], pending_action_from_row)?;
            collect_rows(rows)
        }
    }

    pub fn update_pending_action_status(
        &self,
        id: &str,
        status: PendingActionStatus,
        error_message: Option<&str>,
    ) -> StoreResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"
            UPDATE pending_actions
            SET status = ?1, error_message = ?2, updated_at = ?3
            WHERE id = ?4
            "#,
            params![
                pending_status_to_str(status),
                error_message,
                now_rfc3339(),
                id,
            ],
        )?;
        Ok(())
    }

    pub fn save_ai_settings(&self, settings: &AiSettings) -> StoreResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"
            INSERT INTO ai_settings (
              id, provider_name, base_url, model, api_key, enabled, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(id) DO UPDATE SET
              provider_name=excluded.provider_name,
              base_url=excluded.base_url,
              model=excluded.model,
              api_key=excluded.api_key,
              enabled=excluded.enabled,
              updated_at=excluded.updated_at
            "#,
            params![
                settings.id,
                settings.provider_name,
                settings.base_url,
                settings.model,
                settings.api_key,
                settings.enabled,
                settings.created_at,
                settings.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_ai_settings(&self) -> StoreResult<Option<AiSettings>> {
        let conn = self.conn.lock();
        conn.query_row(
            r#"
            SELECT id, provider_name, base_url, model, api_key, enabled, created_at, updated_at
            FROM ai_settings
            WHERE id = 'default'
            "#,
            [],
            ai_settings_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn clear_ai_settings(&self) -> StoreResult<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM ai_settings", [])?;
        Ok(())
    }

    pub fn save_ai_insight(&self, insight: &AiInsight) -> StoreResult<()> {
        let payload_json = serde_json::to_string(insight)?;
        let conn = self.conn.lock();
        conn.execute(
            r#"
            INSERT INTO ai_insights (id, message_id, kind, payload_json, created_at)
            VALUES (?1, ?2, 'mail_analysis', ?3, ?4)
            ON CONFLICT(id) DO UPDATE SET
              message_id=excluded.message_id,
              kind=excluded.kind,
              payload_json=excluded.payload_json,
              created_at=excluded.created_at
            "#,
            params![
                insight.id,
                insight.message_id,
                payload_json,
                insight.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_ai_insights(&self, message_id: &str) -> StoreResult<Vec<AiInsight>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"
            SELECT payload_json
            FROM ai_insights
            WHERE message_id = ?1 AND kind = 'mail_analysis'
            ORDER BY created_at DESC
            "#,
        )?;
        let rows = stmt.query_map(params![message_id], ai_insight_from_row)?;
        collect_rows(rows)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MessageFlagPatch {
    pub is_read: Option<bool>,
    pub is_starred: Option<bool>,
}

fn collect_rows<T>(rows: impl Iterator<Item = rusqlite::Result<T>>) -> StoreResult<Vec<T>> {
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn ensure_column(conn: &Connection, table: &str, column: &str, alter_sql: &str) -> StoreResult<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(());
        }
    }
    conn.execute(alter_sql, [])?;
    Ok(())
}

fn save_folder_on_conn(conn: &Connection, folder: &MailFolder) -> StoreResult<()> {
    conn.execute(
        r#"
        INSERT INTO folders (id, account_id, name, path, role, unread_count, total_count)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(account_id, path) DO UPDATE SET
          name=excluded.name,
          role=excluded.role,
          unread_count=excluded.unread_count,
          total_count=excluded.total_count
        "#,
        params![
            folder.id,
            folder.account_id,
            folder.name,
            folder.path,
            folder_role_to_str(folder.role),
            folder.unread_count,
            folder.total_count,
        ],
    )?;
    Ok(())
}

fn refresh_folder_counts_on_conn(conn: &Connection, folder_id: &str) -> StoreResult<()> {
    conn.execute(
        r#"
        UPDATE folders
        SET
          unread_count = (
            SELECT COUNT(*)
            FROM messages
            WHERE folder_id = ?1 AND is_read = 0 AND deleted_at IS NULL
          ),
          total_count = (
            SELECT COUNT(*)
            FROM messages
            WHERE folder_id = ?1 AND deleted_at IS NULL
          )
        WHERE id = ?1
        "#,
        params![folder_id],
    )?;
    Ok(())
}

fn write_audit_on_conn(conn: &Connection, audit: &MailActionAudit) -> StoreResult<()> {
    let message_ids_json = serde_json::to_string(&audit.message_ids)?;
    conn.execute(
        r#"
        INSERT INTO action_audits (
          id, account_id, action, message_ids_json, status, error_message, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
        params![
            audit.id,
            audit.account_id,
            action_to_str(audit.action),
            message_ids_json,
            audit_status_to_str(audit.status),
            audit.error_message,
            audit.created_at,
        ],
    )?;
    Ok(())
}

fn save_pending_action_on_conn(conn: &Connection, action: &PendingMailAction) -> StoreResult<()> {
    let message_ids_json = serde_json::to_string(&action.message_ids)?;
    let draft_json = action
        .draft
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    conn.execute(
        r#"
        INSERT INTO pending_actions (
          id, account_id, action, message_ids_json, target_folder_id, local_message_id,
          draft_json, status, error_message, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(id) DO UPDATE SET
          status=excluded.status,
          error_message=excluded.error_message,
          updated_at=excluded.updated_at
        "#,
        params![
            action.id,
            action.account_id,
            action_to_str(action.action),
            message_ids_json,
            action.target_folder_id,
            action.local_message_id,
            draft_json,
            pending_status_to_str(action.status),
            action.error_message,
            action.created_at,
            action.updated_at,
        ],
    )?;
    Ok(())
}

fn upsert_message_tx(tx: &Transaction<'_>, message: &MailMessage) -> StoreResult<String> {
    let recipients_json = serde_json::to_string(&message.recipients)?;
    let cc_json = serde_json::to_string(&message.cc)?;
    let attachments_json = serde_json::to_string(&message.attachments)?;
    let body_for_fts = message.body.as_deref().unwrap_or(&message.body_preview);
    let recipients_for_fts = message.recipients.join(" ");

    let hydrated_placeholder_id = match (&message.uid, &message.message_id_header) {
        (Some(_), Some(message_id_header)) => tx
            .query_row(
                r#"
                SELECT id
                FROM messages
                WHERE account_id = ?1
                  AND folder_id = ?2
                  AND uid IS NULL
                  AND message_id_header = ?3
                  AND deleted_at IS NULL
                ORDER BY received_at DESC
                LIMIT 1
                "#,
                params![message.account_id, message.folder_id, message_id_header],
                |row| row.get::<_, String>(0),
            )
            .optional()?,
        _ => None,
    };

    let stored_message_id = if let Some(existing_id) = hydrated_placeholder_id {
        if let Some(uid) = message.uid.as_deref() {
            let duplicate_id = tx
                .query_row(
                    r#"
                    SELECT id
                    FROM messages
                    WHERE account_id = ?1
                      AND folder_id = ?2
                      AND uid = ?3
                      AND id != ?4
                      AND message_id_header = ?5
                    LIMIT 1
                    "#,
                    params![
                        message.account_id,
                        message.folder_id,
                        uid,
                        existing_id,
                        message.message_id_header,
                    ],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            if let Some(duplicate_id) = duplicate_id {
                tx.execute(
                    "DELETE FROM message_fts WHERE message_id = ?1",
                    params![duplicate_id],
                )?;
                tx.execute("DELETE FROM messages WHERE id = ?1", params![duplicate_id])?;
            }
        }
        tx.execute(
            r#"
            UPDATE messages
            SET uid=?2,
                message_id_header=?3,
                subject=?4,
                sender=?5,
                recipients_json=?6,
                cc_json=?7,
                received_at=?8,
                body_preview=?9,
                body=?10,
                attachments_json=?11,
                is_read=?12,
                is_starred=?13,
                is_answered=?14,
                is_forwarded=?15,
                size_bytes=?16,
                deleted_at=?17
            WHERE id = ?1
            "#,
            params![
                existing_id,
                message.uid,
                message.message_id_header,
                message.subject,
                message.sender,
                recipients_json,
                cc_json,
                message.received_at,
                message.body_preview,
                message.body,
                attachments_json,
                message.flags.is_read,
                message.flags.is_starred,
                message.flags.is_answered,
                message.flags.is_forwarded,
                message.size_bytes,
                message.deleted_at,
            ],
        )?;
        existing_id
    } else {
        tx.execute(
            r#"
            INSERT INTO messages (
              id, account_id, folder_id, uid, message_id_header, subject, sender,
              recipients_json, cc_json, received_at, body_preview, body, attachments_json,
              is_read, is_starred, is_answered, is_forwarded, size_bytes, deleted_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
            ON CONFLICT(account_id, folder_id, uid) DO UPDATE SET
              message_id_header=excluded.message_id_header,
              subject=excluded.subject,
              sender=excluded.sender,
              recipients_json=excluded.recipients_json,
              cc_json=excluded.cc_json,
              received_at=excluded.received_at,
              body_preview=excluded.body_preview,
              body=excluded.body,
              attachments_json=excluded.attachments_json,
              is_read=excluded.is_read,
              is_starred=excluded.is_starred,
              is_answered=excluded.is_answered,
              is_forwarded=excluded.is_forwarded,
              size_bytes=excluded.size_bytes,
              deleted_at=excluded.deleted_at
            "#,
            params![
                message.id,
                message.account_id,
                message.folder_id,
                message.uid,
                message.message_id_header,
                message.subject,
                message.sender,
                recipients_json,
                cc_json,
                message.received_at,
                message.body_preview,
                message.body,
                attachments_json,
                message.flags.is_read,
                message.flags.is_starred,
                message.flags.is_answered,
                message.flags.is_forwarded,
                message.size_bytes,
                message.deleted_at,
            ],
        )?;
        if let Some(uid) = message.uid.as_deref() {
            tx.query_row(
                r#"
                SELECT id
                FROM messages
                WHERE account_id = ?1 AND folder_id = ?2 AND uid = ?3
                LIMIT 1
                "#,
                params![message.account_id, message.folder_id, uid],
                |row| row.get::<_, String>(0),
            )?
        } else {
            message.id.clone()
        }
    };

    tx.execute(
        "DELETE FROM message_fts WHERE message_id = ?1 OR message_id = ?2",
        params![stored_message_id, message.id],
    )?;
    if message.deleted_at.is_none() {
        tx.execute(
            r#"
            INSERT INTO message_fts (message_id, subject, sender, recipients, body, summary)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                stored_message_id,
                message.subject,
                message.sender,
                recipients_for_fts,
                body_for_fts,
                message.body_preview,
            ],
        )?;
    }
    Ok(stored_message_id)
}

fn ensure_ai_insights_message_fk(conn: &mut Connection) -> StoreResult<()> {
    let mut stmt = conn.prepare("PRAGMA foreign_key_list(ai_insights)")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let table: String = row.get(2)?;
        let from: String = row.get(3)?;
        let to: String = row.get(4)?;
        let on_delete: String = row.get(6)?;
        if table == "messages"
            && from == "message_id"
            && to == "id"
            && on_delete.eq_ignore_ascii_case("CASCADE")
        {
            return Ok(());
        }
    }
    drop(rows);
    drop(stmt);

    let tx = conn.transaction()?;
    tx.execute_batch(
        r#"
        DROP TABLE IF EXISTS ai_insights_new;
        CREATE TABLE ai_insights_new (
          id TEXT PRIMARY KEY,
          message_id TEXT NOT NULL,
          kind TEXT NOT NULL,
          payload_json TEXT NOT NULL,
          created_at TEXT NOT NULL,
          FOREIGN KEY(message_id) REFERENCES messages(id) ON DELETE CASCADE
        );
        INSERT INTO ai_insights_new (id, message_id, kind, payload_json, created_at)
        SELECT id, message_id, kind, payload_json, created_at
        FROM ai_insights
        WHERE EXISTS (
          SELECT 1
          FROM messages
          WHERE messages.id = ai_insights.message_id
        );
        DROP TABLE ai_insights;
        ALTER TABLE ai_insights_new RENAME TO ai_insights;
        "#,
    )?;
    tx.commit()?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(())
}

fn account_from_row(row: &Row<'_>) -> rusqlite::Result<MailAccount> {
    Ok(MailAccount {
        id: row.get(0)?,
        display_name: row.get(1)?,
        email: row.get(2)?,
        imap_host: row.get(3)?,
        imap_port: row.get::<_, i64>(4)? as u16,
        imap_tls: row.get(5)?,
        smtp_host: row.get(6)?,
        smtp_port: row.get::<_, i64>(7)? as u16,
        smtp_tls: row.get(8)?,
        sync_enabled: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn folder_from_row(row: &Row<'_>) -> rusqlite::Result<MailFolder> {
    let role: String = row.get(4)?;
    Ok(MailFolder {
        id: row.get(0)?,
        account_id: row.get(1)?,
        name: row.get(2)?,
        path: row.get(3)?,
        role: folder_role_from_str(&role),
        unread_count: row.get::<_, i64>(5)? as u32,
        total_count: row.get::<_, i64>(6)? as u32,
    })
}

fn message_from_row(row: &Row<'_>) -> rusqlite::Result<MailMessage> {
    let recipients_json: String = row.get(7)?;
    let cc_json: String = row.get(8)?;
    let attachments_json: String = row.get(12)?;
    let recipients = serde_json::from_str(&recipients_json).unwrap_or_default();
    let cc = serde_json::from_str(&cc_json).unwrap_or_default();
    let attachments =
        serde_json::from_str::<Vec<AttachmentRef>>(&attachments_json).unwrap_or_default();

    Ok(MailMessage {
        id: row.get(0)?,
        account_id: row.get(1)?,
        folder_id: row.get(2)?,
        uid: row.get(3)?,
        message_id_header: row.get(4)?,
        subject: row.get(5)?,
        sender: row.get(6)?,
        recipients,
        cc,
        received_at: row.get(9)?,
        body_preview: row.get(10)?,
        body: row.get(11)?,
        attachments,
        flags: MessageFlags {
            is_read: row.get(13)?,
            is_starred: row.get(14)?,
            is_answered: row.get(15)?,
            is_forwarded: row.get(16)?,
        },
        size_bytes: row.get(17)?,
        deleted_at: row.get(18)?,
    })
}

fn sync_state_from_row(row: &Row<'_>) -> rusqlite::Result<SyncState> {
    let state: String = row.get(2)?;
    let folder_id: Option<String> = row.get(1)?;
    Ok(SyncState {
        account_id: row.get(0)?,
        folder_id: folder_id.filter(|value| !value.is_empty()),
        state: sync_state_from_str(&state),
        last_uid: row.get(3)?,
        last_synced_at: row.get(4)?,
        error_message: row.get(5)?,
        backoff_until: row.get(6)?,
        failure_count: row.get::<_, i64>(7)? as u32,
    })
}

fn audit_from_row(row: &Row<'_>) -> rusqlite::Result<MailActionAudit> {
    let action: String = row.get(2)?;
    let message_ids_json: String = row.get(3)?;
    let status: String = row.get(4)?;
    let message_ids = serde_json::from_str(&message_ids_json).unwrap_or_default();
    Ok(MailActionAudit {
        id: row.get(0)?,
        account_id: row.get(1)?,
        action: action_from_str(&action),
        message_ids,
        status: audit_status_from_str(&status),
        error_message: row.get(5)?,
        created_at: row.get(6)?,
    })
}

fn pending_action_from_row(row: &Row<'_>) -> rusqlite::Result<PendingMailAction> {
    let action: String = row.get(2)?;
    let message_ids_json: String = row.get(3)?;
    let draft_json: Option<String> = row.get(6)?;
    let status: String = row.get(7)?;
    let message_ids = serde_json::from_str(&message_ids_json).unwrap_or_default();
    let draft = draft_json
        .as_deref()
        .and_then(|value| serde_json::from_str(value).ok());

    Ok(PendingMailAction {
        id: row.get(0)?,
        account_id: row.get(1)?,
        action: action_from_str(&action),
        message_ids,
        target_folder_id: row.get(4)?,
        local_message_id: row.get(5)?,
        draft,
        status: pending_status_from_str(&status),
        error_message: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

fn ai_settings_from_row(row: &Row<'_>) -> rusqlite::Result<AiSettings> {
    Ok(AiSettings {
        id: row.get(0)?,
        provider_name: row.get(1)?,
        base_url: row.get(2)?,
        model: row.get(3)?,
        api_key: row.get(4)?,
        enabled: row.get::<_, bool>(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn ai_insight_from_row(row: &Row<'_>) -> rusqlite::Result<AiInsight> {
    let payload_json: String = row.get(0)?;
    serde_json::from_str::<AiInsight>(&payload_json).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
    })
}

fn folder_role_to_str(role: FolderRole) -> &'static str {
    match role {
        FolderRole::Inbox => "inbox",
        FolderRole::Sent => "sent",
        FolderRole::Archive => "archive",
        FolderRole::Trash => "trash",
        FolderRole::Drafts => "drafts",
        FolderRole::Junk => "junk",
        FolderRole::Custom => "custom",
    }
}

fn folder_role_from_str(role: &str) -> FolderRole {
    match role {
        "inbox" => FolderRole::Inbox,
        "sent" => FolderRole::Sent,
        "archive" => FolderRole::Archive,
        "trash" => FolderRole::Trash,
        "drafts" => FolderRole::Drafts,
        "junk" => FolderRole::Junk,
        _ => FolderRole::Custom,
    }
}

fn sync_state_to_str(state: SyncStateKind) -> &'static str {
    match state {
        SyncStateKind::Idle => "idle",
        SyncStateKind::Syncing => "syncing",
        SyncStateKind::Watching => "watching",
        SyncStateKind::Backoff => "backoff",
        SyncStateKind::Error => "error",
        SyncStateKind::Disabled => "disabled",
    }
}

fn sync_state_from_str(state: &str) -> SyncStateKind {
    match state {
        "syncing" => SyncStateKind::Syncing,
        "watching" => SyncStateKind::Watching,
        "backoff" => SyncStateKind::Backoff,
        "error" => SyncStateKind::Error,
        "disabled" => SyncStateKind::Disabled,
        _ => SyncStateKind::Idle,
    }
}

fn action_to_str(action: MailActionKind) -> &'static str {
    match action {
        MailActionKind::MarkRead => "mark_read",
        MailActionKind::MarkUnread => "mark_unread",
        MailActionKind::Star => "star",
        MailActionKind::Unstar => "unstar",
        MailActionKind::Move => "move",
        MailActionKind::Archive => "archive",
        MailActionKind::Delete => "delete",
        MailActionKind::PermanentDelete => "permanent_delete",
        MailActionKind::Send => "send",
        MailActionKind::Forward => "forward",
        MailActionKind::BatchDelete => "batch_delete",
        MailActionKind::BatchMove => "batch_move",
    }
}

fn action_from_str(action: &str) -> MailActionKind {
    match action {
        "mark_read" => MailActionKind::MarkRead,
        "mark_unread" => MailActionKind::MarkUnread,
        "star" => MailActionKind::Star,
        "unstar" => MailActionKind::Unstar,
        "move" => MailActionKind::Move,
        "archive" => MailActionKind::Archive,
        "delete" => MailActionKind::Delete,
        "permanent_delete" => MailActionKind::PermanentDelete,
        "send" => MailActionKind::Send,
        "forward" => MailActionKind::Forward,
        "batch_delete" => MailActionKind::BatchDelete,
        "batch_move" => MailActionKind::BatchMove,
        _ => MailActionKind::MarkRead,
    }
}

fn audit_status_to_str(status: ActionAuditStatus) -> &'static str {
    match status {
        ActionAuditStatus::Queued => "queued",
        ActionAuditStatus::Accepted => "accepted",
        ActionAuditStatus::Rejected => "rejected",
        ActionAuditStatus::Executed => "executed",
        ActionAuditStatus::Failed => "failed",
    }
}

fn audit_status_from_str(status: &str) -> ActionAuditStatus {
    match status {
        "queued" => ActionAuditStatus::Queued,
        "accepted" => ActionAuditStatus::Accepted,
        "rejected" => ActionAuditStatus::Rejected,
        "failed" => ActionAuditStatus::Failed,
        _ => ActionAuditStatus::Executed,
    }
}

fn pending_status_to_str(status: PendingActionStatus) -> &'static str {
    match status {
        PendingActionStatus::Pending => "pending",
        PendingActionStatus::Accepted => "accepted",
        PendingActionStatus::Rejected => "rejected",
        PendingActionStatus::Executed => "executed",
        PendingActionStatus::Failed => "failed",
    }
}

fn pending_status_from_str(status: &str) -> PendingActionStatus {
    match status {
        "accepted" => PendingActionStatus::Accepted,
        "rejected" => PendingActionStatus::Rejected,
        "executed" => PendingActionStatus::Executed,
        "failed" => PendingActionStatus::Failed,
        _ => PendingActionStatus::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::{
        new_id, AiInsight, AiPriority, AiSettings, FolderRole, MailAccount, MailFolder,
        MailMessage, MessageFlags,
    };

    #[test]
    fn stores_and_searches_messages() {
        let store = MailStore::memory().unwrap();
        let now = now_rfc3339();
        let account = MailAccount {
            id: "acct".to_string(),
            display_name: "Ops".to_string(),
            email: "ops@example.com".to_string(),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_tls: true,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_tls: true,
            sync_enabled: true,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        store.save_account(&account).unwrap();
        let folder = MailFolder {
            id: "inbox".to_string(),
            account_id: account.id.clone(),
            name: "INBOX".to_string(),
            path: "INBOX".to_string(),
            role: FolderRole::Inbox,
            unread_count: 1,
            total_count: 1,
        };
        store.save_folder(&folder).unwrap();
        store
            .upsert_message(&MailMessage {
                id: new_id(),
                account_id: account.id,
                folder_id: folder.id,
                uid: Some("1".to_string()),
                message_id_header: None,
                subject: "Quarterly hardening report".to_string(),
                sender: "security@example.com".to_string(),
                recipients: vec!["ops@example.com".to_string()],
                cc: vec![],
                received_at: now,
                body_preview: "Firewall drift and backup coverage".to_string(),
                body: Some("Firewall drift requires review".to_string()),
                attachments: vec![],
                flags: MessageFlags::default(),
                size_bytes: Some(2048),
                deleted_at: None,
            })
            .unwrap();

        let hits = store.search_messages("Firewall", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].subject, "Quarterly hardening report");
    }

    #[test]
    fn upsert_merges_legacy_placeholder_when_remote_uid_row_already_exists() {
        let store = MailStore::memory().unwrap();
        let now = now_rfc3339();
        let account = MailAccount {
            id: "acct".to_string(),
            display_name: "Ops".to_string(),
            email: "ops@example.com".to_string(),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_tls: true,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_tls: true,
            sync_enabled: true,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        store.save_account(&account).unwrap();
        let folder = MailFolder {
            id: "trash".to_string(),
            account_id: account.id.clone(),
            name: "Trash".to_string(),
            path: "Trash".to_string(),
            role: FolderRole::Trash,
            unread_count: 0,
            total_count: 0,
        };
        store.save_folder(&folder).unwrap();

        let placeholder = MailMessage {
            id: "local-placeholder".to_string(),
            account_id: account.id.clone(),
            folder_id: folder.id.clone(),
            uid: None,
            message_id_header: Some("<dup@example.com>".to_string()),
            subject: "Local placeholder".to_string(),
            sender: "sec@example.com".to_string(),
            recipients: vec![account.email.clone()],
            cc: Vec::new(),
            received_at: now.clone(),
            body_preview: "old local body".to_string(),
            body: Some("old local body".to_string()),
            attachments: Vec::new(),
            flags: MessageFlags::default(),
            size_bytes: Some(100),
            deleted_at: None,
        };
        let mut remote = placeholder.clone();
        remote.id = "remote-duplicate".to_string();
        remote.uid = Some("900".to_string());
        remote.subject = "Remote duplicate".to_string();
        remote.body_preview = "old remote body".to_string();
        remote.body = Some("old remote body".to_string());

        store.upsert_message(&remote).unwrap();
        store.upsert_message(&placeholder).unwrap();

        let mut incoming = remote.clone();
        incoming.id = "incoming-remote".to_string();
        incoming.subject = "Hydrated canonical".to_string();
        incoming.body_preview = "fresh canonical search marker".to_string();
        incoming.body = Some("fresh canonical search marker".to_string());

        store.upsert_message(&incoming).unwrap();

        let messages = store
            .list_messages(&MessageQuery {
                account_id: Some(account.id.clone()),
                folder_id: Some(folder.id.clone()),
                limit: 10,
                offset: 0,
            })
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, placeholder.id);
        assert_eq!(messages[0].uid.as_deref(), Some("900"));
        assert_eq!(messages[0].subject, "Hydrated canonical");

        let hits = store.search_messages("fresh", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, placeholder.id);
        assert!(store.search_messages("old", 10).unwrap().is_empty());
    }

    #[test]
    fn account_level_sync_state_updates_in_place() {
        let store = MailStore::memory().unwrap();
        let now = now_rfc3339();
        let account = MailAccount {
            id: "acct".to_string(),
            display_name: "Ops".to_string(),
            email: "ops@example.com".to_string(),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_tls: true,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_tls: true,
            sync_enabled: true,
            created_at: now.clone(),
            updated_at: now,
        };
        store.save_account(&account).unwrap();

        store
            .save_sync_state(&SyncState {
                account_id: account.id.clone(),
                folder_id: None,
                state: SyncStateKind::Syncing,
                last_uid: Some("10".to_string()),
                last_synced_at: None,
                error_message: None,
                backoff_until: None,
                failure_count: 1,
            })
            .unwrap();
        store
            .save_sync_state(&SyncState {
                account_id: account.id.clone(),
                folder_id: None,
                state: SyncStateKind::Idle,
                last_uid: Some("11".to_string()),
                last_synced_at: Some(now_rfc3339()),
                error_message: None,
                backoff_until: None,
                failure_count: 0,
            })
            .unwrap();

        let states = store.get_sync_status(&account.id).unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].folder_id, None);
        assert_eq!(states[0].last_uid.as_deref(), Some("11"));
        assert_eq!(states[0].failure_count, 0);
    }

    #[test]
    fn account_password_is_stored_in_sqlite_plaintext() {
        let store = MailStore::memory().unwrap();
        let now = now_rfc3339();
        let mut account = MailAccount {
            id: "acct".to_string(),
            display_name: "Ops".to_string(),
            email: "ops@example.com".to_string(),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_tls: true,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_tls: true,
            sync_enabled: true,
            created_at: now.clone(),
            updated_at: now,
        };

        store
            .save_account_with_password(&account, "imap-smtp-secret")
            .unwrap();
        assert_eq!(
            store.get_account_password(&account.id).unwrap(),
            "imap-smtp-secret"
        );

        account.smtp_port = 587;
        account.updated_at = now_rfc3339();
        store
            .save_account_with_password(&account, "updated-secret")
            .unwrap();

        let updated = store.get_account(&account.id).unwrap();
        assert_eq!(updated.smtp_port, 587);
        assert_eq!(
            store.get_account_password(&account.id).unwrap(),
            "updated-secret"
        );
    }

    #[test]
    fn pending_actions_round_trip_and_filter_to_pending() {
        let store = MailStore::memory().unwrap();
        let now = now_rfc3339();
        let account = MailAccount {
            id: "acct".to_string(),
            display_name: "Ops".to_string(),
            email: "ops@example.com".to_string(),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_tls: true,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_tls: true,
            sync_enabled: true,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        store.save_account(&account).unwrap();

        let pending = PendingMailAction {
            id: "pending-1".to_string(),
            account_id: account.id.clone(),
            action: MailActionKind::Send,
            message_ids: Vec::new(),
            target_folder_id: None,
            local_message_id: None,
            draft: Some(mail_core::SendMessageDraft {
                account_id: account.id.clone(),
                to: vec!["sec@example.com".to_string()],
                cc: Vec::new(),
                subject: "Hold point".to_string(),
                body: "confirm before sending".to_string(),
                message_id_header: None,
            }),
            status: PendingActionStatus::Pending,
            error_message: None,
            created_at: now.clone(),
            updated_at: now,
        };
        store.save_pending_action(&pending).unwrap();

        let rows = store.list_pending_actions(Some(&account.id)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].draft.as_ref().unwrap().subject, "Hold point");

        store
            .update_pending_action_status(&pending.id, PendingActionStatus::Rejected, None)
            .unwrap();
        assert!(store
            .list_pending_actions(Some(&account.id))
            .unwrap()
            .is_empty());
        assert_eq!(
            store.get_pending_action(&pending.id).unwrap().status,
            PendingActionStatus::Rejected
        );
    }

    #[test]
    fn ai_settings_round_trip_and_clear() {
        let store = MailStore::memory().unwrap();
        let now = now_rfc3339();
        let settings = AiSettings {
            id: "default".to_string(),
            provider_name: "openai-compatible".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            model: "mail-model".to_string(),
            api_key: "sk-local-test".to_string(),
            enabled: true,
            created_at: now.clone(),
            updated_at: now,
        };

        store.save_ai_settings(&settings).unwrap();
        assert_eq!(
            store.get_ai_settings().unwrap().unwrap().api_key,
            "sk-local-test"
        );

        store.clear_ai_settings().unwrap();
        assert!(store.get_ai_settings().unwrap().is_none());
    }

    #[test]
    fn ai_insights_round_trip_by_message() {
        let store = MailStore::memory().unwrap();
        let now = now_rfc3339();
        let account = MailAccount {
            id: "acct".to_string(),
            display_name: "Ops".to_string(),
            email: "ops@example.com".to_string(),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_tls: true,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_tls: true,
            sync_enabled: true,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        store.save_account(&account).unwrap();
        let folder = MailFolder {
            id: "inbox".to_string(),
            account_id: account.id.clone(),
            name: "INBOX".to_string(),
            path: "INBOX".to_string(),
            role: FolderRole::Inbox,
            unread_count: 1,
            total_count: 1,
        };
        store.save_folder(&folder).unwrap();
        store
            .upsert_message(&MailMessage {
                id: "message-1".to_string(),
                account_id: account.id,
                folder_id: folder.id,
                uid: Some("1".to_string()),
                message_id_header: None,
                subject: "Quarterly hardening report".to_string(),
                sender: "security@example.com".to_string(),
                recipients: vec!["ops@example.com".to_string()],
                cc: vec![],
                received_at: now.clone(),
                body_preview: "Firewall drift and backup coverage".to_string(),
                body: Some("Firewall drift requires review".to_string()),
                attachments: vec![],
                flags: MessageFlags::default(),
                size_bytes: Some(2048),
                deleted_at: None,
            })
            .unwrap();

        let insight = AiInsight {
            id: "insight-1".to_string(),
            message_id: "message-1".to_string(),
            provider_name: "openai-compatible".to_string(),
            model: "mail-model".to_string(),
            summary: "Short summary".to_string(),
            category: "operations".to_string(),
            priority: AiPriority::High,
            todos: vec!["Reply before 18:00".to_string()],
            reply_draft: "Acknowledged.".to_string(),
            raw_json: "{\"summary\":\"Short summary\"}".to_string(),
            created_at: now,
        };

        store.save_ai_insight(&insight).unwrap();
        let rows = store.list_ai_insights("message-1").unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].summary, "Short summary");
        assert_eq!(rows[0].priority, AiPriority::High);
        assert_eq!(rows[0].todos, vec!["Reply before 18:00".to_string()]);
        assert!(store.list_ai_insights("other-message").unwrap().is_empty());
    }

    #[test]
    fn ai_insights_have_message_kind_created_index() {
        let store = MailStore::memory().unwrap();
        let conn = store.conn.lock();
        let mut index_stmt = conn.prepare("PRAGMA index_list(ai_insights)").unwrap();
        let index_names = index_stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(index_names
            .iter()
            .any(|name| name == "idx_ai_insights_message_kind_created"));

        let mut column_stmt = conn
            .prepare("PRAGMA index_info(idx_ai_insights_message_kind_created)")
            .unwrap();
        let columns = column_stmt
            .query_map([], |row| row.get::<_, String>(2))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(columns, vec!["message_id", "kind", "created_at"]);
    }

    #[test]
    fn migrate_rebuilds_ai_insights_legacy_message_foreign_key() {
        let store = MailStore::memory().unwrap();
        let now = now_rfc3339();
        let account = MailAccount {
            id: "acct".to_string(),
            display_name: "Ops".to_string(),
            email: "ops@example.com".to_string(),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_tls: true,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_tls: true,
            sync_enabled: true,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        store.save_account(&account).unwrap();
        let folder = MailFolder {
            id: "inbox".to_string(),
            account_id: account.id.clone(),
            name: "INBOX".to_string(),
            path: "INBOX".to_string(),
            role: FolderRole::Inbox,
            unread_count: 1,
            total_count: 1,
        };
        store.save_folder(&folder).unwrap();
        store
            .upsert_message(&MailMessage {
                id: "message-1".to_string(),
                account_id: account.id,
                folder_id: folder.id,
                uid: Some("1".to_string()),
                message_id_header: None,
                subject: "Quarterly hardening report".to_string(),
                sender: "security@example.com".to_string(),
                recipients: vec!["ops@example.com".to_string()],
                cc: vec![],
                received_at: now.clone(),
                body_preview: "Firewall drift and backup coverage".to_string(),
                body: Some("Firewall drift requires review".to_string()),
                attachments: vec![],
                flags: MessageFlags::default(),
                size_bytes: Some(2048),
                deleted_at: None,
            })
            .unwrap();

        let valid = AiInsight {
            id: "insight-valid".to_string(),
            message_id: "message-1".to_string(),
            provider_name: "openai-compatible".to_string(),
            model: "mail-model".to_string(),
            summary: "Valid summary".to_string(),
            category: "operations".to_string(),
            priority: AiPriority::High,
            todos: vec!["Reply before 18:00".to_string()],
            reply_draft: "Acknowledged.".to_string(),
            raw_json: "{\"summary\":\"Valid summary\"}".to_string(),
            created_at: now.clone(),
        };
        let orphan = AiInsight {
            id: "insight-orphan".to_string(),
            message_id: "missing-message".to_string(),
            provider_name: "openai-compatible".to_string(),
            model: "mail-model".to_string(),
            summary: "Orphan summary".to_string(),
            category: "operations".to_string(),
            priority: AiPriority::Low,
            todos: Vec::new(),
            reply_draft: String::new(),
            raw_json: "{\"summary\":\"Orphan summary\"}".to_string(),
            created_at: now.clone(),
        };

        {
            let conn = store.conn.lock();
            conn.execute_batch(
                r#"
                PRAGMA foreign_keys = OFF;
                DROP TABLE ai_insights;
                CREATE TABLE ai_insights (
                  id TEXT PRIMARY KEY,
                  message_id TEXT NOT NULL,
                  kind TEXT NOT NULL,
                  payload_json TEXT NOT NULL,
                  created_at TEXT NOT NULL,
                  FOREIGN KEY(message_id) REFERENCES messages(id)
                );
                "#,
            )
            .unwrap();
            conn.execute(
                "INSERT INTO ai_insights (id, message_id, kind, payload_json, created_at) VALUES (?1, ?2, 'mail_analysis', ?3, ?4)",
                params![
                    valid.id,
                    valid.message_id,
                    serde_json::to_string(&valid).unwrap(),
                    valid.created_at,
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO ai_insights (id, message_id, kind, payload_json, created_at) VALUES (?1, ?2, 'mail_analysis', ?3, ?4)",
                params![
                    orphan.id,
                    orphan.message_id,
                    serde_json::to_string(&orphan).unwrap(),
                    orphan.created_at,
                ],
            )
            .unwrap();
            conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        }

        store.migrate().unwrap();

        let has_cascade_message_fk = {
            let conn = store.conn.lock();
            let mut stmt = conn
                .prepare("PRAGMA foreign_key_list(ai_insights)")
                .unwrap();
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(6)?,
                    ))
                })
                .unwrap();
            let has_cascade_message_fk =
                rows.map(|row| row.unwrap())
                    .any(|(table, from, to, on_delete)| {
                        table == "messages"
                            && from == "message_id"
                            && to == "id"
                            && on_delete.eq_ignore_ascii_case("CASCADE")
                    });
            has_cascade_message_fk
        };
        assert!(has_cascade_message_fk);

        let valid_rows = store.list_ai_insights("message-1").unwrap();
        assert_eq!(valid_rows.len(), 1);
        assert_eq!(valid_rows[0].summary, "Valid summary");
        assert!(store
            .list_ai_insights("missing-message")
            .unwrap()
            .is_empty());
    }
}
