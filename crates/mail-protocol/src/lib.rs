use std::collections::HashSet;
use std::time::Duration;

use async_imap::extensions::idle::IdleResponse;
use async_trait::async_trait;
use futures::TryStreamExt;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use mail_core::{
    new_id, now_rfc3339, rfc3339_from_unix_timestamp, AttachmentRef, ConnectionSettings,
    ConnectionTestResult, FolderRole, FolderWatchOutcome, MailAccount, MailActionKind, MailFolder,
    MailMessage, MessageFetchRequest, MessageFlags, RemoteMailAction, SendMessageDraft,
};
use mail_parser::{Address, HeaderName, HeaderValue, MessageParser, MimeHeaders};
use thiserror::Error;
use tokio::net::TcpStream;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("connection failed: {0}")]
    Connection(String),
    #[error("authentication failed: {0}")]
    Authentication(String),
    #[error("fetch failed: {0}")]
    Fetch(String),
    #[error("parse failed: {0}")]
    Parse(String),
    #[error("send failed: {0}")]
    Send(String),
    #[error("protocol feature not implemented: {0}")]
    Unsupported(String),
}

pub type ProtocolResult<T> = Result<T, ProtocolError>;

pub fn validate_mailbox_address(value: &str) -> ProtocolResult<()> {
    parse_mailbox(value).map(|_| ())
}

const IMAP_IDLE_TIMEOUT: Duration = Duration::from_secs(29 * 60);

#[async_trait]
pub trait MailProtocol: Send + Sync {
    async fn test_connection(
        &self,
        settings: &ConnectionSettings,
    ) -> ProtocolResult<ConnectionTestResult>;

    async fn fetch_folders(
        &self,
        settings: &ConnectionSettings,
        account: &MailAccount,
    ) -> ProtocolResult<Vec<MailFolder>>;

    async fn fetch_messages(
        &self,
        settings: &ConnectionSettings,
        account: &MailAccount,
        folder: &MailFolder,
        request: &MessageFetchRequest,
    ) -> ProtocolResult<Vec<MailMessage>>;

    async fn send_message(
        &self,
        settings: &ConnectionSettings,
        draft: &SendMessageDraft,
    ) -> ProtocolResult<String>;

    async fn apply_action(
        &self,
        settings: &ConnectionSettings,
        account: &MailAccount,
        action: &RemoteMailAction,
    ) -> ProtocolResult<()>;

    async fn watch_folder_until_change(
        &self,
        _settings: &ConnectionSettings,
        _account: &MailAccount,
        _folder: &MailFolder,
    ) -> ProtocolResult<FolderWatchOutcome> {
        Ok(FolderWatchOutcome::Timeout)
    }
}

#[derive(Debug, Clone, Default)]
pub struct LiveMailProtocol;

#[async_trait]
impl MailProtocol for LiveMailProtocol {
    async fn test_connection(
        &self,
        settings: &ConnectionSettings,
    ) -> ProtocolResult<ConnectionTestResult> {
        let imap_result = login_imap(settings).await;
        let imap_ok = imap_result.is_ok();
        if let Ok(mut session) = imap_result {
            let _ = session.logout().await;
        }

        let smtp_ok = match build_smtp_transport(settings) {
            Ok(transport) => transport
                .test_connection()
                .await
                .map_err(|err| ProtocolError::Connection(sanitize_error(&err.to_string())))
                .unwrap_or(false),
            Err(_) => false,
        };
        let message = match (&imap_ok, &smtp_ok) {
            (true, true) => "IMAP and SMTP connection settings accepted".to_string(),
            (false, true) => "SMTP accepted, IMAP connection or authentication failed".to_string(),
            (true, false) => "IMAP accepted, SMTP configuration failed".to_string(),
            (false, false) => "IMAP and SMTP connection settings failed".to_string(),
        };

        Ok(ConnectionTestResult {
            imap_ok,
            smtp_ok,
            message,
        })
    }

