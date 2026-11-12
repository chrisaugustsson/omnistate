//! The OpenAPI document, generated from the handlers' `#[utoipa::path]`
//! attributes. Each operation gets an `x-omni-access` extension with its entry
//! in the route registry (`agent`, `human` or `both`), so the document and the
//! registry cannot disagree about who may call a route.

use crate::activity::{
    ActivityAgent, ActivityDetail, ActivityEntry, ActivityItem, ActivityListResponse,
    ComponentBody, TitleChange, TokensChange,
};
use crate::api_keys::{ApiKeyBody, CreatedKeyResponse, KeyListResponse, KeyResponse};
use crate::attachments::{AttachmentBody, AttachmentResponse};
use crate::behaviour::{
    AnswerBody, AnswerRequest, AskRequest, CreateNodeRequest, MoveNodeRequest, NodeBody,
    NodeChildBody, NodeListResponse, NodeReadResponse, NodeResponse, NodeSummaryBody,
    PatchNodeRequest, PointerBody, PointerDetailBody, QuestionBody, QuestionListResponse,
    QuestionResponse,
};
use crate::boards::{BoardBody, Manifest, ManifestBoard, ManifestPage};
use crate::comments::{
    CommentBody, CommentListResponse, CommentResponse, CreateCommentRequest, ReplyBody,
    ReplyRequest, SingleCommentResponse,
};
use crate::design::{
    ImportTagsRequest, ImportedHeadComponents, InvalidDesignBody, UsageBoard, UsageResponse,
};
use crate::docs::{
    ConflictBody, ConflictDetails, CreateDocRequest, DocBody, DocListResponse, DocResponse,
    EditRequest, EditResponse, HeadHtmlResponse, InvalidOperationBody, OpResultBody,
    RenameDocRequest,
};
use crate::features::{
    CreateFeatureRequest, FeatureBody, FeatureListResponse, FeatureResponse, RenameFeatureRequest,
};
use crate::home::{FeatureSummary, HomeResponse, StoryCounts, WaitingBody};
use crate::live::{LiveClientMessage, LiveErrorCode, LiveEventName, LiveServerMessage};
use crate::pins::NodeBox;
use crate::projects::{
    CreateProjectRequest, ProjectBody, ProjectListResponse, ProjectResponse, RenameProjectRequest,
};
use crate::routes::{Access, registry};
use crate::session::{
    ErrorBody, LoginRequest, UserBody, UserResponse, WorkspaceBody, WorkspaceListResponse,
};
use crate::stories::{
    BlockStoryRequest, CancelStoryRequest, CheckCriterionRequest, ClaimStoryRequest,
    CreateStoryRequest, CriterionBody, DecisionRequest, FilesRequest, FindingRequest,
    MoveStoryRequest, NewCriterion, PatchStoryRequest, ReleaseStoryRequest, ResolveFindingRequest,
    StoryBody, StoryConflictBody, StoryHeldBody, StoryListResponse, StoryResponse, TickRequest,
};
use crate::threads::{
    ConcludeThreadRequest, CreatePostRequest, DigestEntry, DigestResponse, PostBody, PostResponse,
    SinglePostResponse, StartThreadRequest, ThreadBody, ThreadListResponse, ThreadResponse,
};
use crate::tokens::{
    HeadTokensResponse, ImportTokensRequest, ImportedHeadTokens, InvalidTokensBody,
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
        description = "The HTTP API behind the OmniState web app and the `omni` command line."
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
        crate::features::rename_feature,
        crate::features::delete_feature,
        crate::projects::list_projects,
        crate::projects::create_project,
        crate::projects::rename_project,
        crate::attachments::upload,
        crate::attachments::download,
        crate::docs::create_app_doc,
        crate::docs::create_feature_doc,
        crate::docs::list_docs,
        crate::docs::show_doc,
        crate::docs::rename_doc,
        crate::docs::delete_doc,
        crate::docs::read_doc_html,
        crate::docs::edit_doc,
        crate::activity::list_activity,
        crate::activity::show_activity,
        crate::activity::read_component,
        crate::tokens::read_head_tokens,
        crate::tokens::import_head_tokens,
        crate::design::import_head_components,
        crate::design::component_usage,
        crate::boards::read_head_board,
        crate::exports::board_png,
        crate::behaviour::create_node,
        crate::behaviour::list_nodes,
        crate::behaviour::show_node,
        crate::behaviour::patch_node,
        crate::behaviour::move_node,
        crate::behaviour::ask_question,
        crate::behaviour::list_questions,
        crate::behaviour::show_question,
        crate::behaviour::answer_question,
        crate::stories::create_story,
        crate::stories::list_stories,
        crate::stories::show_story,
        crate::stories::patch_story,
        crate::stories::cancel_story,
        crate::stories::claim_story,
        crate::stories::release_story,
        crate::stories::move_story,
        crate::stories::tick_criterion,
        crate::stories::add_finding,
        crate::stories::resolve_finding,
        crate::stories::add_decision,
        crate::stories::set_files,
        crate::stories::block_story,
        crate::stories::unblock_story,
        crate::stories::check_criterion,
        crate::threads::create_app_thread,
        crate::threads::create_feature_thread,
        crate::threads::list_threads,
        crate::threads::conclude_thread,
        crate::threads::show_thread,
        crate::threads::digest,
        crate::threads::show_post,
        crate::threads::create_post,
        crate::comments::list,
        crate::comments::show,
        crate::comments::reply,
        crate::comments::create_on_doc,
        crate::comments::delete
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
        RenameProjectRequest,
        FeatureBody,
        FeatureResponse,
        RenameFeatureRequest,
        FeatureListResponse,
        CreateFeatureRequest,
        AttachmentBody,
        AttachmentResponse,
        CreateDocRequest,
        RenameDocRequest,
        DocBody,
        DocResponse,
        DocListResponse,
        HeadHtmlResponse,
        NodeBox,
        EditRequest,
        OpResultBody,
        EditResponse,
        ActivityAgent,
        ActivityEntry,
        ActivityListResponse,
        ActivityItem,
        TitleChange,
        TokensChange,
        ActivityDetail,
        ComponentBody,
        ConflictDetails,
        ConflictBody,
        InvalidOperationBody,
        ImportTokensRequest,
        ImportedHeadTokens,
        HeadTokensResponse,
        InvalidTokensBody,
        ImportTagsRequest,
        ImportedHeadComponents,
        InvalidDesignBody,
        UsageBoard,
        UsageResponse,
        BoardBody,
        Manifest,
        ManifestPage,
        ManifestBoard,
        ThreadBody,
        ThreadResponse,
        ThreadListResponse,
        CreateNodeRequest,
        PatchNodeRequest,
        MoveNodeRequest,
        NodeBody,
        NodeSummaryBody,
        NodeChildBody,
        AskRequest,
        AnswerRequest,
        AnswerBody,
        QuestionBody,
        QuestionListResponse,
        QuestionResponse,
        PointerBody,
        PointerDetailBody,
        NodeResponse,
        NodeReadResponse,
        NodeListResponse,
        CreateStoryRequest,
        CriterionBody,
        NewCriterion,
        StoryBody,
        StoryListResponse,
        StoryResponse,
        PatchStoryRequest,
        CancelStoryRequest,
        ClaimStoryRequest,
        ReleaseStoryRequest,
        TickRequest,
        FindingRequest,
        ResolveFindingRequest,
        DecisionRequest,
        FilesRequest,
        BlockStoryRequest,
        CheckCriterionRequest,
        MoveStoryRequest,
        StoryHeldBody,
        StoryConflictBody,
        StartThreadRequest,
        ConcludeThreadRequest,
        CreatePostRequest,
        PostBody,
        PostResponse,
        DigestEntry,
        DigestResponse,
        SinglePostResponse,
        ReplyBody,
        CommentBody,
        CommentResponse,
        SingleCommentResponse,
        CommentListResponse,
        CreateCommentRequest,
        ReplyRequest,
        WaitingBody,
        FeatureSummary,
        StoryCounts,
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
