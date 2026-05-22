use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};
use greentic_sorx_core::{
    CreateDeploymentRequest, DeploymentVisibility, DeterministicEvidenceProvider, EndpointRouter,
    EvidenceProvider, EvidenceQueryFilter, EvidenceQueryResult, GhcrWebhookConfig,
    GhcrWebhookError, GithubWebhookHeaders, LocalDeploymentRegistryStore, OciArtifactResolver,
    OciReference, OntologyAuditEvent, OntologyConceptNode, OntologyGraphService,
    OntologyPolicyAction, OntologyPolicyDecisionKind, OntologyPolicyResource,
    OntologyPolicySubject, OntologyRelationshipEdge, OntologyScope, PackArtifact, PolicyEngine,
    ProviderCompatibilityInput, ProviderCompatibilityStatus, ProviderResolutionMode,
    ResolvedOciArtifact, RollbackAliasRequest, ScopedEntity, SensitivityContext,
    SorxCommandContext, StateMode, build_startup_plan, handle_ghcr_published_webhook,
    mcp_tools_from_metadata, normalize_start_answers,
    ontology_audit_event as core_ontology_audit_event, parse_ghcr_published_metadata,
    resolve_provider_compatibility, runtime_config_from_answers,
};
use greentic_sorx_pack::{
    SorxDoctorReport, SorxInspectReport, doctor_sorla_loaded_pack, doctor_sorla_pack,
    inspect_gtpack_bytes, inspect_sorla_pack, load_sorla_pack, load_sorla_pack_from_bytes,
    startup_schema_from_gtpack_bytes,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod http_runtime;
mod validation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SorxExitCode {
    Success = 0,
    Generic = 1,
    Usage = 2,
    PackValidation = 3,
    AnswersValidation = 4,
    ProviderResolution = 5,
    RuntimeStartup = 6,
    PolicyDenied = 7,
}

impl From<SorxExitCode> for ExitCode {
    fn from(value: SorxExitCode) -> Self {
        ExitCode::from(value as u8)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliError {
    message: String,
    exit_code: SorxExitCode,
}

impl CliError {
    fn new(exit_code: SorxExitCode, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code,
        }
    }

    fn generic(message: impl Into<String>) -> Self {
        Self::new(SorxExitCode::Generic, message)
    }

    fn usage(message: impl Into<String>) -> Self {
        Self::new(SorxExitCode::Usage, message)
    }

    fn pack(message: impl Into<String>) -> Self {
        Self::new(SorxExitCode::PackValidation, message)
    }

    fn answers(message: impl Into<String>) -> Self {
        Self::new(SorxExitCode::AnswersValidation, message)
    }

    fn provider(message: impl Into<String>) -> Self {
        Self::new(SorxExitCode::ProviderResolution, message)
    }

    fn runtime(message: impl Into<String>) -> Self {
        Self::new(SorxExitCode::RuntimeStartup, message)
    }
}

impl From<String> for CliError {
    fn from(value: String) -> Self {
        Self::generic(value)
    }
}

impl From<&str> for CliError {
    fn from(value: &str) -> Self {
        Self::generic(value)
    }
}

type CliResult<T> = Result<T, CliError>;

#[derive(Debug, Parser)]
#[command(
    name = "greentic-sorx",
    version,
    about = "Run SoRLa .gtpack artifacts as Greentic systems of record."
)]
pub struct Cli {
    /// Run without interactive prompts.
    #[arg(long, global = true)]
    non_interactive: bool,

    /// Locale code for localized CLI text, such as en or es.
    #[arg(long, global = true)]
    locale: Option<String>,