    async fn fetch_folders(
        &self,
        settings: &ConnectionSettings,
        account: &MailAccount,
    ) -> ProtocolResult<Vec<MailFolder>> {
        let mut session = login_imap(settings).await?;
        let folders_stream = session
            .list(None, Some("*"))
            .await
            .map_err(|err| ProtocolError::Fetch(err.to_string()))?;
        let remote_folders: Vec<_> = folders_stream
            .try_collect()
            .await
            .map_err(|err| ProtocolError::Fetch(err.to_string()))?;
        let _ = session.logout().await;

        let mut folders = Vec::new();
        let mut seen_paths = HashSet::new();
        for mailbox in remote_folders {
            if mailbox
                .attributes()
                .iter()
                .any(|attribute| matches!(attribute, async_imap::types::NameAttribute::NoSelect))
            {
                continue;
            }
            let path = mailbox.name().to_string();
            if !seen_paths.insert(path.clone()) {
                continue;
            }
            let role = infer_folder_role(&path);
            folders.push(MailFolder {
                id: folder_id(&account.id, &path),
                account_id: account.id.clone(),
                name: display_folder_name(&path),
                path,
                role,
                unread_count: 0,
                total_count: 0,
            });
        }

        if !folders
            .iter()
            .any(|folder| folder.role == FolderRole::Inbox)
        {
            folders.push(MailFolder {
                id: folder_id(&account.id, "INBOX"),
                account_id: account.id.clone(),
                name: "INBOX".to_string(),
                path: "INBOX".to_string(),
                role: FolderRole::Inbox,
                unread_count: 0,
                total_count: 0,
            });
        }

        Ok(folders)
    }

    async fn fetch_messages(
        &self,
        settings: &ConnectionSettings,
        account: &MailAccount,
        folder: &MailFolder,
        request: &MessageFetchRequest,
    ) -> ProtocolResult<Vec<MailMessage>> {
        let mut session = login_imap(settings).await?;
        session
            .select(&folder.path)
            .await
            .map_err(|err| ProtocolError::Fetch(err.to_string()))?;

        let mut uids: Vec<u32> = session
            .uid_search(uid_search_query(request.last_uid.as_deref()))
            .await
            .map_err(|err| ProtocolError::Fetch(err.to_string()))?
            .into_iter()
            .collect();
        uids.sort_unstable();

        let limit = request.limit.max(1) as usize;
        if request.last_uid.is_none() && uids.len() > limit {
            uids = uids.split_off(uids.len() - limit);
        } else {
            uids.truncate(limit);
        }

        if uids.is_empty() {
            let _ = session.logout().await;
            return Ok(Vec::new());
        }

        let uid_set = compact_uid_set(&uids);
        let fetches_stream = session
            .uid_fetch(uid_set, "(UID FLAGS RFC822.SIZE BODY.PEEK[])")
            .await
            .map_err(|err| ProtocolError::Fetch(err.to_string()))?;
        let fetches: Vec<_> = fetches_stream
            .try_collect()
            .await
            .map_err(|err| ProtocolError::Fetch(err.to_string()))?;
        let _ = session.logout().await;

        let mut messages = Vec::with_capacity(fetches.len());
        for fetch in fetches {
            let uid = fetch
                .uid
                .ok_or_else(|| ProtocolError::Fetch("server omitted UID in fetch".to_string()))?;
            let raw = fetch.body().ok_or_else(|| {
                ProtocolError::Fetch(format!("server omitted body for UID {uid}"))
            })?;
            let mut message = parse_message(account, folder, uid, raw, fetch.size)?;
            message.flags = flags_from_fetch(fetch.flags());
            messages.push(message);
        }

        messages.sort_by(|left, right| left.uid.cmp(&right.uid));
        Ok(messages)
    }

