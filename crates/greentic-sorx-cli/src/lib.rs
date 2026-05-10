use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};
use greentic_sorx_core::{
    CreateDeploymentRequest, DeploymentVisibility, EndpointRouter, GhcrWebhookConfig,
    GhcrWebhookError, GithubWebhookHeaders, LocalDeploymentRegistryStore, OciArtifactResolver,
    OciReference, PackArtifact, ResolvedOciArtifact, RollbackAliasRequest, SorxCommandContext,
    StateMode, build_startup_plan, handle_ghcr_published_webhook, mcp_tools_from_metadata,
    normalize_start_answers, parse_ghcr_published_metadata, runtime_config_from_answers,
};
use greentic_sorx_pack::{doctor_sorla_pack, inspect_sorla_pack, load_sorla_pack};

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
pub enum ValidationCommands {
    /// Print the latest stored validation report for a deployment.
    Report {
        /// Deployment identifier.
        deployment_id: String,
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
        #[arg(long = "state-mode", default_value = "isolated")]
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
    let routes = http_runtime::route_list("local", "local", &pack, &router);
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
        build_startup_plan(&pack.pack_name, &pack.pack_version, &normalized.answers)
            .map_err(|err| CliError::answers(err.to_string()))?
    };
    let encoded = serde_json::to_string_pretty(&output)
        .map_err(|err| CliError::generic(format!("failed to encode startup output: {err}")))?;
    println!("{encoded}");
    Ok(())
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
                state_mode: StateMode::Isolated,
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
    fn parses_routes_command() {
        let cli = parse_from(["greentic-sorx", "routes", "landlord.gtpack"]).unwrap();
        assert!(matches!(cli.command, Commands::Routes { .. }));
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
    fn help_mentions_expected_commands() {
        let help = command().render_long_help().to_string();
        assert!(help.contains("doctor"));
        assert!(help.contains("inspect"));
        assert!(help.contains("routes"));
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
