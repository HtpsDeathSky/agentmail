use std::collections::HashSet;
use std::time::Duration;

use async_imap::extensions::idle::IdleResponse;
use async_trait::async_trait;
use futures::TryStreamExt;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use mail_core::{
    new_id, now_rfc3339, rfc3339_from_unix_timestamp, AttachmentRef, ConnectionAuth,
    ConnectionSettings, ConnectionTestResult, FolderRole, FolderWatchOutcome, InlineResource,
    MailAccount, MailActionKind, MailFolder, MailMessage, MessageFetchRequest, MessageFlags,
    RemoteMailAction, SendMessageDraft,
};
use mail_parser::{Address, HeaderName, HeaderValue, MessageParser, MimeHeaders, PartType};
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
            .subject(draft.subject.clone())
            .message_id(draft.message_id_header.clone());
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
        let auth_present = settings.auth.is_present();
        let imap_ok = !settings.email.trim().is_empty()
            && !settings.imap_host.trim().is_empty()
            && settings.imap_port > 0
            && auth_present;
        let smtp_ok =
            !settings.smtp_host.trim().is_empty() && settings.smtp_port > 0 && auth_present;

        Ok(ConnectionTestResult {
            imap_ok,
            smtp_ok,
            message: if imap_ok && smtp_ok {
                "mock protocol accepted account settings".to_string()
            } else {
                "missing host, port, email, or credentials".to_string()
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
                "Confirmation sent directly through SMTP.",
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
                html_body: None,
                raw_mime: None,
                inline_resources: Vec::new(),
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

    match &settings.auth {
        ConnectionAuth::Password { password } => client
            .login(&settings.email, password)
            .await
            .map_err(|(err, _client)| {
                ProtocolError::Authentication(sanitize_error(&err.to_string()))
            }),
        ConnectionAuth::GoogleOAuth { access_token } => {
            let auth = Xoauth2Authenticator {
                email: &settings.email,
                access_token,
            };
            client
                .authenticate("XOAUTH2", auth)
                .await
                .map_err(|(err, _client)| {
                    ProtocolError::Authentication(sanitize_error(&err.to_string()))
                })
        }
    }
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
    let (credentials, mechanisms) = match &settings.auth {
        ConnectionAuth::Password { password } => (
            Credentials::new(settings.email.clone(), password.clone()),
            vec![Mechanism::Plain, Mechanism::Login],
        ),
        ConnectionAuth::GoogleOAuth { access_token } => (
            Credentials::new(settings.email.clone(), access_token.clone()),
            vec![Mechanism::Xoauth2],
        ),
    };
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
        .authentication(mechanisms)
        .build())
}

struct Xoauth2Authenticator<'a> {
    email: &'a str,
    access_token: &'a str,
}

impl async_imap::Authenticator for Xoauth2Authenticator<'_> {
    type Response = String;

    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        build_xoauth2_payload(self.email, self.access_token)
    }
}

fn build_xoauth2_payload(email: &str, access_token: &str) -> String {
    format!("user={email}\x01auth=Bearer {access_token}\x01\x01")
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
    let html_body = parsed.body_html(0).map(|value| value.into_owned());
    let text_body = first_text_body(&parsed);
    let body = normalize_text(
        &text_body
            .or_else(|| html_body.as_ref().map(|value| html_to_text(value)))
            .unwrap_or_default(),
    );
    let body_preview = build_preview(&body);
    let message_id_header = parsed.message_id().and_then(normalize_message_id_header);
    let message_id = format!("{}:{}:{uid}", account.id, folder.id);
    let referenced_content_ids = html_body
        .as_deref()
        .map(extract_cid_references)
        .unwrap_or_default();
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
    let inline_resources = parsed
        .parts
        .iter()
        .enumerate()
        .filter_map(|(index, part)| {
            let content_id = normalize_content_id(part.content_id()?)?;
            if !referenced_content_ids.contains(&content_id.to_ascii_lowercase()) {
                return None;
            }
            let bytes = match &part.body {
                PartType::Binary(bytes) | PartType::InlineBinary(bytes) => bytes.to_vec(),
                _ => return None,
            };
            Some(InlineResource {
                id: format!("{}:{}:{uid}:inline-{index}", account.id, folder.id),
                message_id: message_id.clone(),
                content_id,
                filename: part.attachment_name().map(ToString::to_string),
                mime_type: part_mime_type(part),
                bytes,
            })
        })
        .collect();

    Ok(MailMessage {
        id: message_id,
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
        html_body,
        raw_mime: Some(raw.to_vec()),
        inline_resources,
        attachments,
        flags: MessageFlags::default(),
        size_bytes: size.map(i64::from).or(Some(raw.len() as i64)),
        deleted_at: None,
    })
}

