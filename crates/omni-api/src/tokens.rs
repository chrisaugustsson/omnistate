//! The design tokens of a project's design system doc. A project has one
//! design system doc, started in a change set like any doc; its content is one
//! token tree in the DTCG format with a value per mode (docs/formats/tokens.md,
//! checked by `omni_core::design::tokens`). An agent replaces the whole tree of
//! a change set's draft in one all-or-nothing import, and anyone who may read
//! reads the tree of a draft, of the mainline version or of an older version,
//! whole or for one mode.
//!
//! An import is one row of the draft's log (`draft_ops`, an `import_tokens`
//! operation that holds the tree and names the agent) and one `tokens.imported`
//! event. The draft in memory keeps the last tree, and a draft that is not in
//! memory is rebuilt from its base version and its log, as for any doc. Saving
//! a version of the draft (`versions::save_version`) stores the tree as a chunk
//! of JSON.

use crate::AppState;
use crate::changesets::{parse_body, require_project};
use crate::design::{
    SystemTarget, design_system_text, locked_design_system, read_system_head, write_system_head,
};
use crate::docs::require_changeset;
use crate::docs::{
    AgentIdentity, DraftRow, Tx, current_draft, ensure_draft, load_state, read_tokens,
};
use crate::live;
use crate::session::{ApiError, ErrorBody};
use crate::versions::{chunk_of_version, read_chunk};
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
pub struct DraftTokensResponse {
    /// The project's design system doc; `null` when the project has none.
    pub doc_id: Option<String>,
    pub changeset_id: String,
    /// The change set's draft of the design system; `null` when it has none.
    pub draft_id: Option<String>,
    /// The design system's modes, in order; empty before the first import.
    pub modes: Vec<String>,
    /// The token tree of the draft; `null` before the first import. With
    /// `?mode=`, the tree of that one mode.
    #[schema(value_type = Option<Object>)]
    pub tokens: Option<Value>,
    /// The mode that `tokens` is the tree of; only with `?mode=`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct MainlineTokensResponse {
    pub project_id: String,
    /// The project's design system doc; `null` when the project has none.
    pub doc_id: Option<String>,
    /// The version that was read; `null` when the design system has no
    /// mainline version yet.
    pub version_id: Option<String>,
    /// The design system's modes, in order; empty without a version.
    pub modes: Vec<String>,
    /// The token tree of the version; `null` without a version. With `?mode=`,
    /// the tree of that one mode.
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

/// The token tree of an `import_tokens` operation of the log, `None` for any
/// other operation.
pub(crate) fn imported_in(op: &str) -> Option<Value> {
    let mut op: Value = serde_json::from_str(op).ok()?;
    if op.get("op")?.as_str()? != "import_tokens" {
        return None;
    }
    Some(op.get_mut("tokens")?.take())
}

/// The project's design system doc, if it has one: its ID and its mainline
/// version.
async fn find_design_system(
    tx: &mut Tx,
    project_id: &str,
) -> Result<Option<(String, Option<String>)>, ApiError> {
    sqlx::query_as(
        "SELECT id::text, mainline_version_id::text FROM docs \
         WHERE project_id = $1::uuid AND kind = 'design-system'",
    )
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(ApiError::database)
}

/// Like `find_design_system`, but a project without one is 404
/// `no_design_system`, for the routes that change it.
pub(crate) async fn design_system_of(
    tx: &mut Tx,
    project_id: &str,
) -> Result<(String, Option<String>), ApiError> {
    find_design_system(tx, project_id)
        .await?
        .ok_or(ApiError::Refused(StatusCode::NOT_FOUND, "no_design_system"))
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

/// What an import needs once the tree is checked and the draft exists.
struct Import {
    workspace_id: String,
    project_id: String,
    doc_id: String,
    changeset_id: String,
    draft_id: String,
    agent: Value,
    tree: TokenTree,
}

/// Replaces the token tree of the change set's draft of the project's design
/// system doc, all or nothing. A change set without a draft of the doc gets
/// one, based on the doc's mainline version. The import is logged with the
/// agent and emits `tokens.imported`; a tree that is refused changes, logs and
/// emits nothing.
#[utoipa::path(
    put,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/tokens",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("X-Omni-Run" = Option<String>, Header, description = "The agent's run"),
        ("X-Omni-Role" = Option<String>, Header, description = "The agent's role"),
        ("X-Omni-Model" = Option<String>, Header, description = "The model behind the agent"),
        ("X-Omni-Harness" = Option<String>, Header, description = "The harness that runs the agent"),
    ),
    request_body = ImportTokensRequest,
    responses(
        (status = 200, description = "The tree as stored", body = DraftTokensResponse),
        (status = 400, description = "`tokens` is missing (`invalid_request`), or the tree is refused or names a font file that is not a font of the workspace (`invalid_tokens`, with a message)", body = InvalidTokensBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 403, description = "Called with a session; agents only", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace, or the project has no design system doc (`no_design_system`)", body = ErrorBody),
        (status = 409, description = "The design system has no mainline version, so only its own change set has a draft of it (`no_mainline_version`)", body = ErrorBody),
    )
)]
pub async fn import_tokens(
    State(state): State<AppState>,
    request: AgentRequest,
    identity: AgentIdentity,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Json<DraftTokensResponse>, TokensError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let sent = parse_body::<ImportTokensRequest>(&body);
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    require_changeset(&mut tx, &project_id, &changeset_id).await?;
    let (doc_id, mainline) = design_system_of(&mut tx, &project_id).await?;
    let tokens = sent?
        .tokens
        .ok_or_else(|| ApiError::bad_request("invalid_request"))?;
    let tree = TokenTree::from_json(&tokens).map_err(|e| TokensError::Invalid(e.to_string()))?;
    check_font_files(&mut tx, &tree).await?;

    let draft_id = ensure_draft(
        &mut tx,
        &request.workspace_id,
        &project_id,
        &doc_id,
        &changeset_id,
        mainline.as_deref(),
    )
    .await?;
    tx.commit().await.map_err(ApiError::database)?;

    let import = Import {
        workspace_id: request.workspace_id,
        project_id,
        doc_id,
        changeset_id,
        draft_id,
        agent: identity.to_json(),
        tree,
    };
    // A client that gives up must not leave the draft in memory behind its
    // log, so the import runs to its end whatever happens to this request.
    let answer = tokio::spawn(async move { apply_import(&state, import).await })
        .await
        .map_err(|e| {
            eprintln!("omni-api: a tokens import failed: {e}");
            ApiError::Unavailable
        })??;
    Ok(Json(answer))
}

