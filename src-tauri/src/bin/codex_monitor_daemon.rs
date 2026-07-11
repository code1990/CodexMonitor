#[allow(dead_code)]
#[path = "../backend/mod.rs"]
mod backend;
#[path = "../codex/args.rs"]
mod codex_args;
#[path = "../codex/config.rs"]
mod codex_config;
#[path = "../codex/home.rs"]
mod codex_home;
#[path = "../files/io.rs"]
mod file_io;
#[path = "../files/ops.rs"]
mod file_ops;
#[path = "../files/policy.rs"]
mod file_policy;
#[path = "../git_utils.rs"]
mod git_utils;
#[path = "codex_monitor_daemon/rpc.rs"]
mod rpc;
#[path = "../rules.rs"]
mod rules;
#[path = "../shared/mod.rs"]
mod shared;
#[path = "../storage.rs"]
mod storage;
#[path = "codex_monitor_daemon/transport.rs"]
mod transport;
#[allow(dead_code)]
#[path = "../types.rs"]
mod types;
#[path = "../utils.rs"]
mod utils;
#[path = "../workspaces/macos.rs"]
mod workspace_macos;
#[path = "../workspaces/settings.rs"]
mod workspace_settings;

// Provide feature-style module paths for shared cores when compiled in the daemon.
mod codex {
    pub(crate) mod args {
        pub(crate) use crate::codex_args::*;
    }
    pub(crate) mod config {
        pub(crate) use crate::codex_config::*;
    }
    pub(crate) mod home {
        pub(crate) use crate::codex_home::*;
    }
}

mod files {
    pub(crate) mod io {
        pub(crate) use crate::file_io::*;
    }
    pub(crate) mod ops {
        pub(crate) use crate::file_ops::*;
    }
    pub(crate) mod policy {
        pub(crate) use crate::file_policy::*;
    }
}

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::env;
use std::hash::{Hash, Hasher};
use std::fs::File;
use std::io::{BufRead, BufReader as StdBufReader, Read};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ignore::WalkBuilder;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, Mutex, Semaphore};

use backend::app_server::{spawn_workspace_session, WorkspaceSession};
use backend::events::{AppServerEvent, EventSink, TerminalExit, TerminalOutput};
use chrono::DateTime;
use shared::codex_core::CodexLoginCancelState;
use shared::process_core::kill_child_process_tree;
use shared::prompts_core::{self, CustomPromptEntry};
use shared::{
    agents_config_core, codex_aux_core, codex_core, files_core, git_core, git_ui_core,
    local_usage_core, settings_core, workspaces_core, worktree_core,
};
use storage::{read_settings, read_workspaces};
use types::{
    AppSettings, GitCommitDiff, GitFileDiff, GitHubIssuesResponse, GitHubPullRequestComment,
    GitHubPullRequestDiff, GitHubPullRequestsResponse, GitLogResponse, LocalUsageSnapshot,
    WorkspaceEntry, WorkspaceInfo, WorkspaceSettings, WorktreeSetupStatus,
};
use workspace_settings::apply_workspace_settings_update;

const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:4732";
const MAX_IN_FLIGHT_RPC_PER_CONNECTION: usize = 32;
const DAEMON_NAME: &str = "codex-monitor-daemon";
const MAX_HTTP_BODY_BYTES: usize = 1024 * 1024;

fn spawn_with_client(
    event_sink: DaemonEventSink,
    client_version: String,
    entry: WorkspaceEntry,
    default_bin: Option<String>,
    codex_args: Option<String>,
    codex_home: Option<PathBuf>,
) -> impl std::future::Future<Output = Result<Arc<WorkspaceSession>, String>> {
    spawn_workspace_session(
        entry,
        default_bin,
        codex_args,
        codex_home,
        client_version,
        event_sink,
    )
}

#[derive(Clone)]
struct DaemonEventSink {
    tx: broadcast::Sender<DaemonEvent>,
}

#[derive(Clone)]
enum DaemonEvent {
    AppServer(AppServerEvent),
    TaskUpdated(ServiceTaskRecord),
    ConversationUpdated(ServiceConversationRecord),
    #[allow(dead_code)]
    TerminalOutput(TerminalOutput),
    #[allow(dead_code)]
    TerminalExit(TerminalExit),
}

impl EventSink for DaemonEventSink {
    fn emit_app_server_event(&self, event: AppServerEvent) {
        let _ = self.tx.send(DaemonEvent::AppServer(event));
    }

    fn emit_terminal_output(&self, event: TerminalOutput) {
        let _ = self.tx.send(DaemonEvent::TerminalOutput(event));
    }

    fn emit_terminal_exit(&self, event: TerminalExit) {
        let _ = self.tx.send(DaemonEvent::TerminalExit(event));
    }
}

struct DaemonConfig {
    listen: SocketAddr,
    http_listen: Option<SocketAddr>,
    token: Option<String>,
    data_dir: PathBuf,
}