    async fn send_message(
        &self,
        settings: &ConnectionSettings,
        draft: &SendMessageDraft,
    ) -> ProtocolResult<String> {
        if draft.to.is_empty() {
            return Err(ProtocolError::Send("recipient list is empty".to_string()));
        }
        if draft.subject.trim().is_empty() {
            return Err(ProtocolError::Send("subject is empty".to_string()));
        }

        let mut builder = Message::builder()
            .from(parse_mailbox(&settings.email)?)
            .subject(draft.subject.clone());
        for recipient in &draft.to {
            builder = builder.to(parse_mailbox(recipient)?);
        }
        for recipient in &draft.cc {
            builder = builder.cc(parse_mailbox(recipient)?);
        }

        let email = builder
            .body(draft.body.clone())
            .map_err(|err| ProtocolError::Send(err.to_string()))?;
        let transport = build_smtp_transport(settings)?;
        transport
            .send(email)
            .await
            .map_err(|err| ProtocolError::Send(sanitize_error(&err.to_string())))?;

        Ok(new_id())
    }

    async fn apply_action(
        &self,
        settings: &ConnectionSettings,
        _account: &MailAccount,
        action: &RemoteMailAction,
    ) -> ProtocolResult<()> {
        let uid_set = uid_set_from_strings(&action.uids)?;
        let mut session = login_imap(settings).await?;
        let capabilities = session
            .capabilities()
            .await
            .map_err(|err| ProtocolError::Fetch(sanitize_error(&err.to_string())))?;
        session
            .select(&action.source_folder.path)
            .await
            .map_err(|err| ProtocolError::Fetch(sanitize_error(&err.to_string())))?;

        match action.action {
            MailActionKind::MarkRead => {
                drain_uid_store(&mut session, &uid_set, "+FLAGS.SILENT (\\Seen)").await?;
            }
            MailActionKind::MarkUnread => {
                drain_uid_store(&mut session, &uid_set, "-FLAGS.SILENT (\\Seen)").await?;
            }
            MailActionKind::Star => {
                drain_uid_store(&mut session, &uid_set, "+FLAGS.SILENT (\\Flagged)").await?;
            }
            MailActionKind::Unstar => {
                drain_uid_store(&mut session, &uid_set, "-FLAGS.SILENT (\\Flagged)").await?;
            }
            MailActionKind::Move
            | MailActionKind::Archive
            | MailActionKind::Delete
            | MailActionKind::BatchMove
            | MailActionKind::BatchDelete => {
                let target = action.target_folder.as_ref().ok_or_else(|| {
                    ProtocolError::Unsupported(
                        "target folder required for move-like action".to_string(),
                    )
                })?;
                if capabilities.has_str("MOVE") {
                    session
                        .uid_mv(&uid_set, &target.path)
                        .await
                        .map_err(|err| ProtocolError::Fetch(sanitize_error(&err.to_string())))?;
                } else {
                    session
                        .uid_copy(&uid_set, &target.path)
                        .await
                        .map_err(|err| ProtocolError::Fetch(sanitize_error(&err.to_string())))?;
                    drain_uid_store(&mut session, &uid_set, "+FLAGS.SILENT (\\Deleted)").await?;
                    {
                        let expunge_stream = session.expunge().await.map_err(|err| {
                            ProtocolError::Fetch(sanitize_error(&err.to_string()))
                        })?;
                        let _: Vec<_> = expunge_stream.try_collect().await.map_err(|err| {
                            ProtocolError::Fetch(sanitize_error(&err.to_string()))
                        })?;
                    }
                }
            }
            MailActionKind::PermanentDelete => {
                if !capabilities.has_str("UIDPLUS") {
                    let _ = session.logout().await;
                    return Err(ProtocolError::Unsupported(
                        "IMAP server does not advertise UIDPLUS; precise permanent delete is unavailable".to_string(),
                    ));
                }
                drain_uid_store(&mut session, &uid_set, "+FLAGS.SILENT (\\Deleted)").await?;
                {
                    let expunge_stream = session
                        .uid_expunge(&uid_set)
                        .await
                        .map_err(|err| ProtocolError::Fetch(sanitize_error(&err.to_string())))?;
                    let _: Vec<_> = expunge_stream
                        .try_collect()
                        .await
                        .map_err(|err| ProtocolError::Fetch(sanitize_error(&err.to_string())))?;
                }
            }
            MailActionKind::Send | MailActionKind::Forward => {
                let _ = session.logout().await;
                return Err(ProtocolError::Unsupported(format!(
                    "remote action {:?} is not supported by IMAP adapter",
                    action.action
                )));
            }
        }

        let _ = session.logout().await;
        Ok(())
    }