async fn apply_import(state: &AppState, import: Import) -> Result<DraftTokensResponse, ApiError> {
    let Import {
        workspace_id,
        project_id,
        doc_id,
        changeset_id,
        draft_id,
        agent,
        tree,
    } = import;
    // One lock per draft orders this import with the batches and the saves of
    // the agents that work on the same draft.
    let slot = state.drafts.slot(&workspace_id, &draft_id);
    let mut held = slot.lock().await;
    let mut tx = state.workspace_tx(&workspace_id).await?;
    let draft = current_draft(&mut tx, &draft_id).await?;
    if held.is_none() {
        *held = Some(load_state(state, &mut tx, &workspace_id, &draft).await?);
    }
    let seq = held.as_ref().expect("the draft was just loaded").next_seq;
    let modes = tree.modes().to_vec();
    let tokens = tree.to_json();
    let persisted = async {
        sqlx::query(
            "INSERT INTO draft_ops (workspace_id, draft_id, seq, op, agent) \
             VALUES ($1::uuid, $2::uuid, $3, $4::text::jsonb, $5::text::jsonb)",
        )
        .bind(&workspace_id)
        .bind(&draft_id)
        .bind(seq)
        .bind(json!({ "op": "import_tokens", "tokens": tokens }).to_string())
        .bind(agent.to_string())
        .execute(&mut *tx)
        .await
        .map_err(ApiError::database)?;
        live::emit(
            &mut tx,
            &workspace_id,
            &project_id,
            "tokens.imported",
            &json!({
                "doc_id": doc_id,
                "changeset_id": changeset_id,
                "draft_id": draft_id,
                "modes": modes,
            }),
        )
        .await?;
        tx.commit().await.map_err(ApiError::database)?;
        Ok::<(), ApiError>(())
    }
    .await;
    match persisted {
        Ok(()) => {
            let loaded = held.as_mut().expect("the draft is loaded");
            loaded.tokens = Some(tree);
            loaded.next_seq = seq + 1;
        }
        Err(error) => {
            // The next request loads the draft again from its log.
            *held = None;
            return Err(error);
        }
    }
    drop(held);
    state.live.announce(&workspace_id, &project_id);
    Ok(DraftTokensResponse {
        doc_id: Some(doc_id),
        changeset_id,
        draft_id: Some(draft_id),
        modes,
        tokens: Some(tokens),
        mode: None,
    })
}

