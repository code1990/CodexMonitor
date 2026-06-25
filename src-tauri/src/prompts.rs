use tauri::{AppHandle, State};

use crate::remote_backend;
use crate::shared::prompts_core::{self, CustomPromptEntry};
use crate::state::AppState;

#[tauri::command]
pub(crate) async fn prompts_list(
    state: State<'_, AppState>,
    workspace_id: String,
    app: AppHandle,
) -> Result<Vec<CustomPromptEntry>, String> {
    if remote_backend::is_remote_mode(&*state).await {
        let response = remote_backend::call_remote(
            &*state,
            app,
            "prompts_list",
            serde_json::json!({ "workspaceId": workspace_id }),
        )
        .await?;
        return serde_json::from_value(response).map_err(|err| err.to_string());
    }

    prompts_core::prompts_list_core(&state.workspaces, &state.settings_path, workspace_id).await
}

#[tauri::command]
pub(crate) async fn prompts_workspace_dir(
    state: State<'_, AppState>,
    workspace_id: String,
    app: AppHandle,
) -> Result<String, String> {
    if remote_backend::is_remote_mode(&*state).await {
        let response = remote_backend::call_remote(
            &*state,
            app,
            "prompts_workspace_dir",
            serde_json::json!({ "workspaceId": workspace_id }),
        )
        .await?;
        return serde_json::from_value(response).map_err(|err| err.to_string());
    }

    prompts_core::prompts_workspace_dir_core(&state.workspaces, &state.settings_path, workspace_id)
        .await
}

#[tauri::command]
pub(crate) async fn prompts_global_dir(
    state: State<'_, AppState>,
    workspace_id: String,
    app: AppHandle,
) -> Result<String, String> {
    if remote_backend::is_remote_mode(&*state).await {
        let response = remote_backend::call_remote(
            &*state,
            app,
            "prompts_global_dir",
            serde_json::json!({ "workspaceId": workspace_id }),
        )
        .await?;
        return serde_json::from_value(response).map_err(|err| err.to_string());
    }

    prompts_core::prompts_global_dir_core(&state.workspaces, workspace_id).await
}

#[tauri::command]
pub(crate) async fn prompts_create(
    state: State<'_, AppState>,
    workspace_id: String,
    scope: String,
    name: String,
    description: Option<String>,
    argument_hint: Option<String>,
    content: String,
    app: AppHandle,
) -> Result<CustomPromptEntry, String> {
    if remote_backend::is_remote_mode(&*state).await {
        let response = remote_backend::call_remote(
            &*state,
            app,
            "prompts_create",
            serde_json::json!({
                "workspaceId": workspace_id,
                "scope": scope,
                "name": name,
                "description": description,
                "argumentHint": argument_hint,
                "content": content,
            }),
        )
        .await?;
        return serde_json::from_value(response).map_err(|err| err.to_string());
    }

    prompts_core::prompts_create_core(
        &state.workspaces,
        &state.settings_path,
        workspace_id,
        scope,
        name,
        description,
        argument_hint,
        content,
    )
    .await
}

#[tauri::command]
pub(crate) async fn prompts_update(
    state: State<'_, AppState>,
    workspace_id: String,
    path: String,
    name: String,
    description: Option<String>,
    argument_hint: Option<String>,
    content: String,
    app: AppHandle,
) -> Result<CustomPromptEntry, String> {
    if remote_backend::is_remote_mode(&*state).await {
        let response = remote_backend::call_remote(
            &*state,
            app,
            "prompts_update",
            serde_json::json!({
                "workspaceId": workspace_id,
                "path": path,
                "name": name,
                "description": description,
                "argumentHint": argument_hint,
                "content": content,
            }),
        )
        .await?;
        return serde_json::from_value(response).map_err(|err| err.to_string());
    }

    prompts_core::prompts_update_core(
        &state.workspaces,
        &state.settings_path,
        workspace_id,
        path,
        name,
        description,
        argument_hint,
        content,
    )
    .await
}

#[tauri::command]
pub(crate) async fn prompts_delete(
    state: State<'_, AppState>,
    workspace_id: String,
    path: String,
    app: AppHandle,
) -> Result<(), String> {
    if remote_backend::is_remote_mode(&*state).await {
        remote_backend::call_remote(
            &*state,
            app,
            "prompts_delete",
            serde_json::json!({ "workspaceId": workspace_id, "path": path }),
        )
        .await?;
        return Ok(());
    }

    prompts_core::prompts_delete_core(&state.workspaces, &state.settings_path, workspace_id, path)
        .await
}

#[tauri::command]
pub(crate) async fn prompts_move(
    state: State<'_, AppState>,
    workspace_id: String,
    path: String,
    scope: String,
    app: AppHandle,
) -> Result<CustomPromptEntry, String> {
    if remote_backend::is_remote_mode(&*state).await {
        let response = remote_backend::call_remote(
            &*state,
            app,
            "prompts_move",
            serde_json::json!({ "workspaceId": workspace_id, "path": path, "scope": scope }),
        )
        .await?;
        return serde_json::from_value(response).map_err(|err| err.to_string());
    }

    prompts_core::prompts_move_core(
        &state.workspaces,
        &state.settings_path,
        workspace_id,
        path,
        scope,
    )
    .await
}