    async fn watch_folder_until_change(
        &self,
        settings: &ConnectionSettings,
        _account: &MailAccount,
        folder: &MailFolder,
    ) -> ProtocolResult<FolderWatchOutcome> {
        let mut session = login_imap(settings).await?;
        let capabilities = session
            .capabilities()
            .await
            .map_err(|err| ProtocolError::Fetch(sanitize_error(&err.to_string())))?;
        if !capabilities.has_str("IDLE") {
            let _ = session.logout().await;
            return Err(ProtocolError::Unsupported(
                "IMAP server does not advertise IDLE".to_string(),
            ));
        }

        session
            .select(&folder.path)
            .await
            .map_err(|err| ProtocolError::Fetch(sanitize_error(&err.to_string())))?;

        let mut idle = session.idle();
        idle.init()
            .await
            .map_err(|err| ProtocolError::Fetch(sanitize_error(&err.to_string())))?;
        let response = {
            let (wait, _interrupt) = idle.wait_with_timeout(IMAP_IDLE_TIMEOUT);
            wait.await
        }
        .map_err(|err| ProtocolError::Fetch(sanitize_error(&err.to_string())))?;

        let outcome = match response {
            IdleResponse::NewData(_) => FolderWatchOutcome::Changed,
            IdleResponse::Timeout | IdleResponse::ManualInterrupt => FolderWatchOutcome::Timeout,
        };
        if let Ok(mut session) = idle.done().await {
            let _ = session.logout().await;
        }

        Ok(outcome)
    }
}

#[derive(Debug, Clone, Default)]
pub struct MockMailProtocol;

#[async_trait]
impl MailProtocol for MockMailProtocol {
    async fn test_connection(
        &self,
        settings: &ConnectionSettings,
    ) -> ProtocolResult<ConnectionTestResult> {
        let imap_ok = !settings.email.trim().is_empty()
            && !settings.imap_host.trim().is_empty()
            && settings.imap_port > 0
            && !settings.password.is_empty();
        let smtp_ok = !settings.smtp_host.trim().is_empty()
            && settings.smtp_port > 0
            && !settings.password.is_empty();

        Ok(ConnectionTestResult {
            imap_ok,
            smtp_ok,
            message: if imap_ok && smtp_ok {
                "mock protocol accepted account settings".to_string()
            } else {
                "missing host, port, email, or password".to_string()
            },
        })
    }

    async fn fetch_folders(
        &self,
        _settings: &ConnectionSettings,
        account: &MailAccount,
    ) -> ProtocolResult<Vec<MailFolder>> {
        let folders = [
            ("inbox", "INBOX", FolderRole::Inbox, 3, 12),
            ("sent", "Sent", FolderRole::Sent, 0, 4),
            ("archive", "Archive", FolderRole::Archive, 0, 32),
            ("drafts", "Drafts", FolderRole::Drafts, 0, 1),
            ("trash", "Trash", FolderRole::Trash, 0, 0),
        ];

        Ok(folders
            .into_iter()
            .map(
                |(suffix, name, role, unread_count, total_count)| MailFolder {
                    id: format!("{}:{suffix}", account.id),
                    account_id: account.id.clone(),
                    name: name.to_string(),
                    path: name.to_string(),
                    role,
                    unread_count,
                    total_count,
                },
            )
            .collect())
    }

