use utoipa::{
    Modify, OpenApi,
    openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
};

pub const JOBS_TAG: &str = "Jobs";
pub const JOB_EVENTS_TAG: &str = "Job Events";
pub const APPROVALS_TAG: &str = "Approvals";
pub const RUNS_TAG: &str = "Runs";
pub const PROVIDERS_TAG: &str = "Providers";
pub const DEBUG_TAG: &str = "Debug";

#[derive(OpenApi)]
#[openapi(
    info(
        title = "rove HTTP API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Local-first agent runtime HTTP API for jobs, events, approvals, providers, and run reports."
    ),
    components(
        schemas(
            super::CreateJobRequest,
            super::CreateJobResponse,
            super::CreateJobWorkspace,
            super::CreateJobWorkspaceKind,
            super::JobStateResponse,
            super::JobStreamEvent,
            super::ListRunsResponse,
            super::debug::MemoryListResponse,
            super::debug::MemoryTopicContentResponse,
            super::debug::MemoryTopicResponse,
            super::PendingApprovalResponse,
            super::PendingInputResponse,
            super::ProviderProfileRequest,
            super::ProviderTestRequest,
            super::ProviderTestResponse,
            super::debug::RecallHitResponse,
            super::debug::RecallTestRequest,
            super::debug::RecallTestResponse,
            super::RunSummaryResponse,
            super::SubmitApprovalRequest,
            super::SubmitInputRequest
        )
    ),
    tags(
        (name = JOBS_TAG, description = "Create, inspect, and cancel agent jobs"),
        (name = JOB_EVENTS_TAG, description = "Stream job lifecycle events over Server-Sent Events"),
        (name = APPROVALS_TAG, description = "Resolve pending tool approvals and user input requests"),
        (name = RUNS_TAG, description = "List completed runs and fetch persisted run reports"),
        (name = PROVIDERS_TAG, description = "Validate per-request provider profiles without exposing provider secrets"),
        (name = DEBUG_TAG, description = "Inspect durable memory topics and recall scoring")
    ),
    modifiers(&BearerAuth)
)]
pub struct ApiDoc;

struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "BearerAuth",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("token")
                    .build(),
            ),
        );
    }
}
