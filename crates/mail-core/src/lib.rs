use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use uuid::Uuid;

pub type Timestamp = String;

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn now_rfc3339() -> Timestamp {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub fn rfc3339_from_unix_timestamp(timestamp: i64) -> Option<Timestamp> {
    OffsetDateTime::from_unix_timestamp(timestamp)
        .ok()?
        .format(&Rfc3339)
        .ok()
}

pub fn now_plus_seconds_rfc3339(seconds: i64) -> Timestamp {
    let timestamp = OffsetDateTime::now_utc().unix_timestamp() + seconds.max(0);
    rfc3339_from_unix_timestamp(timestamp).unwrap_or_else(now_rfc3339)
}

pub fn timestamp_is_future(timestamp: &str) -> bool {
    OffsetDateTime::parse(timestamp, &Rfc3339)
        .map(|value| value > OffsetDateTime::now_utc())
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MailProvider {
    #[default]
    GenericImapSmtp,
    Gmail,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MailAuth {
    Password {
        password: String,
    },
    GoogleOAuth {
        refresh_token: String,
        access_token: String,
        expires_at: Timestamp,
    },
}

impl Default for MailAuth {
    fn default() -> Self {
        Self::Password {
            password: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailAccount {
    pub id: String,
    pub display_name: String,
    pub email: String,
    #[serde(default)]
    pub provider: MailProvider,
    #[serde(default)]
    pub auth: MailAuth,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_tls: bool,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_tls: bool,
    pub sync_enabled: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl MailAccount {
    pub fn is_password_auth(&self) -> bool {
        matches!(self.auth, MailAuth::Password { .. })
    }

    pub fn is_oauth_auth(&self) -> bool {
        matches!(self.auth, MailAuth::GoogleOAuth { .. })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailFolder {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub path: String,
    pub role: FolderRole,
    pub unread_count: u32,
    pub total_count: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FolderRole {
    Inbox,
    Sent,
    Archive,
    Trash,
    Drafts,
    Junk,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MessageFlags {
    pub is_read: bool,
    pub is_starred: bool,
    pub is_answered: bool,
    pub is_forwarded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttachmentRef {
    pub id: String,
    pub message_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub local_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailMessage {
    pub id: String,
    pub account_id: String,
    pub folder_id: String,
    pub uid: Option<String>,
    pub message_id_header: Option<String>,
    pub subject: String,
    pub sender: String,
    pub recipients: Vec<String>,
    pub cc: Vec<String>,
    pub received_at: Timestamp,
    pub body_preview: String,
    pub body: Option<String>,
    pub attachments: Vec<AttachmentRef>,
    pub flags: MessageFlags,
    pub size_bytes: Option<i64>,
    pub deleted_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncStateKind {
    Idle,
    Syncing,
    Watching,
    Backoff,
    Error,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncState {
    pub account_id: String,
    pub folder_id: Option<String>,
    pub state: SyncStateKind,
    pub last_uid: Option<String>,
    pub last_synced_at: Option<Timestamp>,
    pub error_message: Option<String>,
    pub backoff_until: Option<Timestamp>,
    pub failure_count: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MailActionKind {
    MarkRead,
    MarkUnread,
    Star,
    Unstar,
    Move,
    Archive,
    Delete,
    PermanentDelete,
    Send,
    Forward,
    BatchDelete,
    BatchMove,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailActionRequest {
    pub action: MailActionKind,
    pub account_id: String,
    pub message_ids: Vec<String>,
    pub target_folder_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MailActionResultKind {
    Executed,
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailActionResult {
    pub kind: MailActionResultKind,
    pub pending_action_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PendingActionStatus {
    Pending,
    Accepted,
    Rejected,
    Executed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingMailAction {
    pub id: String,
    pub account_id: String,
    pub action: MailActionKind,
    pub message_ids: Vec<String>,
    pub target_folder_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_message_id: Option<String>,
    pub draft: Option<SendMessageDraft>,
    pub status: PendingActionStatus,
    pub error_message: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteMailAction {
    pub action: MailActionKind,
    pub source_folder: MailFolder,
    pub target_folder: Option<MailFolder>,
    pub uids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionAuditStatus {
    Queued,
    Accepted,
    Rejected,
    Executed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MailActionAudit {
    pub id: String,
    pub account_id: String,
    pub action: MailActionKind,
    pub message_ids: Vec<String>,
    pub status: ActionAuditStatus,
    pub error_message: Option<String>,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageQuery {
    pub account_id: Option<String>,
    pub folder_id: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

impl Default for MessageQuery {
    fn default() -> Self {
        Self {
            account_id: None,
            folder_id: None,
            limit: 100,
            offset: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SendMessageDraft {
    pub account_id: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id_header: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SendMessageResult {
    pub message_id: String,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionSettings {
    pub account_id: Option<String>,
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_tls: bool,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_tls: bool,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionTestResult {
    pub imap_ok: bool,
    pub smtp_ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageFetchRequest {
    pub last_uid: Option<String>,
    pub limit: u32,
}

impl Default for MessageFetchRequest {
    fn default() -> Self {
        Self {
            last_uid: None,
            limit: 100,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FolderWatchOutcome {
    Changed,
    Timeout,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiPriority {
    Low,
    #[default]
    Normal,
    High,
    Urgent,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiSettings {
    pub id: String,
    pub provider_name: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub enabled: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl std::fmt::Debug for AiSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiSettings")
            .field("id", &self.id)
            .field("provider_name", &self.provider_name)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &"***")
            .field("enabled", &self.enabled)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiSettingsView {
    pub provider_name: String,
    pub base_url: String,
    pub model: String,
    pub enabled: bool,
    pub api_key_mask: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SaveAiSettingsRequest {
    pub provider_name: String,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub enabled: bool,
}

impl std::fmt::Debug for SaveAiSettingsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveAiSettingsRequest")
            .field("provider_name", &self.provider_name)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .field("enabled", &self.enabled)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::{
        self, value::MapAccessDeserializer, DeserializeSeed, IntoDeserializer, MapAccess, Visitor,
    };
    use serde::ser::{
        self, Impossible, SerializeMap, SerializeStruct, SerializeStructVariant, Serializer,
    };
    use serde::Deserialize;
    use std::collections::BTreeMap;
    use std::fmt;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum TestValue {
        String(String),
        U64(u64),
        Bool(bool),
        Map(BTreeMap<String, TestValue>),
    }

    impl TestValue {
        fn as_str(&self) -> Option<&str> {
            match self {
                Self::String(value) => Some(value),
                _ => None,
            }
        }

        fn get(&self, key: &str) -> Option<&Self> {
            match self {
                Self::Map(values) => values.get(key),
                _ => None,
            }
        }
    }

    #[derive(Debug)]
    struct TestSerdeError(String);

    impl fmt::Display for TestSerdeError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(&self.0)
        }
    }

    impl std::error::Error for TestSerdeError {}

    impl ser::Error for TestSerdeError {
        fn custom<T: fmt::Display>(message: T) -> Self {
            Self(message.to_string())
        }
    }

    impl de::Error for TestSerdeError {
        fn custom<T: fmt::Display>(message: T) -> Self {
            Self(message.to_string())
        }
    }

    struct TestSerializer;

    fn unsupported<T>() -> Result<T, TestSerdeError> {
        Err(TestSerdeError(
            "unsupported test serializer value".to_string(),
        ))
    }

    impl Serializer for TestSerializer {
        type Ok = TestValue;
        type Error = TestSerdeError;
        type SerializeSeq = Impossible<TestValue, TestSerdeError>;
        type SerializeTuple = Impossible<TestValue, TestSerdeError>;
        type SerializeTupleStruct = Impossible<TestValue, TestSerdeError>;
        type SerializeTupleVariant = Impossible<TestValue, TestSerdeError>;
        type SerializeMap = TestMapSerializer;
        type SerializeStruct = TestMapSerializer;
        type SerializeStructVariant = TestMapSerializer;

        fn serialize_bool(self, value: bool) -> Result<Self::Ok, Self::Error> {
            Ok(TestValue::Bool(value))
        }

        fn serialize_u16(self, value: u16) -> Result<Self::Ok, Self::Error> {
            Ok(TestValue::U64(u64::from(value)))
        }

        fn serialize_u32(self, value: u32) -> Result<Self::Ok, Self::Error> {
            Ok(TestValue::U64(u64::from(value)))
        }

        fn serialize_u64(self, value: u64) -> Result<Self::Ok, Self::Error> {
            Ok(TestValue::U64(value))
        }

        fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
            Ok(TestValue::String(value.to_string()))
        }

        fn serialize_unit_variant(
            self,
            _name: &'static str,
            _variant_index: u32,
            variant: &'static str,
        ) -> Result<Self::Ok, Self::Error> {
            Ok(TestValue::String(variant.to_string()))
        }

        fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
            Ok(TestMapSerializer::default())
        }

        fn serialize_struct(
            self,
            _name: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeStruct, Self::Error> {
            Ok(TestMapSerializer::default())
        }

        fn serialize_struct_variant(
            self,
            _name: &'static str,
            _variant_index: u32,
            variant: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeStructVariant, Self::Error> {
            let mut serializer = TestMapSerializer::default();
            serializer
                .entries
                .insert("type".to_string(), TestValue::String(variant.to_string()));
            Ok(serializer)
        }

        fn serialize_i8(self, _value: i8) -> Result<Self::Ok, Self::Error> {
            unsupported()
        }

        fn serialize_i16(self, _value: i16) -> Result<Self::Ok, Self::Error> {
            unsupported()
        }

        fn serialize_i32(self, _value: i32) -> Result<Self::Ok, Self::Error> {
            unsupported()
        }

        fn serialize_i64(self, _value: i64) -> Result<Self::Ok, Self::Error> {
            unsupported()
        }

        fn serialize_u8(self, value: u8) -> Result<Self::Ok, Self::Error> {
            Ok(TestValue::U64(u64::from(value)))
        }

        fn serialize_f32(self, _value: f32) -> Result<Self::Ok, Self::Error> {
            unsupported()
        }

        fn serialize_f64(self, _value: f64) -> Result<Self::Ok, Self::Error> {
            unsupported()
        }

        fn serialize_char(self, _value: char) -> Result<Self::Ok, Self::Error> {
            unsupported()
        }

        fn serialize_bytes(self, _value: &[u8]) -> Result<Self::Ok, Self::Error> {
            unsupported()
        }

        fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
            unsupported()
        }

        fn serialize_some<T: ?Sized + Serialize>(
            self,
            _value: &T,
        ) -> Result<Self::Ok, Self::Error> {
            unsupported()
        }

        fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
            unsupported()
        }

        fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
            unsupported()
        }

        fn serialize_newtype_struct<T: ?Sized + Serialize>(
            self,
            _name: &'static str,
            _value: &T,
        ) -> Result<Self::Ok, Self::Error> {
            unsupported()
        }

        fn serialize_newtype_variant<T: ?Sized + Serialize>(
            self,
            _name: &'static str,
            _variant_index: u32,
            _variant: &'static str,
            _value: &T,
        ) -> Result<Self::Ok, Self::Error> {
            unsupported()
        }

        fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
            unsupported()
        }

        fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
            unsupported()
        }

        fn serialize_tuple_struct(
            self,
            _name: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeTupleStruct, Self::Error> {
            unsupported()
        }

        fn serialize_tuple_variant(
            self,
            _name: &'static str,
            _variant_index: u32,
            _variant: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeTupleVariant, Self::Error> {
            unsupported()
        }
    }

    #[derive(Default)]
    struct TestMapSerializer {
        entries: BTreeMap<String, TestValue>,
        pending_key: Option<String>,
    }

    impl SerializeMap for TestMapSerializer {
        type Ok = TestValue;
        type Error = TestSerdeError;

        fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
            let key = key.serialize(TestSerializer)?;
            let TestValue::String(key) = key else {
                return Err(TestSerdeError("map key must be a string".to_string()));
            };
            self.pending_key = Some(key);
            Ok(())
        }

        fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
            let key = self
                .pending_key
                .take()
                .ok_or_else(|| TestSerdeError("missing map key".to_string()))?;
            self.entries.insert(key, value.serialize(TestSerializer)?);
            Ok(())
        }

        fn end(self) -> Result<Self::Ok, Self::Error> {
            Ok(TestValue::Map(self.entries))
        }
    }

    impl SerializeStruct for TestMapSerializer {
        type Ok = TestValue;
        type Error = TestSerdeError;

        fn serialize_field<T: ?Sized + Serialize>(
            &mut self,
            key: &'static str,
            value: &T,
        ) -> Result<(), Self::Error> {
            self.entries
                .insert(key.to_string(), value.serialize(TestSerializer)?);
            Ok(())
        }

        fn end(self) -> Result<Self::Ok, Self::Error> {
            Ok(TestValue::Map(self.entries))
        }
    }

    impl SerializeStructVariant for TestMapSerializer {
        type Ok = TestValue;
        type Error = TestSerdeError;

        fn serialize_field<T: ?Sized + Serialize>(
            &mut self,
            key: &'static str,
            value: &T,
        ) -> Result<(), Self::Error> {
            self.entries
                .insert(key.to_string(), value.serialize(TestSerializer)?);
            Ok(())
        }

        fn end(self) -> Result<Self::Ok, Self::Error> {
            Ok(TestValue::Map(self.entries))
        }
    }

    impl<'de> IntoDeserializer<'de, TestSerdeError> for TestValue {
        type Deserializer = TestDeserializer;

        fn into_deserializer(self) -> Self::Deserializer {
            TestDeserializer(self)
        }
    }

    struct TestDeserializer(TestValue);

    impl<'de> de::Deserializer<'de> for TestDeserializer {
        type Error = TestSerdeError;

        fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            match self.0 {
                TestValue::String(value) => visitor.visit_string(value),
                TestValue::U64(value) => visitor.visit_u64(value),
                TestValue::Bool(value) => visitor.visit_bool(value),
                TestValue::Map(values) => visitor.visit_map(TestMapAccess {
                    entries: values.into_iter().collect(),
                }),
            }
        }

        fn deserialize_enum<V: Visitor<'de>>(
            self,
            name: &'static str,
            variants: &'static [&'static str],
            visitor: V,
        ) -> Result<V::Value, Self::Error> {
            match self.0 {
                TestValue::String(value) => value
                    .into_deserializer()
                    .deserialize_enum(name, variants, visitor),
                value => TestDeserializer(value).deserialize_any(visitor),
            }
        }

        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes byte_buf option
            unit unit_struct newtype_struct seq tuple tuple_struct map struct identifier
            ignored_any
        }
    }

    struct TestMapAccess {
        entries: Vec<(String, TestValue)>,
    }

    impl<'de> MapAccess<'de> for TestMapAccess {
        type Error = TestSerdeError;

        fn next_key_seed<K: DeserializeSeed<'de>>(
            &mut self,
            seed: K,
        ) -> Result<Option<K::Value>, Self::Error> {
            let Some((key, _)) = self.entries.first() else {
                return Ok(None);
            };
            seed.deserialize(key.clone().into_deserializer()).map(Some)
        }

        fn next_value_seed<V: DeserializeSeed<'de>>(
            &mut self,
            seed: V,
        ) -> Result<V::Value, Self::Error> {
            let (_, value) = self.entries.remove(0);
            seed.deserialize(value.into_deserializer())
        }
    }

    fn legacy_account_value() -> TestValue {
        TestValue::Map(BTreeMap::from([
            ("id".to_string(), TestValue::String("account-1".to_string())),
            (
                "display_name".to_string(),
                TestValue::String("Legacy Mail".to_string()),
            ),
            (
                "email".to_string(),
                TestValue::String("user@example.com".to_string()),
            ),
            (
                "imap_host".to_string(),
                TestValue::String("imap.example.com".to_string()),
            ),
            ("imap_port".to_string(), TestValue::U64(993)),
            ("imap_tls".to_string(), TestValue::Bool(true)),
            (
                "smtp_host".to_string(),
                TestValue::String("smtp.example.com".to_string()),
            ),
            ("smtp_port".to_string(), TestValue::U64(465)),
            ("smtp_tls".to_string(), TestValue::Bool(true)),
            ("sync_enabled".to_string(), TestValue::Bool(true)),
            (
                "created_at".to_string(),
                TestValue::String("2026-05-06T00:00:00Z".to_string()),
            ),
            (
                "updated_at".to_string(),
                TestValue::String("2026-05-06T00:00:00Z".to_string()),
            ),
        ]))
    }

    fn deserialize_account(value: TestValue) -> MailAccount {
        MailAccount::deserialize(MapAccessDeserializer::new(TestMapAccess {
            entries: match value {
                TestValue::Map(values) => values.into_iter().collect(),
                _ => panic!("expected map"),
            },
        }))
        .unwrap()
    }

    #[test]
    fn legacy_mail_account_defaults_to_generic_password_auth() {
        let account = deserialize_account(legacy_account_value());

        assert_eq!(account.provider, MailProvider::GenericImapSmtp);
        assert_eq!(
            account.auth,
            MailAuth::Password {
                password: String::new()
            }
        );
        assert!(account.is_password_auth());
        assert!(!account.is_oauth_auth());

        let serialized = account.serialize(TestSerializer).unwrap();

        assert_eq!(
            serialized.get("provider").and_then(TestValue::as_str),
            Some("generic_imap_smtp")
        );
        assert_eq!(
            serialized
                .get("auth")
                .and_then(|auth| auth.get("type"))
                .and_then(TestValue::as_str),
            Some("password")
        );
        assert_eq!(
            serialized
                .get("auth")
                .and_then(|auth| auth.get("password"))
                .and_then(TestValue::as_str),
            Some("")
        );
    }

    #[test]
    fn gmail_oauth_account_round_trips_through_json() {
        let account = MailAccount {
            id: "account-2".to_string(),
            display_name: "Gmail".to_string(),
            email: "user@gmail.com".to_string(),
            provider: MailProvider::Gmail,
            auth: MailAuth::GoogleOAuth {
                refresh_token: "refresh-token".to_string(),
                access_token: "access-token".to_string(),
                expires_at: "2026-05-06T01:00:00Z".to_string(),
            },
            imap_host: "imap.gmail.com".to_string(),
            imap_port: 993,
            imap_tls: true,
            smtp_host: "smtp.gmail.com".to_string(),
            smtp_port: 465,
            smtp_tls: true,
            sync_enabled: true,
            created_at: "2026-05-06T00:00:00Z".to_string(),
            updated_at: "2026-05-06T00:00:00Z".to_string(),
        };

        let serialized = account.serialize(TestSerializer).unwrap();
        let deserialized = deserialize_account(serialized);

        assert_eq!(deserialized, account);
        assert!(deserialized.is_oauth_auth());
        assert!(!deserialized.is_password_auth());
    }

    #[test]
    fn save_ai_settings_request_debug_redacts_api_key() {
        let request = SaveAiSettingsRequest {
            provider_name: "openai-compatible".to_string(),
            base_url: "https://api.example.com/v1".to_string(),
            model: "mail-model".to_string(),
            api_key: Some("sk-plaintext-secret".to_string()),
            enabled: true,
        };

        let debug = format!("{request:?}");

        assert!(!debug.contains("sk-plaintext-secret"));
        assert!(debug.contains("***"));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiAnalysisInput {
    pub message_id: String,
    pub subject: String,
    pub sender: String,
    pub recipients: Vec<String>,
    pub cc: Vec<String>,
    pub received_at: Timestamp,
    pub body_preview: String,
    pub body: Option<String>,
    pub attachments: Vec<AttachmentRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiInsightPayload {
    pub summary: String,
    pub category: String,
    pub priority: AiPriority,
    pub todos: Vec<String>,
    pub reply_draft: String,
    #[serde(default, skip_deserializing)]
    pub raw_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiInsight {
    pub id: String,
    pub message_id: String,
    pub provider_name: String,
    pub model: String,
    pub summary: String,
    pub category: String,
    pub priority: AiPriority,
    pub todos: Vec<String>,
    pub reply_draft: String,
    pub raw_json: String,
    pub created_at: Timestamp,
}