    async fn fetch_messages(
        &self,
        _settings: &ConnectionSettings,
        account: &MailAccount,
        folder: &MailFolder,
        request: &MessageFetchRequest,
    ) -> ProtocolResult<Vec<MailMessage>> {
        let now = now_rfc3339();
        let rows = match folder.role {
            FolderRole::Inbox => vec![
                (
                    "1001",
                    "Security rotation window / action required",
                    "infra-watch@example.net",
                    "Credential rotation window opens tonight. Confirm service owners and blackout exceptions before 18:00.",
                    false,
                    true,
                ),
                (
                    "1002",
                    "Vendor invoice reconciliation",
                    "finance-ops@example.com",
                    "Three invoices are waiting for reconciliation. Attachment metadata is indexed but files are not downloaded yet.",
                    false,
                    false,
                ),
                (
                    "1003",
                    "Release train notes",
                    "release-control@example.org",
                    "Build 42 passed smoke tests. Mail client telemetry remains disabled in this MVP.",
                    true,
                    false,
                ),
            ],
            FolderRole::Sent => vec![(
                "2001",
                "Sent: rotation confirmation",
                account.email.as_str(),
                "Confirmation sent through the pending action queue.",
                true,
                false,
            )],
            FolderRole::Archive => vec![(
                "3001",
                "Archived vendor thread",
                "finance-ops@example.com",
                "Archived reconciliation notes remain searchable outside INBOX.",
                true,
                false,
            )],
            FolderRole::Drafts => vec![(
                "4001",
                "Draft: incident summary",
                account.email.as_str(),
                "Draft content is indexed for folder navigation validation.",
                true,
                false,
            )],
            FolderRole::Trash => vec![(
                "5001",
                "Deleted obsolete alert",
                "alerts@example.net",
                "Trash folder sync proves delete-to-trash can be inspected.",
                true,
                false,
            )],
            FolderRole::Junk | FolderRole::Custom => Vec::new(),
        };
        let min_uid = request
            .last_uid
            .as_deref()
            .and_then(|uid| uid.parse::<u32>().ok())
            .unwrap_or(0);

        Ok(rows
            .into_iter()
            .filter(|(uid, ..)| uid.parse::<u32>().map(|uid| uid > min_uid).unwrap_or(true))
            .map(|(uid, subject, sender, preview, is_read, is_starred)| MailMessage {
                id: format!("{}:{}:{uid}", account.id, folder.id),
                account_id: account.id.clone(),
                folder_id: folder.id.clone(),
                uid: Some(uid.to_string()),
                message_id_header: Some(format!("<{uid}.{}@agentmail.local>", account.id)),
                subject: subject.to_string(),
                sender: sender.to_string(),
                recipients: vec![account.email.clone()],
                cc: Vec::new(),
                received_at: now.clone(),
                body_preview: preview.to_string(),
                body: Some(format!(
                    "{preview}\n\n--\nThis message is generated by the protocol adapter boundary. Replace MockMailProtocol with the live IMAP/SMTP adapter without changing the UI or store APIs."
                )),
                attachments: Vec::new(),
                flags: MessageFlags {
                    is_read,
                    is_starred,
                    is_answered: false,
                    is_forwarded: false,
                },
                size_bytes: Some(4096),
                deleted_at: None,
            })
            .collect())
    }

    async fn send_message(
        &self,
        _settings: &ConnectionSettings,
        draft: &SendMessageDraft,
    ) -> ProtocolResult<String> {
        if draft.to.is_empty() {
            return Err(ProtocolError::Send("recipient list is empty".to_string()));
        }
        if draft.subject.trim().is_empty() {
            return Err(ProtocolError::Send("subject is empty".to_string()));
        }
        Ok(new_id())
    }

    async fn apply_action(
        &self,
        _settings: &ConnectionSettings,
        _account: &MailAccount,
        action: &RemoteMailAction,
    ) -> ProtocolResult<()> {
        if action.uids.is_empty() {
            return Err(ProtocolError::Fetch(
                "message UID list is empty".to_string(),
            ));
        }
        if matches!(
            action.action,
            MailActionKind::Move
                | MailActionKind::Archive
                | MailActionKind::Delete
                | MailActionKind::BatchMove
                | MailActionKind::BatchDelete
        ) && action.target_folder.is_none()
        {
            return Err(ProtocolError::Unsupported(
                "target folder required for move-like action".to_string(),
            ));
        }
        Ok(())
    }
}