struct DaemonState {
    data_dir: PathBuf,
    workspaces: Mutex<HashMap<String, WorkspaceEntry>>,
    sessions: Mutex<HashMap<String, Arc<WorkspaceSession>>>,
    tasks: Mutex<HashMap<String, ServiceTaskRecord>>,
    conversations: Mutex<HashMap<String, ServiceConversationRecord>>,
    tasks_path: PathBuf,
    conversations_path: PathBuf,
    mysql_retry_path: PathBuf,
    storage_path: PathBuf,
    settings_path: PathBuf,
    app_settings: Mutex<AppSettings>,
    mysql_history: Option<MySqlHistoryWriter>,
    mysql_retry_queue: Mutex<HashMap<String, PendingMySqlWrite>>,
    mysql_retry_flush_lock: Mutex<()>,
    event_sink: DaemonEventSink,
    codex_login_cancels: Mutex<HashMap<String, CodexLoginCancelState>>,
    daemon_binary_path: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct WorkspaceFileResponse {
    content: String,
    truncated: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServiceTaskRecord {
    task_id: String,
    status: String,
    workspace_id: String,
    thread_id: String,
    turn_id: Option<String>,
    created_thread: bool,
    submitted_at_ms: u64,
    completed_at_ms: Option<u64>,
    last_error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServiceConversationRecord {
    conversation_id: String,
    workspace_id: String,
    thread_id: String,
    title: String,
    requirement: String,
    status: String,
    operator: Option<String>,
    created_at_ms: u64,
    updated_at_ms: u64,
    last_message_preview: Option<String>,
    final_summary: Option<String>,
    last_error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingMySqlWrite {
    key: String,
    kind: String,
    sql: String,
    created_at_ms: u64,
    updated_at_ms: u64,
    attempt_count: u32,
    last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HistoryBackfillMessage {
    turn_id: Option<String>,
    role: String,
    message_type: String,
    content: String,
    payload_json: Option<String>,
    created_at_ms: u64,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryBackfillReport {
    conversations_selected: usize,
    conversations_upserted: usize,
    tasks_upserted: usize,
    messages_backfilled: usize,
    conversations_failed: usize,
    message_failures: usize,
}

#[derive(Clone)]
struct MySqlHistoryWriter {
    config: MySqlShellConfig,
}

#[derive(Clone)]
struct MySqlShellConfig {
    host: String,
    port: u16,
    username: String,
    password: String,
    database: String,
}

impl MySqlHistoryWriter {
    fn from_env() -> Option<Self> {
        let database_url = env::var("CODEX_MONITOR_MYSQL_URL").ok()?;
        let trimmed = database_url.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Self {
            config: parse_mysql_shell_config(trimmed).ok()?,
        })
    }

    async fn upsert_conversation(
        &self,
        conversation: &ServiceConversationRecord,
        requirement: &str,
    ) -> Result<(), String> {
        self.exec_sql(&build_upsert_conversation_sql(conversation, requirement))
            .await
    }

    async fn upsert_task(
        &self,
        conversation_id: &str,
        task: &ServiceTaskRecord,
    ) -> Result<(), String> {
        self.exec_sql(&build_upsert_task_sql(conversation_id, task)).await
    }

    async fn insert_message(
        &self,
        conversation_id: &str,
        thread_id: &str,
        turn_id: Option<&str>,
        role: &str,
        message_type: &str,
        content: &str,
        payload_json: Option<&str>,
        created_at_ms: u64,
    ) -> Result<(), String> {
        self.exec_sql(&build_insert_message_sql(
            conversation_id,
            thread_id,
            turn_id,
            role,
            message_type,
            content,
            payload_json,
            created_at_ms,
        ))
        .await
    }

    async fn insert_event(
        &self,
        conversation_id: &str,
        thread_id: &str,
        turn_id: Option<&str>,
        event_type: &str,
        event_status: Option<&str>,
        payload_json: &str,
        created_at_ms: u64,
    ) -> Result<(), String> {
        self.exec_sql(&build_insert_event_sql(
            conversation_id,
            thread_id,
            turn_id,
            event_type,
            event_status,
            payload_json,
            created_at_ms,
        ))
        .await
    }

    async fn exec_sql(&self, sql: &str) -> Result<(), String> {
        let output = tokio::process::Command::new("mysql")
            .arg(format!("-h{}", self.config.host))
            .arg(format!("-P{}", self.config.port))
            .arg(format!("-u{}", self.config.username))
            .arg(&self.config.database)
            .arg("-e")
            .arg(sql)
            .env("MYSQL_PWD", &self.config.password)
            .output()
            .await
            .map_err(|err| err.to_string())?;
        if output.status.success() {
            return Ok(());
        }
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn parse_mysql_shell_config(database_url: &str) -> Result<MySqlShellConfig, String> {
    let trimmed = database_url.trim();
    let without_scheme = trimmed
        .strip_prefix("mysql://")
        .ok_or_else(|| "mysql url must start with mysql://".to_string())?;
    let (authority_and_db, _) = without_scheme
        .split_once('?')
        .unwrap_or((without_scheme, ""));
    let (auth, database) = authority_and_db
        .rsplit_once('@')
        .ok_or_else(|| "mysql url missing @".to_string())?;
    let (username, password) = auth
        .split_once(':')
        .ok_or_else(|| "mysql url missing password".to_string())?;
    let (host_port, database) = database
        .split_once('/')
        .ok_or_else(|| "mysql url missing database".to_string())?;
    let (host, port) = host_port
        .rsplit_once(':')
        .ok_or_else(|| "mysql url missing port".to_string())?;
    Ok(MySqlShellConfig {
        host: host.to_string(),
        port: port.parse().map_err(|_| "invalid mysql port".to_string())?,
        username: username.to_string(),
        password: password.to_string(),
        database: database.to_string(),
    })
}

fn sql_string(value: &str) -> String {
    format!("'{}'", sql_escape(value))
}

fn sql_option_string(value: Option<&str>) -> String {
    value.map(sql_string).unwrap_or_else(|| "NULL".to_string())
}

fn sql_u64(value: u64) -> String {
    value.to_string()
}

fn sql_bool(value: bool) -> String {
    if value { "1" } else { "0" }.to_string()
}

fn sql_option_u64(value: Option<u64>) -> String {
    value.map(sql_u64).unwrap_or_else(|| "NULL".to_string())
}

fn sql_null_safe_eq(column: &str, value_sql: &str) -> String {
    format!("(({} = {}) OR ({} IS NULL AND {} IS NULL))", column, value_sql, column, value_sql)
}

fn stable_hash(value: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn build_upsert_conversation_sql(
    conversation: &ServiceConversationRecord,
    requirement: &str,
) -> String {
    format!(
        "INSERT INTO conversation \
        (conversation_id, workspace_id, thread_id, title, requirement, status, operator, last_message_preview, final_summary, last_error, created_at_ms, updated_at_ms) \
        VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}) \
        ON DUPLICATE KEY UPDATE \
          workspace_id = VALUES(workspace_id), \
          thread_id = VALUES(thread_id), \
          title = VALUES(title), \
          requirement = VALUES(requirement), \
          status = VALUES(status), \
          operator = VALUES(operator), \
          last_message_preview = VALUES(last_message_preview), \
          final_summary = VALUES(final_summary), \
          last_error = VALUES(last_error), \
          updated_at_ms = VALUES(updated_at_ms)",
        sql_string(&conversation.conversation_id),
        sql_string(&conversation.workspace_id),
        sql_string(&conversation.thread_id),
        sql_string(&conversation.title),
        sql_string(requirement),
        sql_string(&conversation.status),
        sql_option_string(conversation.operator.as_deref()),
        sql_option_string(conversation.last_message_preview.as_deref()),
        sql_option_string(conversation.final_summary.as_deref()),
        sql_option_string(conversation.last_error.as_deref()),
        sql_u64(conversation.created_at_ms),
        sql_u64(conversation.updated_at_ms),
    )
}

fn build_upsert_task_sql(conversation_id: &str, task: &ServiceTaskRecord) -> String {
    format!(
        "INSERT INTO conversation_task \
        (conversation_id, task_id, workspace_id, thread_id, turn_id, status, created_thread, submitted_at_ms, completed_at_ms, last_error) \
        VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}) \
        ON DUPLICATE KEY UPDATE \
          conversation_id = VALUES(conversation_id), \
          workspace_id = VALUES(workspace_id), \
          thread_id = VALUES(thread_id), \
          turn_id = VALUES(turn_id), \
          status = VALUES(status), \
          created_thread = VALUES(created_thread), \
          submitted_at_ms = VALUES(submitted_at_ms), \
          completed_at_ms = VALUES(completed_at_ms), \
          last_error = VALUES(last_error)",
        sql_string(conversation_id),
        sql_string(&task.task_id),
        sql_string(&task.workspace_id),
        sql_string(&task.thread_id),
        sql_option_string(task.turn_id.as_deref()),
        sql_string(&task.status),
        sql_bool(task.created_thread),
        sql_u64(task.submitted_at_ms),
        sql_option_u64(task.completed_at_ms),
        sql_option_string(task.last_error.as_deref()),
    )
}

fn build_insert_message_sql(
    conversation_id: &str,
    thread_id: &str,
    turn_id: Option<&str>,
    role: &str,
    message_type: &str,
    content: &str,
    payload_json: Option<&str>,
    created_at_ms: u64,
) -> String {
    let conversation_id_sql = sql_string(conversation_id);
    let thread_id_sql = sql_string(thread_id);
    let turn_id_sql = sql_option_string(turn_id);
    let role_sql = sql_string(role);
    let message_type_sql = sql_string(message_type);
    let content_sql = sql_string(content);
    let payload_json_sql = sql_option_string(payload_json);
    let created_at_ms_sql = sql_u64(created_at_ms);
    format!(
        "INSERT INTO conversation_message \
        (conversation_id, thread_id, turn_id, role, message_type, content, payload_json, sequence_no, created_at_ms) \
        SELECT {conversation_id_sql}, {thread_id_sql}, {turn_id_sql}, {role_sql}, {message_type_sql}, {content_sql}, {payload_json_sql}, {created_at_ms_sql}, {created_at_ms_sql} \
        FROM DUAL WHERE NOT EXISTS (\
            SELECT 1 FROM conversation_message WHERE \
              conversation_id = {conversation_id_sql} AND \
              thread_id = {thread_id_sql} AND \
              {turn_id_match} AND \
              role = {role_sql} AND \
              message_type = {message_type_sql} AND \
              content = {content_sql} AND \
              {payload_json_match}\
        )",
        turn_id_match = sql_null_safe_eq("turn_id", &turn_id_sql),
        payload_json_match = sql_null_safe_eq("payload_json", &payload_json_sql),
    )
}

fn build_insert_event_sql(
    conversation_id: &str,
    thread_id: &str,
    turn_id: Option<&str>,
    event_type: &str,
    event_status: Option<&str>,
    payload_json: &str,
    created_at_ms: u64,
) -> String {
    let conversation_id_sql = sql_string(conversation_id);
    let thread_id_sql = sql_string(thread_id);
    let turn_id_sql = sql_option_string(turn_id);
    let event_type_sql = sql_string(event_type);
    let event_status_sql = sql_option_string(event_status);
    let payload_json_sql = sql_string(payload_json);
    let created_at_ms_sql = sql_u64(created_at_ms);
    format!(
        "INSERT INTO conversation_event \
        (conversation_id, thread_id, turn_id, event_type, event_status, payload_json, created_at_ms) \
        SELECT {conversation_id_sql}, {thread_id_sql}, {turn_id_sql}, {event_type_sql}, {event_status_sql}, {payload_json_sql}, {created_at_ms_sql} \
        FROM DUAL WHERE NOT EXISTS (\
            SELECT 1 FROM conversation_event WHERE \
              conversation_id = {conversation_id_sql} AND \
              thread_id = {thread_id_sql} AND \
              {turn_id_match} AND \
              event_type = {event_type_sql} AND \
              {event_status_match} AND \
              payload_json = {payload_json_sql}\
        )",
        turn_id_match = sql_null_safe_eq("turn_id", &turn_id_sql),
        event_status_match = sql_null_safe_eq("event_status", &event_status_sql),
    )
}

fn sql_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\0' => escaped.push_str("\\0"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\u{001A}' => escaped.push_str("\\Z"),
            '\'' => escaped.push_str("\\'"),
            '\\' => escaped.push_str("\\\\"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn resolve_conversation_id_from_records(
    conversations: &[ServiceConversationRecord],
    workspace_id: &str,
    thread_id: &str,
) -> Option<String> {
    conversations
        .iter()
        .filter(|conversation| {
            conversation.workspace_id == workspace_id && conversation.thread_id == thread_id
        })
        .max_by_key(|conversation| conversation.updated_at_ms)
        .map(|conversation| conversation.conversation_id.clone())
}

impl DaemonState {
    fn load(config: &DaemonConfig, event_sink: DaemonEventSink) -> Self {
        let storage_path = config.data_dir.join("workspaces.json");
        let settings_path = config.data_dir.join("settings.json");
        let tasks_path = config.data_dir.join("service_tasks.json");
        let conversations_path = config.data_dir.join("service_conversations.json");
        let mysql_retry_path = config.data_dir.join("service_mysql_retry.json");
        let workspaces = read_workspaces(&storage_path).unwrap_or_default();
        let app_settings = read_settings(&settings_path).unwrap_or_default();
        let tasks = load_service_tasks(&tasks_path);
        let conversations = load_service_conversations(&conversations_path);
        let mysql_retry_queue = load_pending_mysql_writes(&mysql_retry_path);
        let mysql_history = MySqlHistoryWriter::from_env();
        let daemon_binary_path = std::env::current_exe()
            .ok()
            .and_then(|path| path.to_str().map(str::to_string));
        Self {
            data_dir: config.data_dir.clone(),
            workspaces: Mutex::new(workspaces),
            sessions: Mutex::new(HashMap::new()),
            tasks: Mutex::new(tasks),
            conversations: Mutex::new(conversations),
            tasks_path,
            conversations_path,
            mysql_retry_path,
            storage_path,
            settings_path,
            app_settings: Mutex::new(app_settings),
            mysql_history,
            mysql_retry_queue: Mutex::new(mysql_retry_queue),
            mysql_retry_flush_lock: Mutex::new(()),
            event_sink,
            codex_login_cancels: Mutex::new(HashMap::new()),
            daemon_binary_path,
        }
    }

    fn daemon_info(&self) -> Value {
        json!({
            "name": DAEMON_NAME,
            "version": env!("CARGO_PKG_VERSION"),
            "pid": std::process::id(),
            "mode": "tcp",
            "binaryPath": self.daemon_binary_path,
            "mysqlRetry": self.mysql_retry_status_snapshot(),
        })
    }

    fn mysql_retry_status_snapshot(&self) -> Value {
        let queue = self
            .mysql_retry_queue
            .try_lock()
            .map(|queue| queue.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let pending = queue.len();
        let max_attempt_count = queue.iter().map(|entry| entry.attempt_count).max().unwrap_or(0);
        let oldest_created_at_ms = queue.iter().map(|entry| entry.created_at_ms).min();
        let latest_error = queue
            .iter()
            .max_by_key(|entry| entry.updated_at_ms)
            .and_then(|entry| entry.last_error.clone());
        json!({
            "enabled": self.mysql_history.is_some(),
            "pending": pending,
            "maxAttemptCount": max_attempt_count,
            "oldestCreatedAtMs": oldest_created_at_ms,
            "latestError": latest_error,
        })
    }

    async fn insert_task(&self, task: ServiceTaskRecord) -> ServiceTaskRecord {
        {
            let mut tasks = self.tasks.lock().await;
            tasks.insert(task.task_id.clone(), task.clone());
            self.persist_tasks_locked(&tasks);
        }
        self.persist_task_history(&task).await;
        let _ = self.event_sink.tx.send(DaemonEvent::TaskUpdated(task.clone()));
        task
    }

    async fn insert_conversation(
        &self,
        conversation: ServiceConversationRecord,
    ) -> ServiceConversationRecord {
        {
            let mut conversations = self.conversations.lock().await;
            conversations.insert(
                conversation.conversation_id.clone(),
                conversation.clone(),
            );
            self.persist_conversations_locked(&conversations);
        }
        self.persist_conversation_history(&conversation).await;
        let _ = self
            .event_sink
            .tx
            .send(DaemonEvent::ConversationUpdated(conversation.clone()));
        conversation
    }

    async fn get_task(&self, task_id: &str) -> Option<ServiceTaskRecord> {
        self.tasks.lock().await.get(task_id).cloned()
    }

    async fn get_conversation(
        &self,
        conversation_id: &str,
    ) -> Option<ServiceConversationRecord> {
        self.conversations.lock().await.get(conversation_id).cloned()
    }

    async fn list_conversations(&self, workspace_id: Option<&str>) -> Vec<ServiceConversationRecord> {
        let mut items = self
            .conversations
            .lock()
            .await
            .values()
            .filter(|conversation| {
                workspace_id
                    .map(|workspace_id| conversation.workspace_id == workspace_id)
                    .unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<_>>();
        items.sort_by(|a, b| {
            b.updated_at_ms
                .cmp(&a.updated_at_ms)
                .then_with(|| a.conversation_id.cmp(&b.conversation_id))
        });
        items
    }

    async fn read_conversation_messages(&self, conversation_id: &str) -> Result<Value, String> {
        let conversation = self
            .get_conversation(conversation_id)
            .await
            .ok_or_else(|| "conversation not found".to_string())?;
        let messages = self
            .read_thread(
                conversation.workspace_id.clone(),
                conversation.thread_id.clone(),
            )
            .await?;
        Ok(json!({
            "conversation": conversation,
            "messages": normalize_thread_messages(messages),
        }))
    }

    async fn update_task<F>(&self, task_id: &str, update: F) -> Option<ServiceTaskRecord>
    where
        F: FnOnce(&mut ServiceTaskRecord),
    {
        let updated = {
            let mut tasks = self.tasks.lock().await;
            let task = tasks.get_mut(task_id)?;
            update(task);
            let updated = task.clone();
            self.persist_tasks_locked(&tasks);
            updated
        };
        self.persist_task_history(&updated).await;
        let _ = self
            .event_sink
            .tx
            .send(DaemonEvent::TaskUpdated(updated.clone()));
        Some(updated)
    }

    async fn update_conversation<F>(
        &self,
        conversation_id: &str,
        update: F,
    ) -> Option<ServiceConversationRecord>
    where
        F: FnOnce(&mut ServiceConversationRecord),
    {
        let updated = {
            let mut conversations = self.conversations.lock().await;
            let conversation = conversations.get_mut(conversation_id)?;
            update(conversation);
            conversation.updated_at_ms = current_timestamp_ms();
            let updated = conversation.clone();
            self.persist_conversations_locked(&conversations);
            updated
        };
        self.persist_conversation_history(&updated).await;
        let _ = self
            .event_sink
            .tx
            .send(DaemonEvent::ConversationUpdated(updated.clone()));
        Some(updated)
    }

    fn persist_tasks_locked(&self, tasks: &HashMap<String, ServiceTaskRecord>) {
        if let Err(err) = write_service_tasks(&self.tasks_path, tasks) {
            eprintln!(
                "daemon: failed to persist tasks to {}: {err}",
                self.tasks_path.display()
            );
        }
    }

    fn persist_conversations_locked(
        &self,
        conversations: &HashMap<String, ServiceConversationRecord>,
    ) {
        if let Err(err) = write_service_conversations(&self.conversations_path, conversations) {
            eprintln!(
                "daemon: failed to persist conversations to {}: {err}",
                self.conversations_path.display()
            );
        }
    }

    fn persist_mysql_retry_queue_locked(
        &self,
        queue: &HashMap<String, PendingMySqlWrite>,
    ) {
        if let Err(err) = write_pending_mysql_writes(&self.mysql_retry_path, queue) {
            eprintln!(
                "daemon: failed to persist mysql retry queue to {}: {err}",
                self.mysql_retry_path.display()
            );
        }
    }

    async fn enqueue_mysql_retry(&self, key: String, kind: &str, sql: String, error_message: String) {
        let now = current_timestamp_ms();
        let mut queue = self.mysql_retry_queue.lock().await;
        let entry = queue.entry(key.clone()).or_insert_with(|| PendingMySqlWrite {
            key: key.clone(),
            kind: kind.to_string(),
            sql: sql.clone(),
            created_at_ms: now,
            updated_at_ms: now,
            attempt_count: 0,
            last_error: None,
        });
        entry.kind = kind.to_string();
        entry.sql = sql;
        entry.updated_at_ms = now;
        entry.attempt_count = entry.attempt_count.saturating_add(1);
        entry.last_error = Some(error_message);
        self.persist_mysql_retry_queue_locked(&queue);
    }

    async fn clear_mysql_retry(&self, key: &str) {
        let mut queue = self.mysql_retry_queue.lock().await;
        if queue.remove(key).is_some() {
            self.persist_mysql_retry_queue_locked(&queue);
        }
    }

    async fn flush_mysql_retry_queue(&self) {
        let Some(writer) = self.mysql_history.clone() else {
            return;
        };
        let _flush_guard = self.mysql_retry_flush_lock.lock().await;
        let pending = self
            .mysql_retry_queue
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return;
        }

        let mut recovered = 0usize;
        for entry in pending {
            match writer.exec_sql(&entry.sql).await {
                Ok(()) => {
                    recovered += 1;
                    self.clear_mysql_retry(&entry.key).await;
                }
                Err(err) => {
                    let mut queue = self.mysql_retry_queue.lock().await;
                    if let Some(pending_entry) = queue.get_mut(&entry.key) {
                        pending_entry.updated_at_ms = current_timestamp_ms();
                        pending_entry.attempt_count = pending_entry.attempt_count.saturating_add(1);
                        pending_entry.last_error = Some(err.clone());
                    }
                    self.persist_mysql_retry_queue_locked(&queue);
                    eprintln!(
                        "daemon: mysql retry still failing for {} {}: {err}",
                        entry.kind, entry.key
                    );
                }
            }
        }
        if recovered > 0 {
            let remaining = self.mysql_retry_queue.lock().await.len();
            eprintln!(
                "daemon: recovered {recovered} pending mysql writes (remaining: {remaining})"
            );
        }
    }

    async fn run_mysql_retry_worker(self: Arc<Self>) {
        if self.mysql_history.is_none() {
            return;
        }
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            self.flush_mysql_retry_queue().await;
            let snapshot = self.mysql_retry_status_snapshot();
            let pending = snapshot
                .get("pending")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if pending > 0 {
                let max_attempt_count = snapshot
                    .get("maxAttemptCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let latest_error = snapshot
                    .get("latestError")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown mysql error");
                eprintln!(
                    "daemon: mysql retry backlog pending={} max_attempts={} latest_error={}",
                    pending, max_attempt_count, latest_error
                );
            }
        }
    }

    async fn persist_conversation_history(&self, conversation: &ServiceConversationRecord) {
        let Some(writer) = self.mysql_history.clone() else {
            return;
        };
        let retry_key = format!("conversation:{}", conversation.conversation_id);
        let sql = build_upsert_conversation_sql(conversation, &conversation.requirement);
        if let Err(err) = writer.exec_sql(&sql).await {
            eprintln!(
                "daemon: failed to persist conversation {} to mysql: {err}",
                conversation.conversation_id
            );
            self.enqueue_mysql_retry(retry_key, "conversation", sql, err).await;
            return;
        }
        self.clear_mysql_retry(&retry_key).await;
        self.flush_mysql_retry_queue().await;
    }

    async fn persist_task_history(&self, task: &ServiceTaskRecord) {
        let Some(writer) = self.mysql_history.clone() else {
            return;
        };
        let conversation_id = self
            .resolve_conversation_id(&task.workspace_id, &task.thread_id)
            .await;
        let Some(conversation_id) = conversation_id else {
            return;
        };
        let retry_key = format!("task:{}", task.task_id);
        let sql = build_upsert_task_sql(&conversation_id, task);
        if let Err(err) = writer.exec_sql(&sql).await {
            eprintln!(
                "daemon: failed to persist task {} to mysql: {err}",
                task.task_id
            );
            self.enqueue_mysql_retry(retry_key, "task", sql, err).await;
            return;
        }
        self.clear_mysql_retry(&retry_key).await;
        self.flush_mysql_retry_queue().await;
    }

    async fn persist_message_history(
        &self,
        conversation_id: &str,
        thread_id: &str,
        turn_id: Option<&str>,
        role: &str,
        message_type: &str,
        content: &str,
        payload_json: Option<&str>,
        created_at_ms: u64,
    ) {
        let Some(writer) = self.mysql_history.clone() else {
            return;
        };
        let sql = build_insert_message_sql(
            conversation_id,
            thread_id,
            turn_id,
            role,
            message_type,
            content,
            payload_json,
            created_at_ms,
        );
        let retry_key = format!("message:{:016x}", stable_hash(&sql));
        if let Err(err) = writer.exec_sql(&sql).await {
            eprintln!(
                "daemon: failed to persist conversation message {} to mysql: {err}",
                conversation_id
            );
            self.enqueue_mysql_retry(retry_key, "message", sql, err).await;
            return;
        }
        self.clear_mysql_retry(&retry_key).await;
        self.flush_mysql_retry_queue().await;
    }

    async fn persist_event_history(
        &self,
        conversation_id: &str,
        thread_id: &str,
        turn_id: Option<&str>,
        event_type: &str,
        event_status: Option<&str>,
        payload_json: &str,
        created_at_ms: u64,
    ) {
        let Some(writer) = self.mysql_history.clone() else {
            return;
        };
        let sql = build_insert_event_sql(
            conversation_id,
            thread_id,
            turn_id,
            event_type,
            event_status,
            payload_json,
            created_at_ms,
        );
        let retry_key = format!("event:{:016x}", stable_hash(&sql));
        if let Err(err) = writer.exec_sql(&sql).await {
            eprintln!(
                "daemon: failed to persist conversation event {} to mysql: {err}",
                conversation_id
            );
            self.enqueue_mysql_retry(retry_key, "event", sql, err).await;
            return;
        }
        self.clear_mysql_retry(&retry_key).await;
        self.flush_mysql_retry_queue().await;
    }

    async fn resolve_conversation_id(
        &self,
        workspace_id: &str,
        thread_id: &str,
    ) -> Option<String> {
        let conversations = self
            .conversations
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        resolve_conversation_id_from_records(&conversations, workspace_id, thread_id)
    }

    async fn reconcile_history_projection(&self) {
        let Some(writer) = self.mysql_history.clone() else {
            return;
        };
        let conversations = self
            .conversations
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let tasks = self
            .tasks
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();

        let mut conversation_ok = 0usize;
        let mut task_ok = 0usize;
        for conversation in &conversations {
            if let Err(err) = writer
                .upsert_conversation(conversation, &conversation.requirement)
                .await
            {
                eprintln!(
                    "daemon: failed to reconcile conversation {} to mysql: {err}",
                    conversation.conversation_id
                );
            } else {
                conversation_ok += 1;
            }
        }
        for task in &tasks {
            let Some(conversation_id) =
                resolve_conversation_id_from_records(&conversations, &task.workspace_id, &task.thread_id)
            else {
                continue;
            };
            if let Err(err) = writer.upsert_task(&conversation_id, task).await {
                eprintln!(
                    "daemon: failed to reconcile task {} to mysql: {err}",
                    task.task_id
                );
            } else {
                task_ok += 1;
            }
        }
        eprintln!(
            "daemon: reconciled history projection to mysql (conversations: {conversation_ok}/{}, tasks: {task_ok}/{})",
            conversations.len(),
            tasks.len()
        );
    }

    async fn backfill_history_to_mysql(
        &self,
        workspace_id: Option<&str>,
        conversation_id: Option<&str>,
    ) -> Result<HistoryBackfillReport, String> {
        if self.mysql_history.is_none() {
            return Err("mysql history is not enabled".to_string());
        }

        let conversations = self
            .conversations
            .lock()
            .await
            .values()
            .filter(|conversation| {
                workspace_id
                    .map(|value| conversation.workspace_id == value)
                    .unwrap_or(true)
                    && conversation_id
                        .map(|value| conversation.conversation_id == value)
                        .unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<_>>();
        let tasks = self
            .tasks
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();

        let mut report = HistoryBackfillReport {
            conversations_selected: conversations.len(),
            ..HistoryBackfillReport::default()
        };

        for conversation in &conversations {
            self.persist_conversation_history(conversation).await;
            report.conversations_upserted += 1;
        }

        for task in tasks.iter().filter(|task| {
            conversations.iter().any(|conversation| {
                conversation.workspace_id == task.workspace_id
                    && conversation.thread_id == task.thread_id
            })
        }) {
            self.persist_task_history(task).await;
            report.tasks_upserted += 1;
        }

        for conversation in conversations {
            if ensure_workspace_connected_for_http(self, &conversation.workspace_id)
                .await
                .is_err()
            {
                report.conversations_failed += 1;
                continue;
            }

            let thread = match self
                .read_thread(
                    conversation.workspace_id.clone(),
                    conversation.thread_id.clone(),
                )
                .await
            {
                Ok(thread) => thread,
                Err(_) => {
                    report.conversations_failed += 1;
                    continue;
                }
            };

            let messages = collect_history_backfill_messages(thread);
            if messages.is_empty() {
                continue;
            }

            for message in messages {
                let sql = build_insert_message_sql(
                    &conversation.conversation_id,
                    &conversation.thread_id,
                    message.turn_id.as_deref(),
                    &message.role,
                    &message.message_type,
                    &message.content,
                    message.payload_json.as_deref(),
                    message.created_at_ms,
                );
                let retry_key = format!("message:{:016x}", stable_hash(&sql));
                self.persist_message_history(
                    &conversation.conversation_id,
                    &conversation.thread_id,
                    message.turn_id.as_deref(),
                    &message.role,
                    &message.message_type,
                    &message.content,
                    message.payload_json.as_deref(),
                    message.created_at_ms,
                )
                .await;
                let failed = self
                    .mysql_retry_queue
                    .lock()
                    .await
                    .contains_key(&retry_key);
                if failed {
                    report.message_failures += 1;
                } else {
                    report.messages_backfilled += 1;
                }
            }
        }

        Ok(report)
    }

    async fn mark_task_running(
        &self,
        task_id: &str,
        turn_id: Option<String>,
    ) -> Option<ServiceTaskRecord> {
        self.update_task(task_id, move |task| {
            if let Some(turn_id) = turn_id {
                task.turn_id = Some(turn_id);
            }
            if task.status == "accepted" {
                task.status = "running".to_string();
            }
        })
        .await
    }

    async fn mark_task_completed(&self, task_id: &str) -> Option<ServiceTaskRecord> {
        self.update_task(task_id, |task| {
            if !is_terminal_task_status(&task.status) {
                task.status = "completed".to_string();
                task.completed_at_ms = Some(current_timestamp_ms());
                task.last_error = None;
            }
        })
        .await
    }

    async fn mark_task_failed(
        &self,
        task_id: &str,
        error_message: Option<String>,
    ) -> Option<ServiceTaskRecord> {
        self.update_task(task_id, move |task| {
            if !is_terminal_task_status(&task.status) {
                task.status = "failed".to_string();
                task.completed_at_ms = Some(current_timestamp_ms());
            }
            if let Some(message) = error_message {
                task.last_error = Some(message);
            }
        })
        .await
    }

    async fn process_task_event(&self, event: &AppServerEvent) {
        let Some(method) = event.message.get("method").and_then(Value::as_str) else {
            return;
        };
        self.process_conversation_event(event).await;
        let turn_id = extract_task_turn_id(&event.message);
        let thread_id = extract_task_thread_id(&event.message);
        let task_id = {
            let tasks = self.tasks.lock().await;
            find_matching_task_id(
                &tasks,
                &event.workspace_id,
                thread_id.as_deref(),
                turn_id.as_deref(),
            )
        };
        let Some(task_id) = task_id else {
            return;
        };

        match method {
            "turn/started" => {
                let _ = self.mark_task_running(&task_id, turn_id).await;
            }
            "turn/completed" => {
                let _ = self.mark_task_completed(&task_id).await;
            }
            "error" | "turn/error" => {
                let _ = self
                    .mark_task_failed(&task_id, extract_task_error_message(&event.message))
                    .await;
            }
            _ => {}
        }
    }

    async fn process_conversation_event(&self, event: &AppServerEvent) {
        let Some(method) = event.message.get("method").and_then(Value::as_str) else {
            return;
        };
        let Some(thread_id) = extract_task_thread_id(&event.message) else {
            return;
        };
        let conversation_id = {
            let conversations = self.conversations.lock().await;
            conversations
                .values()
                .filter(|conversation| {
                    conversation.workspace_id == event.workspace_id
                        && conversation.thread_id == thread_id
                })
                .max_by_key(|conversation| conversation.updated_at_ms)
                .map(|conversation| conversation.conversation_id.clone())
        };
        let Some(conversation_id) = conversation_id else {
            return;
        };

        let next_status = map_conversation_status(method, &event.message);
        let final_summary = extract_conversation_summary(&event.message);
        let assistant_text = extract_assistant_message_text(&event.message);
        let last_error = extract_task_error_message(&event.message);
        let closure_next_status = next_status.clone();
        let closure_final_summary = final_summary.clone();
        let closure_assistant_text = assistant_text.clone();
        let closure_last_error = last_error.clone();
        let turn_id = extract_task_turn_id(&event.message);
        let event_status = next_status.clone();
        let event_payload_json = serde_json::to_string(&event.message).ok();
        let event_created_at_ms = current_timestamp_ms();
        let _ = self
            .update_conversation(&conversation_id, move |conversation| {
                if let Some(status) = closure_next_status.clone() {
                    conversation.status = status;
                }
                if let Some(summary) = closure_final_summary.clone() {
                    conversation.final_summary = Some(summary);
                } else if let Some(text) = closure_assistant_text.clone() {
                    conversation.final_summary = summary_text(&text);
                }
                if let Some(text) = closure_assistant_text.clone() {
                    conversation.last_message_preview = preview_text(&text);
                }
                if let Some(error) = closure_last_error.clone() {
                    conversation.last_error = Some(error);
                } else if method == "turn/completed" {
                    conversation.last_error = None;
                }
            })
            .await;
        if let Some(text) = assistant_text.as_deref() {
            self.persist_message_history(
                &conversation_id,
                &thread_id,
                turn_id.as_deref(),
                "assistant",
                method,
                text,
                event_payload_json.as_deref(),
                event_created_at_ms,
            )
            .await;
        }
        if event_status.is_some() || final_summary.is_some() || last_error.is_some() || assistant_text.is_some() {
            self.persist_event_history(
                &conversation_id,
                &thread_id,
                turn_id.as_deref(),
                method,
                event_status.as_deref(),
                event_payload_json.as_deref().unwrap_or("{}"),
                event_created_at_ms,
            )
            .await;
        }
    }

    async fn sync_workspaces_from_storage(&self) {
        let stored = match read_workspaces(&self.storage_path) {
            Ok(stored) => stored,
            Err(err) => {
                eprintln!(
                    "daemon: failed to read workspaces from {}: {err}",
                    self.storage_path.display()
                );
                return;
            }
        };
        let workspace_ids: HashSet<String> = stored.keys().cloned().collect();
        {
            let mut workspaces = self.workspaces.lock().await;
            *workspaces = stored;
        }

        let stale_sessions: Vec<(String, Arc<WorkspaceSession>)> = {
            let mut sessions = self.sessions.lock().await;
            sessions
                .keys()
                .filter(|id| !workspace_ids.contains(*id))
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .filter_map(|workspace_id| {
                    sessions
                        .remove(&workspace_id)
                        .map(|session| (workspace_id, session))
                })
                .collect()
        };

        for (workspace_id, session) in stale_sessions {
            let mut child = session.child.lock().await;
            kill_child_process_tree(&mut child).await;
            eprintln!("daemon: pruned stale session for removed workspace {workspace_id}");
        }
    }

    async fn list_workspaces(&self) -> Vec<WorkspaceInfo> {
        self.sync_workspaces_from_storage().await;
        workspaces_core::list_workspaces_core(&self.workspaces, &self.sessions).await
    }

    async fn is_workspace_path_dir(&self, path: String) -> bool {
        workspaces_core::is_workspace_path_dir_core(&path)
    }

    async fn add_workspace(
        &self,
        path: String,
        client_version: String,
    ) -> Result<WorkspaceInfo, String> {
        let client_version = client_version.clone();
        workspaces_core::add_workspace_core(
            path,
            &self.workspaces,
            &self.sessions,
            &self.app_settings,
            &self.storage_path,
            move |entry, default_bin, codex_args, codex_home| {
                spawn_with_client(
                    self.event_sink.clone(),
                    client_version.clone(),
                    entry,
                    default_bin,
                    codex_args,
                    codex_home,
                )
            },
        )
        .await
    }

    async fn add_workspace_from_git_url(
        &self,
        url: String,
        destination_path: String,
        target_folder_name: Option<String>,
        client_version: String,
    ) -> Result<WorkspaceInfo, String> {
        let client_version = client_version.clone();
        workspaces_core::add_workspace_from_git_url_core(
            url,
            destination_path,
            target_folder_name,
            &self.workspaces,
            &self.sessions,
            &self.app_settings,
            &self.storage_path,
            move |entry, default_bin, codex_args, codex_home| {
                spawn_with_client(
                    self.event_sink.clone(),
                    client_version.clone(),
                    entry,
                    default_bin,
                    codex_args,
                    codex_home,
                )
            },
        )
        .await
    }

    async fn add_worktree(
        &self,
        parent_id: String,
        branch: String,
        name: Option<String>,
        copy_agents_md: bool,
        client_version: String,
    ) -> Result<WorkspaceInfo, String> {
        let client_version = client_version.clone();
        workspaces_core::add_worktree_core(
            parent_id,
            branch,
            name,
            copy_agents_md,
            &self.data_dir,
            &self.workspaces,
            &self.sessions,
            &self.app_settings,
            &self.storage_path,
            |value| worktree_core::sanitize_worktree_name(value),
            |root, name| worktree_core::unique_worktree_path_strict(root, name),
            |root, branch_name| {
                let root = root.clone();
                let branch_name = branch_name.to_string();
                async move { git_core::git_branch_exists(&root, &branch_name).await }
            },
            Some(|root: &PathBuf, branch_name: &str| {
                let root = root.clone();
                let branch_name = branch_name.to_string();
                async move { git_core::git_find_remote_tracking_branch_local(&root, &branch_name).await }
            }),
            |root, args| {
                workspaces_core::run_git_command_unit(root, args, git_core::run_git_command_owned)
            },
            move |entry, default_bin, codex_args, codex_home| {
                spawn_with_client(
                    self.event_sink.clone(),
                    client_version.clone(),
                    entry,
                    default_bin,
                    codex_args,
                    codex_home,
                )
            },
        )
        .await
    }

    async fn worktree_setup_status(
        &self,
        workspace_id: String,
    ) -> Result<WorktreeSetupStatus, String> {
        workspaces_core::worktree_setup_status_core(&self.workspaces, &workspace_id, &self.data_dir)
            .await
    }

    async fn worktree_setup_mark_ran(&self, workspace_id: String) -> Result<(), String> {
        workspaces_core::worktree_setup_mark_ran_core(
            &self.workspaces,
            &workspace_id,
            &self.data_dir,
        )
        .await
    }

    async fn remove_workspace(&self, id: String) -> Result<(), String> {
        workspaces_core::remove_workspace_core(
            id,
            &self.workspaces,
            &self.sessions,
            &self.storage_path,
            |root, args| {
                workspaces_core::run_git_command_unit(root, args, git_core::run_git_command_owned)
            },
            |error| git_core::is_missing_worktree_error(error),
            |path| {
                std::fs::remove_dir_all(path)
                    .map_err(|err| format!("Failed to remove worktree folder: {err}"))
            },
            true,
            true,
        )
        .await
    }

    async fn remove_worktree(&self, id: String) -> Result<(), String> {
        workspaces_core::remove_worktree_core(
            id,
            &self.workspaces,
            &self.sessions,
            &self.storage_path,
            |root, args| {
                workspaces_core::run_git_command_unit(root, args, git_core::run_git_command_owned)
            },
            |error| git_core::is_missing_worktree_error(error),
            |path| {
                std::fs::remove_dir_all(path)
                    .map_err(|err| format!("Failed to remove worktree folder: {err}"))
            },
        )
        .await
    }

    async fn rename_worktree(
        &self,
        id: String,
        branch: String,
        client_version: String,
    ) -> Result<WorkspaceInfo, String> {
        let client_version = client_version.clone();
        workspaces_core::rename_worktree_core(
            id,
            branch,
            &self.data_dir,
            &self.workspaces,
            &self.sessions,
            &self.app_settings,
            &self.storage_path,
            |entry| Ok(PathBuf::from(entry.path.clone())),
            |root, name| {
                let root = root.clone();
                let name = name.to_string();
                async move {
                    git_core::unique_branch_name_live(&root, &name, None)
                        .await
                        .map(|(branch_name, _was_suffixed)| branch_name)
                }
            },
            |value| worktree_core::sanitize_worktree_name(value),
            |root, name, current| {
                worktree_core::unique_worktree_path_for_rename(root, name, current)
            },
            |root, args| {
                workspaces_core::run_git_command_unit(root, args, git_core::run_git_command_owned)
            },
            move |entry, default_bin, codex_args, codex_home| {
                spawn_with_client(
                    self.event_sink.clone(),
                    client_version.clone(),
                    entry,
                    default_bin,
                    codex_args,
                    codex_home,
                )
            },
        )
        .await
    }

    async fn rename_worktree_upstream(
        &self,
        id: String,
        old_branch: String,
        new_branch: String,
    ) -> Result<(), String> {
        workspaces_core::rename_worktree_upstream_core(
            id,
            old_branch,
            new_branch,
            &self.workspaces,
            |entry| Ok(PathBuf::from(entry.path.clone())),
            |root, branch_name| {
                let root = root.clone();
                let branch_name = branch_name.to_string();
                async move { git_core::git_branch_exists(&root, &branch_name).await }
            },
            |root, branch_name| {
                let root = root.clone();
                let branch_name = branch_name.to_string();
                async move { git_core::git_find_remote_for_branch_live(&root, &branch_name).await }
            },
            |root, remote| {
                let root = root.clone();
                let remote = remote.to_string();
                async move { git_core::git_remote_exists(&root, &remote).await }
            },
            |root, remote, branch_name| {
                let root = root.clone();
                let remote = remote.to_string();
                let branch_name = branch_name.to_string();
                async move {
                    git_core::git_remote_branch_exists_live(&root, &remote, &branch_name).await
                }
            },
            |root, args| {
                workspaces_core::run_git_command_unit(root, args, git_core::run_git_command_owned)
            },
        )
        .await
    }

    async fn update_workspace_settings(
        &self,
        id: String,
        settings: WorkspaceSettings,
        client_version: String,
    ) -> Result<WorkspaceInfo, String> {
        let client_version = client_version.clone();
        workspaces_core::update_workspace_settings_core(
            id,
            settings,
            &self.workspaces,
            &self.sessions,
            &self.app_settings,
            &self.storage_path,
            |workspaces, workspace_id, next_settings| {
                apply_workspace_settings_update(workspaces, workspace_id, next_settings)
            },
            move |entry, default_bin, codex_args, codex_home| {
                spawn_with_client(
                    self.event_sink.clone(),
                    client_version.clone(),
                    entry,
                    default_bin,
                    codex_args,
                    codex_home,
                )
            },
        )
        .await
    }

    async fn connect_workspace(&self, id: String, client_version: String) -> Result<(), String> {
        {
            let sessions = self.sessions.lock().await;
            if sessions.contains_key(&id) {
                return Ok(());
            }
        }

        let client_version = client_version.clone();
        workspaces_core::connect_workspace_core(
            id,
            &self.workspaces,
            &self.sessions,
            &self.app_settings,
            move |entry, default_bin, codex_args, codex_home| {
                spawn_with_client(
                    self.event_sink.clone(),
                    client_version.clone(),
                    entry,
                    default_bin,
                    codex_args,
                    codex_home,
                )
            },
        )
        .await
    }

    async fn set_workspace_runtime_codex_args(
        &self,
        workspace_id: String,
        codex_args: Option<String>,
        client_version: String,
    ) -> Result<workspaces_core::WorkspaceRuntimeCodexArgsResult, String> {
        workspaces_core::set_workspace_runtime_codex_args_core(
            workspace_id,
            codex_args,
            &self.workspaces,
            &self.sessions,
            &self.app_settings,
            move |entry, default_bin, next_args, codex_home| {
                spawn_with_client(
                    self.event_sink.clone(),
                    client_version.clone(),
                    entry,
                    default_bin,
                    next_args,
                    codex_home,
                )
            },
        )
        .await
    }

    async fn get_app_settings(&self) -> AppSettings {
        settings_core::get_app_settings_core(&self.app_settings).await
    }

    async fn update_app_settings(&self, settings: AppSettings) -> Result<AppSettings, String> {
        settings_core::update_app_settings_core(settings, &self.app_settings, &self.settings_path)
            .await
    }

    async fn set_codex_feature_flag(
        &self,
        feature_key: String,
        enabled: bool,
    ) -> Result<(), String> {
        codex_config::write_feature_enabled(feature_key.as_str(), enabled)
    }

    async fn get_agents_settings(&self) -> Result<agents_config_core::AgentsSettingsDto, String> {
        agents_config_core::get_agents_settings_core()
    }

    async fn set_agents_core_settings(
        &self,
        input: agents_config_core::SetAgentsCoreInput,
    ) -> Result<agents_config_core::AgentsSettingsDto, String> {
        agents_config_core::set_agents_core_settings_core(input)
    }

    async fn create_agent(
        &self,
        input: agents_config_core::CreateAgentInput,
    ) -> Result<agents_config_core::AgentsSettingsDto, String> {
        agents_config_core::create_agent_core(input)
    }

    async fn update_agent(
        &self,
        input: agents_config_core::UpdateAgentInput,
    ) -> Result<agents_config_core::AgentsSettingsDto, String> {
        agents_config_core::update_agent_core(input)
    }

    async fn delete_agent(
        &self,
        input: agents_config_core::DeleteAgentInput,
    ) -> Result<agents_config_core::AgentsSettingsDto, String> {
        agents_config_core::delete_agent_core(input)
    }

    async fn read_agent_config_toml(&self, agent_name: String) -> Result<String, String> {
        agents_config_core::read_agent_config_toml_core(agent_name.as_str())
    }

    async fn write_agent_config_toml(
        &self,
        agent_name: String,
        content: String,
    ) -> Result<(), String> {
        agents_config_core::write_agent_config_toml_core(agent_name.as_str(), content.as_str())
    }

    async fn list_workspace_files(&self, workspace_id: String) -> Result<Vec<String>, String> {
        workspaces_core::list_workspace_files_core(&self.workspaces, &workspace_id, |root| {
            list_workspace_files_inner(root, 20000)
        })
        .await
    }

    async fn read_workspace_file(
        &self,
        workspace_id: String,
        path: String,
    ) -> Result<WorkspaceFileResponse, String> {
        workspaces_core::read_workspace_file_core(
            &self.workspaces,
            &workspace_id,
            &path,
            |root, rel_path| read_workspace_file_inner(root, rel_path),
        )
        .await
    }

    async fn file_read(
        &self,
        scope: file_policy::FileScope,
        kind: file_policy::FileKind,
        workspace_id: Option<String>,
    ) -> Result<file_io::TextFileResponse, String> {
        files_core::file_read_core(&self.workspaces, scope, kind, workspace_id).await
    }

    async fn file_write(
        &self,
        scope: file_policy::FileScope,
        kind: file_policy::FileKind,
        workspace_id: Option<String>,
        content: String,
    ) -> Result<(), String> {
        files_core::file_write_core(&self.workspaces, scope, kind, workspace_id, content).await
    }

    async fn start_thread(&self, workspace_id: String) -> Result<Value, String> {
        codex_core::start_thread_core(&self.sessions, &self.workspaces, workspace_id).await
    }

    async fn resume_thread(
        &self,
        workspace_id: String,
        thread_id: String,
    ) -> Result<Value, String> {
        codex_core::resume_thread_core(&self.sessions, workspace_id, thread_id).await
    }

    async fn read_thread(
        &self,
        workspace_id: String,
        thread_id: String,
    ) -> Result<Value, String> {
        codex_core::read_thread_core(&self.sessions, workspace_id, thread_id).await
    }

    async fn thread_live_subscribe(
        &self,
        workspace_id: String,
        thread_id: String,
    ) -> Result<Value, String> {
        codex_core::thread_live_subscribe_core(
            &self.sessions,
            workspace_id.clone(),
            thread_id.clone(),
        )
        .await?;
        let subscription_id = format!("{}:{}", workspace_id, thread_id);
        self.event_sink.emit_app_server_event(AppServerEvent {
            workspace_id: workspace_id.clone(),
            message: json!({
                "method": "thread/live_attached",
                "params": {
                    "workspaceId": workspace_id,
                    "threadId": thread_id,
                    "subscriptionId": subscription_id,
                }
            }),
        });
        Ok(json!({
            "subscriptionId": subscription_id,
            "state": "live",
        }))
    }

    async fn thread_live_unsubscribe(
        &self,
        workspace_id: String,
        thread_id: String,
    ) -> Result<Value, String> {
        codex_core::thread_live_unsubscribe_core(
            &self.sessions,
            workspace_id.clone(),
            thread_id.clone(),
        )
        .await?;
        self.event_sink.emit_app_server_event(AppServerEvent {
            workspace_id: workspace_id.clone(),
            message: json!({
                "method": "thread/live_detached",
                "params": {
                    "workspaceId": workspace_id,
                    "threadId": thread_id,
                    "reason": "manual",
                }
            }),
        });
        Ok(json!({ "ok": true }))
    }

    async fn fork_thread(&self, workspace_id: String, thread_id: String) -> Result<Value, String> {
        codex_core::fork_thread_core(&self.sessions, workspace_id, thread_id).await
    }

    async fn list_threads(
        &self,
        workspace_id: String,
        cursor: Option<String>,
        limit: Option<u32>,
        sort_key: Option<String>,
    ) -> Result<Value, String> {
        codex_core::list_threads_core(&self.sessions, workspace_id, cursor, limit, sort_key)
            .await
    }

    async fn list_mcp_server_status(
        &self,
        workspace_id: String,
        cursor: Option<String>,
        limit: Option<u32>,
    ) -> Result<Value, String> {
        codex_core::list_mcp_server_status_core(&self.sessions, workspace_id, cursor, limit).await
    }

    async fn archive_thread(
        &self,
        workspace_id: String,
        thread_id: String,
    ) -> Result<Value, String> {
        codex_core::archive_thread_core(&self.sessions, workspace_id, thread_id).await
    }

    async fn compact_thread(
        &self,
        workspace_id: String,
        thread_id: String,
    ) -> Result<Value, String> {
        codex_core::compact_thread_core(&self.sessions, workspace_id, thread_id).await
    }

    async fn set_thread_name(
        &self,
        workspace_id: String,
        thread_id: String,
        name: String,
    ) -> Result<Value, String> {
        codex_core::set_thread_name_core(&self.sessions, workspace_id, thread_id, name).await
    }

    async fn send_user_message(
        &self,
        workspace_id: String,
        thread_id: String,
        text: String,
        model: Option<String>,
        effort: Option<String>,
        service_tier: Option<Option<String>>,
        access_mode: Option<String>,
        images: Option<Vec<String>>,
        app_mentions: Option<Vec<Value>>,
        collaboration_mode: Option<Value>,
    ) -> Result<Value, String> {
        codex_core::send_user_message_core(
            &self.sessions,
            &self.workspaces,
            workspace_id,
            thread_id,
            text,
            model,
            effort,
            service_tier,
            access_mode,
            images,
            app_mentions,
            collaboration_mode,
        )
        .await
    }

    async fn turn_steer(
        &self,
        workspace_id: String,
        thread_id: String,
        turn_id: String,
        text: String,
        images: Option<Vec<String>>,
        app_mentions: Option<Vec<Value>>,
    ) -> Result<Value, String> {
        codex_core::turn_steer_core(
            &self.sessions,
            workspace_id,
            thread_id,
            turn_id,
            text,
            images,
            app_mentions,
        )
        .await
    }

    async fn turn_interrupt(
        &self,
        workspace_id: String,
        thread_id: String,
        turn_id: String,
    ) -> Result<Value, String> {
        codex_core::turn_interrupt_core(&self.sessions, workspace_id, thread_id, turn_id).await
    }

    async fn start_review(
        &self,
        workspace_id: String,
        thread_id: String,
        target: Value,
        delivery: Option<String>,
    ) -> Result<Value, String> {
        codex_core::start_review_core(&self.sessions, workspace_id, thread_id, target, delivery)
            .await
    }

    async fn model_list(&self, workspace_id: String) -> Result<Value, String> {
        codex_core::model_list_core(&self.sessions, workspace_id).await
    }

    async fn experimental_feature_list(
        &self,
        workspace_id: String,
        cursor: Option<String>,
        limit: Option<u32>,
    ) -> Result<Value, String> {
        codex_core::experimental_feature_list_core(&self.sessions, workspace_id, cursor, limit)
            .await
    }

    async fn collaboration_mode_list(&self, workspace_id: String) -> Result<Value, String> {
        codex_core::collaboration_mode_list_core(&self.sessions, workspace_id).await
    }

    async fn account_rate_limits(&self, workspace_id: String) -> Result<Value, String> {
        codex_core::account_rate_limits_core(&self.sessions, workspace_id).await
    }

    async fn account_read(&self, workspace_id: String) -> Result<Value, String> {
        codex_core::account_read_core(&self.sessions, &self.workspaces, workspace_id).await
    }

    async fn codex_login(&self, workspace_id: String) -> Result<Value, String> {
        codex_core::codex_login_core(&self.sessions, &self.codex_login_cancels, workspace_id).await
    }

    async fn codex_login_cancel(&self, workspace_id: String) -> Result<Value, String> {
        codex_core::codex_login_cancel_core(&self.sessions, &self.codex_login_cancels, workspace_id)
            .await
    }

    async fn skills_list(&self, workspace_id: String) -> Result<Value, String> {
        codex_core::skills_list_core(&self.sessions, &self.workspaces, workspace_id).await
    }

    async fn apps_list(
        &self,
        workspace_id: String,
        cursor: Option<String>,
        limit: Option<u32>,
        thread_id: Option<String>,
    ) -> Result<Value, String> {
        codex_core::apps_list_core(&self.sessions, workspace_id, cursor, limit, thread_id).await
    }

    async fn respond_to_server_request(
        &self,
        workspace_id: String,
        request_id: Value,
        result: Value,
    ) -> Result<Value, String> {
        codex_core::respond_to_server_request_core(
            &self.sessions,
            workspace_id,
            request_id,
            result,
        )
        .await?;
        Ok(json!({ "ok": true }))
    }

    async fn remember_approval_rule(
        &self,
        workspace_id: String,
        command: Vec<String>,
    ) -> Result<Value, String> {
        codex_core::remember_approval_rule_core(&self.workspaces, workspace_id, command).await
    }

    async fn get_config_model(&self, workspace_id: String) -> Result<Value, String> {
        codex_core::get_config_model_core(&self.workspaces, workspace_id).await
    }

    async fn add_clone(
        &self,
        source_workspace_id: String,
        copies_folder: String,
        copy_name: String,
        client_version: String,
    ) -> Result<WorkspaceInfo, String> {
        workspaces_core::add_clone_core(
            source_workspace_id,
            copy_name,
            copies_folder,
            &self.workspaces,
            &self.sessions,
            &self.app_settings,
            &self.storage_path,
            |entry, default_bin, codex_args, codex_home| {
                spawn_with_client(
                    self.event_sink.clone(),
                    client_version.clone(),
                    entry,
                    default_bin,
                    codex_args,
                    codex_home,
                )
            },
        )
        .await
    }

    async fn apply_worktree_changes(&self, workspace_id: String) -> Result<(), String> {
        workspaces_core::apply_worktree_changes_core(&self.workspaces, workspace_id).await
    }

    async fn open_workspace_in(
        &self,
        path: String,
        app: Option<String>,
        args: Vec<String>,
        command: Option<String>,
        line: Option<u32>,
        column: Option<u32>,
    ) -> Result<(), String> {
        workspaces_core::open_workspace_in_core(path, app, args, command, line, column).await
    }

    async fn get_open_app_icon(&self, app_name: String) -> Result<Option<String>, String> {
        #[cfg(target_os = "macos")]
        {
            return workspaces_core::get_open_app_icon_core(app_name, |name| {
                workspace_macos::get_open_app_icon_inner(name)
            })
            .await;
        }

        #[cfg(not(target_os = "macos"))]
        {
            workspaces_core::get_open_app_icon_core(app_name, |_name| None).await
        }
    }

    async fn get_git_status(&self, workspace_id: String) -> Result<Value, String> {
        git_ui_core::get_git_status_core(&self.workspaces, workspace_id).await
    }

    async fn init_git_repo(
        &self,
        workspace_id: String,
        branch: String,
        force: bool,
    ) -> Result<Value, String> {
        git_ui_core::init_git_repo_core(&self.workspaces, workspace_id, branch, force).await
    }

    async fn create_github_repo(
        &self,
        workspace_id: String,
        repo: String,
        visibility: String,
        branch: Option<String>,
    ) -> Result<Value, String> {
        git_ui_core::create_github_repo_core(
            &self.workspaces,
            workspace_id,
            repo,
            visibility,
            branch,
        )
        .await
    }

    async fn list_git_roots(
        &self,
        workspace_id: String,
        depth: Option<usize>,
    ) -> Result<Vec<String>, String> {
        git_ui_core::list_git_roots_core(&self.workspaces, workspace_id, depth).await
    }

    async fn get_git_diffs(&self, workspace_id: String) -> Result<Vec<GitFileDiff>, String> {
        git_ui_core::get_git_diffs_core(&self.workspaces, &self.app_settings, workspace_id).await
    }

    async fn get_git_log(
        &self,
        workspace_id: String,
        limit: Option<usize>,
    ) -> Result<GitLogResponse, String> {
        git_ui_core::get_git_log_core(&self.workspaces, workspace_id, limit).await
    }

    async fn get_git_commit_diff(
        &self,
        workspace_id: String,
        sha: String,
    ) -> Result<Vec<GitCommitDiff>, String> {
        git_ui_core::get_git_commit_diff_core(
            &self.workspaces,
            &self.app_settings,
            workspace_id,
            sha,
        )
        .await
    }

    async fn get_git_remote(&self, workspace_id: String) -> Result<Option<String>, String> {
        git_ui_core::get_git_remote_core(&self.workspaces, workspace_id).await
    }

    async fn stage_git_file(&self, workspace_id: String, path: String) -> Result<(), String> {
        git_ui_core::stage_git_file_core(&self.workspaces, workspace_id, path).await
    }

    async fn stage_git_all(&self, workspace_id: String) -> Result<(), String> {
        git_ui_core::stage_git_all_core(&self.workspaces, workspace_id).await
    }

    async fn unstage_git_file(&self, workspace_id: String, path: String) -> Result<(), String> {
        git_ui_core::unstage_git_file_core(&self.workspaces, workspace_id, path).await
    }

    async fn revert_git_file(&self, workspace_id: String, path: String) -> Result<(), String> {
        git_ui_core::revert_git_file_core(&self.workspaces, workspace_id, path).await
    }

    async fn revert_git_all(&self, workspace_id: String) -> Result<(), String> {
        git_ui_core::revert_git_all_core(&self.workspaces, workspace_id).await
    }

    async fn commit_git(&self, workspace_id: String, message: String) -> Result<(), String> {
        git_ui_core::commit_git_core(&self.workspaces, workspace_id, message).await
    }

    async fn push_git(&self, workspace_id: String) -> Result<(), String> {
        git_ui_core::push_git_core(&self.workspaces, workspace_id).await
    }

    async fn pull_git(&self, workspace_id: String) -> Result<(), String> {
        git_ui_core::pull_git_core(&self.workspaces, workspace_id).await
    }

    async fn fetch_git(&self, workspace_id: String) -> Result<(), String> {
        git_ui_core::fetch_git_core(&self.workspaces, workspace_id).await
    }

    async fn sync_git(&self, workspace_id: String) -> Result<(), String> {
        git_ui_core::sync_git_core(&self.workspaces, workspace_id).await
    }

    async fn get_github_issues(
        &self,
        workspace_id: String,
    ) -> Result<GitHubIssuesResponse, String> {
        git_ui_core::get_github_issues_core(&self.workspaces, workspace_id).await
    }

    async fn get_github_pull_requests(
        &self,
        workspace_id: String,
    ) -> Result<GitHubPullRequestsResponse, String> {
        git_ui_core::get_github_pull_requests_core(&self.workspaces, workspace_id).await
    }

    async fn get_github_pull_request_diff(
        &self,
        workspace_id: String,
        pr_number: u64,
    ) -> Result<Vec<GitHubPullRequestDiff>, String> {
        git_ui_core::get_github_pull_request_diff_core(&self.workspaces, workspace_id, pr_number)
            .await
    }

    async fn get_github_pull_request_comments(
        &self,
        workspace_id: String,
        pr_number: u64,
    ) -> Result<Vec<GitHubPullRequestComment>, String> {
        git_ui_core::get_github_pull_request_comments_core(
            &self.workspaces,
            workspace_id,
            pr_number,
        )
        .await
    }

    async fn checkout_github_pull_request(
        &self,
        workspace_id: String,
        pr_number: u64,
    ) -> Result<(), String> {
        git_ui_core::checkout_github_pull_request_core(&self.workspaces, workspace_id, pr_number)
            .await
    }

    async fn list_git_branches(&self, workspace_id: String) -> Result<Value, String> {
        git_ui_core::list_git_branches_core(&self.workspaces, workspace_id).await
    }

    async fn checkout_git_branch(&self, workspace_id: String, name: String) -> Result<(), String> {
        git_ui_core::checkout_git_branch_core(&self.workspaces, workspace_id, name).await
    }

    async fn create_git_branch(&self, workspace_id: String, name: String) -> Result<(), String> {
        git_ui_core::create_git_branch_core(&self.workspaces, workspace_id, name).await
    }

    async fn prompts_list(&self, workspace_id: String) -> Result<Vec<CustomPromptEntry>, String> {
        prompts_core::prompts_list_core(&self.workspaces, &self.settings_path, workspace_id).await
    }

    async fn prompts_workspace_dir(&self, workspace_id: String) -> Result<String, String> {
        prompts_core::prompts_workspace_dir_core(
            &self.workspaces,
            &self.settings_path,
            workspace_id,
        )
        .await
    }

    async fn prompts_global_dir(&self, workspace_id: String) -> Result<String, String> {
        prompts_core::prompts_global_dir_core(&self.workspaces, workspace_id).await
    }

    async fn prompts_create(
        &self,
        workspace_id: String,
        scope: String,
        name: String,
        description: Option<String>,
        argument_hint: Option<String>,
        content: String,
    ) -> Result<CustomPromptEntry, String> {
        prompts_core::prompts_create_core(
            &self.workspaces,
            &self.settings_path,
            workspace_id,
            scope,
            name,
            description,
            argument_hint,
            content,
        )
        .await
    }

    async fn prompts_update(
        &self,
        workspace_id: String,
        path: String,
        name: String,
        description: Option<String>,
        argument_hint: Option<String>,
        content: String,
    ) -> Result<CustomPromptEntry, String> {
        prompts_core::prompts_update_core(
            &self.workspaces,
            &self.settings_path,
            workspace_id,
            path,
            name,
            description,
            argument_hint,
            content,
        )
        .await
    }

    async fn prompts_delete(&self, workspace_id: String, path: String) -> Result<(), String> {
        prompts_core::prompts_delete_core(&self.workspaces, &self.settings_path, workspace_id, path)
            .await
    }

    async fn prompts_move(
        &self,
        workspace_id: String,
        path: String,
        scope: String,
    ) -> Result<CustomPromptEntry, String> {
        prompts_core::prompts_move_core(
            &self.workspaces,
            &self.settings_path,
            workspace_id,
            path,
            scope,
        )
        .await
    }

    async fn codex_doctor(
        &self,
        codex_bin: Option<String>,
        codex_args: Option<String>,
    ) -> Result<Value, String> {
        codex_aux_core::codex_doctor_core(&self.app_settings, codex_bin, codex_args).await
    }

    async fn generate_commit_message(
        &self,
        workspace_id: String,
        commit_message_model_id: Option<String>,
    ) -> Result<String, String> {
        let repo_root = git_ui_core::resolve_repo_root_for_workspace_core(
            &self.workspaces,
            workspace_id.clone(),
        )
        .await?;
        let diff = git_ui_core::collect_workspace_diff_core(&repo_root)?;
        let commit_message_prompt = {
            let settings = self.app_settings.lock().await;
            settings.commit_message_prompt.clone()
        };
        codex_aux_core::generate_commit_message_core(
            &self.sessions,
            &self.workspaces,
            workspace_id,
            &diff,
            &commit_message_prompt,
            commit_message_model_id.as_deref(),
            |workspace_id, thread_id| {
                emit_background_thread_hide(&self.event_sink, workspace_id, thread_id);
            },
        )
        .await
    }

    async fn generate_run_metadata(
        &self,
        workspace_id: String,
        prompt: String,
    ) -> Result<Value, String> {
        codex_aux_core::generate_run_metadata_core(
            &self.sessions,
            &self.workspaces,
            workspace_id,
            &prompt,
            |workspace_id, thread_id| {
                emit_background_thread_hide(&self.event_sink, workspace_id, thread_id);
            },
        )
        .await
    }

    async fn generate_agent_description(
        &self,
        workspace_id: String,
        description: String,
    ) -> Result<codex_aux_core::GeneratedAgentConfiguration, String> {
        codex_aux_core::generate_agent_description_core(
            &self.sessions,
            &self.workspaces,
            workspace_id,
            &description,
            |workspace_id, thread_id| {
                emit_background_thread_hide(&self.event_sink, workspace_id, thread_id);
            },
        )
        .await
    }

    async fn local_usage_snapshot(
        &self,
        days: Option<u32>,
        workspace_path: Option<String>,
    ) -> Result<LocalUsageSnapshot, String> {
        local_usage_core::local_usage_snapshot_core(&self.workspaces, days, workspace_path).await
    }

    async fn menu_set_accelerators(&self, _updates: Vec<Value>) -> Result<(), String> {
        // Daemon has no native menu runtime; treat as no-op for remote parity.
        Ok(())
    }

    async fn is_macos_debug_build(&self) -> bool {
        cfg!(all(target_os = "macos", debug_assertions))
    }

    async fn send_notification_fallback(&self, title: String, body: String) -> Result<(), String> {
        send_notification_fallback_inner(title, body)
    }
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "node_modules" | "dist" | "target" | "release-artifacts"
    )
}

fn normalize_git_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn emit_background_thread_hide(event_sink: &DaemonEventSink, workspace_id: &str, thread_id: &str) {
    event_sink.emit_app_server_event(AppServerEvent {
        workspace_id: workspace_id.to_string(),
        message: json!({
            "method": "codex/backgroundThread",
            "params": {
                "threadId": thread_id,
                "action": "hide"
            }
        }),
    });
}

fn send_notification_fallback_inner(title: String, body: String) -> Result<(), String> {
    #[cfg(all(target_os = "macos", debug_assertions))]
    {
        let escape = |value: &str| value.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            escape(&body),
            escape(&title)
        );

        let status = std::process::Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(script)
            .status()
            .map_err(|error| format!("Failed to run osascript: {error}"))?;

        if status.success() {
            return Ok(());
        }
        return Err(format!("osascript exited with status: {status}"));
    }

    #[cfg(not(all(target_os = "macos", debug_assertions)))]
    {
        let _ = (title, body);
        Err("Notification fallback is only available on macOS debug builds.".to_string())
    }
}

fn list_workspace_files_inner(root: &PathBuf, max_files: usize) -> Vec<String> {
    let mut results = Vec::new();
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .follow_links(false)
        .require_git(false)
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                let name = entry.file_name().to_string_lossy();
                return !should_skip_dir(&name);
            }
            true
        })
        .build();

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        if let Ok(rel_path) = entry.path().strip_prefix(root) {
            let normalized = normalize_git_path(&rel_path.to_string_lossy());
            if !normalized.is_empty() {
                results.push(normalized);
            }
        }
        if results.len() >= max_files {
            break;
        }
    }

    results.sort();
    results
}

const MAX_WORKSPACE_FILE_BYTES: u64 = 400_000;

fn read_workspace_file_inner(
    root: &PathBuf,
    relative_path: &str,
) -> Result<WorkspaceFileResponse, String> {
    let canonical_root = root
        .canonicalize()
        .map_err(|err| format!("Failed to resolve workspace root: {err}"))?;
    let candidate = canonical_root.join(relative_path);
    let canonical_path = candidate
        .canonicalize()
        .map_err(|err| format!("Failed to open file: {err}"))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err("Invalid file path".to_string());
    }
    let metadata = std::fs::metadata(&canonical_path)
        .map_err(|err| format!("Failed to read file metadata: {err}"))?;
    if !metadata.is_file() {
        return Err("Path is not a file".to_string());
    }

    let file = File::open(&canonical_path).map_err(|err| format!("Failed to open file: {err}"))?;
    let mut buffer = Vec::new();
    file.take(MAX_WORKSPACE_FILE_BYTES + 1)
        .read_to_end(&mut buffer)
        .map_err(|err| format!("Failed to read file: {err}"))?;

    let truncated = buffer.len() > MAX_WORKSPACE_FILE_BYTES as usize;
    if truncated {
        buffer.truncate(MAX_WORKSPACE_FILE_BYTES as usize);
    }

    let content = String::from_utf8(buffer).map_err(|_| "File is not valid UTF-8".to_string())?;
    Ok(WorkspaceFileResponse { content, truncated })
}

fn default_data_dir() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_DATA_HOME") {
        let trimmed = xdg.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("codex-monitor-daemon");
        }
    }
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("codex-monitor-daemon")
}

fn usage() -> String {
    format!(
        "\
USAGE:\n  codex-monitor-daemon [--listen <addr>] [--http-listen <addr>] [--data-dir <path>] [--token <token> | --insecure-no-auth]\n\n\
OPTIONS:\n  --listen <addr>          Bind address for TCP JSON-RPC (default: {DEFAULT_LISTEN_ADDR})\n  --http-listen <addr>     Optional bind address for thin HTTP bridge\n  --data-dir <path>        Data dir holding workspaces.json/settings.json\n  --token <token>          Shared token required by TCP and HTTP clients\n  --insecure-no-auth       Disable auth (dev only)\n  -h, --help               Show this help\n"
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HttpTaskSubmitRequest {
    workspace_id: String,
    text: String,
    thread_id: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    service_tier: Option<Option<String>>,
    access_mode: Option<String>,
    images: Option<Vec<String>>,
    app_mentions: Option<Vec<Value>>,
    collaboration_mode: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HttpCreateTaskRequest {
    workspace_id: String,
    text: String,
    thread_id: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    service_tier: Option<Option<String>>,
    access_mode: Option<String>,
    images: Option<Vec<String>>,
    app_mentions: Option<Vec<Value>>,
    collaboration_mode: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HttpConversationStartRequest {
    workspace_id: String,
    title: String,
    requirement: String,
    operator: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    service_tier: Option<Option<String>>,
    access_mode: Option<String>,
    images: Option<Vec<String>>,
    app_mentions: Option<Vec<Value>>,
    collaboration_mode: Option<Value>,
    codex_profile: Option<String>,
    default_prompt_template: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HttpConversationMessageRequest {
    text: String,
    model: Option<String>,
    effort: Option<String>,
    service_tier: Option<Option<String>>,
    access_mode: Option<String>,
    images: Option<Vec<String>>,
    app_mentions: Option<Vec<Value>>,
    collaboration_mode: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HttpHistoryBackfillRequest {
    workspace_id: Option<String>,
    conversation_id: Option<String>,
}

fn current_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or_default()
}

fn parse_timestamp_ms(value: Option<&Value>) -> Option<u64> {
    let text = value?.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|ts| ts.timestamp_millis().max(0) as u64)
}

fn load_service_tasks(path: &PathBuf) -> HashMap<String, ServiceTaskRecord> {
    if !path.exists() {
        return HashMap::new();
    }

    let data = match std::fs::read_to_string(path) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("daemon: failed to read tasks from {}: {err}", path.display());
            return HashMap::new();
        }
    };

    let records: Vec<ServiceTaskRecord> = match serde_json::from_str(&data) {
        Ok(records) => records,
        Err(err) => {
            eprintln!(
                "daemon: failed to deserialize tasks from {}: {err}",
                path.display()
            );
            return HashMap::new();
        }
    };

    records
        .into_iter()
        .map(normalize_loaded_service_task)
        .map(|record| (record.task_id.clone(), record))
        .collect()
}

fn load_service_conversations(path: &PathBuf) -> HashMap<String, ServiceConversationRecord> {
    if !path.exists() {
        return HashMap::new();
    }

    let data = match std::fs::read_to_string(path) {
        Ok(data) => data,
        Err(err) => {
            eprintln!(
                "daemon: failed to read conversations from {}: {err}",
                path.display()
            );
            return HashMap::new();
        }
    };

    let records: Vec<ServiceConversationRecord> = match serde_json::from_str(&data) {
        Ok(records) => records,
        Err(err) => {
            eprintln!(
                "daemon: failed to deserialize conversations from {}: {err}",
                path.display()
            );
            return HashMap::new();
        }
    };

    records
        .into_iter()
        .map(|record| (record.conversation_id.clone(), record))
        .collect()
}

fn load_pending_mysql_writes(path: &PathBuf) -> HashMap<String, PendingMySqlWrite> {
    if !path.exists() {
        return HashMap::new();
    }

    let data = match std::fs::read_to_string(path) {
        Ok(data) => data,
        Err(err) => {
            eprintln!(
                "daemon: failed to read mysql retry queue from {}: {err}",
                path.display()
            );
            return HashMap::new();
        }
    };

    let records: Vec<PendingMySqlWrite> = match serde_json::from_str(&data) {
        Ok(records) => records,
        Err(err) => {
            eprintln!(
                "daemon: failed to deserialize mysql retry queue from {}: {err}",
                path.display()
            );
            return HashMap::new();
        }
    };

    records
        .into_iter()
        .map(|record| (record.key.clone(), record))
        .collect()
}

fn write_service_tasks(
    path: &PathBuf,
    tasks: &HashMap<String, ServiceTaskRecord>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let mut records = tasks.values().cloned().collect::<Vec<_>>();
    records.sort_by(|a, b| {
        a.submitted_at_ms
            .cmp(&b.submitted_at_ms)
            .then_with(|| a.task_id.cmp(&b.task_id))
    });
    let data = serde_json::to_string_pretty(&records).map_err(|err| err.to_string())?;
    std::fs::write(path, data).map_err(|err| err.to_string())
}

fn write_pending_mysql_writes(
    path: &PathBuf,
    writes: &HashMap<String, PendingMySqlWrite>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let mut records = writes.values().cloned().collect::<Vec<_>>();
    records.sort_by(|a, b| {
        a.created_at_ms
            .cmp(&b.created_at_ms)
            .then_with(|| a.key.cmp(&b.key))
    });
    let data = serde_json::to_string_pretty(&records).map_err(|err| err.to_string())?;
    std::fs::write(path, data).map_err(|err| err.to_string())
}

fn write_service_conversations(
    path: &PathBuf,
    conversations: &HashMap<String, ServiceConversationRecord>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let mut records = conversations.values().cloned().collect::<Vec<_>>();
    records.sort_by(|a, b| {
        b.updated_at_ms
            .cmp(&a.updated_at_ms)
            .then_with(|| a.conversation_id.cmp(&b.conversation_id))
    });
    let data = serde_json::to_string_pretty(&records).map_err(|err| err.to_string())?;
    std::fs::write(path, data).map_err(|err| err.to_string())
}

fn normalize_loaded_service_task(mut task: ServiceTaskRecord) -> ServiceTaskRecord {
    if matches!(task.status.as_str(), "accepted" | "running") {
        task.status = "failed".to_string();
        if task.completed_at_ms.is_none() {
            task.completed_at_ms = Some(current_timestamp_ms());
        }
        if task.last_error.is_none() {
            task.last_error = Some("daemon restarted before task completion".to_string());
        }
    }
    task
}

fn is_terminal_task_status(status: &str) -> bool {
    matches!(status, "completed" | "failed")
}

fn extract_task_thread_id(message: &Value) -> Option<String> {
    message
        .get("params")
        .and_then(|params| params.get("threadId"))
        .or_else(|| {
            message
                .get("params")
                .and_then(|params| params.get("thread"))
                .and_then(|thread| thread.get("id"))
        })
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn extract_task_turn_id(message: &Value) -> Option<String> {
    message
        .get("params")
        .and_then(|params| params.get("turnId"))
        .or_else(|| {
            message
                .get("params")
                .and_then(|params| params.get("turn"))
                .and_then(|turn| turn.get("id"))
        })
        .or_else(|| message.get("result").and_then(|result| result.get("id")))
        .or_else(|| {
            message
                .get("result")
                .and_then(|result| result.get("turn"))
                .and_then(|turn| turn.get("id"))
        })
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn extract_response_thread_id(value: &Value) -> Option<String> {
    value
        .get("threadId")
        .and_then(Value::as_str)
        .or_else(|| {
            value.get("result")
                .and_then(|result| result.get("threadId"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            value.get("result")
                .and_then(|result| result.get("thread"))
                .and_then(|thread| thread.get("id"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            value.get("thread")
                .and_then(|thread| thread.get("id"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
}

fn extract_task_error_message(message: &Value) -> Option<String> {
    let params = message.get("params")?;
    params
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            params
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn map_conversation_status(method: &str, message: &Value) -> Option<String> {
    match method {
        "thread/started" => Some("thread_created".to_string()),
        "turn/started" | "item/started" => Some("running".to_string()),
        "turn/completed" => Some("completed".to_string()),
        "error" | "turn/error" => Some("failed".to_string()),
        "thread/status/changed" => match extract_thread_status_type(message).as_deref() {
            Some("active") => Some("running".to_string()),
            Some("systemerror") => Some("failed".to_string()),
            _ => None,
        },
        _ if has_waiting_input_payload(message) => Some("waiting_input".to_string()),
        _ if has_waiting_approval_payload(message) => Some("waiting_approval".to_string()),
        _ if is_streaming_method(method) => Some("streaming".to_string()),
        _ => None,
    }
}

fn extract_thread_status_type(message: &Value) -> Option<String> {
    let status = message.get("params")?.get("status")?;
    if let Some(value) = status.as_str() {
        return Some(
            value
                .trim()
                .to_lowercase()
                .replace([' ', '_', '-'], ""),
        );
    }
    let value = status
        .get("type")
        .or_else(|| status.get("statusType"))
        .or_else(|| status.get("status_type"))?
        .as_str()?;
    Some(
        value
            .trim()
            .to_lowercase()
            .replace([' ', '_', '-'], ""),
    )
}

fn is_streaming_method(method: &str) -> bool {
    matches!(
        method,
        "item/agentMessage/delta"
            | "item/commandExecution/outputDelta"
            | "item/fileChange/outputDelta"
            | "item/plan/delta"
            | "item/reasoning/summaryPartAdded"
            | "item/reasoning/summaryTextDelta"
            | "item/reasoning/textDelta"
            | "turn/diff/updated"
            | "turn/plan/updated"
    ) || method.contains("stream")
        || method.contains("turn/text")
}

fn has_waiting_input_payload(message: &Value) -> bool {
    message
        .get("method")
        .and_then(Value::as_str)
        .map(|method| method == "item/tool/requestUserInput")
        .unwrap_or(false)
}

fn has_waiting_approval_payload(message: &Value) -> bool {
    message
        .get("method")
        .and_then(Value::as_str)
        .map(|method| method.ends_with("requestApproval"))
        .unwrap_or(false)
}

fn extract_conversation_summary(message: &Value) -> Option<String> {
    message
        .get("params")
        .and_then(|params| params.get("summary"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            message
                .get("result")
                .and_then(|result| result.get("summary"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn extract_assistant_message_text(message: &Value) -> Option<String> {
    let params = message.get("params")?;
    let item = params.get("item")?;
    if item.get("type").and_then(Value::as_str) != Some("agentMessage") {
        return None;
    }
    item.get("text").and_then(Value::as_str).map(str::to_string)
}

fn build_requirement_prompt(
    title: &str,
    requirement: &str,
    codex_profile: Option<&str>,
    default_prompt_template: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    if let Some(template) = default_prompt_template.map(str::trim).filter(|value| !value.is_empty()) {
        parts.push(template.to_string());
    }
    if let Some(profile) = codex_profile.map(str::trim).filter(|value| !value.is_empty()) {
        parts.push(format!("Codex Profile:\n{profile}"));
    }
    parts.push(format!(
        "需求标题：\n{title}\n\n需求描述：\n{requirement}\n\n要求：\n1. 先基于当前代码库理解现状\n2. 输出实现过程和最终结果\n3. 若需要补充信息，明确提出"
    ));
    parts.join("\n\n")
}

fn preview_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let preview = trimmed.chars().take(160).collect::<String>();
    Some(preview)
}

fn summary_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let summary = trimmed.chars().take(800).collect::<String>();
    Some(summary)
}

async fn ensure_workspace_connected_for_http(
    state: &DaemonState,
    workspace_id: &str,
) -> Result<(), String> {
    state
        .connect_workspace(workspace_id.to_string(), "daemon-http".to_string())
        .await
}

fn find_matching_task_id(
    tasks: &HashMap<String, ServiceTaskRecord>,
    workspace_id: &str,
    thread_id: Option<&str>,
    turn_id: Option<&str>,
) -> Option<String> {
    if let Some(turn_id) = turn_id {
        if let Some(task) = tasks
            .values()
            .filter(|task| {
                task.workspace_id == workspace_id && task.turn_id.as_deref() == Some(turn_id)
            })
            .max_by_key(|task| task.submitted_at_ms)
        {
            return Some(task.task_id.clone());
        }
    }

    let thread_id = thread_id?;
    tasks
        .values()
        .filter(|task| {
            task.workspace_id == workspace_id
                && task.thread_id == thread_id
                && !is_terminal_task_status(&task.status)
        })
        .max_by_key(|task| task.submitted_at_ms)
        .map(|task| task.task_id.clone())
}

fn app_server_event_matches_thread_stream(
    event: &AppServerEvent,
    workspace_id: &str,
    thread_id: Option<&str>,
) -> bool {
    if event.workspace_id != workspace_id {
        return false;
    }

    match thread_id {
        Some(thread_id) => extract_task_thread_id(&event.message).as_deref() == Some(thread_id),
        None => true,
    }
}

fn http_json_response(status: &str, body: Value) -> String {
    let body_string = serde_json::to_string(&body)
        .unwrap_or_else(|_| "{\"error\":\"serialization failed\"}".to_string());
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body_string.len(),
        body_string
    )
}

fn http_api_success_response(status: &str, data: Value, meta: Option<Value>) -> String {
    let mut body = serde_json::Map::new();
    body.insert("ok".to_string(), Value::Bool(true));
    body.insert("data".to_string(), data);
    if let Some(meta) = meta {
        body.insert("meta".to_string(), meta);
    }
    http_json_response(status, Value::Object(body))
}

fn http_api_error_response(status: &str, code: &str, message: &str) -> String {
    http_json_response(
        status,
        json!({
            "ok": false,
            "error": {
                "code": code,
                "message": message
            }
        }),
    )
}

fn http_error_response(status: &str, message: &str) -> String {
    http_json_response(status, json!({ "error": message }))
}

fn normalize_thread_messages(value: Value) -> Value {
    if let Some(items) = value.get("items").and_then(Value::as_array) {
        return Value::Array(items.clone());
    }
    if let Some(items) = value.as_array() {
        return Value::Array(items.clone());
    }
    if let Some(items) = extract_session_messages(&value) {
        return Value::Array(items);
    }
    Value::Array(Vec::new())
}

fn collect_history_backfill_messages(value: Value) -> Vec<HistoryBackfillMessage> {
    let Value::Array(items) = normalize_thread_messages(value) else {
        return Vec::new();
    };
    items.into_iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            let role = match object.get("type").and_then(Value::as_str) {
                Some("user") => "user",
                Some("assistant") => "assistant",
                _ => return None,
            };
            let content = object
                .get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())?
                .to_string();
            let turn_id = object
                .get("turnId")
                .and_then(Value::as_str)
                .map(str::to_string);
            let message_type = object
                .get("phase")
                .and_then(Value::as_str)
                .filter(|phase| !phase.trim().is_empty())
                .unwrap_or("message")
                .to_string();
            let payload_json = serde_json::to_string(&item).ok();
            let created_at_ms =
                parse_timestamp_ms(object.get("timestamp")).unwrap_or_else(current_timestamp_ms);
            Some(HistoryBackfillMessage {
                turn_id,
                role: role.to_string(),
                message_type,
                content,
                payload_json,
                created_at_ms,
            })
        })
        .collect()
}

fn extract_session_messages(value: &Value) -> Option<Vec<Value>> {
    let path = value
        .get("result")
        .and_then(|result| result.get("thread"))
        .and_then(|thread| thread.get("path"))
        .or_else(|| {
            value.get("thread")
                .and_then(|thread| thread.get("path"))
        })
        .and_then(Value::as_str)?
        .trim();
    if path.is_empty() {
        return None;
    }

    let file = File::open(path).ok()?;
    let reader = StdBufReader::new(file);
    let mut known_turn_ids = HashSet::new();
    let mut current_turn_id: Option<String> = None;
    let mut event_messages = Vec::new();
    let mut messages = Vec::new();

    for (index, line) in reader.lines().enumerate() {
        let Ok(line) = line else {
            continue;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let entry_type = entry.get("type").and_then(Value::as_str).unwrap_or_default();
        if entry_type == "event_msg" {
            let payload = entry.get("payload");
            let payload_type = payload
                .and_then(|payload| payload.get("type"))
                .and_then(Value::as_str);
            match payload_type {
                Some("task_started") => {
                    if let Some(turn_id) = payload
                        .and_then(|payload| payload.get("turn_id"))
                        .and_then(Value::as_str)
                    {
                        let turn_id = turn_id.to_string();
                        known_turn_ids.insert(turn_id.clone());
                        current_turn_id = Some(turn_id);
                    }
                }
                Some("task_complete") => {
                    let completed_turn_id = payload
                        .and_then(|payload| payload.get("turn_id"))
                        .and_then(Value::as_str);
                    if completed_turn_id == current_turn_id.as_deref() {
                        current_turn_id = None;
                    }
                }
                Some("user_message") | Some("agent_message") => {
                    let role = if payload_type == Some("user_message") {
                        "user"
                    } else {
                        "assistant"
                    };
                    let text_key = if payload_type == Some("user_message") {
                        "message"
                    } else {
                        "message"
                    };
                    let Some(turn_id) = current_turn_id.clone() else {
                        continue;
                    };
                    let Some(text) = payload
                        .and_then(|payload| payload.get(text_key))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                    else {
                        continue;
                    };
                    let mut message = serde_json::Map::new();
                    message.insert(
                        "id".to_string(),
                        Value::String(format!("session-event-message-{}", index + 1)),
                    );
                    message.insert("type".to_string(), Value::String(role.to_string()));
                    message.insert("text".to_string(), Value::String(text.to_string()));
                    message.insert("turnId".to_string(), Value::String(turn_id));
                    if let Some(timestamp) = entry.get("timestamp").and_then(Value::as_str) {
                        message.insert("timestamp".to_string(), Value::String(timestamp.to_string()));
                    }
                    if let Some(phase) = payload
                        .and_then(|payload| payload.get("phase"))
                        .and_then(Value::as_str)
                    {
                        message.insert("phase".to_string(), Value::String(phase.to_string()));
                    }
                    event_messages.push(Value::Object(message));
                }
                _ => {}
            }
            continue;
        }
        if entry_type != "response_item" {
            continue;
        }

        let Some(payload) = entry.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let role = match payload.get("role").and_then(Value::as_str) {
            Some("user") => "user",
            Some("assistant") => "assistant",
            _ => continue,
        };
        let Some(turn_id) = payload
            .get("internal_chat_message_metadata_passthrough")
            .and_then(|meta| meta.get("turn_id"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if !known_turn_ids.contains(turn_id) {
            continue;
        }
        let Some(text) = extract_message_text(payload.get("content")) else {
            continue;
        };
        let id = payload
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("session-message-{}", index + 1));
        let mut message = serde_json::Map::new();
        message.insert("id".to_string(), Value::String(id));
        message.insert("type".to_string(), Value::String(role.to_string()));
        message.insert("text".to_string(), Value::String(text));
        message.insert("turnId".to_string(), Value::String(turn_id.to_string()));
        if let Some(timestamp) = entry.get("timestamp").and_then(Value::as_str) {
            message.insert("timestamp".to_string(), Value::String(timestamp.to_string()));
        }
        if let Some(phase) = payload.get("phase").and_then(Value::as_str) {
            message.insert("phase".to_string(), Value::String(phase.to_string()));
        }
        messages.push(Value::Object(message));
    }

    if !event_messages.is_empty() {
        Some(event_messages)
    } else if !messages.is_empty() {
        Some(messages)
    } else {
        None
    }
}

fn extract_message_text(content: Option<&Value>) -> Option<String> {
    let items = content?.as_array()?;
    let parts = items
        .iter()
        .filter_map(|item| {
            let item_type = item.get("type").and_then(Value::as_str)?;
            if !matches!(item_type, "input_text" | "output_text" | "text") {
                return None;
            }
            item.get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

fn conversation_http_error_response(message: &str) -> String {
    match message {
        "missing `title`" => {
            http_api_error_response("400 Bad Request", "missing_title", message)
        }
        "missing `requirement`" => {
            http_api_error_response("400 Bad Request", "missing_requirement", message)
        }
        "missing `text`" => http_api_error_response("400 Bad Request", "missing_text", message),
        "missing `conversationId`" => {
            http_api_error_response("400 Bad Request", "missing_conversation_id", message)
        }
        "conversation not found" => {
            http_api_error_response("404 Not Found", "conversation_not_found", message)
        }
        "workspace not found" => {
            http_api_error_response("404 Not Found", "workspace_not_found", message)
        }
        "workspace not connected" => {
            http_api_error_response("409 Conflict", "workspace_not_connected", message)
        }
        "start_thread response missing `threadId`" => http_api_error_response(
            "502 Bad Gateway",
            "upstream_protocol_error",
            message,
        ),
        _ => http_api_error_response("500 Internal Server Error", "internal_error", message),
    }
}

fn parse_http_query(query: Option<&str>) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let Some(query) = query else {
        return result;
    };

    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let mut parts = pair.splitn(2, '=');
        let raw_key = parts.next().unwrap_or_default();
        let raw_value = parts.next().unwrap_or_default();
        let key = match urlencoding::decode(raw_key) {
            Ok(value) => value.into_owned(),
            Err(_) => continue,
        };
        let value = match urlencoding::decode(raw_value) {
            Ok(value) => value.into_owned(),
            Err(_) => continue,
        };
        result.insert(key, value);
    }

    result
}

fn extract_http_token(headers: &HashMap<String, String>) -> Option<String> {
    if let Some(value) = headers.get("x-codex-token") {
        return Some(value.trim().to_string());
    }

    let auth = headers.get("authorization")?;
    let (scheme, token) = auth.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let trimmed = token.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn is_http_authorized(config: &DaemonConfig, headers: &HashMap<String, String>) -> bool {
    match &config.token {
        Some(expected) => extract_http_token(headers).as_deref() == Some(expected.as_str()),
        None => true,
    }
}

async fn read_http_request(
    socket: &mut TcpStream,
) -> Result<(String, String, Option<String>, HashMap<String, String>, Vec<u8>), String> {
    let mut buffer = Vec::new();
    let mut temp = [0_u8; 4096];
    let header_end;

    loop {
        let bytes_read = socket
            .read(&mut temp)
            .await
            .map_err(|err| format!("failed to read socket: {err}"))?;
        if bytes_read == 0 {
            return Err("connection closed".to_string());
        }
        buffer.extend_from_slice(&temp[..bytes_read]);
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = index + 4;
            break;
        }
        if buffer.len() > MAX_HTTP_BODY_BYTES {
            return Err("request too large".to_string());
        }
    }

    let header_text = std::str::from_utf8(&buffer[..header_end])
        .map_err(|_| "invalid HTTP header encoding".to_string())?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().ok_or_else(|| "missing request line".to_string())?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| "missing HTTP method".to_string())?
        .to_string();
    let target = request_parts
        .next()
        .ok_or_else(|| "missing request target".to_string())?;
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), Some(query.to_string())),
        None => (target.to_string(), None),
    };

    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_HTTP_BODY_BYTES {
        return Err("request body too large".to_string());
    }

    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let bytes_read = socket
            .read(&mut temp)
            .await
            .map_err(|err| format!("failed to read request body: {err}"))?;
        if bytes_read == 0 {
            return Err("connection closed before request body completed".to_string());
        }
        body.extend_from_slice(&temp[..bytes_read]);
    }

    Ok((method, path, query, headers, body))
}

async fn handle_http_task_submit(
    state: &DaemonState,
    request: HttpTaskSubmitRequest,
) -> Result<Value, String> {
    let thread_id = if let Some(thread_id) = request.thread_id.clone() {
        thread_id
    } else {
        let created = state.start_thread(request.workspace_id.clone()).await?;
        extract_response_thread_id(&created)
            .ok_or_else(|| "start_thread response missing `threadId`".to_string())?
    };

    let created_thread = request.thread_id.is_none();
    let turn_response = state
        .send_user_message(
            request.workspace_id.clone(),
            thread_id.clone(),
            request.text,
            request.model,
            request.effort,
            request.service_tier,
            request.access_mode,
            request.images,
            request.app_mentions,
            request.collaboration_mode,
        )
        .await?;
    let turn_id = extract_task_turn_id(&turn_response);

    Ok(json!({
        "accepted": true,
        "workspaceId": request.workspace_id,
        "threadId": thread_id,
        "createdThread": created_thread,
        "turnId": turn_id
    }))
}

async fn handle_http_create_task(
    state: &DaemonState,
    request: HttpCreateTaskRequest,
) -> Result<Value, String> {
    let response = handle_http_task_submit(
        state,
        HttpTaskSubmitRequest {
            workspace_id: request.workspace_id,
            text: request.text,
            thread_id: request.thread_id,
            model: request.model,
            effort: request.effort,
            service_tier: request.service_tier,
            access_mode: request.access_mode,
            images: request.images,
            app_mentions: request.app_mentions,
            collaboration_mode: request.collaboration_mode,
        },
    )
    .await?;

    let workspace_id = response
        .get("workspaceId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "task submission response missing `workspaceId`".to_string())?;
    let thread_id = response
        .get("threadId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "task submission response missing `threadId`".to_string())?;
    let created_thread = response
        .get("createdThread")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let turn_id = response
        .get("turnId")
        .and_then(Value::as_str)
        .map(str::to_string);

    let task = state
        .insert_task(ServiceTaskRecord {
            task_id: uuid::Uuid::new_v4().to_string(),
            status: "accepted".to_string(),
            workspace_id,
            thread_id,
            turn_id: turn_id.clone(),
            created_thread,
            submitted_at_ms: current_timestamp_ms(),
            completed_at_ms: None,
            last_error: None,
        })
        .await;
    let _ = state.mark_task_running(&task.task_id, turn_id).await;

    Ok(json!({
        "task": task
    }))
}

async fn handle_http_conversation_start(
    state: &DaemonState,
    request: HttpConversationStartRequest,
) -> Result<Value, String> {
    let title = request.title.trim().to_string();
    let requirement = request.requirement.trim().to_string();
    if title.is_empty() {
        return Err("missing `title`".to_string());
    }
    if requirement.is_empty() {
        return Err("missing `requirement`".to_string());
    }

    ensure_workspace_connected_for_http(state, &request.workspace_id).await?;
    let start_response = state.start_thread(request.workspace_id.clone()).await?;
    let thread_id = extract_response_thread_id(&start_response)
        .ok_or_else(|| "start_thread response missing `threadId`".to_string())?;

    let prompt = build_requirement_prompt(
        &title,
        &requirement,
        request.codex_profile.as_deref(),
        request.default_prompt_template.as_deref(),
    );
    let turn_response = state
        .send_user_message(
            request.workspace_id.clone(),
            thread_id.clone(),
            prompt,
            request.model,
            request.effort,
            request.service_tier,
            request.access_mode,
            request.images,
            request.app_mentions,
            request.collaboration_mode,
        )
        .await?;
    let turn_id = extract_task_turn_id(&turn_response);
    let now = current_timestamp_ms();
    let conversation = state
        .insert_conversation(ServiceConversationRecord {
            conversation_id: uuid::Uuid::new_v4().to_string(),
            workspace_id: request.workspace_id.clone(),
            thread_id: thread_id.clone(),
            title,
            requirement: requirement.clone(),
            status: "accepted".to_string(),
            operator: request.operator.filter(|value| !value.trim().is_empty()),
            created_at_ms: now,
            updated_at_ms: now,
            last_message_preview: preview_text(&requirement),
            final_summary: None,
            last_error: None,
        })
        .await;
    let task = state
        .insert_task(ServiceTaskRecord {
            task_id: uuid::Uuid::new_v4().to_string(),
            status: "accepted".to_string(),
            workspace_id: request.workspace_id,
            thread_id,
            turn_id: turn_id.clone(),
            created_thread: true,
            submitted_at_ms: now,
            completed_at_ms: None,
            last_error: None,
        })
        .await;
    let task = state
        .mark_task_running(&task.task_id, turn_id)
        .await
        .unwrap_or(task);
    let conversation = state
        .update_conversation(&conversation.conversation_id, |conversation| {
            conversation.status = "thread_created".to_string();
        })
        .await
        .unwrap_or(conversation);
    let start_event_payload = serde_json::to_string(&json!({
        "title": &conversation.title,
        "requirement": &requirement,
        "taskId": &task.task_id,
    }))
    .unwrap_or_else(|_| "{}".to_string());
    state
        .persist_message_history(
            &conversation.conversation_id,
            &conversation.thread_id,
            task.turn_id.as_deref(),
            "user",
            "requirement",
            &requirement,
            None,
            now,
        )
        .await;
    state
        .persist_event_history(
            &conversation.conversation_id,
            &conversation.thread_id,
            task.turn_id.as_deref(),
            "http/start",
            Some("accepted"),
            &start_event_payload,
            now,
        )
        .await;

    Ok(json!({
        "conversation": conversation,
        "task": task,
    }))
}

async fn handle_http_conversation_message(
    state: &DaemonState,
    conversation_id: &str,
    request: HttpConversationMessageRequest,
) -> Result<Value, String> {
    let conversation = state
        .get_conversation(conversation_id)
        .await
        .ok_or_else(|| "conversation not found".to_string())?;
    let text = request.text.trim().to_string();
    if text.is_empty() {
        return Err("missing `text`".to_string());
    }

    ensure_workspace_connected_for_http(state, &conversation.workspace_id).await?;
    let turn_response = state
        .send_user_message(
            conversation.workspace_id.clone(),
            conversation.thread_id.clone(),
            text.clone(),
            request.model,
            request.effort,
            request.service_tier,
            request.access_mode,
            request.images,
            request.app_mentions,
            request.collaboration_mode,
        )
        .await?;
    let turn_id = extract_task_turn_id(&turn_response);
    let task = state
        .insert_task(ServiceTaskRecord {
            task_id: uuid::Uuid::new_v4().to_string(),
            status: "accepted".to_string(),
            workspace_id: conversation.workspace_id.clone(),
            thread_id: conversation.thread_id.clone(),
            turn_id: turn_id.clone(),
            created_thread: false,
            submitted_at_ms: current_timestamp_ms(),
            completed_at_ms: None,
            last_error: None,
        })
        .await;
    let task = state
        .mark_task_running(&task.task_id, turn_id)
        .await
        .unwrap_or(task);
    let preview = preview_text(&text);
    let conversation = state
        .update_conversation(conversation_id, move |conversation| {
            conversation.last_message_preview = preview.clone();
            conversation.status = "accepted".to_string();
            conversation.last_error = None;
        })
        .await
        .unwrap_or(conversation);
    let now = current_timestamp_ms();
    let message_event_payload = serde_json::to_string(&json!({
        "text": &text,
        "taskId": &task.task_id,
    }))
    .unwrap_or_else(|_| "{}".to_string());
    state
        .persist_message_history(
            conversation_id,
            &conversation.thread_id,
            task.turn_id.as_deref(),
            "user",
            "message",
            &text,
            None,
            now,
        )
        .await;
    state
        .persist_event_history(
            conversation_id,
            &conversation.thread_id,
            task.turn_id.as_deref(),
            "http/message",
            Some("accepted"),
            &message_event_payload,
            now,
        )
        .await;

    Ok(json!({
        "conversation": conversation,
        "task": task,
    }))
}

async fn handle_http_history_backfill(
    state: &DaemonState,
    request: HttpHistoryBackfillRequest,
) -> Result<Value, String> {
    let report = state
        .backfill_history_to_mysql(
            request.workspace_id.as_deref(),
            request.conversation_id.as_deref(),
        )
        .await?;
    Ok(json!({
        "backfill": report,
    }))
}

async fn write_sse_event(
    socket: &mut TcpStream,
    event_name: &str,
    payload: &Value,
) -> Result<(), String> {
    let body = format!(
        "event: {event_name}\ndata: {}\n\n",
        serde_json::to_string(payload).map_err(|err| err.to_string())?
    );
    socket
        .write_all(body.as_bytes())
        .await
        .map_err(|err| format!("failed to write SSE event: {err}"))
}

async fn handle_http_task_events_stream(
    state: Arc<DaemonState>,
    mut socket: TcpStream,
    task_id: String,
) {
    let Some(task) = state.get_task(&task_id).await else {
        let response = http_error_response("404 Not Found", "task not found");
        let _ = socket.write_all(response.as_bytes()).await;
        return;
    };

    let headers = concat!(
        "HTTP/1.1 200 OK\r\n",
        "Content-Type: text/event-stream\r\n",
        "Cache-Control: no-cache\r\n",
        "Connection: keep-alive\r\n",
        "\r\n"
    );
    if socket.write_all(headers.as_bytes()).await.is_err() {
        return;
    }

    if write_sse_event(&mut socket, "task", &json!({ "task": task.clone() }))
        .await
        .is_err()
    {
        return;
    }

    let mut rx = state.event_sink.tx.subscribe();
    loop {
        let next = tokio::time::timeout(Duration::from_secs(15), rx.recv()).await;
        match next {
            Ok(Ok(DaemonEvent::TaskUpdated(updated))) if updated.task_id == task_id => {
                if write_sse_event(&mut socket, "task", &json!({ "task": updated.clone() }))
                    .await
                    .is_err()
                {
                    break;
                }
                if is_terminal_task_status(&updated.status) {
                    break;
                }
            }
            Ok(Ok(DaemonEvent::AppServer(event)))
                if event.workspace_id == task.workspace_id
                    && extract_task_thread_id(&event.message).as_deref()
                        == Some(task.thread_id.as_str()) =>
            {
                if write_sse_event(&mut socket, "app-server-event", &json!(event))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => break,
            Err(_) => {
                if socket.write_all(b": keep-alive\n\n").await.is_err() {
                    break;
                }
            }
        }
    }
}

async fn handle_http_thread_events_stream(
    state: Arc<DaemonState>,
    mut socket: TcpStream,
    workspace_id: String,
    thread_id: Option<String>,
) {
    let headers = concat!(
        "HTTP/1.1 200 OK\r\n",
        "Content-Type: text/event-stream\r\n",
        "Cache-Control: no-cache\r\n",
        "Connection: keep-alive\r\n",
        "\r\n"
    );
    if socket.write_all(headers.as_bytes()).await.is_err() {
        return;
    }

    if write_sse_event(
        &mut socket,
        "stream",
        &json!({
            "workspaceId": workspace_id,
            "threadId": thread_id,
            "kind": "thread-events",
        }),
    )
    .await
    .is_err()
    {
        return;
    }

    let mut rx = state.event_sink.tx.subscribe();
    loop {
        let next = tokio::time::timeout(Duration::from_secs(15), rx.recv()).await;
        match next {
            Ok(Ok(DaemonEvent::AppServer(event)))
                if app_server_event_matches_thread_stream(
                    &event,
                    &workspace_id,
                    thread_id.as_deref(),
                ) =>
            {
                if write_sse_event(&mut socket, "app-server-event", &json!(event))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => break,
            Err(_) => {
                if socket.write_all(b": keep-alive\n\n").await.is_err() {
                    break;
                }
            }
        }
    }
}

async fn handle_http_conversation_events_stream(
    state: Arc<DaemonState>,
    mut socket: TcpStream,
    conversation_id: String,
) {
    let Some(conversation) = state.get_conversation(&conversation_id).await else {
        let response = http_error_response("404 Not Found", "conversation not found");
        let _ = socket.write_all(response.as_bytes()).await;
        return;
    };

    let headers = concat!(
        "HTTP/1.1 200 OK\r\n",
        "Content-Type: text/event-stream\r\n",
        "Cache-Control: no-cache\r\n",
        "Connection: keep-alive\r\n",
        "\r\n"
    );
    if socket.write_all(headers.as_bytes()).await.is_err() {
        return;
    }

    if write_sse_event(
        &mut socket,
        "conversation",
        &json!({ "conversation": conversation.clone() }),
    )
    .await
    .is_err()
    {
        return;
    }

    let mut rx = state.event_sink.tx.subscribe();
    loop {
        let next = tokio::time::timeout(Duration::from_secs(15), rx.recv()).await;
        match next {
            Ok(Ok(DaemonEvent::ConversationUpdated(updated)))
                if updated.conversation_id == conversation_id =>
            {
                if write_sse_event(&mut socket, "conversation", &json!({ "conversation": updated }))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(Ok(DaemonEvent::TaskUpdated(task)))
                if task.workspace_id == conversation.workspace_id
                    && task.thread_id == conversation.thread_id =>
            {
                if write_sse_event(&mut socket, "task", &json!({ "task": task }))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(Ok(DaemonEvent::AppServer(event)))
                if event.workspace_id == conversation.workspace_id
                    && extract_task_thread_id(&event.message).as_deref()
                        == Some(conversation.thread_id.as_str()) =>
            {
                if write_sse_event(
                    &mut socket,
                    "lifecycle",
                    &json!({
                        "conversationId": conversation_id,
                        "status": map_conversation_status(
                            event.message.get("method").and_then(Value::as_str).unwrap_or_default(),
                            &event.message,
                        ),
                        "event": event,
                    }),
                )
                .await
                .is_err()
                {
                    break;
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => break,
            Err(_) => {
                if socket.write_all(b": keep-alive\n\n").await.is_err() {
                    break;
                }
            }
        }
    }
}

async fn handle_http_request(
    state: Arc<DaemonState>,
    config: Arc<DaemonConfig>,
    mut socket: TcpStream,
) {
    let response = match read_http_request(&mut socket).await {
        Ok((method, path, query, headers, body)) => {
            if !is_http_authorized(&config, &headers) {
                http_error_response("401 Unauthorized", "unauthorized")
            } else if method == "GET" && path == "/api/v1/events/threads" {
                let query_map = parse_http_query(query.as_deref());
                let Some(workspace_id) = query_map.get("workspaceId").cloned() else {
                    let response =
                        http_error_response("400 Bad Request", "missing `workspaceId`");
                    let _ = socket.write_all(response.as_bytes()).await;
                    return;
                };
                let thread_id = query_map
                    .get("threadId")
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());
                handle_http_thread_events_stream(state, socket, workspace_id, thread_id).await;
                return;
            } else if method == "GET"
                && path.starts_with("/api/v1/conversations/")
                && path.ends_with("/events")
            {
                let conversation_id = path
                    .trim_start_matches("/api/v1/conversations/")
                    .trim_end_matches("/events")
                    .trim_end_matches('/')
                    .to_string();
                if conversation_id.is_empty() {
                    let response =
                        http_error_response("400 Bad Request", "missing `conversationId`");
                    let _ = socket.write_all(response.as_bytes()).await;
                    return;
                }
                handle_http_conversation_events_stream(state, socket, conversation_id).await;
                return;
            } else if method == "GET"
                && path.starts_with("/api/v1/tasks/")
                && path.ends_with("/events")
            {
                let task_id = path
                    .trim_start_matches("/api/v1/tasks/")
                    .trim_end_matches("/events")
                    .trim_end_matches('/')
                    .to_string();
                if task_id.is_empty() {
                    http_error_response("400 Bad Request", "missing `taskId`")
                } else {
                    handle_http_task_events_stream(state, socket, task_id).await;
                    return;
                }
            } else if method == "GET" && path == "/health" {
                http_json_response(
                    "200 OK",
                    json!({
                        "ok": true,
                        "daemon": state.daemon_info(),
                        "http": true,
                        "mysqlRetry": state.mysql_retry_status_snapshot(),
                    }),
                )
            } else if method == "GET" && path == "/api/v1/health" {
                http_json_response(
                    "200 OK",
                    json!({
                        "ok": true,
                        "daemon": state.daemon_info(),
                        "http": true,
                        "version": "v1",
                        "mysqlRetry": state.mysql_retry_status_snapshot(),
                    }),
                )
            } else if method == "GET" && path == "/api/workspaces" {
                let workspaces = state.list_workspaces().await;
                http_json_response("200 OK", json!({ "workspaces": workspaces }))
            } else if method == "POST" && path == "/api/v1/history/backfill" {
                let request = if body.is_empty() {
                    HttpHistoryBackfillRequest::default()
                } else {
                    match serde_json::from_slice::<HttpHistoryBackfillRequest>(&body) {
                        Ok(request) => request,
                        Err(err) => {
                            let response = http_api_error_response(
                                "400 Bad Request",
                                "invalid_json",
                                &err.to_string(),
                            );
                            let _ = socket.write_all(response.as_bytes()).await;
                            return;
                        }
                    }
                };
                match handle_http_history_backfill(&state, request).await {
                    Ok(result) => http_api_success_response("200 OK", result, None),
                    Err(err) if err == "mysql history is not enabled" => {
                        http_api_error_response("409 Conflict", "mysql_history_disabled", &err)
                    }
                    Err(err) => http_api_error_response("500 Internal Server Error", "internal_error", &err),
                }
            } else if method == "POST" && path == "/api/v1/tasks" {
                match serde_json::from_slice::<HttpCreateTaskRequest>(&body) {
                    Ok(request) => match handle_http_create_task(&state, request).await {
                        Ok(result) => http_json_response("200 OK", result),
                        Err(err) => http_error_response("400 Bad Request", &err),
                    },
                    Err(err) => http_error_response("400 Bad Request", &err.to_string()),
                }
            } else if method == "GET" && path.starts_with("/api/v1/tasks/") {
                let task_id = path
                    .trim_start_matches("/api/v1/tasks/")
                    .trim()
                    .to_string();
                if task_id.is_empty() {
                    http_error_response("400 Bad Request", "missing `taskId`")
                } else {
                    match state.get_task(&task_id).await {
                        Some(task) => http_json_response("200 OK", json!({ "task": task })),
                        None => http_error_response("404 Not Found", "task not found"),
                    }
                }
            } else if method == "POST" && path == "/api/v1/conversations/start" {
                match serde_json::from_slice::<HttpConversationStartRequest>(&body) {
                    Ok(request) => match handle_http_conversation_start(&state, request).await {
                        Ok(result) => http_api_success_response("202 Accepted", result, None),
                        Err(err) => conversation_http_error_response(&err),
                    },
                    Err(err) => {
                        http_api_error_response("400 Bad Request", "invalid_json", &err.to_string())
                    }
                }
            } else if method == "GET" && path == "/api/v1/conversations" {
                let query_map = parse_http_query(query.as_deref());
                let workspace_id = query_map.get("workspaceId").map(String::as_str);
                let conversations = state.list_conversations(workspace_id).await;
                http_api_success_response(
                    "200 OK",
                    json!({ "items": conversations }),
                    Some(json!({ "count": conversations.len() })),
                )
            } else if method == "POST"
                && path.starts_with("/api/v1/conversations/")
                && path.ends_with("/messages")
            {
                let conversation_id = path
                    .trim_start_matches("/api/v1/conversations/")
                    .trim_end_matches("/messages")
                    .trim_end_matches('/')
                    .to_string();
                if conversation_id.is_empty() {
                    conversation_http_error_response("missing `conversationId`")
                } else {
                    match serde_json::from_slice::<HttpConversationMessageRequest>(&body) {
                        Ok(request) => {
                            match handle_http_conversation_message(&state, &conversation_id, request)
                                .await
                            {
                                Ok(result) => {
                                    http_api_success_response("202 Accepted", result, None)
                                }
                                Err(err) => conversation_http_error_response(&err),
                            }
                        }
                        Err(err) => {
                            http_api_error_response("400 Bad Request", "invalid_json", &err.to_string())
                        }
                    }
                }
            } else if method == "GET"
                && path.starts_with("/api/v1/conversations/")
                && path.ends_with("/messages")
            {
                let conversation_id = path
                    .trim_start_matches("/api/v1/conversations/")
                    .trim_end_matches("/messages")
                    .trim_end_matches('/')
                    .to_string();
                if conversation_id.is_empty() {
                    conversation_http_error_response("missing `conversationId`")
                } else {
                    match state.read_conversation_messages(&conversation_id).await {
                        Ok(result) => {
                            let count = result
                                .get("messages")
                                .and_then(Value::as_array)
                                .map_or(0, |items| items.len());
                            http_api_success_response(
                                "200 OK",
                                result,
                                Some(json!({ "count": count })),
                            )
                        }
                        Err(err) => conversation_http_error_response(&err),
                    }
                }
            } else if method == "GET" && path.starts_with("/api/v1/conversations/") {
                let trimmed = path.trim_start_matches("/api/v1/conversations/").trim_matches('/');
                if trimmed.is_empty() || trimmed.contains('/') {
                    http_api_error_response("404 Not Found", "not_found", "not found")
                } else {
                    match state.get_conversation(trimmed).await {
                        Some(conversation) => {
                            http_api_success_response(
                                "200 OK",
                                json!({ "conversation": conversation }),
                                None,
                            )
                        }
                        None => conversation_http_error_response("conversation not found"),
                    }
                }
            } else if method == "GET" && path == "/api/threads" {
                let query_map = parse_http_query(query.as_deref());
                match query_map.get("workspaceId") {
                    Some(workspace_id) => match state
                        .list_threads(workspace_id.to_string(), None, None, None)
                        .await
                    {
                        Ok(result) => http_json_response("200 OK", result),
                        Err(err) => http_error_response("400 Bad Request", &err),
                    },
                    None => http_error_response("400 Bad Request", "missing `workspaceId`"),
                }
            } else if method == "GET" && path == "/api/thread" {
                let query_map = parse_http_query(query.as_deref());
                match (query_map.get("workspaceId"), query_map.get("threadId")) {
                    (Some(workspace_id), Some(thread_id)) => match state
                        .read_thread(workspace_id.to_string(), thread_id.to_string())
                        .await
                    {
                        Ok(result) => http_json_response("200 OK", result),
                        Err(err) => http_error_response("400 Bad Request", &err),
                    },
                    _ => http_error_response(
                        "400 Bad Request",
                        "missing `workspaceId` or `threadId`",
                    ),
                }
            } else if method == "POST" && path == "/api/task/submit" {
                match serde_json::from_slice::<HttpTaskSubmitRequest>(&body) {
                    Ok(request) => match handle_http_task_submit(&state, request).await {
                        Ok(result) => http_json_response("200 OK", result),
                        Err(err) => http_error_response("400 Bad Request", &err),
                    },
                    Err(err) => http_error_response("400 Bad Request", &err.to_string()),
                }
            } else {
                http_error_response("404 Not Found", "not found")
            }
        }
        Err(err) => http_error_response("400 Bad Request", &err),
    };

    let _ = socket.write_all(response.as_bytes()).await;
}

async fn forward_task_state_updates(
    state: Arc<DaemonState>,
    mut rx: broadcast::Receiver<DaemonEvent>,
) {
    loop {
        match rx.recv().await {
            Ok(DaemonEvent::AppServer(event)) => state.process_task_event(&event).await,
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

fn parse_args() -> Result<DaemonConfig, String> {
    let mut listen = DEFAULT_LISTEN_ADDR
        .parse::<SocketAddr>()
        .map_err(|err| err.to_string())?;
    let mut http_listen: Option<SocketAddr> = None;
    let mut token = env::var("CODEX_MONITOR_DAEMON_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let mut insecure_no_auth = false;
    let mut data_dir: Option<PathBuf> = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{}", usage());
                std::process::exit(0);
            }
            "--listen" => {
                let value = args.next().ok_or("--listen requires a value")?;
                listen = value.parse::<SocketAddr>().map_err(|err| err.to_string())?;
            }
            "--http-listen" => {
                let value = args.next().ok_or("--http-listen requires a value")?;
                http_listen = Some(value.parse::<SocketAddr>().map_err(|err| err.to_string())?);
            }
            "--token" => {
                let value = args.next().ok_or("--token requires a value")?;
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return Err("--token requires a non-empty value".to_string());
                }
                token = Some(trimmed.to_string());
            }
            "--data-dir" => {
                let value = args.next().ok_or("--data-dir requires a value")?;
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return Err("--data-dir requires a non-empty value".to_string());
                }
                data_dir = Some(PathBuf::from(trimmed));
            }
            "--insecure-no-auth" => {
                insecure_no_auth = true;
                token = None;
            }
            _ => return Err(format!("Unknown argument: {arg}")),
        }
    }

    if token.is_none() && !insecure_no_auth {
        return Err(
            "Missing --token (or set CODEX_MONITOR_DAEMON_TOKEN). Use --insecure-no-auth for local dev only."
                .to_string(),
        );
    }

    Ok(DaemonConfig {
        listen,
        http_listen,
        token,
        data_dir: data_dir.unwrap_or_else(default_data_dir),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::process_core::kill_child_process_tree;
    use crate::storage::write_workspaces;
    use crate::types::WorkspaceKind;
    use serde_json::json;
    use std::future::Future;
    use std::path::PathBuf;
    use std::process::Stdio;
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tokio::process::Command;
    use tokio::task::JoinHandle;

    fn run_async_test<F>(future: F)
    where
        F: Future<Output = ()>,
    {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(future);
    }

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "codex-monitor-{prefix}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn test_state(data_dir: &std::path::Path) -> DaemonState {
        let (tx, _rx) = broadcast::channel::<DaemonEvent>(32);
        DaemonState {
            data_dir: data_dir.to_path_buf(),
            workspaces: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            tasks: Mutex::new(HashMap::new()),
            conversations: Mutex::new(HashMap::new()),
            tasks_path: data_dir.join("service_tasks.json"),
            conversations_path: data_dir.join("service_conversations.json"),
            mysql_retry_path: data_dir.join("service_mysql_retry.json"),
            storage_path: data_dir.join("workspaces.json"),
            settings_path: data_dir.join("settings.json"),
            app_settings: Mutex::new(AppSettings::default()),
            mysql_history: None,
            mysql_retry_queue: Mutex::new(HashMap::new()),
            mysql_retry_flush_lock: Mutex::new(()),
            event_sink: DaemonEventSink { tx },
            codex_login_cancels: Mutex::new(HashMap::new()),
            daemon_binary_path: Some("/tmp/codex-monitor-daemon".to_string()),
        }
    }

    async fn run_http_request(
        state: Arc<DaemonState>,
        config: Arc<DaemonConfig>,
        request: &str,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let server: JoinHandle<()> = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("accept test socket");
            handle_http_request(state, config, socket).await;
        });

        let mut client = TcpStream::connect(addr).await.expect("connect test listener");
        client
            .write_all(request.as_bytes())
            .await
            .expect("write request");
        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .await
            .expect("read response");
        server.await.expect("join test server");
        String::from_utf8(response).expect("utf8 response")
    }

    async fn insert_workspace(state: &DaemonState, workspace_id: &str, workspace_path: &str) {
        let entry = WorkspaceEntry {
            id: workspace_id.to_string(),
            name: "Workspace".to_string(),
            path: workspace_path.to_string(),
            kind: WorkspaceKind::Main,
            parent_id: None,
            worktree: None,
            settings: WorkspaceSettings {
                ..WorkspaceSettings::default()
            },
        };
        state
            .workspaces
            .lock()
            .await
            .insert(workspace_id.to_string(), entry);
    }

    fn make_workspace_entry(workspace_id: &str, workspace_path: &str) -> WorkspaceEntry {
        WorkspaceEntry {
            id: workspace_id.to_string(),
            name: workspace_id.to_string(),
            path: workspace_path.to_string(),
            kind: WorkspaceKind::Main,
            parent_id: None,
            worktree: None,
            settings: WorkspaceSettings::default(),
        }
    }

    fn make_session(entry: WorkspaceEntry) -> Arc<WorkspaceSession> {
        let owner_workspace_id = entry.id;
        let mut cmd = if cfg!(windows) {
            let mut cmd = Command::new("cmd");
            cmd.args(["/C", "more"]);
            cmd
        } else {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "cat"]);
            cmd
        };

        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let mut child = cmd.spawn().expect("spawn dummy child");
        let stdin = child.stdin.take().expect("dummy child stdin");

        Arc::new(WorkspaceSession {
            codex_args: None,
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            pending: Mutex::new(HashMap::new()),
            request_context: Mutex::new(HashMap::new()),
            thread_workspace: Mutex::new(HashMap::new()),
            hidden_thread_ids: Mutex::new(HashSet::new()),
            next_id: AtomicU64::new(0),
            background_thread_callbacks: Mutex::new(HashMap::new()),
            workspace_ids: Mutex::new(HashSet::from([owner_workspace_id.clone()])),
            workspace_roots: Mutex::new(HashMap::new()),
            owner_workspace_id,
        })
    }

    #[test]
    fn parse_http_query_decodes_expected_values() {
        let parsed = parse_http_query(Some("workspaceId=abc&threadId=thread%201"));
        assert_eq!(parsed.get("workspaceId").map(String::as_str), Some("abc"));
        assert_eq!(parsed.get("threadId").map(String::as_str), Some("thread 1"));
    }

    #[test]
    fn extract_http_token_supports_bearer_and_custom_header() {
        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), "Bearer secret".to_string());
        assert_eq!(extract_http_token(&headers).as_deref(), Some("secret"));

        headers.clear();
        headers.insert("x-codex-token".to_string(), "other".to_string());
        assert_eq!(extract_http_token(&headers).as_deref(), Some("other"));
    }

    #[test]
    fn mysql_shell_config_parses_basic_url() {
        let config = parse_mysql_shell_config("mysql://root:123456@127.0.0.1:3306/ruoyi-fastapi")
            .expect("mysql config");

        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 3306);
        assert_eq!(config.username, "root");
        assert_eq!(config.password, "123456");
        assert_eq!(config.database, "ruoyi-fastapi");
    }

    #[test]
    fn sql_escape_handles_quotes_and_newlines() {
        assert_eq!(sql_escape("a'b\\c\n"), "a\\'b\\\\c\\n");
    }

    #[test]
    fn build_insert_message_sql_is_idempotent() {
        let sql = build_insert_message_sql(
            "conv-1",
            "thread-1",
            Some("turn-1"),
            "assistant",
            "item/completed",
            "hello",
            Some("{\"ok\":true}"),
            1001,
        );

        assert!(sql.contains("INSERT INTO conversation_message"));
        assert!(sql.contains("FROM DUAL WHERE NOT EXISTS"));
        assert!(sql.contains("conversation_id = 'conv-1'"));
        assert!(sql.contains("thread_id = 'thread-1'"));
        assert!(sql.contains("((turn_id = 'turn-1') OR (turn_id IS NULL AND 'turn-1' IS NULL))"));
        assert!(sql.contains("((payload_json = '{\"ok\":true}') OR (payload_json IS NULL AND '{\"ok\":true}' IS NULL))"));
    }

    #[test]
    fn build_insert_event_sql_is_idempotent() {
        let sql = build_insert_event_sql(
            "conv-1",
            "thread-1",
            None,
            "turn/completed",
            Some("completed"),
            "{\"ok\":true}",
            1002,
        );

        assert!(sql.contains("INSERT INTO conversation_event"));
        assert!(sql.contains("FROM DUAL WHERE NOT EXISTS"));
        assert!(sql.contains("conversation_id = 'conv-1'"));
        assert!(sql.contains("thread_id = 'thread-1'"));
        assert!(sql.contains("((turn_id = NULL) OR (turn_id IS NULL AND NULL IS NULL))"));
        assert!(sql.contains("((event_status = 'completed') OR (event_status IS NULL AND 'completed' IS NULL))"));
        assert!(sql.contains("payload_json = '{\"ok\":true}'"));
    }

    #[test]
    fn build_upsert_task_sql_uses_numeric_bool_for_created_thread() {
        let task = ServiceTaskRecord {
            task_id: "task-1".to_string(),
            status: "accepted".to_string(),
            workspace_id: "ws-1".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            created_thread: true,
            submitted_at_ms: 1001,
            completed_at_ms: None,
            last_error: None,
        };

        let sql = build_upsert_task_sql("conv-1", &task);

        assert!(sql.contains("created_thread"));
        assert!(sql.contains(", 1, 1001, NULL, NULL)"));
        assert!(!sql.contains("'true'"));
        assert!(!sql.contains("'false'"));
    }

    #[test]
    fn collect_history_backfill_messages_reads_normalized_thread_items() {
        let messages = collect_history_backfill_messages(json!({
            "items": [
                {
                    "id": "msg-1",
                    "type": "user",
                    "text": "hello",
                    "turnId": "turn-1",
                    "timestamp": "2026-07-08T09:43:17.110Z"
                },
                {
                    "id": "msg-2",
                    "type": "assistant",
                    "text": "world",
                    "turnId": "turn-1",
                    "phase": "final_answer",
                    "timestamp": "2026-07-08T09:43:17.246Z"
                }
            ]
        }));

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].message_type, "message");
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[0].turn_id.as_deref(), Some("turn-1"));
        assert_eq!(messages[0].created_at_ms, 1751967797110);
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].message_type, "final_answer");
        assert_eq!(messages[1].content, "world");
    }

    #[test]
    fn resolve_conversation_id_from_records_prefers_latest_snapshot() {
        let conversations = vec![
            ServiceConversationRecord {
                conversation_id: "conv-old".to_string(),
                workspace_id: "ws-1".to_string(),
                thread_id: "thread-1".to_string(),
                title: "Old".to_string(),
                requirement: "Old".to_string(),
                status: "running".to_string(),
                operator: None,
                created_at_ms: 1,
                updated_at_ms: 10,
                last_message_preview: None,
                final_summary: None,
                last_error: None,
            },
            ServiceConversationRecord {
                conversation_id: "conv-new".to_string(),
                workspace_id: "ws-1".to_string(),
                thread_id: "thread-1".to_string(),
                title: "New".to_string(),
                requirement: "New".to_string(),
                status: "completed".to_string(),
                operator: None,
                created_at_ms: 2,
                updated_at_ms: 20,
                last_message_preview: None,
                final_summary: None,
                last_error: None,
            },
        ];

        let resolved =
            resolve_conversation_id_from_records(&conversations, "ws-1", "thread-1");

        assert_eq!(resolved.as_deref(), Some("conv-new"));
    }

    #[test]
    fn mysql_history_writer_persists_rows_when_local_mysql_is_available() {
        run_async_test(async {
            let writer = MySqlHistoryWriter {
                config: MySqlShellConfig {
                    host: "127.0.0.1".to_string(),
                    port: 3306,
                    username: "root".to_string(),
                    password: "123456".to_string(),
                    database: "ruoyi-fastapi".to_string(),
                },
            };
            let ping = tokio::process::Command::new("mysql")
                .arg("-h127.0.0.1")
                .arg("-P3306")
                .arg("-uroot")
                .arg("ruoyi-fastapi")
                .arg("-Nse")
                .arg("SELECT 1")
                .env("MYSQL_PWD", "123456")
                .output()
                .await
                .ok();
            if ping.as_ref().map(|output| !output.status.success()).unwrap_or(true) {
                return;
            }

            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos();
            let conversation_id = format!("it-conv-{suffix}");
            let task_id = format!("it-task-{suffix}");
            let thread_id = format!("it-thread-{suffix}");
            let turn_id = format!("it-turn-{suffix}");
            let conversation = ServiceConversationRecord {
                conversation_id: conversation_id.clone(),
                workspace_id: "ws-it".to_string(),
                thread_id: thread_id.clone(),
                title: "MySQL smoke".to_string(),
                requirement: "verify dual write".to_string(),
                status: "completed".to_string(),
                operator: Some("tester".to_string()),
                created_at_ms: 1001,
                updated_at_ms: 1002,
                last_message_preview: Some("assistant reply".to_string()),
                final_summary: Some("done".to_string()),
                last_error: None,
            };
            let task = ServiceTaskRecord {
                task_id: task_id.clone(),
                status: "completed".to_string(),
                workspace_id: "ws-it".to_string(),
                thread_id: thread_id.clone(),
                turn_id: Some(turn_id.clone()),
                created_thread: true,
                submitted_at_ms: 1003,
                completed_at_ms: Some(1004),
                last_error: None,
            };

            writer
                .upsert_conversation(&conversation, &conversation.requirement)
                .await
                .expect("persist conversation");
            writer
                .upsert_task(&conversation_id, &task)
                .await
                .expect("persist task");
            writer
                .insert_message(
                    &conversation_id,
                    &thread_id,
                    Some(&turn_id),
                    "user",
                    "message",
                    "hello mysql",
                    Some("{\"ok\":true}"),
                    1005,
                )
                .await
                .expect("persist message");
            writer
                .insert_message(
                    &conversation_id,
                    &thread_id,
                    Some(&turn_id),
                    "user",
                    "message",
                    "hello mysql",
                    Some("{\"ok\":true}"),
                    1999,
                )
                .await
                .expect("dedupe message");
            writer
                .insert_event(
                    &conversation_id,
                    &thread_id,
                    Some(&turn_id),
                    "http/message",
                    Some("accepted"),
                    "{\"ok\":true}",
                    1006,
                )
                .await
                .expect("persist event");
            writer
                .insert_event(
                    &conversation_id,
                    &thread_id,
                    Some(&turn_id),
                    "http/message",
                    Some("accepted"),
                    "{\"ok\":true}",
                    2999,
                )
                .await
                .expect("dedupe event");

            let verify_sql = format!(
                "SELECT \
                    (SELECT COUNT(*) FROM conversation WHERE conversation_id = '{}'),\
                    (SELECT COUNT(*) FROM conversation_task WHERE task_id = '{}'),\
                    (SELECT COUNT(*) FROM conversation_message WHERE conversation_id = '{}'),\
                    (SELECT COUNT(*) FROM conversation_event WHERE conversation_id = '{}')",
                conversation_id, task_id, conversation_id, conversation_id
            );
            let output = tokio::process::Command::new("mysql")
                .arg("-h127.0.0.1")
                .arg("-P3306")
                .arg("-uroot")
                .arg("ruoyi-fastapi")
                .arg("-Nse")
                .arg(&verify_sql)
                .env("MYSQL_PWD", "123456")
                .output()
                .await
                .expect("verify mysql rows");
            assert!(output.status.success());
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "1\t1\t1\t1");

            let cleanup_sql = format!(
                "DELETE FROM conversation_message WHERE conversation_id = '{}';\
                 DELETE FROM conversation_event WHERE conversation_id = '{}';\
                 DELETE FROM conversation_task WHERE conversation_id = '{}';\
                 DELETE FROM conversation WHERE conversation_id = '{}';",
                conversation_id, conversation_id, conversation_id, conversation_id
            );
            let cleanup = tokio::process::Command::new("mysql")
                .arg("-h127.0.0.1")
                .arg("-P3306")
                .arg("-uroot")
                .arg("ruoyi-fastapi")
                .arg("-e")
                .arg(&cleanup_sql)
                .env("MYSQL_PWD", "123456")
                .output()
                .await
                .expect("cleanup mysql rows");
            assert!(cleanup.status.success());
        });
    }

    #[test]
    fn http_health_requires_auth_when_token_is_configured() {
        run_async_test(async {
            let tmp = make_temp_dir("http-health-auth");
            let state = Arc::new(test_state(&tmp));
            let config = Arc::new(DaemonConfig {
                listen: "127.0.0.1:0".parse().expect("listen addr"),
                http_listen: None,
                token: Some("secret".to_string()),
                data_dir: tmp.clone(),
            });

            let response = run_http_request(
                state,
                config,
                "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .await;

            assert!(response.starts_with("HTTP/1.1 401 Unauthorized"));
            assert!(response.contains("\"error\":\"unauthorized\""));
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn http_health_returns_ok_with_valid_auth() {
        run_async_test(async {
            let tmp = make_temp_dir("http-health-ok");
            let state = Arc::new(test_state(&tmp));
            let config = Arc::new(DaemonConfig {
                listen: "127.0.0.1:0".parse().expect("listen addr"),
                http_listen: None,
                token: Some("secret".to_string()),
                data_dir: tmp.clone(),
            });

            let response = run_http_request(
                state,
                config,
                "GET /health HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer secret\r\n\r\n",
            )
            .await;

            assert!(response.starts_with("HTTP/1.1 200 OK"));
            assert!(response.contains("\"ok\":true"));
            assert!(response.contains("\"http\":true"));
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn http_threads_requires_workspace_id_query() {
        run_async_test(async {
            let tmp = make_temp_dir("http-threads-query");
            let state = Arc::new(test_state(&tmp));
            let config = Arc::new(DaemonConfig {
                listen: "127.0.0.1:0".parse().expect("listen addr"),
                http_listen: None,
                token: None,
                data_dir: tmp.clone(),
            });

            let response = run_http_request(
                state,
                config,
                "GET /api/threads HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .await;

            assert!(response.starts_with("HTTP/1.1 400 Bad Request"));
            assert!(response.contains("missing `workspaceId`"));
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn http_task_submit_rejects_invalid_json() {
        run_async_test(async {
            let tmp = make_temp_dir("http-task-submit-json");
            let state = Arc::new(test_state(&tmp));
            let config = Arc::new(DaemonConfig {
                listen: "127.0.0.1:0".parse().expect("listen addr"),
                http_listen: None,
                token: None,
                data_dir: tmp.clone(),
            });

            let request = concat!(
                "POST /api/task/submit HTTP/1.1\r\n",
                "Host: localhost\r\n",
                "Content-Type: application/json\r\n",
                "Content-Length: 1\r\n",
                "\r\n",
                "{"
            );
            let response = run_http_request(state, config, request).await;

            assert!(response.starts_with("HTTP/1.1 400 Bad Request"));
            assert!(response.contains("\"error\""));
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn http_task_submit_returns_workspace_not_connected_for_unconnected_workspace() {
        run_async_test(async {
            let tmp = make_temp_dir("http-task-submit-unconnected");
            let state = Arc::new(test_state(&tmp));
            insert_workspace(&state, "ws-http", &tmp.join("workspace").to_string_lossy()).await;
            let config = Arc::new(DaemonConfig {
                listen: "127.0.0.1:0".parse().expect("listen addr"),
                http_listen: None,
                token: None,
                data_dir: tmp.clone(),
            });

            let body =
                r#"{"workspaceId":"ws-http","text":"run remote task","accessMode":"full-access"}"#;
            let request = format!(
                concat!(
                    "POST /api/task/submit HTTP/1.1\r\n",
                    "Host: localhost\r\n",
                    "Content-Type: application/json\r\n",
                    "Content-Length: {}\r\n",
                    "\r\n",
                    "{}"
                ),
                body.len(),
                body
            );
            let response = run_http_request(state, config, &request).await;

            assert!(response.starts_with("HTTP/1.1 400 Bad Request"));
            assert!(response.contains("workspace not connected"));
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn http_task_submit_accepts_connected_workspace_thread() {
        run_async_test(async {
            let tmp = make_temp_dir("http-task-submit-connected");
            let state = Arc::new(test_state(&tmp));
            insert_workspace(&state, "ws-http", &tmp.join("workspace").to_string_lossy()).await;
            let session = make_session(make_workspace_entry(
                "ws-http",
                &tmp.join("workspace").to_string_lossy(),
            ));
            state
                .sessions
                .lock()
                .await
                .insert("ws-http".to_string(), Arc::clone(&session));
            let config = Arc::new(DaemonConfig {
                listen: "127.0.0.1:0".parse().expect("listen addr"),
                http_listen: None,
                token: None,
                data_dir: tmp.clone(),
            });

            let session_for_response = Arc::clone(&session);
            let response_task = tokio::spawn(async move {
                loop {
                    if let Some(tx) = session_for_response.pending.lock().await.remove(&0) {
                        tx.send(json!({ "result": { "id": "turn-1" } }))
                            .expect("send mocked daemon response");
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            });

            let body = r#"{"workspaceId":"ws-http","threadId":"thread-http","text":"run remote task","accessMode":"full-access"}"#;
            let request = format!(
                concat!(
                    "POST /api/task/submit HTTP/1.1\r\n",
                    "Host: localhost\r\n",
                    "Content-Type: application/json\r\n",
                    "Content-Length: {}\r\n",
                    "\r\n",
                    "{}"
                ),
                body.len(),
                body
            );
            let response = run_http_request(Arc::clone(&state), config, &request).await;
            response_task.await.expect("join mocked response task");

            assert!(response.starts_with("HTTP/1.1 200 OK"));
            assert!(response.contains("\"accepted\":true"));
            assert!(response.contains("\"workspaceId\":\"ws-http\""));
            assert!(response.contains("\"threadId\":\"thread-http\""));
            assert!(response.contains("\"createdThread\":false"));

            if let Some(session) = state.sessions.lock().await.remove("ws-http") {
                let mut child = session.child.lock().await;
                kill_child_process_tree(&mut child).await;
            }
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn http_v1_task_create_and_get_round_trip() {
        run_async_test(async {
            let tmp = make_temp_dir("http-v1-task-roundtrip");
            let state = Arc::new(test_state(&tmp));
            insert_workspace(&state, "ws-http", &tmp.join("workspace").to_string_lossy()).await;
            let session = make_session(make_workspace_entry(
                "ws-http",
                &tmp.join("workspace").to_string_lossy(),
            ));
            state
                .sessions
                .lock()
                .await
                .insert("ws-http".to_string(), Arc::clone(&session));
            let config = Arc::new(DaemonConfig {
                listen: "127.0.0.1:0".parse().expect("listen addr"),
                http_listen: None,
                token: None,
                data_dir: tmp.clone(),
            });

            let session_for_response = Arc::clone(&session);
            let response_task = tokio::spawn(async move {
                loop {
                    if let Some(tx) = session_for_response.pending.lock().await.remove(&0) {
                        tx.send(json!({ "result": { "id": "turn-1" } }))
                            .expect("send mocked daemon response");
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            });

            let body = r#"{"workspaceId":"ws-http","threadId":"thread-http","text":"run remote task","accessMode":"full-access"}"#;
            let request = format!(
                concat!(
                    "POST /api/v1/tasks HTTP/1.1\r\n",
                    "Host: localhost\r\n",
                    "Content-Type: application/json\r\n",
                    "Content-Length: {}\r\n",
                    "\r\n",
                    "{}"
                ),
                body.len(),
                body
            );
            let create_response = run_http_request(Arc::clone(&state), Arc::clone(&config), &request).await;
            response_task.await.expect("join mocked response task");

            assert!(create_response.starts_with("HTTP/1.1 200 OK"));
            assert!(create_response.contains("\"status\":\"accepted\""));
            assert!(create_response.contains("\"workspaceId\":\"ws-http\""));
            assert!(create_response.contains("\"threadId\":\"thread-http\""));

            let create_body = create_response
                .split("\r\n\r\n")
                .nth(1)
                .expect("http body present");
            let parsed: Value = serde_json::from_str(create_body).expect("valid json body");
            let task_id = parsed
                .get("task")
                .and_then(|task| task.get("taskId"))
                .and_then(Value::as_str)
                .expect("task id")
                .to_string();

            let get_request = format!(
                "GET /api/v1/tasks/{task_id} HTTP/1.1\r\nHost: localhost\r\n\r\n"
            );
            let get_response = run_http_request(Arc::clone(&state), config, &get_request).await;
            assert!(get_response.starts_with("HTTP/1.1 200 OK"));
            assert!(get_response.contains(&format!("\"taskId\":\"{task_id}\"")));
            assert!(get_response.contains("\"status\":\"running\""));
            assert!(get_response.contains("\"turnId\":\"turn-1\""));

            if let Some(session) = state.sessions.lock().await.remove("ws-http") {
                let mut child = session.child.lock().await;
                kill_child_process_tree(&mut child).await;
            }
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn http_conversation_list_returns_stable_envelope() {
        run_async_test(async {
            let tmp = make_temp_dir("http-conversation-list-shape");
            let state = Arc::new(test_state(&tmp));
            state
                .insert_conversation(ServiceConversationRecord {
                    conversation_id: "conv-1".to_string(),
                    workspace_id: "ws-http".to_string(),
                    thread_id: "thread-http".to_string(),
                    title: "Title".to_string(),
                    requirement: "Requirement".to_string(),
                    status: "completed".to_string(),
                    operator: Some("tester".to_string()),
                    created_at_ms: 1,
                    updated_at_ms: 2,
                    last_message_preview: Some("preview".to_string()),
                    final_summary: Some("summary".to_string()),
                    last_error: None,
                })
                .await;
            let config = Arc::new(DaemonConfig {
                listen: "127.0.0.1:0".parse().expect("listen addr"),
                http_listen: None,
                token: None,
                data_dir: tmp.clone(),
            });

            let response = run_http_request(
                state,
                config,
                "GET /api/v1/conversations HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .await;

            assert!(response.starts_with("HTTP/1.1 200 OK"));
            assert!(response.contains("\"ok\":true"));
            assert!(response.contains("\"data\":{\"items\":["));
            assert!(response.contains("\"meta\":{\"count\":1}"));
            assert!(response.contains("\"items\":["));
            assert!(response.contains("\"conversationId\":\"conv-1\""));
            assert!(!response.contains("\"conversations\":["));
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn http_conversation_start_creates_conversation_and_task() {
        run_async_test(async {
            let tmp = make_temp_dir("http-conversation-start");
            let state = Arc::new(test_state(&tmp));
            insert_workspace(&state, "ws-http", &tmp.join("workspace").to_string_lossy()).await;
            let session = make_session(make_workspace_entry(
                "ws-http",
                &tmp.join("workspace").to_string_lossy(),
            ));
            state
                .sessions
                .lock()
                .await
                .insert("ws-http".to_string(), Arc::clone(&session));
            let config = Arc::new(DaemonConfig {
                listen: "127.0.0.1:0".parse().expect("listen addr"),
                http_listen: None,
                token: None,
                data_dir: tmp.clone(),
            });

            let session_for_response = Arc::clone(&session);
            let response_task = tokio::spawn(async move {
                loop {
                    if let Some(tx) = session_for_response.pending.lock().await.remove(&0) {
                        tx.send(json!({ "result": { "threadId": "thread-http" } }))
                            .expect("send mocked start_thread response");
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                loop {
                    if let Some(tx) = session_for_response.pending.lock().await.remove(&1) {
                        tx.send(json!({ "result": { "id": "turn-1" } }))
                            .expect("send mocked send_user_message response");
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            });

            let body = r#"{"workspaceId":"ws-http","title":"Need summary","requirement":"Read repo and summarize.","accessMode":"full-access"}"#;
            let request = format!(
                concat!(
                    "POST /api/v1/conversations/start HTTP/1.1\r\n",
                    "Host: localhost\r\n",
                    "Content-Type: application/json\r\n",
                    "Content-Length: {}\r\n",
                    "\r\n",
                    "{}"
                ),
                body.len(),
                body
            );
            let response = run_http_request(Arc::clone(&state), Arc::clone(&config), &request).await;
            response_task.await.expect("join mocked response task");

            assert!(response.starts_with("HTTP/1.1 202 Accepted"));
            assert!(response.contains("\"ok\":true"));
            assert!(response.contains("\"conversationId\":\""));
            assert!(response.contains("\"taskId\":\""));

            let response_body = response
                .split("\r\n\r\n")
                .nth(1)
                .expect("http body present");
            let parsed: Value = serde_json::from_str(response_body).expect("valid json body");
            let conversation_id = parsed
                .get("data")
                .and_then(|data| data.get("conversation"))
                .and_then(|conversation| conversation.get("conversationId"))
                .and_then(Value::as_str)
                .expect("conversation id");
            let task_id = parsed
                .get("data")
                .and_then(|data| data.get("task"))
                .and_then(|task| task.get("taskId"))
                .and_then(Value::as_str)
                .expect("task id");

            let conversation = state
                .get_conversation(conversation_id)
                .await
                .expect("conversation persisted");
            assert_eq!(conversation.title, "Need summary");
            assert_eq!(conversation.requirement, "Read repo and summarize.");
            assert_eq!(conversation.status, "thread_created");
            assert_eq!(conversation.thread_id, "thread-http");

            let task = state.get_task(task_id).await.expect("task persisted");
            assert_eq!(task.workspace_id, "ws-http");
            assert_eq!(task.thread_id, "thread-http");
            assert_eq!(task.turn_id.as_deref(), Some("turn-1"));
            assert_eq!(task.status, "running");
            assert!(task.created_thread);

            let detail_request = format!(
                "GET /api/v1/conversations/{conversation_id} HTTP/1.1\r\nHost: localhost\r\n\r\n"
            );
            let detail_response = run_http_request(Arc::clone(&state), config, &detail_request).await;
            assert!(detail_response.starts_with("HTTP/1.1 200 OK"));
            assert!(detail_response.contains("\"ok\":true"));
            assert!(detail_response.contains("\"data\":{\"conversation\":{"));
            assert!(detail_response.contains(&format!("\"conversationId\":\"{conversation_id}\"")));
            assert!(detail_response.contains("\"threadId\":\"thread-http\""));

            if let Some(session) = state.sessions.lock().await.remove("ws-http") {
                let mut child = session.child.lock().await;
                kill_child_process_tree(&mut child).await;
            }
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn http_conversation_message_creates_follow_up_task_and_updates_preview() {
        run_async_test(async {
            let tmp = make_temp_dir("http-conversation-message");
            let state = Arc::new(test_state(&tmp));
            insert_workspace(&state, "ws-http", &tmp.join("workspace").to_string_lossy()).await;
            state
                .insert_conversation(ServiceConversationRecord {
                    conversation_id: "conv-1".to_string(),
                    workspace_id: "ws-http".to_string(),
                    thread_id: "thread-http".to_string(),
                    title: "Title".to_string(),
                    requirement: "Requirement".to_string(),
                    status: "completed".to_string(),
                    operator: Some("tester".to_string()),
                    created_at_ms: 1,
                    updated_at_ms: 2,
                    last_message_preview: Some("preview".to_string()),
                    final_summary: Some("summary".to_string()),
                    last_error: Some("old error".to_string()),
                })
                .await;
            let session = make_session(make_workspace_entry(
                "ws-http",
                &tmp.join("workspace").to_string_lossy(),
            ));
            state
                .sessions
                .lock()
                .await
                .insert("ws-http".to_string(), Arc::clone(&session));
            let config = Arc::new(DaemonConfig {
                listen: "127.0.0.1:0".parse().expect("listen addr"),
                http_listen: None,
                token: None,
                data_dir: tmp.clone(),
            });

            let session_for_response = Arc::clone(&session);
            let response_task = tokio::spawn(async move {
                loop {
                    if let Some(tx) = session_for_response.pending.lock().await.remove(&0) {
                        tx.send(json!({ "result": { "id": "turn-2" } }))
                            .expect("send mocked follow-up response");
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            });

            let body = r#"{"text":"Need a shorter answer.","accessMode":"full-access"}"#;
            let request = format!(
                concat!(
                    "POST /api/v1/conversations/conv-1/messages HTTP/1.1\r\n",
                    "Host: localhost\r\n",
                    "Content-Type: application/json\r\n",
                    "Content-Length: {}\r\n",
                    "\r\n",
                    "{}"
                ),
                body.len(),
                body
            );
            let response = run_http_request(Arc::clone(&state), config, &request).await;
            response_task.await.expect("join mocked response task");

            assert!(response.starts_with("HTTP/1.1 202 Accepted"));
            assert!(response.contains("\"ok\":true"));
            assert!(response.contains("\"conversationId\":\"conv-1\""));
            assert!(response.contains("\"taskId\":\""));

            let response_body = response
                .split("\r\n\r\n")
                .nth(1)
                .expect("http body present");
            let parsed: Value = serde_json::from_str(response_body).expect("valid json body");
            let task_id = parsed
                .get("data")
                .and_then(|data| data.get("task"))
                .and_then(|task| task.get("taskId"))
                .and_then(Value::as_str)
                .expect("task id");

            let conversation = state
                .get_conversation("conv-1")
                .await
                .expect("conversation persisted");
            assert_eq!(
                conversation.last_message_preview.as_deref(),
                Some("Need a shorter answer.")
            );
            assert_eq!(conversation.status, "accepted");
            assert_eq!(conversation.last_error, None);

            let task = state.get_task(task_id).await.expect("task persisted");
            assert_eq!(task.thread_id, "thread-http");
            assert_eq!(task.turn_id.as_deref(), Some("turn-2"));
            assert_eq!(task.status, "running");
            assert!(!task.created_thread);

            if let Some(session) = state.sessions.lock().await.remove("ws-http") {
                let mut child = session.child.lock().await;
                kill_child_process_tree(&mut child).await;
            }
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn http_conversation_messages_reads_from_conversation_service() {
        run_async_test(async {
            let tmp = make_temp_dir("http-conversation-messages");
            let state = Arc::new(test_state(&tmp));
            insert_workspace(&state, "ws-http", &tmp.join("workspace").to_string_lossy()).await;
            state
                .insert_conversation(ServiceConversationRecord {
                    conversation_id: "conv-1".to_string(),
                    workspace_id: "ws-http".to_string(),
                    thread_id: "thread-http".to_string(),
                    title: "Title".to_string(),
                    requirement: "Requirement".to_string(),
                    status: "completed".to_string(),
                    operator: Some("tester".to_string()),
                    created_at_ms: 1,
                    updated_at_ms: 2,
                    last_message_preview: Some("preview".to_string()),
                    final_summary: Some("summary".to_string()),
                    last_error: None,
                })
                .await;
            let session = make_session(make_workspace_entry(
                "ws-http",
                &tmp.join("workspace").to_string_lossy(),
            ));
            state
                .sessions
                .lock()
                .await
                .insert("ws-http".to_string(), Arc::clone(&session));
            let config = Arc::new(DaemonConfig {
                listen: "127.0.0.1:0".parse().expect("listen addr"),
                http_listen: None,
                token: None,
                data_dir: tmp.clone(),
            });

            let session_for_response = Arc::clone(&session);
            let response_task = tokio::spawn(async move {
                loop {
                    if let Some(tx) = session_for_response.pending.lock().await.remove(&0) {
                        tx.send(json!({
                            "items": [
                                { "id": "msg-1", "type": "user", "text": "hello" }
                            ]
                        }))
                        .expect("send mocked daemon response");
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            });

            let response = run_http_request(
                Arc::clone(&state),
                config,
                "GET /api/v1/conversations/conv-1/messages HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .await;
            response_task.await.expect("join mocked response task");

            assert!(response.starts_with("HTTP/1.1 200 OK"));
            assert!(response.contains("\"ok\":true"));
            assert!(response.contains("\"data\":{\"conversation\":{"));
            assert!(response.contains("\"messages\":[{\"id\":\"msg-1\""));
            assert!(response.contains("\"meta\":{\"count\":1}"));

            if let Some(session) = state.sessions.lock().await.remove("ws-http") {
                let mut child = session.child.lock().await;
                kill_child_process_tree(&mut child).await;
            }
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn http_conversation_messages_falls_back_to_session_jsonl() {
        run_async_test(async {
            let tmp = make_temp_dir("http-conversation-messages-jsonl");
            let state = Arc::new(test_state(&tmp));
            insert_workspace(&state, "ws-http", &tmp.join("workspace").to_string_lossy()).await;
            state
                .insert_conversation(ServiceConversationRecord {
                    conversation_id: "conv-1".to_string(),
                    workspace_id: "ws-http".to_string(),
                    thread_id: "thread-http".to_string(),
                    title: "Title".to_string(),
                    requirement: "Requirement".to_string(),
                    status: "completed".to_string(),
                    operator: Some("tester".to_string()),
                    created_at_ms: 1,
                    updated_at_ms: 2,
                    last_message_preview: Some("preview".to_string()),
                    final_summary: Some("summary".to_string()),
                    last_error: None,
                })
                .await;
            let session = make_session(make_workspace_entry(
                "ws-http",
                &tmp.join("workspace").to_string_lossy(),
            ));
            state
                .sessions
                .lock()
                .await
                .insert("ws-http".to_string(), Arc::clone(&session));
            let config = Arc::new(DaemonConfig {
                listen: "127.0.0.1:0".parse().expect("listen addr"),
                http_listen: None,
                token: None,
                data_dir: tmp.clone(),
            });
            let session_path = tmp.join("session.jsonl");
            std::fs::write(
                &session_path,
                concat!(
                    "{\"timestamp\":\"2026-07-08T09:43:16.970Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"turn-1\"}}\n",
                    "{\"timestamp\":\"2026-07-08T09:43:17.110Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"hello\"}],\"internal_chat_message_metadata_passthrough\":{\"turn_id\":\"turn-1\"}}}\n",
                    "{\"timestamp\":\"2026-07-08T09:43:17.246Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"world\"}],\"phase\":\"final_answer\",\"internal_chat_message_metadata_passthrough\":{\"turn_id\":\"turn-1\"}}}\n",
                    "{\"timestamp\":\"2026-07-08T09:43:17.300Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"ignore me\"}]}}\n"
                ),
            )
            .expect("write session jsonl");

            let session_for_response = Arc::clone(&session);
            let session_path_string = session_path.to_string_lossy().to_string();
            let response_task = tokio::spawn(async move {
                loop {
                    if let Some(tx) = session_for_response.pending.lock().await.remove(&0) {
                        tx.send(json!({
                            "result": {
                                "thread": {
                                    "id": "thread-http",
                                    "path": session_path_string,
                                    "turns": []
                                }
                            }
                        }))
                        .expect("send mocked daemon response");
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            });

            let response = run_http_request(
                Arc::clone(&state),
                config,
                "GET /api/v1/conversations/conv-1/messages HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .await;
            response_task.await.expect("join mocked response task");

            assert!(response.starts_with("HTTP/1.1 200 OK"));
            assert!(response.contains("\"ok\":true"));
            assert!(response.contains("\"messages\":["));
            assert!(response.contains("\"type\":\"user\""));
            assert!(response.contains("\"text\":\"hello\""));
            assert!(response.contains("\"type\":\"assistant\""));
            assert!(response.contains("\"text\":\"world\""));
            assert!(response.contains("\"meta\":{\"count\":2}"));
            assert!(!response.contains("ignore me"));

            if let Some(session) = state.sessions.lock().await.remove("ws-http") {
                let mut child = session.child.lock().await;
                kill_child_process_tree(&mut child).await;
            }
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn http_conversation_missing_returns_stable_error_shape() {
        run_async_test(async {
            let tmp = make_temp_dir("http-conversation-missing");
            let state = Arc::new(test_state(&tmp));
            let config = Arc::new(DaemonConfig {
                listen: "127.0.0.1:0".parse().expect("listen addr"),
                http_listen: None,
                token: None,
                data_dir: tmp.clone(),
            });

            let response = run_http_request(
                state,
                config,
                "GET /api/v1/conversations/missing HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .await;

            assert!(response.starts_with("HTTP/1.1 404 Not Found"));
            assert!(response.contains("\"ok\":false"));
            assert!(response.contains("\"code\":\"conversation_not_found\""));
            assert!(response.contains("\"message\":\"conversation not found\""));
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn task_status_transitions_from_app_server_events() {
        run_async_test(async {
            let tmp = make_temp_dir("task-status-events");
            let state = test_state(&tmp);
            let task = state
                .insert_task(ServiceTaskRecord {
                    task_id: "task-1".to_string(),
                    status: "accepted".to_string(),
                    workspace_id: "ws-http".to_string(),
                    thread_id: "thread-http".to_string(),
                    turn_id: Some("turn-1".to_string()),
                    created_thread: false,
                    submitted_at_ms: 1,
                    completed_at_ms: None,
                    last_error: None,
                })
                .await;

            state
                .process_task_event(&AppServerEvent {
                    workspace_id: "ws-http".to_string(),
                    message: json!({
                        "method": "turn/started",
                        "params": {
                            "threadId": "thread-http",
                            "turnId": "turn-1",
                        }
                    }),
                })
                .await;
            let running = state.get_task(&task.task_id).await.expect("task present");
            assert_eq!(running.status, "running");

            state
                .process_task_event(&AppServerEvent {
                    workspace_id: "ws-http".to_string(),
                    message: json!({
                        "method": "turn/completed",
                        "params": {
                            "threadId": "thread-http",
                            "turnId": "turn-1",
                        }
                    }),
                })
                .await;
            let completed = state.get_task(&task.task_id).await.expect("task present");
            assert_eq!(completed.status, "completed");
            assert!(completed.completed_at_ms.is_some());

            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn app_server_event_thread_stream_filter_scopes_workspace_and_thread() {
        let matching = AppServerEvent {
            workspace_id: "ws-1".to_string(),
            message: json!({
                "method": "turn/started",
                "params": {
                    "threadId": "thread-1",
                }
            }),
        };
        let wrong_thread = AppServerEvent {
            workspace_id: "ws-1".to_string(),
            message: json!({
                "method": "turn/started",
                "params": {
                    "threadId": "thread-2",
                }
            }),
        };
        let wrong_workspace = AppServerEvent {
            workspace_id: "ws-2".to_string(),
            message: json!({
                "method": "turn/started",
                "params": {
                    "threadId": "thread-1",
                }
            }),
        };

        assert!(app_server_event_matches_thread_stream(
            &matching,
            "ws-1",
            Some("thread-1"),
        ));
        assert!(!app_server_event_matches_thread_stream(
            &wrong_thread,
            "ws-1",
            Some("thread-1"),
        ));
        assert!(!app_server_event_matches_thread_stream(
            &wrong_workspace,
            "ws-1",
            Some("thread-1"),
        ));
        assert!(app_server_event_matches_thread_stream(&matching, "ws-1", None));
    }

    #[test]
    fn conversation_status_maps_request_user_input_event() {
        let status = map_conversation_status(
            "item/tool/requestUserInput",
            &json!({
                "method": "item/tool/requestUserInput",
                "id": 1,
                "params": {
                    "threadId": "thread-1",
                }
            }),
        );

        assert_eq!(status.as_deref(), Some("waiting_input"));
    }

    #[test]
    fn conversation_status_maps_request_approval_event() {
        let status = map_conversation_status(
            "item/permissions/requestApproval",
            &json!({
                "method": "item/permissions/requestApproval",
                "id": 1,
                "params": {
                    "threadId": "thread-1",
                }
            }),
        );

        assert_eq!(status.as_deref(), Some("waiting_approval"));
    }

    #[test]
    fn conversation_status_maps_item_started_event() {
        let status = map_conversation_status(
            "item/started",
            &json!({
                "method": "item/started",
                "params": {
                    "threadId": "thread-1",
                    "item": {
                        "id": "item-1",
                        "type": "agentMessage",
                    }
                }
            }),
        );

        assert_eq!(status.as_deref(), Some("running"));
    }

    #[test]
    fn conversation_status_maps_streaming_delta_event() {
        let status = map_conversation_status(
            "item/agentMessage/delta",
            &json!({
                "method": "item/agentMessage/delta",
                "params": {
                    "threadId": "thread-1",
                    "delta": "hello",
                }
            }),
        );

        assert_eq!(status.as_deref(), Some("streaming"));
    }

    #[test]
    fn conversation_status_maps_thread_status_changed_event() {
        let running = map_conversation_status(
            "thread/status/changed",
            &json!({
                "method": "thread/status/changed",
                "params": {
                    "threadId": "thread-1",
                    "status": { "type": "active" }
                }
            }),
        );
        let failed = map_conversation_status(
            "thread/status/changed",
            &json!({
                "method": "thread/status/changed",
                "params": {
                    "threadId": "thread-1",
                    "status": { "type": "systemError" }
                }
            }),
        );

        assert_eq!(running.as_deref(), Some("running"));
        assert_eq!(failed.as_deref(), Some("failed"));
    }

    #[test]
    fn conversation_event_updates_summary_from_completed_assistant_message() {
        run_async_test(async {
            let tmp = make_temp_dir("conversation-summary-from-assistant");
            let state = test_state(&tmp);
            state
                .insert_conversation(ServiceConversationRecord {
                    conversation_id: "conv-1".to_string(),
                    workspace_id: "ws-http".to_string(),
                    thread_id: "thread-http".to_string(),
                    title: "Title".to_string(),
                    requirement: "Requirement".to_string(),
                    status: "running".to_string(),
                    operator: Some("tester".to_string()),
                    created_at_ms: 1,
                    updated_at_ms: 1,
                    last_message_preview: None,
                    final_summary: None,
                    last_error: None,
                })
                .await;

            state
                .process_conversation_event(&AppServerEvent {
                    workspace_id: "ws-http".to_string(),
                    message: json!({
                        "method": "item/completed",
                        "params": {
                            "threadId": "thread-http",
                            "item": {
                                "type": "agentMessage",
                                "id": "item-1",
                                "text": "assistant final output"
                            }
                        }
                    }),
                })
                .await;

            let conversation = state
                .get_conversation("conv-1")
                .await
                .expect("conversation present");
            assert_eq!(
                conversation.last_message_preview.as_deref(),
                Some("assistant final output")
            );
            assert_eq!(
                conversation.final_summary.as_deref(),
                Some("assistant final output")
            );

            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn service_tasks_round_trip_to_disk() {
        let tmp = make_temp_dir("service-tasks-persist");
        let path = tmp.join("service_tasks.json");
        let mut tasks = HashMap::new();
        tasks.insert(
            "task-1".to_string(),
            ServiceTaskRecord {
                task_id: "task-1".to_string(),
                status: "running".to_string(),
                workspace_id: "ws-1".to_string(),
                thread_id: "thread-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                created_thread: false,
                submitted_at_ms: 10,
                completed_at_ms: None,
                last_error: None,
            },
        );

        write_service_tasks(&path, &tasks).expect("persist tasks");
        let loaded = load_service_tasks(&path);
        let loaded_task = loaded.get("task-1").expect("task present");
        assert_eq!(loaded_task.status, "failed");
        assert_eq!(loaded_task.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(
            loaded_task.last_error.as_deref(),
            Some("daemon restarted before task completion")
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn service_conversations_round_trip_to_disk() {
        let tmp = make_temp_dir("service-conversations-persist");
        let path = tmp.join("service_conversations.json");
        let mut conversations = HashMap::new();
        conversations.insert(
            "conv-1".to_string(),
            ServiceConversationRecord {
                conversation_id: "conv-1".to_string(),
                workspace_id: "ws-1".to_string(),
                thread_id: "thread-1".to_string(),
                title: "Title".to_string(),
                requirement: "Requirement".to_string(),
                status: "completed".to_string(),
                operator: Some("tester".to_string()),
                created_at_ms: 10,
                updated_at_ms: 20,
                last_message_preview: Some("preview".to_string()),
                final_summary: Some("summary".to_string()),
                last_error: None,
            },
        );

        write_service_conversations(&path, &conversations).expect("persist conversations");
        let loaded = load_service_conversations(&path);
        let loaded_conversation = loaded.get("conv-1").expect("conversation present");
        assert_eq!(loaded_conversation.thread_id, "thread-1");
        assert_eq!(loaded_conversation.status, "completed");
        assert_eq!(loaded_conversation.final_summary.as_deref(), Some("summary"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rpc_add_clone_uses_workspace_core_validation() {
        run_async_test(async {
            let tmp = make_temp_dir("rpc-add-clone");
            let state = test_state(&tmp);

            let err = rpc::handle_rpc_request(
                &state,
                "add_clone",
                json!({
                    "sourceWorkspaceId": "source",
                    "copiesFolder": tmp.to_string_lossy().to_string(),
                    "copyName": "   "
                }),
                "daemon-test".to_string(),
            )
            .await
            .expect_err("expected validation error");

            assert_eq!(err, "Copy name is required.");
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn rpc_prompts_list_reads_workspace_prompts() {
        run_async_test(async {
            let tmp = make_temp_dir("rpc-prompts-list");
            let workspace_id = "ws-prompts";
            let workspace_dir = tmp.join("workspace");
            std::fs::create_dir_all(&workspace_dir).expect("create workspace dir");

            let state = test_state(&tmp);
            insert_workspace(&state, workspace_id, &workspace_dir.to_string_lossy()).await;

            let prompts_dir = tmp.join("workspaces").join(workspace_id).join("prompts");
            std::fs::create_dir_all(&prompts_dir).expect("create prompts dir");
            std::fs::write(prompts_dir.join("review.md"), "Prompt body").expect("write prompt");

            let result = rpc::handle_rpc_request(
                &state,
                "prompts_list",
                json!({ "workspaceId": workspace_id }),
                "daemon-test".to_string(),
            )
            .await
            .expect("prompts_list should succeed");

            let prompts = result.as_array().expect("array result");
            assert!(
                prompts.iter().any(|entry| {
                    entry
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| name == "review")
                }),
                "expected prompts_list to include workspace prompt"
            );
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn rpc_local_usage_snapshot_returns_snapshot_shape() {
        run_async_test(async {
            let tmp = make_temp_dir("rpc-local-usage");
            let state = test_state(&tmp);

            let result = rpc::handle_rpc_request(
                &state,
                "local_usage_snapshot",
                json!({ "days": 7 }),
                "daemon-test".to_string(),
            )
            .await
            .expect("local_usage_snapshot should succeed");

            assert!(result.get("days").and_then(Value::as_array).is_some());
            assert!(result.get("totals").is_some());
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn rpc_daemon_info_reports_identity() {
        run_async_test(async {
            let tmp = make_temp_dir("rpc-daemon-info");
            let state = test_state(&tmp);

            let result = rpc::handle_rpc_request(
                &state,
                "daemon_info",
                json!({}),
                "daemon-test".to_string(),
            )
            .await
            .expect("daemon_info should succeed");

            assert_eq!(
                result.get("name").and_then(Value::as_str),
                Some(DAEMON_NAME)
            );
            assert_eq!(result.get("mode").and_then(Value::as_str), Some("tcp"));
            assert_eq!(
                result.get("version").and_then(Value::as_str),
                Some(env!("CARGO_PKG_VERSION"))
            );
            let _ = std::fs::remove_dir_all(&tmp);
        });
    }
    #[test]
    fn list_workspaces_syncs_from_storage_file() {
        run_async_test(async {
            let tmp = make_temp_dir("list-workspaces-sync");
            let state = test_state(&tmp);

            let persisted = vec![WorkspaceEntry {
                id: "ws-sync".to_string(),
                name: "Synced Workspace".to_string(),
                path: tmp.join("workspace").to_string_lossy().to_string(),
                kind: WorkspaceKind::Main,
                parent_id: None,
                worktree: None,
                settings: WorkspaceSettings::default(),
            }];
            write_workspaces(&state.storage_path, &persisted).expect("write workspaces");

            let listed = state.list_workspaces().await;
            assert!(
                listed.iter().any(|workspace| workspace.id == "ws-sync"),
                "expected daemon list_workspaces to include workspace added on disk"
            );

            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn list_workspaces_sync_prunes_stale_sessions() {
        run_async_test(async {
            let tmp = make_temp_dir("list-workspaces-sync-prune");
            let state = test_state(&tmp);
            let keep_path = tmp.join("workspace-keep");
            let stale_path = tmp.join("workspace-stale");

            let persisted = vec![make_workspace_entry(
                "ws-keep",
                &keep_path.to_string_lossy(),
            )];
            write_workspaces(&state.storage_path, &persisted).expect("write workspaces");

            let keep_session = make_session(make_workspace_entry(
                "ws-keep",
                &keep_path.to_string_lossy(),
            ));
            let stale_session = make_session(make_workspace_entry(
                "ws-stale",
                &stale_path.to_string_lossy(),
            ));
            {
                let mut sessions = state.sessions.lock().await;
                sessions.insert("ws-keep".to_string(), keep_session);
                sessions.insert("ws-stale".to_string(), stale_session.clone());
            }

            let listed = state.list_workspaces().await;
            assert!(
                listed.iter().any(|workspace| workspace.id == "ws-keep"),
                "expected daemon list_workspaces to include persisted workspace"
            );

            {
                let sessions = state.sessions.lock().await;
                assert!(
                    sessions.contains_key("ws-keep"),
                    "expected connected persisted workspace session to remain"
                );
                assert!(
                    !sessions.contains_key("ws-stale"),
                    "expected stale session to be removed"
                );
            }

            let stale_session_exited = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let exited = stale_session
                        .child
                        .lock()
                        .await
                        .try_wait()
                        .expect("query stale session child");
                    if exited.is_some() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await
            .is_ok();
            assert!(
                stale_session_exited,
                "expected stale session child process to terminate"
            );

            if let Some(keep_session) = state.sessions.lock().await.remove("ws-keep") {
                let mut child = keep_session.child.lock().await;
                kill_child_process_tree(&mut child).await;
            }

            if stale_session
                .child
                .lock()
                .await
                .try_wait()
                .expect("query stale session child")
                .is_none()
            {
                let mut child = stale_session.child.lock().await;
                kill_child_process_tree(&mut child).await;
            }

            let _ = std::fs::remove_dir_all(&tmp);
        });
    }

    #[test]
    fn pending_mysql_writes_roundtrip() {
        let tmp = make_temp_dir("mysql-retry-roundtrip");
        let path = tmp.join("service_mysql_retry.json");
        let mut writes = HashMap::new();
        writes.insert(
            "message:0001".to_string(),
            PendingMySqlWrite {
                key: "message:0001".to_string(),
                kind: "message".to_string(),
                sql: "INSERT INTO conversation_message VALUES (1)".to_string(),
                created_at_ms: 100,
                updated_at_ms: 200,
                attempt_count: 3,
                last_error: Some("mysql down".to_string()),
            },
        );

        write_pending_mysql_writes(&path, &writes).expect("write retry queue");
        let loaded = load_pending_mysql_writes(&path);
        let record = loaded.get("message:0001").expect("loaded retry record");
        assert_eq!(record.kind, "message");
        assert_eq!(record.attempt_count, 3);
        assert_eq!(record.last_error.as_deref(), Some("mysql down"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn enqueue_mysql_retry_overwrites_and_increments_attempts() {
        run_async_test(async {
            let tmp = make_temp_dir("mysql-retry-enqueue");
            let state = test_state(&tmp);
            state
                .enqueue_mysql_retry(
                    "message:0001".to_string(),
                    "message",
                    "INSERT INTO a VALUES (1)".to_string(),
                    "first".to_string(),
                )
                .await;
            state
                .enqueue_mysql_retry(
                    "message:0001".to_string(),
                    "message",
                    "INSERT INTO a VALUES (2)".to_string(),
                    "second".to_string(),
                )
                .await;

            let queue = state.mysql_retry_queue.lock().await;
            let record = queue.get("message:0001").expect("retry record");
            assert_eq!(record.sql, "INSERT INTO a VALUES (2)");
            assert_eq!(record.attempt_count, 2);
            assert_eq!(record.last_error.as_deref(), Some("second"));

            let _ = std::fs::remove_dir_all(&tmp);
        });
    }
}

fn main() {
    let config = match parse_args() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("{err}\n\n{}", usage());
            std::process::exit(2);
        }
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    runtime.block_on(async move {
        let (events_tx, _events_rx) = broadcast::channel::<DaemonEvent>(2048);
        let event_sink = DaemonEventSink {
            tx: events_tx.clone(),
        };
        let state = Arc::new(DaemonState::load(&config, event_sink));
        state.reconcile_history_projection().await;
        state.flush_mysql_retry_queue().await;
        let config = Arc::new(config);
        tokio::spawn(Arc::clone(&state).run_mysql_retry_worker());
        tokio::spawn(forward_task_state_updates(
            Arc::clone(&state),
            events_tx.subscribe(),
        ));

        let listener = match TcpListener::bind(config.listen).await {
            Ok(listener) => listener,
            Err(err) => {
                eprintln!("failed to bind {}: {err}", config.listen);
                std::process::exit(2);
            }
        };
        eprintln!(
            "codex-monitor-daemon listening on {} (data dir: {})",
            config.listen,
            state
                .storage_path
                .parent()
                .unwrap_or(&state.storage_path)
                .display()
        );

        if let Some(http_listen) = config.http_listen {
            let http_state = Arc::clone(&state);
            let http_config = Arc::clone(&config);
            tokio::spawn(async move {
                let listener = match TcpListener::bind(http_listen).await {
                    Ok(listener) => listener,
                    Err(err) => {
                        eprintln!("failed to bind HTTP bridge {}: {err}", http_listen);
                        std::process::exit(2);
                    }
                };
                eprintln!("codex-monitor-daemon HTTP bridge listening on {}", http_listen);
                loop {
                    match listener.accept().await {
                        Ok((socket, _addr)) => {
                            let state = Arc::clone(&http_state);
                            let config = Arc::clone(&http_config);
                            tokio::spawn(async move {
                                handle_http_request(state, config, socket).await;
                            });
                        }
                        Err(_) => continue,
                    }
                }
            });
        }

        loop {
            match listener.accept().await {
                Ok((socket, _addr)) => {
                    let config = Arc::clone(&config);
                    let state = Arc::clone(&state);
                    let events = events_tx.clone();
                    tokio::spawn(async move {
                        transport::handle_client(socket, config, state, events).await;
                    });
                }
                Err(_) => continue,
            }
        }
    });
}
