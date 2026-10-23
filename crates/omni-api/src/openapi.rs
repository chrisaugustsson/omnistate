//! The OpenAPI document, generated from the handlers' `#[utoipa::path]`
//! attributes. Each operation gets an `x-omni-access` extension with its entry
//! in the route registry (`agent`, `human` or `both`), so the document and the
//! registry cannot disagree about who may call a route.

use crate::api_keys::{ApiKeyBody, CreatedKeyResponse, KeyListResponse, KeyResponse};
use crate::attachments::{AttachmentBody, AttachmentResponse};
use crate::boards::{BoardBody, Manifest, ManifestBoard, ManifestPage};
use crate::changesets::{
    ApprovedVersion, ChangesetBody, ChangesetListResponse, ChangesetResponse,
    CreateChangesetRequest, CreateProjectRequest, HistoryEntry, LinkedDecision, ProjectBody,
    ProjectListResponse, ProjectResponse, RejectRequest,
};
use crate::claims::{ClaimBody, ClaimRequest, ClaimResponse, ClaimedBody};
use crate::comments::{
    AddressedRequest, CommentBody, CommentListResponse, CommentResponse, CreateCommentRequest,
    ReplyBody, ReplyRequest, SingleCommentResponse,
};
use crate::decisions::{
    DecisionBody, DecisionFile, DecisionOption, DecisionResponse, Dissent, RejectDecisionRequest,
};
use crate::design::{
    ImportTagsRequest, ImportedComponents, ImportedDesign, InvalidDesignBody, UsageBoard,
    UsageResponse,
};
use crate::docs::{
    ConflictBody, ConflictDetails, CreateDocRequest, CreatedDocResponse, DocBody, DocListResponse,
    DocResponse, DraftBody, DraftHtmlResponse, DraftResponse, DraftWithHtml, EditRequest,
    EditResponse, InvalidOperationBody, MainlineHtmlResponse, OpResultBody,
};
use crate::features::{CreateFeatureRequest, FeatureBody, FeatureListResponse, FeatureResponse};
use crate::home::{HomeResponse, WaitingBody};
use crate::live::{LiveClientMessage, LiveErrorCode, LiveEventName, LiveServerMessage};
use crate::rebase::{
    ConflictKind, ConflictResolution, ConflictSide, RebaseResponse, RecordedConflict,
    RecordedConflictListResponse, RecordedConflictResponse, ResolveConflictRequest,
};
use crate::review::{ReviewChanges, ReviewResponse, ReviewedDoc};
use crate::routes::{Access, registry};
use crate::session::{
    ErrorBody, LoginRequest, UserBody, UserResponse, WorkspaceBody, WorkspaceListResponse,
};
use crate::threads::{
    CreateDirectiveRequest, CreatePostRequest, CreateThreadRequest, DigestEntry, DigestResponse,
    PhaseBody, PostBody, PostResponse, PosterBody, SinglePostResponse, StartPhaseRequest,
    ThreadBody, ThreadResponse,
};
use crate::tokens::{
    DraftTokensResponse, ImportTokensRequest, InvalidTokensBody, MainlineTokensResponse,
};
use crate::versions::{
    DiffBody, DiffResponse, MovedNode, NodeEntry, SaveVersionRequest, VersionBody, VersionItem,
    VersionListResponse, VersionResponse,
};
use crate::workspace::{ActorBody, WhoamiResponse};
use axum::Json;
use serde_json::{Value, json};
use std::sync::OnceLock;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "OmniState API",
        description = "The HTTP API behind the OmniState web app and the `omni` command line. An agent request that succeeds inside a change set (a path under `/changesets/{changeset_id}` or `/threads/{thread_id}`) is answered with a `directives` list next to its usual fields: the human's directive posts that the agent identity (run and role) has not been given yet, oldest first. The field is left out when there are none."
    ),
    paths(
        crate::health,
        document,
        crate::session::login,
        crate::session::logout,
        crate::session::me,
        crate::session::workspaces,
        crate::workspace::whoami,
        crate::api_keys::list,
        crate::api_keys::create,
        crate::api_keys::revoke,
        crate::home::show,
        crate::features::list_features,
        crate::features::create_feature,
        crate::features::show_feature,
        crate::features::done,
        crate::features::reopen,
        crate::changesets::list_projects,
        crate::changesets::create_project,
        crate::changesets::list_changesets,
        crate::changesets::create_changeset,
        crate::changesets::show_changeset,
        crate::review::show,
        crate::changesets::submit,
        crate::changesets::reject,
        crate::changesets::approve,
        crate::rebase::rebase,
        crate::rebase::list_conflicts,
        crate::rebase::resolve,
        crate::changesets::reopen,
        crate::changesets::abandon,
        crate::attachments::upload,
        crate::attachments::download,
        crate::docs::create_doc,
        crate::docs::list_docs,
        crate::docs::show_doc,
        crate::docs::read_doc_html,
        crate::docs::open_draft,
        crate::docs::read_draft,
        crate::docs::edit_draft,
        crate::claims::take_claim,
        crate::claims::release_claim,
        crate::versions::save_version,
        crate::versions::remove_doc,
        crate::versions::list_versions,
        crate::versions::show_version,
        crate::versions::diff_versions,
        crate::tokens::import_tokens,
        crate::tokens::read_draft_tokens,
        crate::tokens::read_mainline_tokens,
        crate::design::import_components,
        crate::design::import_design,
        crate::design::component_usage,
        crate::boards::read_draft_board,
        crate::boards::read_board_of_version,
        crate::threads::create_thread,
        crate::threads::show_thread,
        crate::threads::digest,
        crate::threads::show_post,
        crate::threads::start_phase,
        crate::threads::create_post,
        crate::threads::create_directive,
        crate::decisions::propose,
        crate::decisions::show,
        crate::decisions::approve,
        crate::decisions::reject,
        crate::decisions::link,
        crate::comments::create,
        crate::comments::list,
        crate::comments::show,
        crate::comments::reply,
        crate::comments::addressed,
        crate::comments::resolve,
        crate::comments::reopen
    ),
    components(schemas(
        LoginRequest,
        UserBody,
        UserResponse,
        WorkspaceBody,
        WorkspaceListResponse,
        ErrorBody,
        LiveClientMessage,
        LiveServerMessage,
        LiveEventName,
        LiveErrorCode,
        WhoamiResponse,
        ActorBody,
        ApiKeyBody,
        KeyResponse,
        KeyListResponse,
        CreatedKeyResponse,
        ProjectBody,
        ProjectResponse,
        ProjectListResponse,
        CreateProjectRequest,
        FeatureBody,
        FeatureResponse,
        FeatureListResponse,
        CreateFeatureRequest,
        CreateChangesetRequest,
        RejectRequest,
        HistoryEntry,
        ApprovedVersion,
        LinkedDecision,
        ChangesetBody,
        ChangesetResponse,
        ChangesetListResponse,
        AttachmentBody,
        AttachmentResponse,
        CreateDocRequest,
        DocBody,
        DocResponse,
        DocListResponse,
        CreatedDocResponse,
        DraftBody,
        DraftResponse,
        DraftWithHtml,
        DraftHtmlResponse,
        MainlineHtmlResponse,
        EditRequest,
        OpResultBody,
        EditResponse,
        ConflictDetails,
        ConflictBody,
        InvalidOperationBody,
        ClaimBody,
        ClaimRequest,
        ClaimResponse,
        ClaimedBody,
        NodeEntry,
        VersionBody,
        VersionResponse,
        VersionItem,
        VersionListResponse,
        MovedNode,
        DiffBody,
        DiffResponse,
        SaveVersionRequest,
        ConflictKind,
        ConflictSide,
        ConflictResolution,
        RecordedConflict,
        RebaseResponse,
        RecordedConflictResponse,
        RecordedConflictListResponse,
        ResolveConflictRequest,
        ImportTokensRequest,
        DraftTokensResponse,
        MainlineTokensResponse,
        InvalidTokensBody,
        ImportTagsRequest,
        ImportedComponents,
        ImportedDesign,
        InvalidDesignBody,
        UsageBoard,
        UsageResponse,
        BoardBody,
        Manifest,
        ManifestPage,
        ManifestBoard,
        PhaseBody,
        PosterBody,
        ThreadBody,
        ThreadResponse,
        CreateThreadRequest,
        StartPhaseRequest,
        CreatePostRequest,
        CreateDirectiveRequest,
        PostBody,
        PostResponse,
        DigestEntry,
        DigestResponse,
        SinglePostResponse,
        DecisionOption,
        Dissent,
        DecisionFile,
        DecisionBody,
        DecisionResponse,
        RejectDecisionRequest,
        ReplyBody,
        CommentBody,
        CommentResponse,
        SingleCommentResponse,
        CommentListResponse,
        CreateCommentRequest,
        ReplyRequest,
        AddressedRequest,
        WaitingBody,
        ReviewChanges,
        ReviewedDoc,
        ReviewResponse,
        HomeResponse
    ))
)]
struct ApiDoc;

fn access_name(access: Access) -> &'static str {
    match access {
        Access::Agent => "agent",
        Access::Human => "human",
        Access::Both => "both",
    }
}

pub fn build() -> Value {
    let mut document = serde_json::to_value(ApiDoc::openapi()).expect("the OpenAPI document");
    for route in registry() {
        let operation = document
            .pointer_mut(&format!(
                "/paths/{}/{}",
                route.path.replace('~', "~0").replace('/', "~1"),
                route.method.to_lowercase()
            ))
            .and_then(Value::as_object_mut);
        if let Some(operation) = operation {
            operation.insert(
                "x-omni-access".to_string(),
                json!(access_name(route.access)),
            );
        }
    }
    document
}

/// The OpenAPI document of the API.
#[utoipa::path(
    get,
    path = "/api/openapi.json",
    responses((status = 200, description = "The OpenAPI 3 document", content_type = "application/json"))
)]
pub async fn document() -> Json<Value> {
    static DOCUMENT: OnceLock<Value> = OnceLock::new();
    Json(DOCUMENT.get_or_init(build).clone())
}