async fn login_imap(
    settings: &ConnectionSettings,
) -> ProtocolResult<async_imap::Session<async_native_tls::TlsStream<TcpStream>>> {
    if !settings.imap_tls {
        return Err(ProtocolError::Unsupported(
            "plain IMAP is disabled in the live adapter; enable IMAP TLS".to_string(),
        ));
    }

    let tcp = TcpStream::connect((settings.imap_host.as_str(), settings.imap_port))
        .await
        .map_err(|err| ProtocolError::Connection(sanitize_error(&err.to_string())))?;
    let tls = async_native_tls::TlsConnector::new()
        .connect(settings.imap_host.as_str(), tcp)
        .await
        .map_err(|err| ProtocolError::Connection(sanitize_error(&err.to_string())))?;
    let mut client = async_imap::Client::new(tls);
    client
        .read_response()
        .await
        .map_err(|err| ProtocolError::Connection(sanitize_error(&err.to_string())))?
        .ok_or_else(|| ProtocolError::Connection("server closed before greeting".to_string()))?;

    client
        .login(&settings.email, &settings.password)
        .await
        .map_err(|(err, _client)| ProtocolError::Authentication(sanitize_error(&err.to_string())))
}

async fn drain_uid_store(
    session: &mut async_imap::Session<async_native_tls::TlsStream<TcpStream>>,
    uid_set: &str,
    query: &str,
) -> ProtocolResult<()> {
    let store_stream = session
        .uid_store(uid_set, query)
        .await
        .map_err(|err| ProtocolError::Fetch(sanitize_error(&err.to_string())))?;
    let _: Vec<_> = store_stream
        .try_collect()
        .await
        .map_err(|err| ProtocolError::Fetch(sanitize_error(&err.to_string())))?;
    Ok(())
}

fn build_smtp_transport(
    settings: &ConnectionSettings,
) -> ProtocolResult<AsyncSmtpTransport<Tokio1Executor>> {
    let credentials = Credentials::new(settings.email.clone(), settings.password.clone());
    let builder = if settings.smtp_tls && settings.smtp_port == 587 {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&settings.smtp_host)
            .map_err(|err| ProtocolError::Connection(err.to_string()))?
    } else if settings.smtp_tls {
        AsyncSmtpTransport::<Tokio1Executor>::relay(&settings.smtp_host)
            .map_err(|err| ProtocolError::Connection(err.to_string()))?
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&settings.smtp_host)
    };

    Ok(builder
        .port(settings.smtp_port)
        .credentials(credentials)
        .build())
}

fn parse_mailbox(value: &str) -> ProtocolResult<Mailbox> {
    value
        .parse()
        .map_err(|err| ProtocolError::Send(format!("invalid mailbox {value}: {err}")))
}

fn parse_message(
    account: &MailAccount,
    folder: &MailFolder,
    uid: u32,
    raw: &[u8],
    size: Option<u32>,
) -> ProtocolResult<MailMessage> {
    let parsed = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| ProtocolError::Parse(format!("failed to parse UID {uid}")))?;
    let subject = header_text(parsed.header(HeaderName::Subject))
        .unwrap_or_else(|| "(no subject)".to_string());
    let sender = parsed
        .from()
        .and_then(address_text)
        .unwrap_or_else(|| header_text(parsed.header(HeaderName::From)).unwrap_or_default());
    let recipients = address_list(parsed.to());
    let cc = address_list(parsed.cc());
    let body = normalize_text(
        &parsed
            .body_text(0)
            .map(|value| value.into_owned())
            .or_else(|| parsed.body_html(0).map(|value| html_to_text(&value)))
            .unwrap_or_default(),
    );
    let body_preview = build_preview(&body);
    let message_id_header = parsed.message_id().map(ToString::to_string);
    let attachments = parsed
        .attachments()
        .enumerate()
        .map(|(index, part)| AttachmentRef {
            id: format!("{}:{}:{uid}:att-{index}", account.id, folder.id),
            message_id: format!("{}:{}:{uid}", account.id, folder.id),
            filename: part
                .attachment_name()
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("attachment-{index}")),
            mime_type: part
                .content_type()
                .map(|content_type| {
                    format!(
                        "{}/{}",
                        content_type.ctype(),
                        content_type.subtype().unwrap_or("octet-stream")
                    )
                })
                .unwrap_or_else(|| "application/octet-stream".to_string()),
            size_bytes: part.len() as i64,
            local_path: None,
        })
        .collect();

    Ok(MailMessage {
        id: format!("{}:{}:{uid}", account.id, folder.id),
        account_id: account.id.clone(),
        folder_id: folder.id.clone(),
        uid: Some(uid.to_string()),
        message_id_header,
        subject,
        sender,
        recipients,
        cc,
        received_at: parsed
            .date()
            .and_then(|date| rfc3339_from_unix_timestamp(date.to_timestamp()))
            .unwrap_or_else(now_rfc3339),
        body_preview,
        body: Some(body),
        attachments,
        flags: MessageFlags::default(),
        size_bytes: size.map(i64::from).or(Some(raw.len() as i64)),
        deleted_at: None,
    })
}

