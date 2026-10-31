//! The picture of a board that `omni export` writes beside the board's tags
//! (US-121).

use crate::AppState;
use crate::boards::{board_id, read_board, unknown_board};
use crate::docs::find_doc;
use crate::previews::{self, BoardPng};
use crate::session::{ApiError, ErrorBody};
use crate::workspace::{WorkspaceRequest, parse_uuid};
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use omni_render::view::BOARD_PNG_MAX_SIDE;
use serde_json::json;

/// The pixels of the picture for each pixel of the board.
const SCALE: f32 = 2.0;

/// A board as a PNG at twice its size, on a transparent background, cut at
/// the board's box, drawn as the viewer draws it against the project's design
/// system in its first mode.
#[utoipa::path(
    get,
    path = "/api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/boards/{board_id}/png",
    params(
        ("workspace_id" = String, Path, description = "The workspace's UUID"),
        ("project_id" = String, Path, description = "The project's UUID"),
        ("doc_id" = String, Path, description = "The design doc's UUID"),
        ("board_id" = String, Path, description = "The board's node ID"),
    ),
    responses(
        (status = 200, description = "The board as `image/png`, each side its size times 2 rounded up, at least 1 px", content_type = "image/png", body = Vec<u8>),
        (status = 400, description = "The doc is no design (`not_a_design`)", body = ErrorBody),
        (status = 401, description = "No valid API key or session", body = ErrorBody),
        (status = 404, description = "No such doc in this project and workspace, or the head has no such board (`unknown_board`)", body = ErrorBody),
        (status = 422, description = "A side of the picture would pass 16384 px (`board_too_large`); `message` names the limit", body = ErrorBody),
    )
)]
pub async fn board_png(
    State(state): State<AppState>,
    request: WorkspaceRequest,
    Path((_, project_id, doc_id, board)): Path<(String, String, String, String)>,
) -> Result<Response, ApiError> {
    let project_id = parse_uuid(&project_id).ok_or(ApiError::NotFound)?;
    let doc_id = parse_uuid(&doc_id).ok_or(ApiError::NotFound)?;
    let mut tx = state.workspace_tx(&request.workspace_id).await?;
    let doc = find_doc(&mut tx, &project_id, &doc_id).await?;
    if doc.kind != "design" {
        return Err(ApiError::bad_request("not_a_design"));
    }
    tx.commit().await.map_err(ApiError::database)?;
    let board = board_id(&board)?;
    let chunk = doc.head.ok_or_else(unknown_board)?;
    let (_, node) = read_board(&state, &request.workspace_id, &chunk, &board).await?;
    match previews::full_size(&state, &request.workspace_id, &project_id, node, SCALE).await? {
        BoardPng::Made(png) => Ok(([(header::CONTENT_TYPE, "image/png")], png).into_response()),
        BoardPng::TooLarge => Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            axum::Json(json!({
                "error": "board_too_large",
                "message": format!(
                    "the board's picture would be more than {BOARD_PNG_MAX_SIDE} px on a side at {SCALE}x; {BOARD_PNG_MAX_SIDE} px is the limit"
                ),
            })),
        )
            .into_response()),
        BoardPng::Undrawable => Err(unknown_board()),
    }
}