fn part_mime_type(part: &mail_parser::MessagePart<'_>) -> String {
    part.content_type()
        .map(|content_type| {
            format!(
                "{}/{}",
                content_type.ctype(),
                content_type.subtype().unwrap_or("octet-stream")
            )
        })
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

fn first_text_body(parsed: &mail_parser::Message<'_>) -> Option<String> {
    parsed
        .text_body
        .iter()
        .filter_map(|part_id| parsed.parts.get(*part_id as usize))
        .find_map(|part| match &part.body {
            PartType::Text(text) => Some(text.as_ref().to_string()),
            _ => None,
        })
        .or_else(|| {
            parsed
                .parts
                .iter()
                .filter(|part| is_fallback_text_body_part(part))
                .find_map(|part| match &part.body {
                    PartType::Text(text) => Some(text.as_ref().to_string()),
                    _ => None,
                })
        })
}

fn is_fallback_text_body_part(part: &mail_parser::MessagePart<'_>) -> bool {
    !part
        .content_disposition()
        .is_some_and(|value| value.is_attachment())
        && part.attachment_name().is_none()
        && part.content_type().is_some_and(|content_type| {
            content_type.ctype().eq_ignore_ascii_case("text")
                && content_type
                    .subtype()
                    .is_some_and(|subtype| subtype.eq_ignore_ascii_case("plain"))
        })
}

fn normalize_content_id(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_start_matches('<').trim_end_matches('>');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn extract_cid_references(html: &str) -> HashSet<String> {
    let mut references = HashSet::new();
    let mut remaining = html;
    while let Some(index) = find_ascii_case_insensitive(remaining, "cid:") {
        let after_cid = &remaining[index + 4..];
        let raw_cid = after_cid
            .split(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | '<' | '>' | ')'))
            .next()
            .unwrap_or_default();
        if let Some(content_id) = normalize_content_id(raw_cid) {
            references.insert(content_id.to_ascii_lowercase());
        }
        remaining = &after_cid[raw_cid.len()..];
    }
    references
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
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
    if is_noise_preview_text(&compact) {
        return String::new();
    }
    compact.chars().take(240).collect()
}

fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut index = 0;

    while index < html.len() {
        let remaining = &html[index..];

        if remaining.starts_with("<!--") {
            out.push(' ');
            index += remaining
                .find("-->")
                .map(|end| end + 3)
                .unwrap_or(remaining.len());
            continue;
        }

        if remaining.starts_with('<') {
            if remaining.starts_with("<!") || remaining.starts_with("<?") {
                out.push(' ');
                index += remaining
                    .find('>')
                    .map(|end| end + 1)
                    .unwrap_or(remaining.len());
                continue;
            }

            if let Some(tag) = parse_html_tag(remaining) {
                out.push(' ');
                if !tag.is_closing && !tag.is_self_closing && is_html_raw_text_tag(tag.name) {
                    index +=
                        find_html_closing_tag_end(remaining, tag.name).unwrap_or(remaining.len());
                } else {
                    if !tag.is_closing && inserts_text_break(tag.name) {
                        out.push(' ');
                    }
                    index += tag.end_index;
                }
                continue;
            }
        }

        if let Some((decoded, consumed)) = decode_html_entity(remaining) {
            out.push_str(decoded);
            index += consumed;
            continue;
        }

        let ch = remaining
            .chars()
            .next()
            .expect("index is within string bounds");
        out.push(ch);
        index += ch.len_utf8();
    }

    normalize_text(&out)
}