fn header_text(value: Option<&HeaderValue<'_>>) -> Option<String> {
    match value? {
        HeaderValue::Text(text) => Some(text.to_string()),
        HeaderValue::TextList(values) => Some(
            values
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        ),
        HeaderValue::Address(address) => address_text(address),
        HeaderValue::DateTime(value) => Some(format!("{value:?}")),
        HeaderValue::ContentType(value) => Some(format!(
            "{}/{}",
            value.ctype(),
            value.subtype().unwrap_or("octet-stream")
        )),
        HeaderValue::Received(value) => Some(format!("{value:?}")),
        HeaderValue::Empty => None,
    }
}

fn address_list(value: Option<&Address<'_>>) -> Vec<String> {
    value
        .and_then(address_text)
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn address_text(value: &Address<'_>) -> Option<String> {
    match value {
        Address::List(addresses) => Some(
            addresses
                .iter()
                .map(|addr| match (&addr.name, &addr.address) {
                    (Some(name), Some(address)) => format!("{name} <{address}>"),
                    (None, Some(address)) => address.to_string(),
                    (Some(name), None) => name.to_string(),
                    (None, None) => String::new(),
                })
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(", "),
        ),
        Address::Group(groups) => Some(
            groups
                .iter()
                .flat_map(|group| group.addresses.iter())
                .map(|addr| match (&addr.name, &addr.address) {
                    (Some(name), Some(address)) => format!("{name} <{address}>"),
                    (None, Some(address)) => address.to_string(),
                    (Some(name), None) => name.to_string(),
                    (None, None) => String::new(),
                })
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(", "),
        ),
    }
}

fn build_preview(body: &str) -> String {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(240).collect()
}

fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut entity = String::new();
    let mut in_entity = false;

    for ch in html.chars() {
        if in_entity {
            if ch == ';' {
                out.push_str(match entity.as_str() {
                    "nbsp" => " ",
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => "",
                });
                entity.clear();
                in_entity = false;
            } else if entity.len() < 12 {
                entity.push(ch);
            } else {
                entity.clear();
                in_entity = false;
            }
            continue;
        }

        match ch {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' => in_tag = false,
            '&' if !in_tag => {
                entity.clear();
                in_entity = true;
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }

    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_text(value: &str) -> String {
    value
        .replace('\u{a0}', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn flags_from_fetch<'a>(flags: impl Iterator<Item = async_imap::types::Flag<'a>>) -> MessageFlags {
    let mut out = MessageFlags::default();
    for flag in flags {
        match flag {
            async_imap::types::Flag::Seen => out.is_read = true,
            async_imap::types::Flag::Answered => out.is_answered = true,
            async_imap::types::Flag::Flagged => out.is_starred = true,
            _ => {}
        }
    }
    out
}

fn uid_search_query(last_uid: Option<&str>) -> String {
    let next_uid = last_uid
        .and_then(|value| value.parse::<u32>().ok())
        .and_then(|value| value.checked_add(1))
        .unwrap_or(1);
    format!("UID {next_uid}:*")
}

fn compact_uid_set(uids: &[u32]) -> String {
    uids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn uid_set_from_strings(uids: &[String]) -> ProtocolResult<String> {
    if uids.is_empty() {
        return Err(ProtocolError::Fetch(
            "message UID list is empty".to_string(),
        ));
    }
    let mut parsed = Vec::with_capacity(uids.len());
    for uid in uids {
        let value = uid
            .parse::<u32>()
            .map_err(|_| ProtocolError::Fetch(format!("invalid message UID: {uid}")))?;
        parsed.push(value);
    }
    Ok(compact_uid_set(&parsed))
}

fn infer_folder_role(path: &str) -> FolderRole {
    match path.to_ascii_lowercase().as_str() {
        "inbox" => FolderRole::Inbox,
        "sent" | "sent messages" | "sent items" => FolderRole::Sent,
        "archive" | "archives" => FolderRole::Archive,
        "trash" | "deleted" | "deleted messages" | "deleted items" => FolderRole::Trash,
        "drafts" | "draft" => FolderRole::Drafts,
        "spam" | "junk" | "junk email" => FolderRole::Junk,
        _ => FolderRole::Custom,
    }
}

fn display_folder_name(path: &str) -> String {
    path.rsplit(['/', '.']).next().unwrap_or(path).to_string()
}

fn folder_id(account_id: &str, path: &str) -> String {
    let safe_path = path
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_ascii_lowercase();
    format!("{account_id}:{safe_path}")
}

fn sanitize_error(value: &str) -> String {
    value.replace('\r', " ").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_text_message_with_attachment_metadata() {
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
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        };
        let folder = MailFolder {
            id: "acct:inbox".to_string(),
            account_id: account.id.clone(),
            name: "INBOX".to_string(),
            path: "INBOX".to_string(),
            role: FolderRole::Inbox,
            unread_count: 0,
            total_count: 0,
        };
        let raw = b"From: Security <sec@example.com>\r\nTo: Ops <ops@example.com>\r\nSubject: Test report\r\nMessage-ID: <a@example.com>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello from IMAP.\r\n--x\r\nContent-Type: text/plain; name=report.txt\r\nContent-Disposition: attachment; filename=report.txt\r\n\r\nreport-body\r\n--x--\r\n";

        let parsed = parse_message(&account, &folder, 42, raw, Some(raw.len() as u32)).unwrap();
        assert_eq!(parsed.uid.as_deref(), Some("42"));
        assert_eq!(parsed.subject, "Test report");
        assert!(parsed.body.as_deref().unwrap_or_default().contains("Hello"));
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].filename, "report.txt");
    }

    #[test]
    fn builds_incremental_uid_query() {
        assert_eq!(uid_search_query(None), "UID 1:*");
        assert_eq!(uid_search_query(Some("100")), "UID 101:*");
    }

    #[test]
    fn builds_validated_uid_action_set() {
        assert_eq!(
            uid_set_from_strings(&["42".to_string(), "99".to_string()]).unwrap(),
            "42,99"
        );
        assert!(uid_set_from_strings(&["not-a-uid".to_string()]).is_err());
    }

    #[test]
    fn validates_mailbox_addresses_with_protocol_parser() {
        assert!(validate_mailbox_address("ops@example.com").is_ok());
        assert!(validate_mailbox_address("app test").is_err());
    }

    #[test]
    fn parses_date_header_and_html_only_body() {
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
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        };
        let folder = MailFolder {
            id: "acct:inbox".to_string(),
            account_id: account.id.clone(),
            name: "INBOX".to_string(),
            path: "INBOX".to_string(),
            role: FolderRole::Inbox,
            unread_count: 0,
            total_count: 0,
        };
        let raw = b"From: sec@example.com\r\nTo: ops@example.com\r\nDate: Tue, 20 Feb 2024 10:30:00 +0800\r\nSubject: HTML report\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><h1>Report&nbsp;Ready</h1><p>A&amp;B</p></body></html>\r\n";

        let parsed = parse_message(&account, &folder, 43, raw, None).unwrap();
        assert!(parsed.received_at.starts_with("2024-02-20T02:30:00"));
        let body = parsed.body.as_deref().unwrap_or_default();
        assert!(body.contains("Report Ready"));
        assert!(body.contains("A&B"));
        assert!(parsed.body_preview.contains("Report Ready"));
    }
}
