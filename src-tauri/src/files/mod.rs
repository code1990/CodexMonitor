use serde_json::json;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};

use self::io::TextFileResponse;
use self::policy::{FileKind, FileScope};
use crate::remote_backend;
use crate::shared::codex_core;
use crate::shared::files_core::{file_read_core, file_write_core};
use crate::state::AppState;

pub(crate) mod io;
pub(crate) mod ops;
pub(crate) mod policy;

fn sanitize_user_path(path: &str) -> String {
    path.chars()
        .filter(|ch| {
            !matches!(
                ch,
                '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'
                    | '\u{202b}'
                    | '\u{202c}'
                    | '\u{202d}'
                    | '\u{202e}'
                    | '\u{2066}'
                    | '\u{2067}'
                    | '\u{2068}'
                    | '\u{2069}'
            )
        })
        .collect::<String>()
        .trim()
        .trim_matches(|ch| ch == '"' || ch == '\'')
        .trim()
        .to_string()
}

fn ensure_unique_destination_path(destination: &Path) -> PathBuf {
    if !destination.exists() {
        return destination.to_path_buf();
    }

    let parent = destination
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let stem = destination
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("file");
    let extension = destination.extension().and_then(|value| value.to_str());

    for index in 1.. {
        let candidate_name = match extension {
            Some(ext) if !ext.is_empty() => format!("{stem} ({index}).{ext}"),
            _ => format!("{stem} ({index})"),
        };
        let candidate = parent.join(candidate_name);
        if !candidate.exists() {
            return candidate;
        }
    }

    destination.to_path_buf()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DirectoryTextFileEntry {
    pub name: String,
    pub path: String,
    pub content: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DirectoryFileEntry {
    pub name: String,
    pub path: String,
}

async fn file_read_impl(
    scope: FileScope,
    kind: FileKind,
    workspace_id: Option<String>,
    state: &AppState,
    app: &AppHandle,
) -> Result<TextFileResponse, String> {
    if remote_backend::is_remote_mode(state).await {
        let response = remote_backend::call_remote(
            state,
            app.clone(),
            "file_read",
            json!({ "scope": scope, "kind": kind, "workspaceId": workspace_id }),
        )
        .await?;
        return serde_json::from_value(response).map_err(|err| err.to_string());
    }

    file_read_core(&state.workspaces, scope, kind, workspace_id).await
}

async fn file_write_impl(
    scope: FileScope,
    kind: FileKind,
    workspace_id: Option<String>,
    content: String,
    state: &AppState,
    app: &AppHandle,
) -> Result<(), String> {
    if remote_backend::is_remote_mode(state).await {
        remote_backend::call_remote(
            state,
            app.clone(),
            "file_write",
            json!({
                "scope": scope,
                "kind": kind,
                "workspaceId": workspace_id,
                "content": content,
            }),
        )
        .await?;
        return Ok(());
    }

    file_write_core(&state.workspaces, scope, kind, workspace_id, content).await
}

#[tauri::command]
pub(crate) async fn file_read(
    scope: FileScope,
    kind: FileKind,
    workspace_id: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<TextFileResponse, String> {
    file_read_impl(scope, kind, workspace_id, &*state, &app).await
}

#[tauri::command]
pub(crate) async fn file_write(
    scope: FileScope,
    kind: FileKind,
    workspace_id: Option<String>,
    content: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    file_write_impl(scope, kind, workspace_id, content, &*state, &app).await
}

#[tauri::command]
pub(crate) async fn read_image_as_data_url(
    path: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<String, String> {
    let trimmed_path = path.trim();
    if trimmed_path.is_empty() {
        return Err("Image path is required".to_string());
    }

    let mobile_runtime = cfg!(any(target_os = "ios", target_os = "android"));
    let remote_mode = remote_backend::is_remote_mode(&*state).await;
    if !mobile_runtime && !remote_mode {
        return Err("Image conversion is only supported in remote backend mode or on mobile runtimes".to_string());
    }

    let normalized = codex_core::normalize_file_path(trimmed_path);
    if normalized.is_empty() {
        return Err("Image path is required".to_string());
    }

    let _ = app;
    codex_core::read_image_as_data_url_core(&normalized)
}

#[tauri::command]
pub(crate) fn write_text_file(path: String, content: String) -> Result<(), String> {
    let sanitized_path = sanitize_user_path(&path);
    let target = PathBuf::from(&sanitized_path);
    if target.as_os_str().is_empty() {
        return Err("Path is required".to_string());
    }
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("Failed to create export directory: {err}"))?;
        }
    }
    std::fs::write(&target, content).map_err(|err| format!("Failed to write export file: {err}"))
}

#[tauri::command]
pub(crate) fn move_text_file(source_path: String, destination_path: String) -> Result<String, String> {
    let sanitized_source = sanitize_user_path(&source_path);
    let sanitized_destination = sanitize_user_path(&destination_path);
    let source = PathBuf::from(&sanitized_source);
    let requested_destination = PathBuf::from(&sanitized_destination);
    if source.as_os_str().is_empty() || requested_destination.as_os_str().is_empty() {
        return Err("Source and destination paths are required".to_string());
    }
    if !source.is_file() {
        return Err("Source file does not exist".to_string());
    }

    let target = ensure_unique_destination_path(&requested_destination);
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("Failed to create destination directory: {err}"))?;
        }
    }

    match std::fs::rename(&source, &target) {
        Ok(()) => Ok(target.to_string_lossy().to_string()),
        Err(_) => {
            std::fs::copy(&source, &target)
                .map_err(|err| format!("Failed to copy source file: {err}"))?;
            std::fs::remove_file(&source)
                .map_err(|err| format!("Failed to remove original source file: {err}"))?;
            Ok(target.to_string_lossy().to_string())
        }
    }
}