struct HtmlTag<'a> {
    name: &'a str,
    is_closing: bool,
    is_self_closing: bool,
    end_index: usize,
}

fn parse_html_tag(input: &str) -> Option<HtmlTag<'_>> {
    if !input.starts_with('<') {
        return None;
    }
    let end_index = input.find('>')? + 1;
    let mut inner = input[1..end_index - 1].trim_start();
    let is_closing = inner.starts_with('/');
    if is_closing {
        inner = inner[1..].trim_start();
    }

    let name_end = inner
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == ':'))
        .unwrap_or(inner.len());
    let name = &inner[..name_end];
    if name.is_empty() || !name.as_bytes()[0].is_ascii_alphabetic() {
        return None;
    }

    Some(HtmlTag {
        name,
        is_closing,
        is_self_closing: inner.trim_end().ends_with('/'),
        end_index,
    })
}

fn find_html_closing_tag_end(html: &str, tag: &str) -> Option<usize> {
    let close_needle = format!("</{tag}");
    let mut search_start = 0;

    while search_start < html.len() {
        let relative_index = find_ascii_case_insensitive(&html[search_start..], &close_needle)?;
        let close_index = search_start + relative_index;
        if let Some(close_tag) = parse_html_tag(&html[close_index..]) {
            if close_tag.is_closing && close_tag.name.eq_ignore_ascii_case(tag) {
                return Some(close_index + close_tag.end_index);
            }
        }
        search_start = close_index + close_needle.len();
    }

    None
}

fn is_html_raw_text_tag(name: &str) -> bool {
    name.eq_ignore_ascii_case("script") || name.eq_ignore_ascii_case("style")
}

fn inserts_text_break(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "br"
            | "dd"
            | "div"
            | "dl"
            | "dt"
            | "figcaption"
            | "figure"
            | "footer"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "li"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "td"
            | "th"
            | "tr"
            | "ul"
    )
}

fn decode_html_entity(input: &str) -> Option<(&'static str, usize)> {
    let rest = input.strip_prefix('&')?;
    let semicolon_index = rest.find(';')?;
    if semicolon_index > 12 {
        return None;
    }

    let decoded = match &rest[..semicolon_index] {
        "nbsp" => " ",
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => "",
    };

    Some((decoded, semicolon_index + 2))
}

