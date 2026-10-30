//! The design tokens of a project's design system doc. A project has one
//! design system doc; its content is one token tree in the DTCG format with a
//! value per mode (docs/formats/tokens.md, checked by
//! `omni_core::design::tokens`). An agent replaces the whole tree of the
//! design system's head in one all-or-nothing import, and anyone who may read
//! reads the tree of the head, whole or for one mode.
//!
//! An import is a write to the doc: a new head, one row of `doc_changes` and
//! one `doc.changed` event.

use crate::AppState;
use crate::chunks::read_chunk;
use crate::design::{
    self, SystemTarget, design_system_text, locked_design_system, read_system_head,
    write_system_head,
};
use crate::docs::{AgentIdentity, Tx};
use crate::projects::{parse_body, require_project};
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{AgentRequest, WorkspaceRequest, parse_uuid};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use omni_core::design::tokens::TokenTree;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct ImportTokensRequest {
    /// The whole token tree, in the DTCG format of docs/formats/tokens.md.
    #[schema(value_type = Object)]
    pub tokens: Option<Value>,
}

#[derive(Serialize, ToSchema)]
pub struct HeadTokensResponse {
    pub project_id: String,
    /// The project's design system doc; `null` when the project has none.
    pub doc_id: Option<String>,
    /// The design system's modes, in order; empty without a head.
    pub modes: Vec<String>,
    /// The token tree of the head; `null` without a head. With `?mode=`, the
    /// tree of that one mode.
    #[schema(value_type = Option<Object>)]
    pub tokens: Option<Value>,
    /// The mode that `tokens` is the tree of; only with `?mode=`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct InvalidTokensBody {
    /// `invalid_tokens`
    pub error: String,
    /// What is wrong; it names the token, the mode or the attachment.
    pub message: String,
}

/// Why a tokens request failed: the usual API errors, or a token tree that the
/// rules refuse.
pub enum TokensError {
    Api(ApiError),
    Invalid(String),
}

impl From<ApiError> for TokensError {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl IntoResponse for TokensError {
    fn into_response(self) -> Response {
        match self {
            Self::Api(error) => error.into_response(),
            Self::Invalid(message) => (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid_tokens", "message": message })),
            )
                .into_response(),
        }
    }
}

/// The project's design system doc and its head, if it has one.
async fn find_design_system(
    tx: &mut Tx,
    project_id: &str,
) -> Result<Option<(String, Option<String>)>, ApiError> {
    sqlx::query_as(
        "SELECT id::text, head FROM docs \
         WHERE project_id = $1::uuid AND kind = 'design-system' AND deleted_at IS NULL",
    )
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)
}

/// The modes and the tree to answer with: the whole tree, or the tree of one
/// mode. A mode that the design system does not list is 400 `unknown_mode`.
fn answer_for(
    tokens: Option<&TokenTree>,
    mode: Option<&str>,
) -> Result<(Vec<String>, Option<Value>), ApiError> {
    let unknown = || ApiError::bad_request("unknown_mode");
    match (tokens, mode) {
        (None, None) => Ok((Vec::new(), None)),
        (None, Some(_)) => Err(unknown()),
        (Some(tokens), None) => Ok((tokens.modes().to_vec(), Some(tokens.to_json()))),
        (Some(tokens), Some(mode)) => {
            let tree = tokens.for_mode(mode).ok_or_else(unknown)?;
            Ok((tokens.modes().to_vec(), Some(tree)))
        }
    }
}