/// The token tree of a change set's draft of the design system, or, with
/// `?mode=`, the tree of one mode. A project without a design system doc, and
/// a change set without a draft of it, have no tokens: the answer is empty.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/tokens",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("changeset_id" = String, Path, description = "The change set's UUID"),
        ("mode" = Option<String>, Query, description = "Read the tree of this mode: each `$value` is the mode's value and no token has mode values"),
    ),
    responses(
        (status = 200, description = "The draft's tokens", body = DraftTokensResponse),
        (status = 400, description = "The design system does not list the mode (`unknown_mode`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such change set in this project and workspace", body = ErrorBody),
    )
)]
pub async fn read_draft_tokens(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, changeset_id)): Path<(String, String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<DraftTokensResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let changeset_id = parse_uuid(&changeset_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    require_changeset(&mut tx, &project_id, &changeset_id).await?;
    // A project without a design system, and a change set without a draft of
    // it, have no tokens to read: that is an empty answer, not an error.
    let doc_id = find_design_system(&mut tx, &project_id)
        .await?
        .map(|(doc_id, _)| doc_id);
    let draft_id: Option<String> = match &doc_id {
        Some(doc_id) => sqlx::query_scalar(
            "SELECT id::text FROM drafts WHERE doc_id = $1::uuid AND changeset_id = $2::uuid",
        )
        .bind(doc_id)
        .bind(&changeset_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::database)?,
        None => None,
    };
    tx.commit().await.map_err(ApiError::database)?;

    let mut tokens = None;
    if let Some(draft_id) = &draft_id {
        let slot = state.drafts.slot(&request.workspace_id, draft_id);
        let mut held = slot.lock().await;
        let mut tx = state.workspace_tx(&request.workspace_id).await?;
        let draft: DraftRow = current_draft(&mut tx, draft_id).await?;
        if held.is_none() {
            *held = Some(load_state(&state, &mut tx, &request.workspace_id, &draft).await?);
        }
        tx.commit().await.map_err(ApiError::database)?;
        tokens = held.as_ref().and_then(|loaded| loaded.tokens.clone());
    }
    let mode = query.get("mode").map(String::as_str);
    let (modes, tree) = answer_for(tokens.as_ref(), mode)?;
    Ok(Json(DraftTokensResponse {
        doc_id,
        changeset_id,
        draft_id,
        modes,
        tokens: tree,
        mode: mode.map(str::to_string),
    }))
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

/// Replaces the token tree of the project's design system head, all or nothing,
/// with no change set; the components stay. The write makes a new head, adds a
/// row of `doc_changes` and emits `doc.changed`; a tree that is refused
/// changes, logs and emits nothing. Designs read the new tree at their next
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

/// The token tree of the design system's head, or of the version named by
/// `?version=`, or, with `?mode=`, the tree of one mode. Before the design
/// system has a head (or a doc) there is no tree. `version_id` is the version
/// that holds the head, `null` for a head that no version holds.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/tokens",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("version" = Option<String>, Query, description = "The version to read instead of the mainline version"),
        ("mode" = Option<String>, Query, description = "Read the tree of this mode: each `$value` is the mode's value and no token has mode values"),
    ),
    responses(
        (status = 200, description = "The version's tokens", body = MainlineTokensResponse),
        (status = 400, description = "The design system does not list the mode (`unknown_mode`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such project in this workspace, or the design system has no such version", body = ErrorBody),
    )
)]
pub async fn read_mainline_tokens(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<MainlineTokensResponse>, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    require_project(&mut tx, &project_id).await?;
    let found = find_design_system(&mut tx, &project_id).await?;
    let doc_id = found.as_ref().map(|(doc_id, _)| doc_id.clone());
    let (version_id, chunk) = match (query.get("version"), &doc_id) {
        (Some(version), Some(doc_id)) => {
            let version_id = parse_uuid(version).ok_or(ApiError::NotFound)?;
            // A version that holds no content (the removal of the doc) has no
            // tree.
            let chunk = chunk_of_version(&mut tx, doc_id, &version_id).await?;
            (Some(version_id), chunk)
        }
        (Some(version), None) => {
            parse_uuid(version).ok_or(ApiError::NotFound)?;
            return Err(ApiError::NotFound);
        }
        (None, Some(doc_id)) => {
            let head: Option<String> =
                sqlx::query_scalar("SELECT head FROM docs WHERE id = $1::uuid")
                    .bind(doc_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(ApiError::database)?;
            let version: Option<String> = match &head {
                Some(head) => sqlx::query_scalar(
                    "SELECT v.id::text FROM versions v JOIN docs d ON d.mainline_version_id = v.id \
                     WHERE d.id = $1::uuid AND v.chunk = $2",
                )
                .bind(doc_id)
                .bind(head)
                .fetch_optional(&mut *tx)
                .await
                .map_err(ApiError::database)?,
                None => None,
            };
            (version, head)
        }
        (None, None) => (None, None),
    };
    tx.commit().await.map_err(ApiError::database)?;
    let tokens = match chunk {
        Some(chunk) => {
            let text = read_chunk(&state, &request.workspace_id, &chunk).await?;
            read_tokens(
                &text,
                &format!("the design system {}", doc_id.as_deref().unwrap_or("")),
            )?
        }
        None => None,
    };
    let mode = query.get("mode").map(String::as_str);
    let (modes, tree) = answer_for(tokens.as_ref(), mode)?;
    Ok(Json(MainlineTokensResponse {
        project_id,
        doc_id,
        version_id,
        modes,
        tokens: tree,
        mode: mode.map(str::to_string),
    }))
}
