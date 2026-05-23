use std::path::PathBuf;

mod approval;
mod audit;
mod deployment;
mod error;
mod evidence;
mod ghcr_webhook;
mod mcp;
mod model;
mod ontology_graph;
mod policy;
mod provider;
mod provider_compatibility;
pub mod providers;
mod router;
mod runtime;
mod startup;

pub use approval::{
    ApprovalBroker, ApprovalDecision, ApprovalRequest, ApprovalStatus, LocalAutoApproveBroker,
    LocalDenyBroker, LocalPendingBroker,
};
pub use audit::{AuditSink, DisabledAuditSink, MemoryAuditSink, SorxAuditEvent, StdoutAuditSink};
pub use deployment::{
    CreateDeploymentRequest, DEPLOYMENT_PROMOTION_STATUS_SCHEMA,
    DEPLOYMENT_PUBLIC_ROUTE_TABLE_SCHEMA, DEPLOYMENT_REGISTRY_SCHEMA,
    DEPLOYMENT_ROUTE_TABLE_SCHEMA, DeploymentAlias, DeploymentRegistry, DeploymentRegistryError,
    DeploymentRoute, DeploymentRouteTable, DeploymentStatus, DeploymentVisibility,
    LocalDeploymentRegistryStore, PackArtifact, PromotionAuditEvent, PromotionStatus,
    RollbackAliasRequest, SorxDeployment, StateMode, TrafficHeaderMatch, TrafficMode, TrafficSplit,
};
pub use error::{SorxError, SorxResult};
pub use evidence::{
    DeterministicEvidenceProvider, EvidenceExplain, EvidenceItem, EvidenceProvider,
    EvidenceQueryFilter, EvidenceQueryResult, OntologyAuditEvent, OntologyScope, ScopedEntity,
    ontology_audit_event, redact_audit_value,
};
pub use ghcr_webhook::{
    GhcrPublishedMetadata, GhcrWebhookConfig, GhcrWebhookError, GhcrWebhookOutcome,
    GithubWebhookHeaders, OciArtifactResolver, OciReference, PromotionPolicy, ResolvedOciArtifact,
    github_signature, handle_ghcr_published_webhook, parse_ghcr_published_metadata,
    verify_github_signature,
};
pub use mcp::{McpRuntime, McpToolDefinition, McpToolList, mcp_tools_from_metadata};
pub use model::{
    ApprovalRequirement, CallerContext, CommandSpec, CommandStep, EndpointDefinition,
    EndpointInvocation, EndpointMethod, EndpointResult, EndpointStatus, IndexRequirement,
    InvocationSource, OperationKind, QueryPlan, RiskLevel, RuntimePack, SorxEvent,
    TraversalRequirement, ViewTransform,
};
pub use ontology_graph::{
    OntologyConceptNode, OntologyGraphService, OntologyRelationshipEdge, TypePath,
};
pub use policy::{
    OntologyPolicyAction, OntologyPolicyDecision, OntologyPolicyDecisionKind, OntologyPolicyReason,
    OntologyPolicyRedaction, OntologyPolicyResource, OntologyPolicySubject, PolicyAction,
    PolicyConfig, PolicyDecision, PolicyEngine, PolicyMode, RelationshipTraversalPermission,
    SensitivityContext,
};
pub use provider::{
    AppendEventOp, BindingResolver, CreateOp, DeleteOp, DeleteResult, EntityRecord, EventRecord,
    EvidenceResult, ExternalRef, ExternalRefsOp, ExternalRefsResult, GetOp, IndexQueryOp,
    IndexQueryResult, ProviderBinding, ProviderNamespace, ProviderRegistry, QueryOp, QueryResult,
    SorStoreProvider, SorxCanonicalStore, StoreEvidenceOp, StoreProviderKind, TraverseOp,
    TraverseResult, UpdateOp, default_collection_name,
};
pub use provider_compatibility::{
    ProviderCapabilityRequirement, ProviderCompatibilityInput, ProviderCompatibilityIssue,
    ProviderCompatibilityIssueCategory, ProviderCompatibilityReport, ProviderCompatibilityStatus,
    ProviderResolutionMode, ResolvedProviderBinding, resolve_provider_compatibility,
};
pub use providers::{FoundationDbProviderAdapter, FoundationDbProviderConfig, MemoryStoreProvider};
pub use router::EndpointRouter;
pub use runtime::{SorxRuntime, empty_object, invocation, runtime_pack};
pub use startup::{
    AuditConfig, DeploymentConfig, ExposureConfig, GhcrConfig, GhcrWebhookAnswerConfig, McpConfig,
    ProviderBindingConfig, ServerConfig, SorxNormalizedAnswers, SorxRuntimeConfig,
    SorxStartAnswers, SorxStartupError, SorxStartupIssue, build_startup_plan, default_start_schema,
    normalize_start_answers, runtime_config_from_answers,
};

pub const SORX_VERSION_SCHEMA: &str = "greentic.sorx.version.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SorxVersion {
    pub schema: String,
    pub version: String,
}

impl SorxVersion {
    pub fn current() -> Self {
        Self {
            schema: SORX_VERSION_SCHEMA.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SorxCommandContext {
    pub working_dir: PathBuf,
    pub non_interactive: bool,
}

impl SorxCommandContext {
    pub fn new(working_dir: PathBuf, non_interactive: bool) -> Self {
        Self {
            working_dir,
            non_interactive,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SorxCommand {
    Doctor,
    Inspect,
    Routes,
    Start,
    Run,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedSorlaPack;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_version_uses_sorx_schema() {
        let version = SorxVersion::current();
        assert_eq!(version.schema, SORX_VERSION_SCHEMA);
        assert!(!version.version.is_empty());
    }

    #[test]
    fn context_records_working_directory_and_interactivity() {
        let context = SorxCommandContext::new(PathBuf::from("/tmp/sorx"), true);
        assert_eq!(context.working_dir, PathBuf::from("/tmp/sorx"));
        assert!(context.non_interactive);
    }
}