/// Checks that every font file of the tree is a font attached to the workspace
/// (row-level security shows this workspace's attachments only).
async fn check_font_files(tx: &mut Tx, tree: &TokenTree) -> Result<(), TokensError> {
    for file in tree.font_files() {
        let found: Option<(String,)> =
            sqlx::query_as("SELECT content_type FROM attachments WHERE id = $1::uuid")
                .bind(file.id())
                .fetch_optional(&mut **tx)
                .await
                .map_err(ApiError::database)?;
        match found {
            None => {
                return Err(TokensError::Invalid(format!(
                    "the font file {} is no file of this workspace: upload it with `omni attach`",
                    file.as_str()
                )));
            }
            Some((content_type,)) if !content_type.starts_with("font/") => {
                return Err(TokensError::Invalid(format!(
                    "the font file {} is not a font (it is {content_type}): font files are WOFF2, TTF or OTF files",
                    file.as_str()
                )));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

#[derive(Serialize, ToSchema)]
pub struct ImportedHeadTokens {
    pub project_id: String,
    /// The project's design system doc.
    pub doc_id: String,
    /// The design system's new head.
    pub head: String,
    /// The design system's modes, in order.
    pub modes: Vec<String>,
    /// The token tree as stored.
    #[schema(value_type = Object)]
    pub tokens: Value,
}

/// Replaces the token tree of the project's design system head, all or nothing;
/// the components stay. The write makes a new head, adds a row of
/// `doc_changes` and emits `doc.changed`; a tree that is refused changes, logs
/// and emits nothing. Designs read the new tree at their next
/// read.
#[utoipa::path(
    put,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/tokens",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = ImportTokensRequest,
    responses(
        (status = 200, description = "The tree as stored", body = ImportedHeadTokens),
        (status = 400, description = "`tokens` is missing (`invalid_request`), or the tree is refused or names a font file that is not a font of the workspace (`invalid_tokens`, with a message)", body = InvalidTokensBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such project in this workspace, or the project has no design system doc (`no_design_system`)", body = ErrorBody),
    )
)]
pub async fn import_head_tokens(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<ImportedHeadTokens>, TokensError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let workspace_id = request.workspace_id;
    let sent = parse_body::<ImportTokensRequest>(&body);
    let mut tx = state.workspace_tx(&workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let (doc_id, head_before) = locked_design_system(&mut tx, &project_id).await?;
    let tokens = sent?
        .tokens
        .ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let tree = TokenTree::from_json(&tokens).map_err(|e| TokensError::Invalid(e.to_string()))?;
    check_font_files(&mut tx, &tree).await?;

    let current = read_system_head(&state, &workspace_id, &doc_id, head_before.as_deref()).await?;
    let components: Vec<_> = current.library.into_components();
    let agent = identity.to_json();
    let target = SystemTarget {
        workspace_id: &workspace_id,
        project_id: &project_id,
        doc_id: &doc_id,
        head_before: head_before.as_deref(),
        agent: &agent,
    };
    let modes = tree.modes().to_vec();
    let tokens = tree.to_json();
    let text = design_system_text(Some(&tree), &components);
    let operations = json!([{ "op": "import_tokens", "tokens": tokens }]);
    let head = write_system_head(&state, &mut tx, &target, &operations, text).await?;
    tx.commit().await.map_err(ApiError::database)?;
    state.live.announce(&workspace_id, &project_id);
    Ok(Json(ImportedHeadTokens {
        project_id,
        doc_id,
        head,
        modes,
        tokens,
    }))
}

/// The token tree of the design system's head, or, with `?mode=`, the tree of
/// one mode. Before the design system has a head (or a doc) there is no tree.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/tokens",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("mode" = Option<String>, Query, description = "Read the tree of this mode: each `$value` is the mode's value and no token has mode values"),
    ),
    responses(
        (status = 200, description = "The head's tokens", body = HeadTokensResponse),
        (status = 400, description = "The design system does not list the mode (`unknown_mode`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such project in this workspace", body = ErrorBody),
    )
)]
pub async fn read_head_tokens(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<HeadTokensResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let found = find_design_system(&mut tx, &project_id).await?;
    tx.commit().await.map_err(ApiError::database)?;
    let (doc_id, head) = match found {
        Some((doc_id, head)) => (Some(doc_id), head),
        None => (None, None),
    };
    let tokens = match head {
        Some(head) => {
            let text = read_chunk(&state, &request.workspace_id, &head).await?;
            let what = format!("the design system {}", doc_id.as_deref().unwrap_or(""));
            design::read_design_system(&text, &what)?.0
        }
        None => None,
    };
    let mode = query.get("mode").map(String::as_str);
    let (modes, tree) = answer_for(tokens.as_ref(), mode)?;
    Ok(Json(HeadTokensResponse {
        project_id,
        doc_id,
        modes,
        tokens: tree,
        mode: mode.map(str::to_string),
    }))
}