fn is_noise_preview_text(value: &str) -> bool {
    value
        .trim_matches(|ch: char| !ch.is_alphanumeric())
        .eq_ignore_ascii_case("undefined")
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

fn normalize_message_id_header(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('<') && trimmed.ends_with('>') {
        Some(trimmed.to_string())
    } else {
        Some(format!("<{trimmed}>"))
    }
}

fn infer_folder_role(path: &str) -> FolderRole {
    let normalized = path.to_ascii_lowercase();
    let display_name = normalized
        .rsplit(['/', '.'])
        .next()
        .unwrap_or(normalized.as_str());
    match display_name {
        "inbox" => FolderRole::Inbox,
        "sent" | "sent mail" | "sent messages" | "sent items" => FolderRole::Sent,
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
    value.replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::{MailAuth, MailProvider};

    fn test_account() -> MailAccount {
        MailAccount {
            id: "acct".to_string(),
            display_name: "Ops".to_string(),
            email: "ops@example.com".to_string(),
            provider: MailProvider::GenericImapSmtp,
            auth: MailAuth::Password {
                password: String::new(),
            },
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_tls: true,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_tls: true,
            sync_enabled: true,
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        }
    }

    fn test_folder(account: &MailAccount) -> MailFolder {
        MailFolder {
            id: "acct:inbox".to_string(),
            account_id: account.id.clone(),
            name: "INBOX".to_string(),
            path: "INBOX".to_string(),
            role: FolderRole::Inbox,
            unread_count: 0,
            total_count: 0,
        }
    }

    #[test]
    fn builds_xoauth2_sasl_payload() {
        let payload = build_xoauth2_payload("user@gmail.com", "access-token");

        assert_eq!(
            payload,
            "user=user@gmail.com\x01auth=Bearer access-token\x01\x01"
        );
    }

    #[test]
    fn parses_plain_text_message_with_attachment_metadata() {
        let account = test_account();
        let folder = test_folder(&account);
        let raw = b"From: Security <sec@example.com>\r\nTo: Ops <ops@example.com>\r\nSubject: Test report\r\nMessage-ID: <a@example.com>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello from IMAP.\r\n--x\r\nContent-Type: text/plain; name=report.txt\r\nContent-Disposition: attachment; filename=report.txt\r\n\r\nreport-body\r\n--x--\r\n";

        let parsed = parse_message(&account, &folder, 42, raw, Some(raw.len() as u32)).unwrap();
        assert_eq!(parsed.uid.as_deref(), Some("42"));
        assert_eq!(parsed.message_id_header.as_deref(), Some("<a@example.com>"));
        assert_eq!(parsed.subject, "Test report");
        assert!(parsed.body.as_deref().unwrap_or_default().contains("Hello"));
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].filename, "report.txt");
    }

    #[test]
    fn parses_html_body_without_losing_plaintext() {
        let account = test_account();
        let folder = test_folder(&account);
        let raw = b"From: Security <sec@example.com>\r\nTo: Ops <ops@example.com>\r\nSubject: HTML alternative\r\nMessage-ID: <html-alt@example.com>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=alt\r\n\r\n--alt\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nPlain fallback\r\n--alt\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><p>HTML Body</p><img src=\"https://example.com/logo.png\"></body></html>\r\n--alt--\r\n";

        let parsed = parse_message(&account, &folder, 44, raw, Some(raw.len() as u32)).unwrap();

        assert_eq!(parsed.body.as_deref(), Some("Plain fallback"));
        assert!(parsed
            .html_body
            .as_deref()
            .unwrap_or_default()
            .contains("HTML Body"));
        assert_eq!(parsed.raw_mime.as_deref(), Some(raw.as_slice()));
    }

    #[test]
    fn extracts_inline_cid_image_resource() {
        let account = test_account();
        let folder = test_folder(&account);
        let raw = b"From: Security <sec@example.com>\r\nTo: Ops <ops@example.com>\r\nSubject: CID image\r\nMessage-ID: <cid-image@example.com>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=rel\r\n\r\n--rel\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><img src=\"cid:LOGO@example.com\"></body></html>\r\n--rel\r\nContent-Type: image/png\r\nContent-Transfer-Encoding: base64\r\nContent-ID: <logo@example.com>\r\nContent-Disposition: inline; filename=\"logo.png\"\r\n\r\naW1hZ2UtYnl0ZXM=\r\n--rel\r\nContent-Type: application/pdf\r\nContent-Transfer-Encoding: base64\r\nContent-ID: <unused@example.com>\r\nContent-Disposition: attachment; filename=\"unused.pdf\"\r\n\r\ndW51c2VkLWJ5dGVz\r\n--rel--\r\n";

        let parsed = parse_message(&account, &folder, 45, raw, Some(raw.len() as u32)).unwrap();

        assert_eq!(parsed.inline_resources.len(), 1);
        let resource = &parsed.inline_resources[0];
        assert_eq!(resource.content_id, "logo@example.com");
        assert_eq!(resource.mime_type, "image/png");
        assert_eq!(resource.bytes, b"image-bytes");
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
    fn infers_common_provider_sent_folder_paths() {
        assert_eq!(infer_folder_role("Sent Mail"), FolderRole::Sent);
        assert_eq!(infer_folder_role("[Gmail]/Sent Mail"), FolderRole::Sent);
    }

    #[test]
    fn parses_date_header_and_html_only_body() {
        let account = test_account();
        let folder = test_folder(&account);
        let raw = b"From: sec@example.com\r\nTo: ops@example.com\r\nDate: Tue, 20 Feb 2024 10:30:00 +0800\r\nSubject: HTML report\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><h1>Report&nbsp;Ready</h1><p>A&amp;B</p></body></html>\r\n";

        let parsed = parse_message(&account, &folder, 43, raw, None).unwrap();
        assert!(parsed.received_at.starts_with("2024-02-20T02:30:00"));
        let body = parsed.body.as_deref().unwrap_or_default();
        assert!(body.contains("Report Ready"));
        assert!(body.contains("A&B"));
        assert!(parsed.body_preview.contains("Report Ready"));
    }

    #[test]
    fn parses_css_heavy_html_without_leaking_css_into_preview() {
        let account = test_account();
        let folder = test_folder(&account);
        let raw = b"From: promo@example.com\r\nTo: ops@example.com\r\nSubject: Promo\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><head><style>@media only screen and (max-width:480px){table{font-family:Arial;width:100%}}</style></head><body><p>Deal ready</p><script>undefined</script></body></html>\r\n";

        let parsed = parse_message(&account, &folder, 44, raw, None).unwrap();

        assert_eq!(parsed.body.as_deref(), Some("Deal ready"));
        assert_eq!(parsed.body_preview, "Deal ready");
        assert!(!parsed.body_preview.contains("@media"));
        assert!(!parsed.body_preview.contains("undefined"));
    }

    #[test]
    fn parses_html_void_and_prefix_tags_without_dropping_body() {
        let account = test_account();
        let folder = test_folder(&account);
        let raw = b"From: promo@example.com\r\nTo: ops@example.com\r\nSubject: Header prefix\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><head><meta charset=\"utf-8\"><link rel=\"stylesheet\" href=\"https://cdn.example.com/mail.css\"></head><body><header>Top banner</header><p>Readable body</p></body></html>\r\n";

        let parsed = parse_message(&account, &folder, 45, raw, None).unwrap();

        assert_eq!(parsed.body.as_deref(), Some("Top banner Readable body"));
        assert_eq!(parsed.body_preview, "Top banner Readable body");
    }

    #[test]
    fn html_to_text_strips_unclosed_raw_text_tags_without_dropping_prior_text() {
        assert_eq!(
            html_to_text("<p>Before</p><style>@media only screen { .x { font-family: Arial; }"),
            "Before"
        );
    }

    #[test]
    fn keeps_plain_text_css_examples_in_preview() {
        let account = test_account();
        let folder = test_folder(&account);
        let raw = b"From: dev@example.com\r\nTo: ops@example.com\r\nSubject: CSS example\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nPlease review this CSS: @media only screen { .card { width: 320px; font-family: Arial; } }\r\n";

        let parsed = parse_message(&account, &folder, 47, raw, None).unwrap();

        assert!(parsed.body_preview.contains("@media only screen"));
        assert!(parsed.body_preview.contains("font-family"));
    }

    #[test]
    fn keeps_undefined_token_in_full_plain_text_body() {
        let account = test_account();
        let folder = test_folder(&account);
        let raw = b"From: dev@example.com\r\nTo: ops@example.com\r\nSubject: Undefined example\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe value is undefined in this payload.\r\n";

        let parsed = parse_message(&account, &folder, 48, raw, None).unwrap();

        assert_eq!(
            parsed.body.as_deref(),
            Some("The value is undefined in this payload.")
        );
        assert_eq!(
            parsed.body_preview,
            "The value is undefined in this payload."
        );
    }

    #[test]
    fn keeps_undefined_token_in_html_only_body() {
        let account = test_account();
        let folder = test_folder(&account);
        let raw = b"From: dev@example.com\r\nTo: ops@example.com\r\nSubject: HTML undefined example\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><p>The value is undefined in this payload.</p></body></html>\r\n";

        let parsed = parse_message(&account, &folder, 52, raw, None).unwrap();

        assert_eq!(
            parsed.body.as_deref(),
            Some("The value is undefined in this payload.")
        );
        assert_eq!(
            parsed.body_preview,
            "The value is undefined in this payload."
        );
    }

    #[test]
    fn clears_preview_when_body_is_only_undefined() {
        let account = test_account();
        let folder = test_folder(&account);
        let raw = b"From: dev@example.com\r\nTo: ops@example.com\r\nSubject: Undefined placeholder\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nundefined\r\n";

        let parsed = parse_message(&account, &folder, 53, raw, None).unwrap();

        assert_eq!(parsed.body.as_deref(), Some("undefined"));
        assert_eq!(parsed.body_preview, "");
    }

    #[test]
    fn prefers_later_real_text_part_over_html_text_fallback() {
        let account = test_account();
        let folder = test_folder(&account);
        let raw = b"From: dev@example.com\r\nTo: ops@example.com\r\nSubject: Mixed related\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=rel\r\n\r\n--rel\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><p>HTML fallback</p></body></html>\r\n--rel\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nReal plain body\r\n--rel--\r\n";

        let parsed = parse_message(&account, &folder, 49, raw, None).unwrap();

        assert_eq!(parsed.body.as_deref(), Some("Real plain body"));
        assert_eq!(parsed.body_preview, "Real plain body");
    }

    #[test]
    fn ignores_inline_non_plain_text_parts_when_falling_back_to_html() {
        let account = test_account();
        let folder = test_folder(&account);
        let raw = b"From: dev@example.com\r\nTo: ops@example.com\r\nSubject: Related invite\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=rel\r\n\r\n--rel\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><p>HTML summary</p></body></html>\r\n--rel\r\nContent-Type: text/calendar; charset=utf-8\r\nContent-Disposition: inline\r\n\r\nBEGIN:VCALENDAR\r\nSUMMARY:Calendar should not be body\r\nEND:VCALENDAR\r\n--rel\r\nContent-Type: text/css; charset=utf-8\r\nContent-Disposition: inline\r\n\r\n.invite { font-family: Arial; }\r\n--rel--\r\n";

        let parsed = parse_message(&account, &folder, 54, raw, None).unwrap();

        assert_eq!(parsed.body.as_deref(), Some("HTML summary"));
        assert_eq!(parsed.body_preview, "HTML summary");
        assert!(!parsed.body_preview.contains("VCALENDAR"));
        assert!(!parsed.body_preview.contains("font-family"));
    }

    #[test]
    fn does_not_use_text_attachment_as_message_body() {
        let account = test_account();
        let folder = test_folder(&account);
        let raw = b"From: dev@example.com\r\nTo: ops@example.com\r\nSubject: Attachment only\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=mix\r\n\r\n--mix\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><p>HTML body</p></body></html>\r\n--mix\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Disposition: attachment; filename=\"note.txt\"\r\n\r\nAttachment text\r\n--mix--\r\n";

        let parsed = parse_message(&account, &folder, 50, raw, None).unwrap();

        assert_eq!(parsed.body.as_deref(), Some("HTML body"));
        assert_eq!(parsed.body_preview, "HTML body");
    }

    #[test]
    fn does_not_use_named_text_part_as_message_body() {
        let account = test_account();
        let folder = test_folder(&account);
        let raw = b"From: dev@example.com\r\nTo: ops@example.com\r\nSubject: Named text attachment\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=mix\r\n\r\n--mix\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><p>HTML body</p></body></html>\r\n--mix\r\nContent-Type: text/plain; charset=utf-8; name=\"note.txt\"\r\n\r\nAttachment text\r\n--mix--\r\n";

        let parsed = parse_message(&account, &folder, 51, raw, None).unwrap();

        assert_eq!(parsed.body.as_deref(), Some("HTML body"));
        assert_eq!(parsed.body_preview, "HTML body");
    }
}