#[tauri::command]
pub(crate) fn read_text_file(path: String) -> Result<String, String> {
    let sanitized_path = sanitize_user_path(&path);
    let target = PathBuf::from(&sanitized_path);
    if target.as_os_str().is_empty() {
        return Err("Path is required".to_string());
    }
    std::fs::read_to_string(&target).map_err(|err| format!("Failed to read file: {err}"))
}

#[tauri::command]
pub(crate) fn list_text_files_in_directory(
    directory: String,
    extensions: Vec<String>,
) -> Result<Vec<DirectoryTextFileEntry>, String> {
    let sanitized_directory = sanitize_user_path(&directory);
    let dir = PathBuf::from(&sanitized_directory);
    if dir.as_os_str().is_empty() {
        return Err("Directory is required".to_string());
    }
    if !dir.is_dir() {
        return Err("Selected path is not a directory".to_string());
    }

    let allowed_extensions: Vec<String> = extensions
        .into_iter()
        .map(|value| value.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect();

    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|err| format!("Failed to list directory: {err}"))? {
        let entry = entry.map_err(|err| format!("Failed to read directory entry: {err}"))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let extension = match path.extension().and_then(|value| value.to_str()) {
            Some(value) => value.to_ascii_lowercase(),
            None => continue,
        };
        if !allowed_extensions.is_empty() && !allowed_extensions.iter().any(|value| value == &extension)
        {
            continue;
        }
        let content =
            std::fs::read_to_string(&path).map_err(|err| format!("Failed to read file: {err}"))?;
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "Failed to read file name".to_string())?
            .to_string();
        entries.push(DirectoryTextFileEntry {
            name,
            path: path.to_string_lossy().to_string(),
            content,
        });
    }

    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(entries)
}

#[tauri::command]
pub(crate) fn list_text_file_names_in_directory(
    directory: String,
    extensions: Vec<String>,
) -> Result<Vec<DirectoryFileEntry>, String> {
    let sanitized_directory = sanitize_user_path(&directory);
    let dir = PathBuf::from(&sanitized_directory);
    if dir.as_os_str().is_empty() {
        return Err("Directory is required".to_string());
    }
    if !dir.is_dir() {
        return Err("Selected path is not a directory".to_string());
    }
    let allowed_extensions: Vec<String> = extensions
        .into_iter()
        .map(|value| value.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect();
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|err| format!("Failed to list directory: {err}"))? {
        let entry = entry.map_err(|err| format!("Failed to read directory entry: {err}"))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let extension = match path.extension().and_then(|value| value.to_str()) {
            Some(value) => value.to_ascii_lowercase(),
            None => continue,
        };
        if !allowed_extensions.is_empty() && !allowed_extensions.iter().any(|value| value == &extension) {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "Failed to read file name".to_string())?
            .to_string();
        entries.push(DirectoryFileEntry {
            name,
            path: path.to_string_lossy().to_string(),
        });
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(entries)
}