    /// Path to the local deployment registry JSON file.
    #[arg(long, global = true)]
    registry: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Validate a SoRLa .gtpack for SORX runtime use.
    Doctor {
        /// Path to a SoRLa .gtpack archive.
        pack: PathBuf,

        /// Emit the doctor report as stable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Validate or inspect generated artifact inputs.
    Artifact {
        #[command(subcommand)]
        command: ArtifactCommands,
    },
    /// Inspect a SoRLa .gtpack and print stable metadata.
    Inspect {
        /// Path to a SoRLa .gtpack archive.
        pack: PathBuf,

        /// Emit stable JSON. This is currently the default output format.
        #[arg(long)]
        json: bool,
    },
    /// List endpoint routes declared by a SoRLa .gtpack.
    Routes {
        /// Path to a SoRLa .gtpack archive.
        pack: Option<PathBuf>,

        /// Existing deployment identifier to inspect.
        #[arg(long)]
        deployment: Option<String>,

        /// Emit stable JSON. This is currently the default output format for pack routes.
        #[arg(long)]
        json: bool,
    },
    /// List MCP tools declared by a SoRLa .gtpack.
    McpTools {
        /// Path to a SoRLa .gtpack archive.
        pack: PathBuf,
    },
    /// Traverse static ontology graphs.
    Graph {
        #[command(subcommand)]
        command: GraphCommands,
    },
    /// Query ontology-scoped evidence.
    Evidence {
        #[command(subcommand)]
        command: EvidenceCommands,
    },
    /// MCP runtime commands.
    Mcp {
        #[command(subcommand)]
        command: McpCommands,
    },
    /// Manage local SORX deployments.
    Deployments {
        #[command(subcommand)]
        command: DeploymentCommands,
    },
    /// Manage deployment aliases.
    Aliases {
        #[command(subcommand)]
        command: AliasCommands,
    },
    /// Verify or replay GHCR publish webhook fixtures.
    Webhook {
        #[command(subcommand)]
        command: WebhookCommands,
    },
    /// Execute pack-embedded validation suites.
    Validate {
        /// Path to a SoRLa .gtpack archive.
        pack: PathBuf,

        /// Path to startup answers JSON.
        #[arg(long)]
        answers: PathBuf,

        /// Provider mode: in-memory, configured, or mock.
        #[arg(long = "provider-mode", default_value = "in-memory")]
        provider_mode: String,

        /// Preserve ephemeral state on failure.
        #[arg(long)]
        preserve_state_on_failure: bool,

        /// Emit stable JSON. This is currently the default output format.
        #[arg(long)]
        json: bool,

        /// Optional JUnit output path.
        #[arg(long = "junit-out")]
        junit_out: Option<PathBuf>,
    },
    /// Validation report commands.
    Validation {
        #[command(subcommand)]
        command: ValidationCommands,
    },
    /// Plan and apply canonical state migrations.
    Migrate {
        #[command(subcommand)]
        command: MigrationCommands,
    },
    /// Start a SORX runtime from a SoRLa .gtpack and startup answers.
    Start {
        /// Path to a SoRLa .gtpack archive.
        pack: PathBuf,

        /// Emit the startup answer schema and exit.
        #[arg(long)]
        schema: bool,

        /// Path to startup answers JSON.
        #[arg(long)]
        answers: Option<PathBuf>,

        /// Validate pack and answers, emit a startup plan, and do not start a runtime.
        #[arg(long)]
        dry_run: bool,

        /// Emit normalized full answers instead of a startup plan.
        #[arg(long)]
        emit_answers: bool,

        /// Emit machine-readable JSON for schema, dry-run, or normalized answer output.
        #[arg(long)]
        json: bool,
    },
    /// Alias for start.
    Run {
        /// Path to a SoRLa .gtpack archive.
        pack: PathBuf,

        /// Path to startup answers JSON.
        #[arg(long)]
        answers: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ArtifactCommands {
    /// Validate a .gtpack file or Designer artifact JSON.
    Validate {
        /// Path to a SoRLa .gtpack archive.
        #[arg(long)]
        file: Option<PathBuf>,

        /// Path to Designer generic artifact JSON.
        #[arg(long = "artifact-json")]
        artifact_json: Option<PathBuf>,

        /// Optional startup answers JSON for provider compatibility.
        #[arg(long)]
        answers: Option<PathBuf>,

        /// Emit stable JSON. This is currently the default output format.
        #[arg(long)]
        json: bool,
    },
    /// Inspect a .gtpack file or Designer artifact JSON.
    Inspect {
        /// Path to a SoRLa .gtpack archive.
        #[arg(long)]
        file: Option<PathBuf>,

        /// Path to Designer generic artifact JSON.
        #[arg(long = "artifact-json")]
        artifact_json: Option<PathBuf>,

        /// Emit stable JSON. This is currently the default output format.
        #[arg(long)]
        json: bool,
    },
    /// Emit the embedded startup schema from a .gtpack file or Designer artifact JSON.
    StartupSchema {
        /// Path to a SoRLa .gtpack archive.
        #[arg(long)]
        file: Option<PathBuf>,

        /// Path to Designer generic artifact JSON.
        #[arg(long = "artifact-json")]
        artifact_json: Option<PathBuf>,

        /// Emit stable JSON. This is currently the default output format.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum WebhookCommands {
    /// Parse and validate a fixture payload shape without mutating the registry.
    VerifyFixture {
        /// Fixture JSON path.
        fixture: PathBuf,
    },
    /// Replay a signed fixture through the local deployment registry.
    Replay {
        /// Fixture JSON path.
        #[arg(long)]
        fixture: PathBuf,

        /// X-Hub-Signature-256 value.
        #[arg(long)]
        signature: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum GraphCommands {
    /// List ontology concepts.
    Concepts {
        pack: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// List ontology relationships.
    Relationships {
        pack: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Find type paths between ontology concepts.
    Paths {
        pack: PathBuf,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long, default_value_t = 4)]
        max_depth: u8,
        #[arg(long)]
        json: bool,
    },
    /// List static ontology relationships near an entity type.
    Neighbors {
        pack: PathBuf,
        #[arg(long = "entity-type")]
        entity_type: String,
        #[arg(long = "entity-id")]
        entity_id: String,
        #[arg(long, default_value_t = 1)]
        depth: u8,
        #[arg(long)]
        json: bool,
    },
    /// Explain type paths between ontology concepts.
    Explain {
        pack: PathBuf,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long, default_value_t = 4)]
        max_depth: u8,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum EvidenceCommands {
    /// Query evidence within an ontology scope.
    Query {
        pack: PathBuf,
        #[arg(long)]
        answers: PathBuf,
        #[arg(long)]
        query: String,
        #[arg(long = "entity-type")]
        entity_type: String,
        #[arg(long = "entity-id")]
        entity_id: String,
        #[arg(long, default_value_t = 1)]
        max_depth: u8,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ValidationCommands {
    /// Print the latest stored validation report for a deployment.
    Report {
        /// Deployment identifier.
        deployment_id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum MigrationCommands {
    /// Build a deterministic migration plan between two packs.
    Plan {
        #[arg(long = "from")]
        from: PathBuf,
        #[arg(long = "to")]
        to: PathBuf,
        #[arg(long)]
        tenant: String,
        #[arg(long)]
        sor: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Validate and report what a migration plan would do.
    DryRun {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        answers: PathBuf,
    },
    /// Mark a migration plan as applied after validation.
    Apply {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        answers: PathBuf,
        #[arg(long = "allow-destructive")]
        allow_destructive: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum DeploymentCommands {
    /// List deployments in the local registry.
    List,
    /// Inspect one deployment.
    Inspect {
        /// Deployment identifier.
        deployment_id: String,
    },
    /// Create a deployment record from a .gtpack artifact.
    Create {
        /// Path to a SoRLa .gtpack archive.
        #[arg(long)]
        pack: PathBuf,

        /// Tenant identifier.
        #[arg(long)]
        tenant: String,

        /// SOR name for this deployment family.
        #[arg(long)]
        sor: String,

        /// Runtime environment.
        #[arg(long)]
        environment: String,

        /// API version label, such as v1 or v1.1.
        #[arg(long = "api-version")]
        api_version: String,

        /// Base path for versioned routes.
        #[arg(long = "base-path")]
        base_path: String,

        /// Visibility: private, internal, or public.
        #[arg(long, default_value = "private")]
        visibility: String,

        /// Optional state namespace override.
        #[arg(long = "state-namespace")]
        state_namespace: Option<String>,

        /// State mode: isolated, shared_compatible, or shared_requires_migration.
        #[arg(long = "state-mode", default_value = "shared_compatible")]
        state_mode: String,

        /// Allow reusing an API version label for a different digest.
        #[arg(long)]
        allow_api_version_conflict: bool,

        /// Allow sharing a state namespace without compatible pack metadata.
        #[arg(long)]
        allow_shared_state_conflict: bool,
    },
    /// Mark a deployment as validated.
    Validate {
        /// Deployment identifier.
        deployment_id: String,
    },
    /// Activate a deployment as private/internal only.
    Activate {
        /// Deployment identifier.
        deployment_id: String,

        /// Activate without public exposure.
        #[arg(long)]
        private: bool,
    },
    /// Promote a validated deployment to private/public route tables or an alias.
    Promote {
        /// Deployment identifier.
        deployment_id: String,

        /// Promote public exposure for the versioned route.
        #[arg(long)]
        public: bool,

        /// Move an alias, such as preview or latest, to this deployment.
        #[arg(long)]
        alias: Option<String>,

        /// Actor for audit records.
        #[arg(long, default_value = "local-admin")]
        actor: String,

        /// Automation source for audit records.
        #[arg(long = "automation-source")]
        automation_source: Option<String>,
    },
    /// Roll back an alias to a previous deployment.
    Rollback {
        #[arg(long)]
        tenant: String,

        #[arg(long)]
        sor: String,

        #[arg(long)]
        alias: String,

        #[arg(long = "to")]
        to_deployment_id: String,

        #[arg(long, default_value = "manual rollback")]
        reason: String,

        #[arg(long, default_value = "local-admin")]
        actor: String,

        #[arg(long = "automation-source")]
        automation_source: Option<String>,
    },
    /// Retire old active deployments, keeping the newest N active records.
    RetireOld {
        #[arg(long)]
        tenant: String,

        #[arg(long)]
        sor: String,

        #[arg(long)]
        keep: usize,
    },
    /// List deployments that are exposed in the public route table.
    PublicRoutes,
    /// Print promotion readiness for a deployment.
    PromotionStatus {
        /// Deployment identifier.
        deployment_id: String,
    },
    /// Retire a deployment and remove aliases that point to it.
    Retire {
        /// Deployment identifier.
        deployment_id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum AliasCommands {
    /// Set or move an alias to a deployment.
    Set {
        #[arg(long)]
        tenant: String,

        #[arg(long)]
        sor: String,

        #[arg(long)]
        alias: String,

        #[arg(long)]
        target: String,
    },
    /// List aliases, optionally scoped by tenant and SOR.
    List {
        #[arg(long)]
        tenant: Option<String>,

        #[arg(long)]
        sor: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum McpCommands {
    /// Build the MCP tool runtime from a SoRLa .gtpack and startup answers.
    Start {
        /// Path to a SoRLa .gtpack archive.
        pack: PathBuf,

        /// Path to startup answers JSON.
        #[arg(long)]
        answers: PathBuf,
    },
}

pub fn run<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    if help_requested(&args)
        && let Some(locale) = locale_from_args(&args)
        && locale != "en"
    {
        print!("{}", localized_help(&locale));
        return ExitCode::SUCCESS;
    }

    match Cli::try_parse_from(args) {
        Ok(cli) => {
            let working_dir = match std::env::current_dir() {
                Ok(path) => path,
                Err(err) => {
                    eprintln!("failed to read current directory: {err}");
                    return ExitCode::FAILURE;
                }
            };
            let context = SorxCommandContext::new(working_dir, cli.non_interactive);
            let _locale = cli.locale.as_deref().unwrap_or("en");
            let registry_path = cli.registry.clone();
            match dispatch(cli.command, &context, registry_path) {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("{}", err.message);
                    err.exit_code.into()
                }
            }
        }
        Err(err) => {
            let _ = err.print();
            match err.kind() {
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
                    ExitCode::SUCCESS
                }
                _ => ExitCode::from(2),
            }
        }
    }
}

pub fn command() -> clap::Command {
    Cli::command()
}

fn help_requested(args: &[OsString]) -> bool {
    args.iter().any(|arg| arg == "--help" || arg == "-h")
}

fn locale_from_args(args: &[OsString]) -> Option<String> {
    for (index, arg) in args.iter().enumerate() {
        if arg == "--locale" {
            return args
                .get(index + 1)
                .and_then(|value| value.to_str())
                .map(ToString::to_string);
        }
        let Some(arg) = arg.to_str() else {
            continue;
        };
        if let Some(value) = arg.strip_prefix("--locale=") {
            return Some(value.to_string());
        }
    }
    None
}

fn localized_help(locale: &str) -> String {
    let catalog = I18nCatalog::load(locale);
    let mut cmd = command();
    cmd = cmd.about(catalog.text("cli.about"));
    set_subcommand_about(&mut cmd, "doctor", catalog.text("cli.command.doctor.about"));
    set_subcommand_about(
        &mut cmd,
        "inspect",
        catalog.text("cli.command.inspect.about"),
    );
    set_subcommand_about(&mut cmd, "routes", catalog.text("cli.command.routes.about"));
    set_subcommand_about(
        &mut cmd,
        "mcp-tools",
        catalog.text("cli.command.mcp-tools.about"),
    );
    set_subcommand_about(&mut cmd, "graph", catalog.text("cli.command.graph.about"));
    set_subcommand_about(
        &mut cmd,
        "evidence",
        catalog.text("cli.command.evidence.about"),
    );
    set_subcommand_about(&mut cmd, "mcp", catalog.text("cli.command.mcp.about"));
    set_subcommand_about(
        &mut cmd,
        "deployments",
        catalog.text("cli.command.deployments.about"),
    );
    set_subcommand_about(
        &mut cmd,
        "aliases",
        catalog.text("cli.command.aliases.about"),
    );
    set_subcommand_about(
        &mut cmd,
        "webhook",
        catalog.text("cli.command.webhook.about"),
    );
    set_subcommand_about(
        &mut cmd,
        "validate",
        catalog.text("cli.command.validate.about"),
    );
    set_subcommand_about(
        &mut cmd,
        "validation",
        catalog.text("cli.command.validation.about"),
    );
    set_subcommand_about(&mut cmd, "run", catalog.text("cli.command.run.about"));
    set_subcommand_about(&mut cmd, "start", catalog.text("cli.command.start.about"));
    set_subcommand_about(&mut cmd, "help", catalog.text("cli.command.help.about"));
    cmd = cmd
        .mut_arg("non_interactive", |arg| {
            arg.help(catalog.text("cli.option.non_interactive.help"))
        })
        .mut_arg("locale", |arg| {
            arg.help(catalog.text("cli.option.locale.help"))
        })
        .mut_arg("registry", |arg| {
            arg.help(catalog.text("cli.option.registry.help"))
        })
        .help_template(catalog.text("cli.help.template"));
    let mut help = cmd.render_long_help().to_string();
    for (from, to) in catalog.replacements() {
        help = help.replace(from, &to);
    }
    help
}

fn set_subcommand_about(cmd: &mut clap::Command, name: &str, about: String) {
    if let Some(subcommand) = cmd.find_subcommand_mut(name) {
        let next = std::mem::take(subcommand).about(about);
        *subcommand = next;
    }
}

#[derive(Debug, Clone)]
struct I18nCatalog {
    values: serde_json::Map<String, serde_json::Value>,
}

impl I18nCatalog {
    fn load(locale: &str) -> Self {
        let requested = read_i18n_catalog(locale);
        let fallback = read_i18n_catalog("en")
            .or_else(|| serde_json::from_str(include_str!("../i18n/en.json")).ok())
            .and_then(|value: serde_json::Value| value.as_object().cloned())
            .unwrap_or_default();
        let mut values = fallback;
        if let Some(requested) = requested.and_then(|value| value.as_object().cloned()) {
            values.extend(requested);
        }
        Self { values }
    }

    fn text(&self, key: &str) -> String {
        self.values
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or(key)
            .to_string()
    }

    fn replacements(&self) -> Vec<(&'static str, String)> {
        vec![
            ("Print help", self.text("cli.option.help.help")),
            ("Print version", self.text("cli.option.version.help")),
            (
                "Print this message or the help of the given subcommand(s)",
                self.text("cli.command.help.about"),
            ),
        ]
    }
}

fn read_i18n_catalog(locale: &str) -> Option<serde_json::Value> {
    let relative = PathBuf::from("i18n").join(format!("{locale}.json"));
    let raw = fs::read_to_string(&relative).ok().or_else(|| {
        fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(relative),
        )
        .ok()
    });
    let raw = raw.as_deref().or_else(|| embedded_i18n_catalog(locale))?;
    serde_json::from_str(raw).ok()
}

fn embedded_i18n_catalog(locale: &str) -> Option<&'static str> {
    match locale {
        "ar-AE" => Some(include_str!("../i18n/ar-AE.json")),
        "ar-DZ" => Some(include_str!("../i18n/ar-DZ.json")),
        "ar-EG" => Some(include_str!("../i18n/ar-EG.json")),
        "ar-IQ" => Some(include_str!("../i18n/ar-IQ.json")),
        "ar-MA" => Some(include_str!("../i18n/ar-MA.json")),
        "ar-SA" => Some(include_str!("../i18n/ar-SA.json")),
        "ar-SD" => Some(include_str!("../i18n/ar-SD.json")),
        "ar-SY" => Some(include_str!("../i18n/ar-SY.json")),
        "ar-TN" => Some(include_str!("../i18n/ar-TN.json")),
        "ar" => Some(include_str!("../i18n/ar.json")),
        "ay" => Some(include_str!("../i18n/ay.json")),
        "bg" => Some(include_str!("../i18n/bg.json")),
        "bn" => Some(include_str!("../i18n/bn.json")),
        "cs" => Some(include_str!("../i18n/cs.json")),
        "da" => Some(include_str!("../i18n/da.json")),
        "de" => Some(include_str!("../i18n/de.json")),
        "el" => Some(include_str!("../i18n/el.json")),
        "en-GB" => Some(include_str!("../i18n/en-GB.json")),
        "en" => Some(include_str!("../i18n/en.json")),
        "es" => Some(include_str!("../i18n/es.json")),
        "et" => Some(include_str!("../i18n/et.json")),
        "fa" => Some(include_str!("../i18n/fa.json")),
        "fi" => Some(include_str!("../i18n/fi.json")),
        "fr" => Some(include_str!("../i18n/fr.json")),
        "gn" => Some(include_str!("../i18n/gn.json")),
        "gu" => Some(include_str!("../i18n/gu.json")),
        "hi" => Some(include_str!("../i18n/hi.json")),
        "hr" => Some(include_str!("../i18n/hr.json")),
        "ht" => Some(include_str!("../i18n/ht.json")),
        "hu" => Some(include_str!("../i18n/hu.json")),
        "id" => Some(include_str!("../i18n/id.json")),
        "it" => Some(include_str!("../i18n/it.json")),
        "ja" => Some(include_str!("../i18n/ja.json")),
        "km" => Some(include_str!("../i18n/km.json")),
        "kn" => Some(include_str!("../i18n/kn.json")),
        "ko" => Some(include_str!("../i18n/ko.json")),
        "lo" => Some(include_str!("../i18n/lo.json")),
        "lt" => Some(include_str!("../i18n/lt.json")),
        "lv" => Some(include_str!("../i18n/lv.json")),
        "ml" => Some(include_str!("../i18n/ml.json")),
        "mr" => Some(include_str!("../i18n/mr.json")),
        "ms" => Some(include_str!("../i18n/ms.json")),
        "my" => Some(include_str!("../i18n/my.json")),
        "nah" => Some(include_str!("../i18n/nah.json")),
        "ne" => Some(include_str!("../i18n/ne.json")),
        "nl" => Some(include_str!("../i18n/nl.json")),
        "no" => Some(include_str!("../i18n/no.json")),
        "pa" => Some(include_str!("../i18n/pa.json")),
        "pl" => Some(include_str!("../i18n/pl.json")),
        "pt" => Some(include_str!("../i18n/pt.json")),
        "qu" => Some(include_str!("../i18n/qu.json")),
        "ro" => Some(include_str!("../i18n/ro.json")),
        "ru" => Some(include_str!("../i18n/ru.json")),
        "si" => Some(include_str!("../i18n/si.json")),
        "sk" => Some(include_str!("../i18n/sk.json")),
        "sr" => Some(include_str!("../i18n/sr.json")),
        "sv" => Some(include_str!("../i18n/sv.json")),
        "ta" => Some(include_str!("../i18n/ta.json")),
        "te" => Some(include_str!("../i18n/te.json")),
        "th" => Some(include_str!("../i18n/th.json")),
        "tl" => Some(include_str!("../i18n/tl.json")),
        "tr" => Some(include_str!("../i18n/tr.json")),
        "uk" => Some(include_str!("../i18n/uk.json")),
        "ur" => Some(include_str!("../i18n/ur.json")),
        "vi" => Some(include_str!("../i18n/vi.json")),
        "zh" => Some(include_str!("../i18n/zh.json")),
        _ => None,
    }
}

pub fn parse_from<I, T>(args: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    Cli::try_parse_from(args)
}

fn dispatch(
    command: Commands,
    _context: &SorxCommandContext,
    registry_path: Option<PathBuf>,
) -> CliResult<()> {
    match command {
        Commands::Doctor { pack, json } => run_doctor(pack, json),
        Commands::Artifact { command } => run_artifact(command, _context),
        Commands::Inspect { pack, json } => run_inspect(pack, json),
        Commands::Routes {
            pack,
            deployment,
            json,
        } => match (pack, deployment) {
            (Some(pack), None) => run_routes(pack, json),
            (None, Some(deployment)) => run_deployment_routes(deployment, registry_path, json),
            (Some(_), Some(_)) => Err(CliError::usage(
                "routes accepts either a .gtpack path or --deployment, not both",
            )),
            (None, None) => Err(CliError::usage(
                "routes requires a .gtpack path or --deployment",
            )),
        },
        Commands::McpTools { pack } => run_mcp_tools(pack),
        Commands::Graph { command } => run_graph(command),
        Commands::Evidence { command } => run_evidence(command, _context),
        Commands::Mcp {
            command: McpCommands::Start { pack, answers },
        } => run_mcp_start(pack, answers, _context),
        Commands::Deployments { command } => run_deployments(command, registry_path),
        Commands::Aliases { command } => run_aliases(command, registry_path),
        Commands::Webhook { command } => run_webhook(command, registry_path),
        Commands::Validate {
            pack,
            answers,
            provider_mode,
            preserve_state_on_failure,
            json,
            junit_out,
        } => run_validate(
            pack,
            answers,
            provider_mode,
            preserve_state_on_failure,
            json,
            junit_out,
            _context,
        ),
        Commands::Validation { command } => run_validation_command(command, registry_path),
        Commands::Migrate { command } => run_migrate(command),
        Commands::Start {
            pack,
            schema,
            answers,
            dry_run,
            emit_answers,
            json,
        } => {
            if !schema && answers.is_none() {
                return Err(CliError::usage(
                    "start requires --schema or --answers <FILE>",
                ));
            }
            run_start(pack, schema, answers, dry_run, emit_answers, json, _context)
        }
        Commands::Run { pack, answers } => {
            if answers.is_none() {
                return Err(CliError::usage("run requires --answers <FILE>"));
            }
            run_start(pack, false, answers, false, false, false, _context)
        }
    }
}

fn run_deployments(command: DeploymentCommands, registry_path: Option<PathBuf>) -> CliResult<()> {
    let store = LocalDeploymentRegistryStore::new(resolve_registry_path(registry_path)?);
    match command {
        DeploymentCommands::List => {
            let registry = store.load().map_err(registry_error)?;
            print_json(&serde_json::json!({
                "schema": "greentic.sorx.deployments.v1",
                "deployments": registry.deployments
            }))
        }
        DeploymentCommands::Inspect { deployment_id } => {
            let registry = store.load().map_err(registry_error)?;
            let deployment = registry.deployment(&deployment_id).ok_or_else(|| {
                CliError::usage(format!("deployment `{deployment_id}` does not exist"))
            })?;
            print_json(deployment)
        }
        DeploymentCommands::Create {
            pack,
            tenant,
            sor,
            environment,
            api_version,
            base_path,
            visibility,
            state_namespace,
            state_mode,
            allow_api_version_conflict,
            allow_shared_state_conflict,
        } => {
            let loaded = load_sorla_pack(&pack).map_err(|err| CliError::pack(err.to_string()))?;
            let visibility = DeploymentVisibility::parse(&visibility).ok_or_else(|| {
                CliError::usage("visibility must be private, internal, or public")
            })?;
            if visibility == DeploymentVisibility::Public {
                return Err(CliError::usage(
                    "deployment create cannot bypass public gates; create as private and use deployments promote",
                ));
            }
            let state_mode = StateMode::parse(&state_mode).ok_or_else(|| {
                CliError::usage(
                    "state mode must be isolated, shared_compatible, or shared_requires_migration",
                )
            })?;
            let mut registry = store.load().map_err(registry_error)?;
            let deployment = registry
                .create_deployment(CreateDeploymentRequest {
                    artifact: PackArtifact {
                        source: pack.display().to_string(),
                        name: loaded.pack_name.clone(),
                        version: loaded.pack_version.clone(),
                        digest: loaded
                            .pack_digest
                            .clone()
                            .unwrap_or_else(|| "sha256:unknown".to_string()),
                        signature: loaded
                            .manifest
                            .integrity
                            .as_ref()
                            .and_then(|integrity| integrity.signature.clone()),
                        signature_ref: loaded
                            .manifest
                            .integrity
                            .as_ref()
                            .and_then(|integrity| integrity.signature_ref.clone()),
                    },
                    tenant_id: tenant,
                    sor_name: sor,
                    environment,
                    api_version_label: api_version,
                    base_path,
                    visibility,
                    state_mode,
                    state_namespace,
                    deployment_id: None,
                    allow_api_version_conflict,
                    allow_shared_state_conflict,
                })
                .map_err(registry_error)?;
            store.save(&registry).map_err(registry_error)?;
            print_json(&deployment)
        }
        DeploymentCommands::Validate { deployment_id } => {
            let mut registry = store.load().map_err(registry_error)?;
            let deployment = registry
                .validate_deployment(&deployment_id)
                .map_err(registry_error)?;
            store.save(&registry).map_err(registry_error)?;
            print_json(&deployment)
        }
        DeploymentCommands::Activate {
            deployment_id,
            private,
        } => {
            if !private {
                return Err(CliError::usage(
                    "activate requires --private; use deployments promote for gated public exposure",
                ));
            }
            let mut registry = store.load().map_err(registry_error)?;
            let deployment = registry
                .activate_private(&deployment_id)
                .map_err(registry_error)?;
            store.save(&registry).map_err(registry_error)?;
            print_json(&deployment)
        }
        DeploymentCommands::Promote {
            deployment_id,
            public,
            alias,
            actor,
            automation_source,
        } => {
            let mut registry = store.load().map_err(registry_error)?;
            let output = if let Some(alias) = alias {
                let alias = registry
                    .promote_alias(&deployment_id, &alias, public, actor, automation_source)
                    .map_err(registry_error)?;
                serde_json::to_value(alias).map_err(|err| {
                    CliError::generic(format!("failed to encode promotion alias: {err}"))
                })?
            } else if public {
                let deployment = registry
                    .promote_public(&deployment_id, actor, automation_source)
                    .map_err(registry_error)?;
                serde_json::to_value(deployment).map_err(|err| {
                    CliError::generic(format!("failed to encode promoted deployment: {err}"))
                })?
            } else {
                let deployment = registry
                    .promote_private(&deployment_id, actor, automation_source)
                    .map_err(registry_error)?;
                serde_json::to_value(deployment).map_err(|err| {
                    CliError::generic(format!("failed to encode promoted deployment: {err}"))
                })?
            };
            store.save(&registry).map_err(registry_error)?;
            print_json(&output)
        }
        DeploymentCommands::Rollback {
            tenant,
            sor,
            alias,
            to_deployment_id,
            reason,
            actor,
            automation_source,
        } => {
            let mut registry = store.load().map_err(registry_error)?;
            let alias = registry
                .rollback_alias(RollbackAliasRequest {
                    tenant_id: tenant,
                    sor_name: sor,
                    alias,
                    to_deployment_id,
                    reason,
                    actor,
                    automation_source,
                })
                .map_err(registry_error)?;
            store.save(&registry).map_err(registry_error)?;
            print_json(&alias)
        }
        DeploymentCommands::RetireOld { tenant, sor, keep } => {
            let mut registry = store.load().map_err(registry_error)?;
            let retired = registry
                .retire_old(&tenant, &sor, keep)
                .map_err(registry_error)?;
            store.save(&registry).map_err(registry_error)?;
            print_json(&serde_json::json!({
                "schema": "greentic.sorx.retired-deployments.v1",
                "retired": retired
            }))
        }
        DeploymentCommands::PublicRoutes => {
            let registry = store.load().map_err(registry_error)?;
            print_json(&serde_json::json!({
                "schema": "greentic.sorx.public-routes.v1",
                "deployments": registry.public_deployments()
            }))
        }
        DeploymentCommands::PromotionStatus { deployment_id } => {
            let registry = store.load().map_err(registry_error)?;
            let status = registry
                .promotion_status(&deployment_id)
                .map_err(registry_error)?;
            print_json(&status)
        }
        DeploymentCommands::Retire { deployment_id } => {
            let mut registry = store.load().map_err(registry_error)?;
            let deployment = registry.retire(&deployment_id).map_err(registry_error)?;
            store.save(&registry).map_err(registry_error)?;
            print_json(&deployment)
        }
    }
}

fn run_aliases(command: AliasCommands, registry_path: Option<PathBuf>) -> CliResult<()> {
    let store = LocalDeploymentRegistryStore::new(resolve_registry_path(registry_path)?);
    match command {
        AliasCommands::Set {
            tenant,
            sor,
            alias,
            target,
        } => {
            let mut registry = store.load().map_err(registry_error)?;
            let alias = registry
                .set_alias(tenant, sor, alias, target)
                .map_err(registry_error)?;
            store.save(&registry).map_err(registry_error)?;
            print_json(&alias)
        }
        AliasCommands::List { tenant, sor } => {
            let registry = store.load().map_err(registry_error)?;
            print_json(&serde_json::json!({
                "schema": "greentic.sorx.aliases.v1",
                "aliases": registry.aliases_for(tenant.as_deref(), sor.as_deref())
            }))
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct WebhookFixture {
    #[serde(default = "default_event")]
    event: String,
    #[serde(default = "default_delivery")]
    delivery: String,
    #[serde(default)]
    secret: Option<String>,
    #[serde(default)]
    resolved_digest: Option<String>,
    payload: serde_json::Value,
}

fn default_event() -> String {
    "repository_dispatch".to_string()
}

fn default_delivery() -> String {
    "fixture-delivery".to_string()
}

fn run_webhook(command: WebhookCommands, registry_path: Option<PathBuf>) -> CliResult<()> {
    match command {
        WebhookCommands::VerifyFixture { fixture } => {
            let fixture = read_webhook_fixture(&fixture)?;
            let payload = fixture_payload_bytes(&fixture)?;
            let metadata = parse_ghcr_published_metadata(&payload).map_err(ghcr_webhook_error)?;
            print_json(&serde_json::json!({
                "schema": "greentic.sorx.webhook.fixture.v1",
                "ok": true,
                "event": fixture.event,
                "delivery": fixture.delivery,
                "metadata": metadata
            }))
        }
        WebhookCommands::Replay { fixture, signature } => {
            let fixture = read_webhook_fixture(&fixture)?;
            let payload = fixture_payload_bytes(&fixture)?;
            let metadata = parse_ghcr_published_metadata(&payload).map_err(ghcr_webhook_error)?;
            let secret = fixture.secret.as_deref().ok_or_else(|| {
                CliError::usage("webhook replay fixture must include a test-only `secret` field")
            })?;
            let headers = GithubWebhookHeaders {
                signature_256: signature,
                event: fixture.event,
                delivery: fixture.delivery,
            };
            let resolver = FixtureOciResolver {
                digest: fixture.resolved_digest.unwrap_or(metadata.digest),
            };
            let mut config = GhcrWebhookConfig::local_test("secret://fixture/github/webhook");
            config.allowed_repositories.sort();
            let store = LocalDeploymentRegistryStore::new(resolve_registry_path(registry_path)?);
            let mut registry = store.load().map_err(registry_error)?;
            let outcome = handle_ghcr_published_webhook(
                &config,
                &mut registry,
                &headers,
                &payload,
                secret.as_bytes(),
                &resolver,
            )
            .map_err(ghcr_webhook_error)?;
            store.save(&registry).map_err(registry_error)?;
            print_json(&outcome)
        }
    }
}

fn run_validate(
    pack: PathBuf,
    answers: PathBuf,
    provider_mode: String,
    preserve_state_on_failure: bool,
    _json: bool,
    junit_out: Option<PathBuf>,
    context: &SorxCommandContext,
) -> CliResult<()> {
    let pack = load_sorla_pack(&pack).map_err(|err| CliError::pack(err.to_string()))?;
    if pack.validation_suite_status == greentic_sorx_pack::ValidationSuiteStatus::Missing {
        let report = validation::missing_suite_report(&pack, "local-validation", false);
        print_json(&report)?;
        return Ok(());
    }
    let raw = fs::read_to_string(&answers).map_err(|err| {
        CliError::answers(format!(
            "failed to read answers {}: {err}",
            answers.display()
        ))
    })?;
    let answers_json: serde_json::Value = serde_json::from_str(&raw).map_err(|err| {
        CliError::answers(format!(
            "answers {} are invalid JSON: {err}",
            answers.display()
        ))
    })?;
    let normalized = normalize_start_answers(
        &pack.sorx_assets.start_schema_json,
        &answers_json,
        context.non_interactive,
    )
    .map_err(|err| CliError::answers(err.to_string()))?;
    let config = runtime_config_from_answers(&pack.pack_name, &normalized.answers)
        .map_err(|err| CliError::answers(err.to_string()))?;
    let provider_mode = validation::ProviderMode::parse(&provider_mode)
        .ok_or_else(|| CliError::usage("provider mode must be in-memory, configured, or mock"))?;
    let report = validation::execute_validation_suite(
        &pack,
        &config,
        &validation::ValidationOptions {
            deployment_id: "local-validation".to_string(),
            provider_mode,
            preserve_state_on_failure,
        },
    )
    .map_err(|err| CliError::generic(format!("{}: {}", err.code, err.message)))?;
    if let Some(path) = junit_out {
        fs::write(&path, junit_xml(&report)).map_err(|err| {
            CliError::generic(format!(
                "failed to write JUnit report {}: {err}",
                path.display()
            ))
        })?;
    }
    print_json(&report)
}

fn run_validation_command(
    command: ValidationCommands,
    registry_path: Option<PathBuf>,
) -> CliResult<()> {
    match command {
        ValidationCommands::Report { deployment_id } => {
            let store = LocalDeploymentRegistryStore::new(resolve_registry_path(registry_path)?);
            let registry = store.load().map_err(registry_error)?;
            let report = registry
                .validation_reports
                .iter()
                .rev()
                .find(|report| {
                    report
                        .get("deployment_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(deployment_id.as_str())
                })
                .ok_or_else(|| {
                    CliError::usage(format!(
                        "validation report for deployment `{deployment_id}` does not exist"
                    ))
                })?;
            print_json(report)
        }
    }
}

fn run_migrate(command: MigrationCommands) -> CliResult<()> {
    match command {
        MigrationCommands::Plan {
            from,
            to,
            tenant,
            sor,
            out,
        } => {
            let from_pack =
                load_sorla_pack(&from).map_err(|err| CliError::pack(err.to_string()))?;
            let to_pack = load_sorla_pack(&to).map_err(|err| CliError::pack(err.to_string()))?;
            let migration_id = migration_id(
                &tenant,
                &sor,
                &from_pack.pack_version,
                &to_pack.pack_version,
            );
            let plan = serde_json::json!({
                "schema": "greentic.sorx.migration-plan.v1",
                "migration_id": migration_id,
                "tenant_id": tenant,
                "sor_name": sor,
                "from": {
                    "source": from.display().to_string(),
                    "pack_name": from_pack.pack_name,
                    "pack_version": from_pack.pack_version,
                    "pack_digest": from_pack.pack_digest
                },
                "to": {
                    "source": to.display().to_string(),
                    "pack_name": to_pack.pack_name,
                    "pack_version": to_pack.pack_version,
                    "pack_digest": to_pack.pack_digest
                },
                "state_namespace": format!("sorx/{}/{}", clean_migration_segment(&tenant), clean_migration_segment(&sor)),
                "destructive": false,
                "steps": migration_steps(&from_pack.pack_version, &to_pack.pack_version)
            });
            write_json_file(&out, &plan)?;
            print_json(&plan)
        }
        MigrationCommands::DryRun { plan, answers } => {
            let plan_json = read_json_file(&plan, "migration plan")?;
            validate_migration_answers(&answers)?;
            print_json(&serde_json::json!({
                "schema": "greentic.sorx.migration-dry-run.v1",
                "migration_id": plan_json["migration_id"],
                "status": "pass",
                "would_apply": plan_json["steps"],
                "status_path": migration_status_path(&plan).display().to_string()
            }))
        }
        MigrationCommands::Apply {
            plan,
            answers,
            allow_destructive,
        } => {
            let plan_json = read_json_file(&plan, "migration plan")?;
            validate_migration_answers(&answers)?;
            if plan_json
                .get("destructive")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
                && !allow_destructive
            {
                return Err(CliError::usage(
                    "destructive migration steps require --allow-destructive",
                ));
            }
            let status_path = migration_status_path(&plan);
            if status_path.exists() {
                let status = read_json_file(&status_path, "migration status")?;
                if status.get("status").and_then(serde_json::Value::as_str) == Some("completed") {
                    return print_json(&status);
                }
            }
            let status = serde_json::json!({
                "schema": "greentic.sorx.migration-status.v1",
                "migration_id": plan_json["migration_id"],
                "tenant_id": plan_json["tenant_id"],
                "sor_name": plan_json["sor_name"],
                "state_namespace": plan_json["state_namespace"],
                "status": "completed",
                "steps_applied": plan_json["steps"].as_array().map(Vec::len).unwrap_or(0)
            });
            write_json_file(&status_path, &status)?;
            print_json(&status)
        }
    }
}

fn migration_steps(from_version: &str, to_version: &str) -> Vec<serde_json::Value> {
    if from_version == to_version {
        Vec::new()
    } else {
        vec![serde_json::json!({
            "id": "metadata.version-transition",
            "kind": "metadata",
            "from_version": from_version,
            "to_version": to_version,
            "destructive": false
        })]
    }
}

fn migration_id(tenant: &str, sor: &str, from_version: &str, to_version: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(tenant.as_bytes());
    hasher.update(b"\0");
    hasher.update(sor.as_bytes());
    hasher.update(b"\0");
    hasher.update(from_version.as_bytes());
    hasher.update(b"\0");
    hasher.update(to_version.as_bytes());
    format!("mig-{}", hex::encode(&hasher.finalize()[..8]))
}

fn migration_status_path(plan: &Path) -> PathBuf {
    plan.with_extension("status.json")
}

fn validate_migration_answers(path: &Path) -> CliResult<()> {
    let answers = read_json_file(path, "answers")?;
    let normalized =
        normalize_start_answers(&greentic_sorx_core::default_start_schema(), &answers, true)
            .map_err(|err| CliError::answers(err.to_string()))?;
    runtime_config_from_answers("migration", &normalized.answers)
        .map_err(|err| CliError::answers(err.to_string()))?;
    Ok(())
}

fn read_json_file(path: &Path, label: &str) -> CliResult<serde_json::Value> {
    let raw = fs::read_to_string(path).map_err(|err| {
        CliError::generic(format!("failed to read {label} {}: {err}", path.display()))
    })?;
    serde_json::from_str(&raw).map_err(|err| {
        CliError::generic(format!("{label} {} is invalid JSON: {err}", path.display()))
    })
}

fn write_json_file(path: &Path, value: &serde_json::Value) -> CliResult<()> {
    let encoded = serde_json::to_string_pretty(value)
        .map_err(|err| CliError::generic(format!("failed to encode JSON: {err}")))?;
    fs::write(path, format!("{encoded}\n"))
        .map_err(|err| CliError::generic(format!("failed to write {}: {err}", path.display())))
}

fn clean_migration_segment(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if cleaned.is_empty() {
        "unnamed".to_string()
    } else {
        cleaned
    }
}

fn junit_xml(report: &validation::ValidationReport) -> String {
    let failures = report
        .tests
        .iter()
        .filter(|test| test.result == validation::ValidationResult::Fail)
        .count();
    let mut out = format!(
        "<testsuite name=\"{}\" tests=\"{}\" failures=\"{}\">",
        escape_xml(&report.suite_id),
        report.tests.len(),
        failures
    );
    for test in &report.tests {
        out.push_str(&format!(
            "<testcase name=\"{}\" time=\"{}\">",
            escape_xml(&test.id),
            test.duration_ms as f64 / 1000.0
        ));
        if test.result == validation::ValidationResult::Fail {
            out.push_str(&format!(
                "<failure>{}</failure>",
                escape_xml(test.message.as_deref().unwrap_or("validation failed"))
            ));
        }
        out.push_str("</testcase>");
    }
    out.push_str("</testsuite>");
    out
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn read_webhook_fixture(path: &PathBuf) -> CliResult<WebhookFixture> {
    let raw = fs::read_to_string(path)
        .map_err(|err| CliError::usage(format!("failed to read {}: {err}", path.display())))?;
    serde_json::from_str(&raw).map_err(|err| {
        CliError::usage(format!("fixture {} is invalid JSON: {err}", path.display()))
    })
}

fn fixture_payload_bytes(fixture: &WebhookFixture) -> CliResult<Vec<u8>> {
    serde_json::to_vec(&fixture.payload)
        .map_err(|err| CliError::generic(format!("failed to encode fixture payload: {err}")))
}

fn ghcr_webhook_error(err: GhcrWebhookError) -> CliError {
    CliError::generic(format!("{}: {}", err.code, err.message))
}

#[derive(Debug)]
struct FixtureOciResolver {
    digest: String,
}

impl OciArtifactResolver for FixtureOciResolver {
    fn resolve(&self, reference: &OciReference) -> Result<ResolvedOciArtifact, GhcrWebhookError> {
        Ok(ResolvedOciArtifact {
            original_ref: reference.value.clone(),
            resolved_digest: self.digest.clone(),
            media_type: "application/vnd.greentic.sorla.gtpack".to_string(),
            size: 0,
            annotations: Default::default(),
            local_cache_path: None,
        })
    }
}

fn resolve_registry_path(path: Option<PathBuf>) -> CliResult<PathBuf> {
    if let Some(path) = path {
        return Ok(path);
    }
    if let Some(path) = std::env::var_os("SORX_REGISTRY_PATH") {
        return Ok(PathBuf::from(path));
    }
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(config_home)
            .join("greentic-sorx")
            .join("deployment-registry.json"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home)
            .join(".config")
            .join("greentic-sorx")
            .join("deployment-registry.json"));
    }
    Err(CliError::usage(
        "registry path is required when HOME or XDG_CONFIG_HOME is unavailable",
    ))
}

fn registry_error(err: greentic_sorx_core::DeploymentRegistryError) -> CliError {
    CliError::generic(format!("{}: {}", err.code, err.message))
}

fn print_json(value: &impl serde::Serialize) -> CliResult<()> {
    let encoded = serde_json::to_string_pretty(value)
        .map_err(|err| CliError::generic(format!("failed to encode JSON: {err}")))?;
    println!("{encoded}");
    Ok(())
}

fn join_url_path(base: &str, route: &str) -> String {
    if base == "/" {
        format!("/{}", route.trim_start_matches('/'))
    } else {
        format!(
            "{}/{}",
            base.trim_end_matches('/'),
            route.trim_start_matches('/')
        )
    }
}

fn run_doctor(pack: PathBuf, json: bool) -> CliResult<()> {
    let report = doctor_sorla_pack(&pack);
    if json {
        let encoded = serde_json::to_string_pretty(&report)
            .map_err(|err| CliError::generic(format!("failed to encode doctor report: {err}")))?;
        println!("{encoded}");
    } else if report.ok {
        println!("ok: {}", pack.display());
        for warning in &report.warnings {
            eprintln!("warning: {}", warning.message);
        }
    } else {
        for error in &report.errors {
            eprintln!("error: {}", error.message);
        }
        for warning in &report.warnings {
            eprintln!("warning: {}", warning.message);
        }
    }

    if report.ok {
        Ok(())
    } else {
        Err(CliError::pack("doctor failed"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GeneratedArtifactLike {
    kind: String,
    filename: String,
    media_type: String,
    sha256: String,
    bytes_base64: String,
    #[serde(default)]
    metadata_json: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ArtifactValidationReport {
    schema: String,
    valid: bool,
    artifact: ArtifactReportMetadata,
    doctor: SorxDoctorReport,
    inspect: Option<SorxInspectReport>,
    startup_schema: Option<serde_json::Value>,
    provider_compatibility: Option<greentic_sorx_core::ProviderCompatibilityReport>,
    diagnostics: Vec<ArtifactDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ArtifactReportMetadata {
    filename: String,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ArtifactDiagnostic {
    code: String,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArtifactInput {
    bytes: Vec<u8>,
    filename: String,
    sha256: String,
}

fn run_artifact(command: ArtifactCommands, context: &SorxCommandContext) -> CliResult<()> {
    match command {
        ArtifactCommands::Validate {
            file,
            artifact_json,
            answers,
            json: _,
        } => run_artifact_validate(file, artifact_json, answers, context),
        ArtifactCommands::Inspect {
            file,
            artifact_json,
            json: _,
        } => {
            let input = read_artifact_input(file, artifact_json)?;
            let report = inspect_gtpack_bytes(&input.bytes)
                .map_err(|err| CliError::pack(err.to_string()))?;
            print_json(&report)
        }
        ArtifactCommands::StartupSchema {
            file,
            artifact_json,
            json: _,
        } => {
            let input = read_artifact_input(file, artifact_json)?;
            let schema = startup_schema_from_gtpack_bytes(&input.bytes)
                .map_err(|err| CliError::pack(err.to_string()))?;
            print_json(&schema)
        }
    }
}

fn run_artifact_validate(
    file: Option<PathBuf>,
    artifact_json: Option<PathBuf>,
    answers: Option<PathBuf>,
    context: &SorxCommandContext,
) -> CliResult<()> {
    let input = read_artifact_input(file, artifact_json)?;
    let loaded = load_sorla_pack_from_bytes(&input.bytes);
    let doctor = match &loaded {
        Ok(pack) => doctor_sorla_loaded_pack(pack),
        Err(err) => SorxDoctorReport {
            ok: false,
            errors: vec![greentic_sorx_pack::SorxDoctorIssue {
                level: greentic_sorx_pack::SorxDoctorIssueLevel::Error,
                code: err.code().to_string(),
                message: err.to_string(),
            }],
            warnings: Vec::new(),
        },
    };
    let inspect = loaded
        .as_ref()
        .ok()
        .and_then(|_| inspect_gtpack_bytes(&input.bytes).ok());
    let startup_schema = loaded
        .as_ref()
        .ok()
        .map(|pack| pack.sorx_assets.start_schema_json.clone());
    let mut diagnostics = Vec::new();
    let provider_compatibility = match (loaded.as_ref().ok(), answers) {
        (Some(pack), Some(answers)) => Some(provider_compatibility_for_answers(
            pack,
            &answers,
            context,
            &mut diagnostics,
        )?),
        _ => None,
    };
    let provider_compatible = provider_compatibility.as_ref().is_none_or(|report| {
        report.status == greentic_sorx_core::ProviderCompatibilityStatus::Passed
    });
    let valid = doctor.ok && diagnostics.is_empty() && provider_compatible;
    let report = ArtifactValidationReport {
        schema: "greentic.sorx.artifact.validation-report.v1".to_string(),
        valid,
        artifact: ArtifactReportMetadata {
            filename: input.filename,
            sha256: input.sha256,
        },
        doctor,
        inspect,
        startup_schema,
        provider_compatibility,
        diagnostics,
    };
    print_json(&report)?;
    if report.valid {
        Ok(())
    } else {
        Err(CliError::pack("artifact validation failed"))
    }
}

fn provider_compatibility_for_answers(
    pack: &greentic_sorx_pack::LoadedSorlaPack,
    answers_path: &Path,
    context: &SorxCommandContext,
    diagnostics: &mut Vec<ArtifactDiagnostic>,
) -> CliResult<greentic_sorx_core::ProviderCompatibilityReport> {
    let raw = fs::read_to_string(answers_path).map_err(|err| {
        CliError::answers(format!(
            "failed to read answers {}: {err}",
            answers_path.display()
        ))
    })?;
    let answers_json: serde_json::Value = serde_json::from_str(&raw).map_err(|err| {
        CliError::answers(format!(
            "answers {} are invalid JSON: {err}",
            answers_path.display()
        ))
    })?;
    let normalized = normalize_start_answers(
        &pack.sorx_assets.start_schema_json,
        &answers_json,
        context.non_interactive,
    )
    .map_err(|err| CliError::answers(err.to_string()))?;
    let config = runtime_config_from_answers(&pack.pack_name, &normalized.answers)
        .map_err(|err| CliError::answers(err.to_string()))?;
    let report = resolve_provider_compatibility(
        &config,
        &provider_compatibility_input(pack),
        ProviderResolutionMode::DryRun,
    );
    if report.status != greentic_sorx_core::ProviderCompatibilityStatus::Passed {
        diagnostics.push(ArtifactDiagnostic {
            code: "provider_compatibility_failed".to_string(),
            message: "provider compatibility failed for supplied answers".to_string(),
        });
    }
    Ok(report)
}

fn read_artifact_input(
    file: Option<PathBuf>,
    artifact_json: Option<PathBuf>,
) -> CliResult<ArtifactInput> {
    match (file, artifact_json) {
        (Some(file), None) => read_artifact_file(&file),
        (None, Some(artifact_json)) => read_artifact_json(&artifact_json),
        (Some(_), Some(_)) => Err(CliError::usage(
            "artifact commands accept either --file or --artifact-json, not both",
        )),
        (None, None) => Err(CliError::usage(
            "artifact commands require --file or --artifact-json",
        )),
    }
}

fn read_artifact_file(path: &Path) -> CliResult<ArtifactInput> {
    let bytes = fs::read(path)
        .map_err(|err| CliError::pack(format!("failed to read {}: {err}", path.display())))?;
    let sha256 = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
    Ok(ArtifactInput {
        bytes,
        filename: path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("generated.gtpack")
            .to_string(),
        sha256,
    })
}

fn read_artifact_json(path: &Path) -> CliResult<ArtifactInput> {
    let raw = fs::read_to_string(path)
        .map_err(|err| CliError::pack(format!("failed to read {}: {err}", path.display())))?;
    let artifact: GeneratedArtifactLike = serde_json::from_str(&raw).map_err(|err| {
        CliError::pack(format!(
            "artifact JSON {} is invalid: {err}",
            path.display()
        ))
    })?;
    if artifact.kind != "gtpack" {
        return Err(CliError::pack(format!(
            "artifact kind must be `gtpack`, got `{}`",
            artifact.kind
        )));
    }
    if artifact.media_type != "application/vnd.greentic.gtpack" {
        return Err(CliError::pack(format!(
            "artifact media_type must be `application/vnd.greentic.gtpack`, got `{}`",
            artifact.media_type
        )));
    }
    let bytes = decode_base64(&artifact.bytes_base64)?;
    let actual = hex::encode(Sha256::digest(&bytes));
    let expected = artifact
        .sha256
        .strip_prefix("sha256:")
        .unwrap_or(&artifact.sha256);
    if expected != actual {
        return Err(CliError::pack(format!(
            "artifact sha256 mismatch: expected {}, got sha256:{}",
            artifact.sha256, actual
        )));
    }
    Ok(ArtifactInput {
        bytes,
        filename: artifact.filename,
        sha256: format!("sha256:{actual}"),
    })
}

fn decode_base64(input: &str) -> CliResult<Vec<u8>> {
    let values = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .map(base64_value)
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() % 4 != 0 {
        return Err(CliError::pack(
            "artifact bytes_base64 length is not a multiple of 4",
        ));
    }
    let mut out = Vec::new();
    for chunk in values.chunks(4) {
        let pad = chunk.iter().filter(|value| **value == 64).count();
        if chunk[0] == 64 || chunk[1] == 64 || (chunk[2] == 64 && chunk[3] != 64) || pad > 2 {
            return Err(CliError::pack("artifact bytes_base64 has invalid padding"));
        }
        let first = (chunk[0] << 2) | (chunk[1] >> 4);
        out.push(first);
        if chunk[2] != 64 {
            let second = ((chunk[1] & 0x0f) << 4) | (chunk[2] >> 2);
            out.push(second);
        }
        if chunk[3] != 64 {
            let third = ((chunk[2] & 0x03) << 6) | chunk[3];
            out.push(third);
        }
    }
    Ok(out)
}

fn base64_value(byte: u8) -> Result<u8, CliError> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        b'=' => Ok(64),
        _ => Err(CliError::pack(
            "artifact bytes_base64 contains invalid base64",
        )),
    }
}

fn run_inspect(pack: PathBuf, _json: bool) -> CliResult<()> {
    let report = inspect_sorla_pack(&pack).map_err(|err| CliError::pack(err.to_string()))?;
    let encoded = serde_json::to_string_pretty(&report)
        .map_err(|err| CliError::generic(format!("failed to encode inspect report: {err}")))?;
    println!("{encoded}");
    Ok(())
}

fn run_routes(pack: PathBuf, _json: bool) -> CliResult<()> {
    let pack = load_sorla_pack(&pack).map_err(|err| CliError::pack(err.to_string()))?;
    let router = greentic_sorx_core::EndpointRouter::from_agent_gateway(
        &pack.sorla_assets.agent_gateway_json,
    )
    .map_err(|err| CliError::pack(err.to_string()))?;
    let versions = http_runtime::RouteVersionMetadata {
        api_version_label: "local".to_string(),
        view_version: "local".to_string(),
        canonical_version: pack.pack_version.clone(),
        state_namespace: format!("sorx/local/{}", pack.pack_name),
    };
    let routes = http_runtime::route_list("local", "local", &pack, &router, &versions);
    let encoded = serde_json::to_string_pretty(&routes)
        .map_err(|err| CliError::generic(format!("failed to encode routes: {err}")))?;
    println!("{encoded}");
    Ok(())
}

fn run_empty_deployment_routes() -> CliResult<()> {
    let encoded = serde_json::to_string_pretty(&serde_json::json!({
        "schema": "greentic.sorx.routes.v1",
        "routes": []
    }))
    .map_err(|err| CliError::generic(format!("failed to encode routes: {err}")))?;
    println!("{encoded}");
    Ok(())
}

fn run_deployment_routes(
    deployment_id: String,
    registry_path: Option<PathBuf>,
    _json: bool,
) -> CliResult<()> {
    if deployment_id == "local" {
        return run_empty_deployment_routes();
    }
    let store = LocalDeploymentRegistryStore::new(resolve_registry_path(registry_path)?);
    let registry = store.load().map_err(registry_error)?;
    let deployment = registry
        .deployment(&deployment_id)
        .ok_or_else(|| CliError::usage(format!("deployment `{deployment_id}` does not exist")))?;
    let pack_path = PathBuf::from(&deployment.artifact.source);
    let pack = load_sorla_pack(&pack_path).map_err(|err| CliError::pack(err.to_string()))?;
    if pack.pack_digest.as_deref() != Some(deployment.pack_digest.as_str()) {
        return Err(CliError::pack(format!(
            "pack digest for `{}` no longer matches deployment `{deployment_id}`",
            pack_path.display()
        )));
    }
    let router = EndpointRouter::from_agent_gateway(&pack.sorla_assets.agent_gateway_json)
        .map_err(|err| CliError::pack(err.to_string()))?;
    let mut routes = http_runtime::route_list(
        &deployment.deployment_id,
        &deployment.visibility.to_string(),
        &pack,
        &router,
        &http_runtime::RouteVersionMetadata::from_deployment(deployment),
    );
    for route in &mut routes.routes {
        route.path = join_url_path(&deployment.base_path, &route.path);
    }
    print_json(&routes)
}

fn run_mcp_tools(pack: PathBuf) -> CliResult<()> {
    let pack = load_sorla_pack(&pack).map_err(|err| CliError::pack(err.to_string()))?;
    let router = EndpointRouter::from_agent_gateway(&pack.sorla_assets.agent_gateway_json)
        .map_err(|err| CliError::pack(err.to_string()))?;
    let tools = mcp_tools_from_metadata(pack.sorla_assets.mcp_tools_json.as_ref(), &router)
        .map_err(|err| CliError::pack(err.to_string()))?;
    let encoded = serde_json::to_string_pretty(&tools)
        .map_err(|err| CliError::generic(format!("failed to encode MCP tools: {err}")))?;
    println!("{encoded}");
    Ok(())
}

fn run_graph(command: GraphCommands) -> CliResult<()> {
    match command {
        GraphCommands::Concepts { pack, json: _ } => {
            let service = ontology_graph_service(&pack)?;
            let graph_hash = ontology_hash_from_pack(&pack)?;
            print_json(&serde_json::json!({
                "schema": "greentic.sorx.graph.concepts.v1",
                "concepts": service.concepts(),
                "audit_events": [ontology_audit_event("ontology.graph.loaded", serde_json::json!({
                    "ontology_graph_hash": graph_hash
                }))]
            }))
        }
        GraphCommands::Relationships { pack, json: _ } => {
            let service = ontology_graph_service(&pack)?;
            let graph_hash = ontology_hash_from_pack(&pack)?;
            print_json(&serde_json::json!({
                "schema": "greentic.sorx.graph.relationships.v1",
                "relationships": service.relationships(),
                "audit_events": [ontology_audit_event("ontology.graph.loaded", serde_json::json!({
                    "ontology_graph_hash": graph_hash
                }))]
            }))
        }
        GraphCommands::Paths {
            pack,
            from,
            to,
            max_depth,
            json: _,
        } => {
            let service = ontology_graph_service(&pack)?;
            let paths = service
                .find_type_paths(&from, &to, max_depth)
                .map_err(|err| CliError::pack(err.to_string()))?;
            enforce_relationship_policy_for_paths(&pack, &paths)?;
            let graph_hash = ontology_hash_from_pack(&pack)?;
            print_json(&serde_json::json!({
                "schema": "greentic.sorx.graph.paths.v1",
                "from": from,
                "to": to,
                "max_depth": max_depth,
                "paths": paths.clone(),
                "explain": {
                    "ontology_graph_hash": graph_hash,
                    "concepts_used": concepts_used_from_paths(&paths),
                    "relationships_used": relationships_used_from_paths(&paths),
                    "providers_used": [],
                    "evidence_used": [],
                    "policy_decisions": [],
                    "redactions": [],
                    "graph_paths_considered": paths
                },
                "audit_events": [
                    ontology_audit_event("ontology.graph.loaded", serde_json::json!({"ontology_graph_hash": graph_hash})),
                    ontology_audit_event("ontology.path.resolved", serde_json::json!({"from": from, "to": to}))
                ]
            }))
        }
        GraphCommands::Neighbors {
            pack,
            entity_type,
            entity_id,
            depth,
            json: _,
        } => {
            let service = ontology_graph_service(&pack)?;
            let relationships = service
                .neighbors(&entity_type, depth)
                .map_err(|err| CliError::pack(err.to_string()))?;
            enforce_relationship_policy_for_relationships(&pack, &relationships)?;
            let graph_hash = ontology_hash_from_pack(&pack)?;
            print_json(&serde_json::json!({
                "schema": "greentic.sorx.graph.neighbors.v1",
                "entity": {
                    "type": entity_type,
                    "id": entity_id
                },
                "depth": depth,
                "relationships": relationships.clone(),
                "explain": {
                    "ontology_graph_hash": graph_hash,
                    "relationships_used": relationships.iter().map(|relationship| relationship.id.clone()).collect::<Vec<_>>()
                },
                "audit_events": [
                    ontology_audit_event("ontology.graph.loaded", serde_json::json!({"ontology_graph_hash": graph_hash})),
                    ontology_audit_event("ontology.path.resolved", serde_json::json!({"entity_type": entity_type, "depth": depth}))
                ]
            }))
        }
        GraphCommands::Explain {
            pack,
            from,
            to,
            max_depth,
            json: _,
        } => {
            let service = ontology_graph_service(&pack)?;
            let paths = service
                .find_type_paths(&from, &to, max_depth)
                .map_err(|err| CliError::pack(err.to_string()))?;
            enforce_relationship_policy_for_paths(&pack, &paths)?;
            let graph_hash = ontology_hash_from_pack(&pack)?;
            print_json(&serde_json::json!({
                "schema": "greentic.sorx.graph.explain.v1",
                "from": from,
                "to": to,
                "explain": {
                    "ontology_graph_hash": graph_hash,
                    "max_depth": max_depth,
                    "path_count": paths.len(),
                    "paths": paths.clone(),
                    "concepts_used": concepts_used_from_paths(&paths),
                    "relationships_used": relationships_used_from_paths(&paths),
                    "providers_used": [],
                    "evidence_used": [],
                    "policy_decisions": [],
                    "redactions": [],
                    "provider_backed_instances": false
                },
                "audit_events": [
                    ontology_audit_event("ontology.graph.loaded", serde_json::json!({"ontology_graph_hash": graph_hash})),
                    ontology_audit_event("ontology.path.resolved", serde_json::json!({"from": from, "to": to}))
                ]
            }))
        }
    }
}

fn ontology_graph_service(pack: &Path) -> CliResult<OntologyGraphService> {
    let pack = load_sorla_pack(pack).map_err(|err| CliError::pack(err.to_string()))?;
    let ontology =
        pack.sorla_assets.ontology.as_ref().ok_or_else(|| {
            CliError::pack("pack does not contain assets/sorla/ontology.graph.json")
        })?;
    ontology_graph_service_from_ontology(ontology)
}

fn ontology_graph_service_from_ontology(
    ontology: &greentic_sorx_pack::OntologyAssets,
) -> CliResult<OntologyGraphService> {
    let concepts = ontology
        .graph
        .concepts
        .iter()
        .map(|concept| OntologyConceptNode {
            id: concept.id.clone(),
            label: concept.label.clone(),
        })
        .collect::<Vec<_>>();
    let relationships = ontology
        .graph
        .relationships
        .iter()
        .map(|relationship| {
            let from = relationship.from.clone().ok_or_else(|| {
                CliError::pack(format!(
                    "ontology relationship `{}` is missing from/source concept",
                    relationship.id
                ))
            })?;
            let to = relationship.to.clone().ok_or_else(|| {
                CliError::pack(format!(
                    "ontology relationship `{}` is missing to/target concept",
                    relationship.id
                ))
            })?;
            Ok(OntologyRelationshipEdge {
                id: relationship.id.clone(),
                from,
                to,
                label: relationship.label.clone(),
            })
        })
        .collect::<CliResult<Vec<_>>>()?;
    OntologyGraphService::new(concepts, relationships)
        .map_err(|err| CliError::pack(err.to_string()))
}

fn run_evidence(command: EvidenceCommands, context: &SorxCommandContext) -> CliResult<()> {
    match command {
        EvidenceCommands::Query {
            pack,
            answers,
            query,
            entity_type,
            entity_id,
            max_depth,
            json: _,
        } => run_evidence_query(
            pack,
            answers,
            query,
            entity_type,
            entity_id,
            max_depth,
            context,
        ),
    }
}

fn run_evidence_query(
    pack: PathBuf,
    answers: PathBuf,
    query: String,
    entity_type: String,
    entity_id: String,
    max_depth: u8,
    context: &SorxCommandContext,
) -> CliResult<()> {
    let pack = load_sorla_pack(&pack).map_err(|err| CliError::pack(err.to_string()))?;
    let ontology =
        pack.sorla_assets.ontology.as_ref().ok_or_else(|| {
            CliError::pack("pack does not contain assets/sorla/ontology.graph.json")
        })?;
    let raw = fs::read_to_string(&answers).map_err(|err| {
        CliError::answers(format!(
            "failed to read answers {}: {err}",
            answers.display()
        ))
    })?;
    let answers_json: serde_json::Value = serde_json::from_str(&raw).map_err(|err| {
        CliError::answers(format!(
            "answers {} are invalid JSON: {err}",
            answers.display()
        ))
    })?;
    let normalized = normalize_start_answers(
        &pack.sorx_assets.start_schema_json,
        &answers_json,
        context.non_interactive,
    )
    .map_err(|err| CliError::answers(err.to_string()))?;
    let config = runtime_config_from_answers(&pack.pack_name, &normalized.answers)
        .map_err(|err| CliError::answers(err.to_string()))?;
    let compatibility = resolve_provider_compatibility(
        &config,
        &provider_compatibility_input(&pack),
        ProviderResolutionMode::RuntimeStartup,
    );
    let mut audit_events = vec![ontology_audit_event(
        "provider.compatibility.checked",
        serde_json::json!({
            "status": compatibility.status,
            "bindings": compatibility.bindings.clone(),
            "issue_count": compatibility.issues.len()
        }),
    )];
    if compatibility.status != ProviderCompatibilityStatus::Passed {
        return Err(CliError::provider(format!(
            "provider compatibility failed: {}",
            compatibility
                .issues
                .first()
                .map(|issue| issue.message.as_str())
                .unwrap_or("missing evidence provider")
        )));
    }
    let provider_id = compatibility
        .bindings
        .iter()
        .find(|binding| binding.requirement == "evidence.query")
        .map(|binding| binding.provider_id.clone())
        .ok_or_else(|| CliError::provider("missing evidence provider binding"))?;
    let service = ontology_graph_service_from_ontology(ontology)?;
    service.concept(&entity_type).ok_or_else(|| {
        CliError::pack(format!("ontology concept `{entity_type}` does not exist"))
    })?;
    let evidence_policy = PolicyEngine::default().decide_ontology(
        &local_policy_subject(),
        &OntologyPolicyResource::Evidence {
            entity_type: entity_type.clone(),
            entity_id: entity_id.clone(),
        },
        OntologyPolicyAction::RetrieveEvidence,
        &sensitivity_context_from_ontology(ontology),
    );
    audit_events.push(ontology_audit_event(
        "policy.ontology.decision",
        serde_json::to_value(&evidence_policy).map_err(|err| {
            CliError::generic(format!("failed to encode ontology policy decision: {err}"))
        })?,
    ));
    if evidence_policy.decision != OntologyPolicyDecisionKind::Allow {
        return Err(CliError::new(
            SorxExitCode::PolicyDenied,
            serde_json::to_string(&evidence_policy)
                .unwrap_or_else(|_| "ontology policy denied evidence query".to_string()),
        ));
    }
    let relationships = service
        .neighbors(&entity_type, max_depth)
        .map_err(|err| CliError::pack(err.to_string()))?;
    enforce_relationship_policy_for_relationships_from_ontology(ontology, &relationships)?;
    let graph_paths_considered = service
        .concepts()
        .into_iter()
        .filter(|concept| concept.id != entity_type)
        .flat_map(|concept| {
            service
                .find_type_paths(&entity_type, &concept.id, max_depth)
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    let scope = OntologyScope {
        root_entities: vec![ScopedEntity {
            entity_type: entity_type.clone(),
            entity_id: entity_id.clone(),
        }],
        concepts: scoped_concepts(&entity_type, &relationships),
        relationships: relationships
            .iter()
            .map(|relationship| relationship.id.clone())
            .collect(),
    };
    audit_events.push(ontology_audit_event(
        "evidence.query.planned",
        serde_json::json!({
            "provider_id": provider_id.clone(),
            "query": query.clone(),
            "scope": scope.clone(),
            "max_depth": max_depth
        }),
    ));
    let provider = DeterministicEvidenceProvider::new(provider_id.clone());
    let evidence = provider
        .query(EvidenceQueryFilter {
            query: query.clone(),
            scope: scope.clone(),
            max_depth,
        })
        .map_err(|err| CliError::provider(err.to_string()))?;
    audit_events.push(ontology_audit_event(
        "entity.links.resolved",
        serde_json::json!({
            "linked_entities": evidence
                .iter()
                .flat_map(|item| item.linked_entities.iter())
                .collect::<Vec<_>>()
        }),
    ));
    let ontology_graph_hash = ontology_graph_hash(ontology);
    let concepts_used = graph_paths_considered
        .iter()
        .flat_map(|path| path.concepts.iter().cloned())
        .chain(scope.concepts.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let evidence_used = evidence
        .iter()
        .map(|item| item.evidence_id.clone())
        .collect::<Vec<_>>();
    let evidence_count = evidence.len();
    let relationships_used = scope.relationships.clone();
    audit_events.push(ontology_audit_event(
        "evidence.query.executed",
        serde_json::json!({
            "provider_id": provider_id.clone(),
            "evidence_count": evidence_count,
            "evidence_ids": evidence_used.clone()
        }),
    ));
    let policy_decisions = vec![format!("{:?}", evidence_policy.decision)];
    let redactions = evidence_policy
        .redactions
        .iter()
        .map(|redaction| format!("{}.{}", redaction.entity_type, redaction.field))
        .collect::<Vec<_>>();
    let result = EvidenceQueryResult {
        schema: "greentic.sorx.evidence-query-result.v1".to_string(),
        query,
        ontology_scope: scope,
        evidence,
        explain: greentic_sorx_core::EvidenceExplain {
            retrieval_binding: ontology
                .retrieval_bindings
                .as_ref()
                .and_then(|bindings| bindings.bindings.first())
                .map(|binding| binding.id.clone()),
            provider_id: provider_id.clone(),
            graph_paths_considered,
            ontology_graph_hash: ontology_graph_hash.clone(),
            concepts_used,
            relationships_used,
            providers_used: vec![provider_id.clone()],
            evidence_used,
            policy_decisions,
            redactions,
        },
        audit_events,
    };
    print_json(&result)
}

fn scoped_concepts(root: &str, relationships: &[OntologyRelationshipEdge]) -> Vec<String> {
    let mut concepts = std::collections::BTreeSet::from([root.to_string()]);
    for relationship in relationships {
        concepts.insert(relationship.from.clone());
        concepts.insert(relationship.to.clone());
    }
    concepts.into_iter().collect()
}

fn ontology_hash_from_pack(pack: &Path) -> CliResult<String> {
    let pack = load_sorla_pack(pack).map_err(|err| CliError::pack(err.to_string()))?;
    let ontology =
        pack.sorla_assets.ontology.as_ref().ok_or_else(|| {
            CliError::pack("pack does not contain assets/sorla/ontology.graph.json")
        })?;
    Ok(ontology_graph_hash(ontology))
}

fn ontology_graph_hash(ontology: &greentic_sorx_pack::OntologyAssets) -> String {
    let encoded = serde_json::to_vec(&ontology.graph_json).unwrap_or_default();
    format!("sha256:{}", hex::encode(Sha256::digest(encoded)))
}

fn ontology_audit_event(event: &str, details: serde_json::Value) -> OntologyAuditEvent {
    core_ontology_audit_event(event, "local-cli", details)
}

fn concepts_used_from_paths(paths: &[greentic_sorx_core::TypePath]) -> Vec<String> {
    paths
        .iter()
        .flat_map(|path| path.concepts.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn relationships_used_from_paths(paths: &[greentic_sorx_core::TypePath]) -> Vec<String> {
    paths
        .iter()
        .flat_map(|path| path.relationships.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn enforce_relationship_policy_for_paths(
    pack: &Path,
    paths: &[greentic_sorx_core::TypePath],
) -> CliResult<()> {
    let pack = load_sorla_pack(pack).map_err(|err| CliError::pack(err.to_string()))?;
    let Some(ontology) = &pack.sorla_assets.ontology else {
        return Ok(());
    };
    let relationships = paths
        .iter()
        .flat_map(|path| path.relationships.iter())
        .map(|id| OntologyRelationshipEdge {
            id: id.clone(),
            from: String::new(),
            to: String::new(),
            label: None,
        })
        .collect::<Vec<_>>();
    enforce_relationship_policy_for_relationships_from_ontology(ontology, &relationships)
}

fn enforce_relationship_policy_for_relationships(
    pack: &Path,
    relationships: &[OntologyRelationshipEdge],
) -> CliResult<()> {
    let pack = load_sorla_pack(pack).map_err(|err| CliError::pack(err.to_string()))?;
    let Some(ontology) = &pack.sorla_assets.ontology else {
        return Ok(());
    };
    enforce_relationship_policy_for_relationships_from_ontology(ontology, relationships)
}

fn enforce_relationship_policy_for_relationships_from_ontology(
    ontology: &greentic_sorx_pack::OntologyAssets,
    relationships: &[OntologyRelationshipEdge],
) -> CliResult<()> {
    let sensitivity = sensitivity_context_from_ontology(ontology);
    let engine = PolicyEngine::default();
    for relationship in relationships {
        let decision = engine.decide_ontology(
            &local_policy_subject(),
            &OntologyPolicyResource::Relationship {
                relationship: relationship.id.clone(),
            },
            OntologyPolicyAction::Traverse,
            &sensitivity,
        );
        if decision.decision != OntologyPolicyDecisionKind::Allow {
            return Err(CliError::new(
                SorxExitCode::PolicyDenied,
                serde_json::to_string(&decision).unwrap_or_else(|_| {
                    "ontology policy denied relationship traversal".to_string()
                }),
            ));
        }
    }
    Ok(())
}

fn local_policy_subject() -> OntologyPolicySubject {
    OntologyPolicySubject {
        subject: "local-cli".to_string(),
        roles: Vec::new(),
    }
}

fn sensitivity_context_from_ontology(
    ontology: &greentic_sorx_pack::OntologyAssets,
) -> SensitivityContext {
    let mut context = SensitivityContext::default();
    let Some(policy) = ontology.graph_json.get("policy") else {
        return context;
    };
    if policy
        .get("evidence_requires_approval")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        context.evidence_requires_approval = true;
    }
    if let Some(relationships) = policy
        .get("deny_relationships")
        .and_then(serde_json::Value::as_array)
    {
        context.denied_relationships.extend(
            relationships
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToString::to_string),
        );
    }
    if let Some(fields) = policy
        .get("sensitive_fields")
        .and_then(serde_json::Value::as_object)
    {
        for (entity_type, values) in fields {
            context.sensitive_fields.insert(
                entity_type.clone(),
                values
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(serde_json::Value::as_str)
                    .map(ToString::to_string)
                    .collect(),
            );
        }
    }
    context
}

fn run_mcp_start(pack: PathBuf, answers: PathBuf, context: &SorxCommandContext) -> CliResult<()> {
    let pack = load_sorla_pack(&pack).map_err(|err| CliError::pack(err.to_string()))?;
    let raw = fs::read_to_string(&answers).map_err(|err| {
        CliError::answers(format!(
            "failed to read answers {}: {err}",
            answers.display()
        ))
    })?;
    let answers_json: serde_json::Value = serde_json::from_str(&raw).map_err(|err| {
        CliError::answers(format!(
            "answers {} are invalid JSON: {err}",
            answers.display()
        ))
    })?;
    let normalized = normalize_start_answers(
        &pack.sorx_assets.start_schema_json,
        &answers_json,
        context.non_interactive,
    )
    .map_err(|err| CliError::answers(err.to_string()))?;
    let config = runtime_config_from_answers(&pack.pack_name, &normalized.answers)
        .map_err(|err| CliError::answers(err.to_string()))?;
    let router = EndpointRouter::from_agent_gateway(&pack.sorla_assets.agent_gateway_json)
        .map_err(|err| CliError::pack(err.to_string()))?;
    let tools = mcp_tools_from_metadata(pack.sorla_assets.mcp_tools_json.as_ref(), &router)
        .map_err(|err| CliError::pack(err.to_string()))?;
    let encoded = serde_json::to_string_pretty(&serde_json::json!({
        "schema": "greentic.sorx.mcp.runtime.v1",
        "transport": "adapter_only",
        "bind": config.mcp.bind,
        "enabled": config.mcp.enabled,
        "tools": tools.tools
    }))
    .map_err(|err| CliError::generic(format!("failed to encode MCP runtime plan: {err}")))?;
    println!("{encoded}");
    Ok(())
}

fn run_start(
    pack: PathBuf,
    schema: bool,
    answers: Option<PathBuf>,
    dry_run: bool,
    emit_answers: bool,
    _json: bool,
    context: &SorxCommandContext,
) -> CliResult<()> {
    if schema && (answers.is_some() || emit_answers) {
        return Err(CliError::usage(
            "start --schema cannot be combined with --answers or --emit-answers",
        ));
    }

    let pack = load_sorla_pack(&pack).map_err(|err| CliError::pack(err.to_string()))?;
    if schema {
        let encoded = serde_json::to_string_pretty(&pack.sorx_assets.start_schema_json)
            .map_err(|err| CliError::generic(format!("failed to encode startup schema: {err}")))?;
        println!("{encoded}");
        return Ok(());
    }

    let answers_path = answers.ok_or_else(|| CliError::usage("start requires --answers <FILE>"))?;
    let raw = fs::read_to_string(&answers_path).map_err(|err| {
        CliError::answers(format!(
            "failed to read answers {}: {err}",
            answers_path.display()
        ))
    })?;
    let answers_json: serde_json::Value = serde_json::from_str(&raw).map_err(|err| {
        CliError::answers(format!(
            "answers {} are invalid JSON: {err}",
            answers_path.display()
        ))
    })?;
    let normalized = normalize_start_answers(
        &pack.sorx_assets.start_schema_json,
        &answers_json,
        context.non_interactive,
    )
    .map_err(|err| CliError::answers(err.to_string()))?;

    if !dry_run && !emit_answers {
        let config = runtime_config_from_answers(&pack.pack_name, &normalized.answers)
            .map_err(|err| CliError::answers(err.to_string()))?;
        let bind = config.server.bind.clone();
        let server =
            http_runtime::HttpRuntime::from_pack("local", &pack, config).map_err(|err| {
                let message = format!("failed to build HTTP runtime from pack metadata: {err}");
                if err.code == "provider_unsupported" || err.code == "provider_unavailable" {
                    CliError::provider(message)
                } else {
                    CliError::runtime(message)
                }
            })?;
        let listener = std::net::TcpListener::bind(&bind).map_err(|err| {
            CliError::runtime(format!("failed to bind HTTP server on {bind}: {err}"))
        })?;
        eprintln!("greentic-sorx HTTP runtime listening on http://{bind}");
        return server
            .serve(listener)
            .map_err(|err| CliError::runtime(format!("HTTP server failed: {err}")));
    }

    let output = if emit_answers {
        serde_json::to_value(&normalized).map_err(|err| {
            CliError::generic(format!("failed to encode normalized answers: {err}"))
        })?
    } else {
        let mut plan = build_startup_plan(&pack.pack_name, &pack.pack_version, &normalized.answers)
            .map_err(|err| CliError::answers(err.to_string()))?;
        let config = runtime_config_from_answers(&pack.pack_name, &normalized.answers)
            .map_err(|err| CliError::answers(err.to_string()))?;
        let compatibility = resolve_provider_compatibility(
            &config,
            &provider_compatibility_input(&pack),
            ProviderResolutionMode::DryRun,
        );
        if let Some(object) = plan.as_object_mut() {
            object.insert(
                "provider_compatibility".to_string(),
                serde_json::to_value(compatibility).map_err(|err| {
                    CliError::generic(format!("failed to encode provider compatibility: {err}"))
                })?,
            );
        }
        plan
    };
    let encoded = serde_json::to_string_pretty(&output)
        .map_err(|err| CliError::generic(format!("failed to encode startup output: {err}")))?;
    println!("{encoded}");
    Ok(())
}

fn provider_compatibility_input(
    pack: &greentic_sorx_pack::LoadedSorlaPack,
) -> ProviderCompatibilityInput {
    let required_capabilities = route_required_capabilities(&pack.sorla_assets.agent_gateway_json);
    let Some(ontology) = &pack.sorla_assets.ontology else {
        let mut input = ProviderCompatibilityInput::none();
        input.required_capabilities = required_capabilities;
        return input;
    };
    ProviderCompatibilityInput {
        ontology_present: true,
        ontology_schema_supported: ontology.graph.schema == "greentic.sorla.ontology.graph.v1",
        retrieval_bindings_present: ontology.retrieval_bindings.is_some(),
        retrieval_bindings_schema_supported: ontology
            .retrieval_bindings
            .as_ref()
            .is_none_or(|bindings| bindings.schema == "greentic.sorla.retrieval-bindings.v1"),
        requires_entity_link: ontology_requires_entity_link(ontology),
        required_capabilities,
    }
}

fn route_required_capabilities(gateway: &serde_json::Value) -> Vec<String> {
    let mut capabilities = gateway
        .get("endpoints")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|endpoint| {
            let requires = endpoint
                .get("requires")
                .or_else(|| endpoint.get("query_plan"))
                .and_then(serde_json::Value::as_object);
            let mut values = Vec::new();
            if let Some(index) = requires.and_then(|requires| requires.get("index")) {
                values.push(
                    index
                        .get("capability")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("exact-index-query")
                        .to_string(),
                );
            }
            if let Some(traversal) = requires.and_then(|requires| requires.get("traversal")) {
                values.push(
                    traversal
                        .get("capability")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("bounded-graph-traversal")
                        .to_string(),
                );
            }
            values
        })
        .collect::<Vec<_>>();
    capabilities.sort();
    capabilities.dedup();
    capabilities
}

fn ontology_requires_entity_link(ontology: &greentic_sorx_pack::OntologyAssets) -> bool {
    let graph_requires = ontology
        .graph_json
        .get("requires_entity_link")
        .or_else(|| ontology.graph_json.get("requires_entity_linking"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let bindings_require = ontology
        .retrieval_bindings_json
        .as_ref()
        .is_some_and(value_requires_entity_link);
    graph_requires || bindings_require
}

fn value_requires_entity_link(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::Array(values) => values.iter().any(value_requires_entity_link),
        serde_json::Value::Object(object) => object.iter().any(|(key, value)| {
            (matches!(
                key.as_str(),
                "requires_entity_link" | "requires_entity_linking" | "entity_link"
            ) && value.as_bool().unwrap_or(false))
                || value_requires_entity_link(value)
        }),
        serde_json::Value::Null | serde_json::Value::Number(_) | serde_json::Value::String(_) => {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::error::ErrorKind;
    use greentic_sorx_core::{
        CreateDeploymentRequest, DeploymentRegistry, LocalDeploymentRegistryStore, PackArtifact,
        StateMode,
    };
    use tempfile::TempDir;

    use super::*;

    fn registry_with_pending_deployment() -> (TempDir, PathBuf, String) {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("registry.json");
        let mut registry = DeploymentRegistry::default();
        let deployment = registry
            .create_deployment(CreateDeploymentRequest {
                artifact: PackArtifact {
                    source: "fixtures/landlord.gtpack".to_string(),
                    name: "landlord".to_string(),
                    version: "0.1.0".to_string(),
                    digest: "sha256:test".to_string(),
                    signature: None,
                    signature_ref: None,
                },
                tenant_id: "acme".to_string(),
                sor_name: "landlord".to_string(),
                environment: "production".to_string(),
                api_version_label: "v1".to_string(),
                base_path: "/sorx/acme/landlord/v1".to_string(),
                visibility: DeploymentVisibility::Private,
                state_mode: StateMode::SharedCompatible,
                state_namespace: None,
                deployment_id: None,
                allow_api_version_conflict: false,
                allow_shared_state_conflict: false,
            })
            .unwrap();
        LocalDeploymentRegistryStore::new(&path)
            .save(&registry)
            .unwrap();
        (temp, path, deployment.deployment_id)
    }

    #[test]
    fn parses_doctor_command() {
        let cli = parse_from(["greentic-sorx", "doctor", "landlord.gtpack"]).unwrap();
        assert!(matches!(cli.command, Commands::Doctor { .. }));
    }

    #[test]
    fn parses_inspect_command() {
        let cli = parse_from(["greentic-sorx", "inspect", "landlord.gtpack"]).unwrap();
        assert!(matches!(cli.command, Commands::Inspect { .. }));
    }

    #[test]
    fn parses_artifact_validate_command() {
        let cli = parse_from([
            "greentic-sorx",
            "artifact",
            "validate",
            "--artifact-json",
            "generated-artifact.json",
            "--answers",
            "answers.json",
            "--json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Artifact {
                command: ArtifactCommands::Validate { .. }
            }
        ));
    }

    #[test]
    fn parses_routes_command() {
        let cli = parse_from(["greentic-sorx", "routes", "landlord.gtpack"]).unwrap();
        assert!(matches!(cli.command, Commands::Routes { .. }));
    }

    #[test]
    fn parses_graph_paths_command() {
        let cli = parse_from([
            "greentic-sorx",
            "graph",
            "paths",
            "landlord.gtpack",
            "--from",
            "Tenant",
            "--to",
            "Payment",
            "--json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Graph {
                command: GraphCommands::Paths { .. }
            }
        ));
    }

    #[test]
    fn parses_evidence_query_command() {
        let cli = parse_from([
            "greentic-sorx",
            "evidence",
            "query",
            "landlord.gtpack",
            "--answers",
            "answers.json",
            "--query",
            "lease status",
            "--entity-type",
            "Tenant",
            "--entity-id",
            "tenant-1",
            "--json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Evidence {
                command: EvidenceCommands::Query { .. }
            }
        ));
    }

    #[test]
    fn parses_start_schema_command() {
        let cli = parse_from(["greentic-sorx", "start", "landlord.gtpack", "--schema"]).unwrap();
        assert!(matches!(cli.command, Commands::Start { schema: true, .. }));
    }

    #[test]
    fn parses_start_answers_command() {
        let cli = parse_from([
            "greentic-sorx",
            "start",
            "landlord.gtpack",
            "--answers",
            "answers.json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Start {
                answers: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn parses_start_dry_run_and_emit_answers() {
        let cli = parse_from([
            "greentic-sorx",
            "start",
            "landlord.gtpack",
            "--answers",
            "answers.json",
            "--dry-run",
            "--emit-answers",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Start {
                dry_run: true,
                emit_answers: true,
                ..
            }
        ));
    }

    #[test]
    fn parses_run_alias_command() {
        let cli = parse_from([
            "greentic-sorx",
            "run",
            "landlord.gtpack",
            "--answers",
            "answers.json",
        ])
        .unwrap();
        assert!(matches!(cli.command, Commands::Run { .. }));
    }

    #[test]
    fn parses_deployment_create_command() {
        let cli = parse_from([
            "greentic-sorx",
            "deployments",
            "create",
            "--pack",
            "landlord.gtpack",
            "--tenant",
            "acme",
            "--sor",
            "landlord",
            "--environment",
            "production",
            "--api-version",
            "v1",
            "--base-path",
            "/sorx/acme/landlord/v1",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Deployments {
                command: DeploymentCommands::Create { .. }
            }
        ));
    }

    #[test]
    fn parses_alias_set_command() {
        let cli = parse_from([
            "greentic-sorx",
            "aliases",
            "set",
            "--tenant",
            "acme",
            "--sor",
            "landlord",
            "--alias",
            "stable",
            "--target",
            "deployment-1",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Aliases {
                command: AliasCommands::Set { .. }
            }
        ));
    }

    #[test]
    fn parses_deployment_promote_command() {
        let cli = parse_from([
            "greentic-sorx",
            "deployments",
            "promote",
            "deployment-1",
            "--alias",
            "latest",
            "--public",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Deployments {
                command: DeploymentCommands::Promote { public: true, .. }
            }
        ));
    }

    #[test]
    fn parses_deployment_rollback_command() {
        let cli = parse_from([
            "greentic-sorx",
            "deployments",
            "rollback",
            "--tenant",
            "acme",
            "--sor",
            "landlord",
            "--alias",
            "latest",
            "--to",
            "deployment-1",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Deployments {
                command: DeploymentCommands::Rollback { .. }
            }
        ));
    }

    #[test]
    fn parses_webhook_replay_command() {
        let cli = parse_from([
            "greentic-sorx",
            "webhook",
            "replay",
            "--fixture",
            "fixtures/github-ghcr-published.json",
            "--signature",
            "sha256=test",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Webhook {
                command: WebhookCommands::Replay { .. }
            }
        ));
    }

    #[test]
    fn deployment_command_handlers_cover_registry_read_paths() {
        let (_temp, path, deployment_id) = registry_with_pending_deployment();

        run_deployments(DeploymentCommands::List, Some(path.clone())).unwrap();
        run_deployments(
            DeploymentCommands::Inspect {
                deployment_id: deployment_id.clone(),
            },
            Some(path.clone()),
        )
        .unwrap();
        run_deployments(DeploymentCommands::PublicRoutes, Some(path.clone())).unwrap();
        run_deployments(
            DeploymentCommands::PromotionStatus { deployment_id },
            Some(path),
        )
        .unwrap();
    }

    #[test]
    fn registry_command_handlers_return_clear_missing_errors() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("registry.json");
        let err = run_deployments(
            DeploymentCommands::Inspect {
                deployment_id: "missing".to_string(),
            },
            Some(path.clone()),
        )
        .unwrap_err();
        assert_eq!(err.exit_code, SorxExitCode::Usage);
        assert!(err.message.contains("deployment `missing` does not exist"));

        let err = run_validation_command(
            ValidationCommands::Report {
                deployment_id: "missing".to_string(),
            },
            Some(path),
        )
        .unwrap_err();
        assert_eq!(err.exit_code, SorxExitCode::Usage);
        assert!(err.message.contains("validation report"));
    }

    #[test]
    fn alias_list_handler_accepts_empty_registry() {
        let temp = TempDir::new().unwrap();
        run_aliases(
            AliasCommands::List {
                tenant: Some("acme".to_string()),
                sor: Some("landlord".to_string()),
            },
            Some(temp.path().join("registry.json")),
        )
        .unwrap();
    }

    #[test]
    fn junit_xml_escapes_failure_messages() {
        let report = validation::ValidationReport {
            schema: "greentic.sorx.validation-report.v1".to_string(),
            deployment_id: "local".to_string(),
            pack_name: "landlord".to_string(),
            pack_version: "0.1.0".to_string(),
            pack_digest: "sha256:test".to_string(),
            suite_id: "suite<&\"".to_string(),
            started_at: "1970-01-01T00:00:00Z".to_string(),
            finished_at: "1970-01-01T00:00:00Z".to_string(),
            result: validation::ValidationResult::Fail,
            public_exposure_allowed: false,
            tests: vec![validation::ValidationTestReport {
                id: "case<&\"".to_string(),
                result: validation::ValidationResult::Fail,
                level: validation::ValidationLevel::Required,
                duration_ms: 12,
                message: Some("bad <value> & \"quote\"".to_string()),
            }],
        };
        let xml = junit_xml(&report);
        assert!(xml.contains("suite&lt;&amp;&quot;"));
        assert!(xml.contains("case&lt;&amp;&quot;"));
        assert!(xml.contains("bad &lt;value&gt; &amp; &quot;quote&quot;"));
    }

    #[test]
    fn parses_validate_command() {
        let cli = parse_from([
            "greentic-sorx",
            "validate",
            "landlord.gtpack",
            "--answers",
            "answers.json",
            "--provider-mode",
            "in-memory",
        ])
        .unwrap();
        assert!(matches!(cli.command, Commands::Validate { .. }));
    }

    #[test]
    fn parses_migrate_plan_command() {
        let cli = parse_from([
            "greentic-sorx",
            "migrate",
            "plan",
            "--from",
            "old.gtpack",
            "--to",
            "new.gtpack",
            "--tenant",
            "acme",
            "--sor",
            "landlord",
            "--out",
            "plan.json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Migrate {
                command: MigrationCommands::Plan { .. }
            }
        ));
    }

    #[test]
    fn help_mentions_expected_commands() {
        let help = command().render_long_help().to_string();
        assert!(help.contains("doctor"));
        assert!(help.contains("artifact"));
        assert!(help.contains("inspect"));
        assert!(help.contains("routes"));
        assert!(help.contains("migrate"));
        assert!(help.contains("start"));
    }

    #[test]
    fn localized_help_uses_requested_catalog() {
        let help = localized_help("es");
        assert!(help.contains("Ejecuta artefactos .gtpack de SoRLa"));
        assert!(help.contains("Valida un .gtpack de SoRLa"));
        assert!(help.contains("Inicia un runtime SORX"));

        let help = localized_help("nl");
        assert!(help.contains("Voer SoRLa .gtpack-artifacten uit"));
        assert!(help.contains("Gebruik:"));
        assert!(help.contains("Beheer lokale SORX-deployments"));
    }

    #[test]
    fn locale_catalogs_are_embedded_in_binary() {
        let nl = embedded_i18n_catalog("nl").expect("Dutch catalog should be embedded");
        assert!(nl.contains("cli.command.deployments.about"));
        assert!(nl.contains("Beheer lokale SORX-deployments"));
        assert!(embedded_i18n_catalog("missing").is_none());
    }

    #[test]
    fn version_output_works() {
        let err = parse_from(["greentic-sorx", "--version"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
        assert!(err.to_string().contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn invalid_command_fails_clearly() {
        let err = parse_from(["greentic-sorx", "unknown"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
        assert!(err.to_string().contains("unrecognized subcommand"));
    }

    #[test]
    fn pack_argument_is_required() {
        let err = parse_from(["greentic-sorx", "doctor"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }
}
