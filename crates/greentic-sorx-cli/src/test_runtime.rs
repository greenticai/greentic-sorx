use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use greentic_sorx_pack::{inspect_sorla_pack, load_sorla_pack};
use serde_json::{Value, json};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const WEBCHAT_REF: &str = "oci://ghcr.io/greenticai/packs/messaging/messaging-webchat-gui:stable";
const MANAGER_HOOK_MARKER: &str = "__greenticManagerSubmitHook";

#[derive(Debug)]
pub struct TestOptions {
    pub pack: PathBuf,
    pub sorx_answers: Option<PathBuf>,
    pub setup_answers: Option<PathBuf>,
    pub bundle_dir: Option<PathBuf>,
    pub sorx_url: String,
    pub webchat_url: String,
    pub role: Option<String>,
    pub locale: String,
    pub force: bool,
    pub no_start: bool,
}

struct TestContext {
    options: TestOptions,
    pack_abs: PathBuf,
    pack_base: String,
    pack_id: String,
    bundle_id: String,
    bundle_dir: PathBuf,
    work_dir: PathBuf,
    create_answers: PathBuf,
    setup_answers: PathBuf,
    sorx_answers: PathBuf,
    sorx_metadata_dir: PathBuf,
    selected_role: String,
    available_roles: Vec<String>,
    sorx_url: ParsedHttpUrl,
    webchat_url: String,
}

#[derive(Debug, Clone)]
struct ParsedHttpUrl {
    base: String,
    host: String,
    port: u16,
}

impl ParsedHttpUrl {
    fn parse(input: &str, label: &str) -> Result<Self, String> {
        let base = input.trim().trim_end_matches('/').to_string();
        let rest = base
            .strip_prefix("http://")
            .ok_or_else(|| format!("{label} must look like http://host:port"))?;
        let host_port = rest
            .split('/')
            .next()
            .ok_or_else(|| format!("{label} must look like http://host:port"))?;
        let (host, port) = host_port
            .rsplit_once(':')
            .ok_or_else(|| format!("{label} must include a port"))?;
        let host = host.to_string();
        let port = port
            .parse::<u16>()
            .map_err(|_| format!("{label} has an invalid port"))?;
        if host.trim().is_empty() {
            return Err(format!("{label} must include a host"));
        }
        Ok(Self { base, host, port })
    }

    fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

pub fn run(options: TestOptions) -> Result<(), String> {
    let mut ctx = prepare_context(options)?;
    prepare_workspace(&ctx)?;
    print_summary(&ctx);
    write_create_answers(&ctx)?;

    run_command(
        Command::new("greentic-bundle")
            .arg("wizard")
            .arg("apply")
            .arg("--answers")
            .arg(&ctx.create_answers),
        "greentic-bundle wizard apply",
    )?;

    patch_webchat_manager_submit_hook(&ctx)?;
    copy_sorx_pack_metadata(&ctx)?;
    prepare_sorx_answers(&mut ctx)?;
    write_setup_answers_if_needed(&mut ctx)?;

    run_command(
        Command::new("gtc")
            .arg("setup")
            .arg(&ctx.bundle_dir)
            .arg("--no-ui")
            .arg("--non-interactive")
            .arg("--answers")
            .arg(&ctx.setup_answers),
        "gtc setup",
    )?;

    install_sorx_handoff_pack(&ctx)?;

    println!();
    println!(
        "Sorx manager card endpoint: {}/v1/sorx/manager/cards/dashboard",
        ctx.sorx_url.base
    );
    println!("WebChat route:               {}/webchat", ctx.webchat_url);
    println!(
        "Sorx runtime answers:        {}",
        ctx.sorx_answers.display()
    );
    println!();

    if ctx.options.no_start {
        println!("Bundle is ready at {}", ctx.bundle_dir.display());
        return Ok(());
    }

    ensure_port_available(&ctx.sorx_url)?;
    let mut sorx = start_sorx_runtime(&ctx)?;
    if let Err(err) = wait_for_manager_card(&ctx, "dashboard", Duration::from_secs(45), &mut sorx) {
        terminate_child(&mut sorx);
        return Err(err);
    }
    refresh_sorx_dashboard_card(&ctx)?;

    println!("Starting WebChat bundle; press Ctrl-C to stop.");
    let status = Command::new("gtc")
        .arg("start")
        .arg(&ctx.bundle_dir)
        .status()
        .map_err(|err| format!("failed to start WebChat bundle: {err}"))?;
    terminate_child(&mut sorx);
    if status.success() {
        Ok(())
    } else {
        Err(format!("gtc start exited with status {status}"))
    }
}

fn prepare_context(options: TestOptions) -> Result<TestContext, String> {
    if !options.pack.is_file() {
        return Err(format!("SORX pack not found: {}", options.pack.display()));
    }
    if options.locale.trim().is_empty() || options.webchat_url.trim().is_empty() {
        return Err("--webchat-url and --locale require non-empty values".to_string());
    }

    let sorx_url = ParsedHttpUrl::parse(&options.sorx_url, "--sorx-url")?;
    let webchat_url = options.webchat_url.trim().trim_end_matches('/').to_string();
    let pack_abs = fs::canonicalize(&options.pack)
        .map_err(|err| format!("failed to resolve {}: {err}", options.pack.display()))?;
    let pack_base = pack_abs
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid pack path {}", pack_abs.display()))?
        .to_string();
    let pack_id = sanitize_id(pack_base.strip_suffix(".gtpack").unwrap_or(&pack_base));
    let bundle_id = format!("sorx-manager-{pack_id}");
    let bundle_dir = match &options.bundle_dir {
        Some(path) => absolutize(path)?,
        None => std::env::temp_dir().join(format!("{bundle_id}-bundle")),
    };
    let work_dir = bundle_dir.join(".test-sorx");

    let inspect =
        inspect_sorla_pack(&pack_abs).map_err(|err| format!("failed to inspect pack: {err}"))?;
    let available_roles = inspect
        .sorla
        .roles
        .iter()
        .map(|role| role.id.clone())
        .collect::<Vec<_>>();
    let selected_role = match options
        .role
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(role) => {
            if !available_roles.is_empty() && !available_roles.iter().any(|value| value == role) {
                return Err(format!(
                    "selected role `{role}` is not declared by the pack; available roles: {}",
                    available_roles.join(", ")
                ));
            }
            role.to_string()
        }
        None => available_roles
            .first()
            .cloned()
            .unwrap_or_else(|| "admin".to_string()),
    };

    let create_answers = work_dir.join("create-answers.json");
    let setup_answers = options
        .setup_answers
        .clone()
        .unwrap_or_else(|| work_dir.join("setup-answers.json"));
    let sorx_answers = work_dir.join("sorx-answers.json");
    let sorx_metadata_dir = bundle_dir.join("sorx");

    Ok(TestContext {
        options,
        pack_abs,
        pack_base,
        pack_id,
        bundle_id,
        bundle_dir,
        work_dir,
        create_answers,
        setup_answers,
        sorx_answers,
        sorx_metadata_dir,
        selected_role,
        available_roles,
        sorx_url,
        webchat_url,
    })
}

fn prepare_workspace(ctx: &TestContext) -> Result<(), String> {
    let marker = ctx.work_dir.join("created-by-greentic-sorx-test");
    if ctx.bundle_dir.exists() && !marker.is_file() && !ctx.options.force {
        return Err(format!(
            "bundle directory already exists and was not created by greentic-sorx test: {}\npass --force to replace it",
            ctx.bundle_dir.display()
        ));
    }
    if ctx.bundle_dir.exists() {
        fs::remove_dir_all(&ctx.bundle_dir).map_err(|err| {
            format!(
                "failed to remove existing bundle directory {}: {err}",
                ctx.bundle_dir.display()
            )
        })?;
    }
    fs::create_dir_all(&ctx.work_dir)
        .map_err(|err| format!("failed to create {}: {err}", ctx.work_dir.display()))?;
    fs::write(marker, b"greentic-sorx test\n").map_err(|err| {
        format!(
            "failed to write workspace marker in {}: {err}",
            ctx.work_dir.display()
        )
    })?;
    fs::create_dir_all(&ctx.sorx_metadata_dir).map_err(|err| {
        format!(
            "failed to create Sorx metadata directory {}: {err}",
            ctx.sorx_metadata_dir.display()
        )
    })
}

fn print_summary(ctx: &TestContext) {
    println!("Preparing Sorx/WebChat test bundle");
    println!("  SORX pack:        {}", ctx.pack_abs.display());
    println!("  bundle workspace: {}", ctx.bundle_dir.display());
    println!(
        "  manager card URL: {}/v1/sorx/manager/cards/dashboard",
        ctx.sorx_url.base
    );
    println!("  WebChat URL:      {}/webchat", ctx.webchat_url);
    println!("  selected role:    {}", ctx.selected_role);
    println!("  selected locale:  {}", ctx.options.locale);
    println!("  available roles:  {}", available_roles_label(ctx));
    println!("  WebChat pack ref: {WEBCHAT_REF}");
    println!("  OCI resolution:   greentic-bundle/gtc distributor-backed pack fetch");
}

fn available_roles_label(ctx: &TestContext) -> String {
    if ctx.available_roles.is_empty() {
        "(none declared; using fallback role)".to_string()
    } else {
        ctx.available_roles.join(", ")
    }
}

fn write_create_answers(ctx: &TestContext) -> Result<(), String> {
    let value = json!({
        "wizard_id": "greentic-bundle.wizard.run",
        "schema_id": "greentic-bundle.wizard.answers",
        "schema_version": "1.0.0",
        "locale": ctx.options.locale,
        "answers": {
            "access_rules": [],
            "advanced_setup": false,
            "app_pack_entries": [],
            "app_packs": [],
            "bundle_id": ctx.bundle_id,
            "bundle_name": ctx.bundle_id,
            "export_intent": false,
            "extension_provider_entries": [{
                "detected_kind": "oci",
                "display_name": "Greentic Messaging WebChat GUI (stable)",
                "provider_id": "greentic.messaging.webchat-gui.stable",
                "reference": WEBCHAT_REF,
                "version": "stable"
            }],
            "extension_providers": [WEBCHAT_REF],
            "mode": "create",
            "output_dir": ctx.bundle_dir,
            "remote_catalogs": [],
            "setup_answers": {},
            "setup_execution_intent": false,
            "setup_specs": {}
        }
    });
    write_json(&ctx.create_answers, &value)
}

fn patch_webchat_manager_submit_hook(ctx: &TestContext) -> Result<(), String> {
    let pack_path = ctx
        .bundle_dir
        .join("providers/messaging/messaging-webchat-gui.gtpack");
    let work_dir = ctx.work_dir.join("webchat-provider-pack");
    if !pack_path.is_file() {
        return Err(format!(
            "WebChat provider pack was not generated: {}",
            pack_path.display()
        ));
    }
    extract_zip_to_dir(&pack_path, &work_dir)?;
    let hooks_path = work_dir.join("assets/webchat-gui/skins/default/webchat/hooks.js");
    let mut source = fs::read_to_string(&hooks_path)
        .map_err(|err| format!("failed to read {}: {err}", hooks_path.display()))?;
    if !source.contains(MANAGER_HOOK_MARKER) {
        let needle = "    const result = next(action);\n";
        let replacement = r#"    if (isGreenticManagerSubmitAction(action)) {
      handleGreenticManagerSubmit(store, action.payload.activity);
      return;
    }
    if (isGreenticManagerOpenAction(action)) {
      handleGreenticManagerOpen(store, action.payload.activity);
      return;
    }

    const result = next(action);
"#;
        if !source.contains(needle) {
            return Err("unable to find WebChat hook middleware insertion point".to_string());
        }
        source = source.replacen(needle, replacement, 1);
        source.push_str(MANAGER_HOOK_JS);
        fs::write(&hooks_path, source)
            .map_err(|err| format!("failed to write {}: {err}", hooks_path.display()))?;
    }
    pack_dir(&work_dir, &pack_path)?;
    println!("Patched WebChat manager submit hook");
    Ok(())
}

fn copy_sorx_pack_metadata(ctx: &TestContext) -> Result<(), String> {
    fs::create_dir_all(&ctx.sorx_metadata_dir).map_err(|err| {
        format!(
            "failed to create Sorx metadata directory {}: {err}",
            ctx.sorx_metadata_dir.display()
        )
    })?;
    fs::copy(&ctx.pack_abs, ctx.sorx_metadata_dir.join(&ctx.pack_base))
        .map_err(|err| format!("failed to copy SORX pack metadata: {err}"))?;
    Ok(())
}

fn prepare_sorx_answers(ctx: &mut TestContext) -> Result<(), String> {
    let source = if let Some(path) = &ctx.options.sorx_answers {
        path.clone()
    } else if ctx.pack_id == "landlord-tenant-sor" {
        let fixture = PathBuf::from(
            "crates/greentic-sorx-cli/tests/e2e/fixtures/landlord_tenant/answers.memory.json",
        );
        if fixture.is_file() {
            fixture
        } else {
            generate_sorx_answers(ctx)?
        }
    } else {
        generate_sorx_answers(ctx)?
    };
    if !source.is_file() {
        return Err(format!(
            "SORX startup answers not found: {}",
            source.display()
        ));
    }

    let text = fs::read_to_string(&source)
        .map_err(|err| format!("failed to read {}: {err}", source.display()))?;
    let mut value: Value = serde_json::from_str(&text).map_err(|err| {
        format!(
            "SORX startup answers {} are invalid JSON: {err}",
            source.display()
        )
    })?;
    let root = if value.get("answers").is_some_and(Value::is_object) {
        value
            .get_mut("answers")
            .and_then(Value::as_object_mut)
            .expect("answers object checked above")
    } else {
        value
            .as_object_mut()
            .ok_or_else(|| "SORX startup answers must be a JSON object".to_string())?
    };
    let server = root
        .entry("server".to_string())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| "SORX startup answers `server` must be an object".to_string())?;
    server.insert("bind".to_string(), json!(ctx.sorx_url.bind_addr()));
    server.insert("public_base_url".to_string(), json!(ctx.sorx_url.base));
    write_json(&ctx.sorx_answers, &value)
}

fn generate_sorx_answers(ctx: &TestContext) -> Result<PathBuf, String> {
    let pack = load_sorla_pack(&ctx.pack_abs)
        .map_err(|err| format!("failed to load pack for startup answer generation: {err}"))?;
    let mut entities = serde_json::Map::new();
    for endpoint in pack
        .sorla_assets
        .agent_gateway_json
        .get("endpoints")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(entity) = endpoint
            .get("entity")
            .or_else(|| endpoint.get("record"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let collection = endpoint
            .get("collection")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| default_collection_name(entity));
        entities
            .entry(entity.to_string())
            .or_insert_with(|| json!({"provider": "store", "collection": collection}));
    }
    let value = json!({
        "tenant": {"tenant_id": "demo", "environment": "local"},
        "server": {
            "bind": ctx.sorx_url.bind_addr(),
            "public_base_url": ctx.sorx_url.base,
            "auth": {"mode": "none"}
        },
        "mcp": {"enabled": false, "bind": "127.0.0.1:8790"},
        "providers": {"store": {"kind": "memory", "config_ref": "providers.memory.local"}},
        "bindings": {"entities": entities},
        "policy": {"approvals": {"low": "auto", "medium": "auto", "high": "require_approval", "critical": "deny"}},
        "audit": {"sink": "stdout"},
        "deployment": {
            "tenant_id": "demo",
            "sor_name": pack.pack_name,
            "environment": "local",
            "deployment_mode": "local_single",
            "api_version_label": "local",
            "base_path": "/"
        },
        "exposure": {},
        "ghcr": {}
    });
    let path = ctx.work_dir.join("sorx-answers.generated.json");
    write_json(&path, &value)?;
    println!(
        "Generated local memory Sorx startup answers: {}",
        path.display()
    );
    Ok(path)
}

fn write_setup_answers_if_needed(ctx: &mut TestContext) -> Result<(), String> {
    if ctx.options.setup_answers.is_some() {
        return Ok(());
    }
    let value = json!({
        "bundle_source": ".",
        "env": "dev",
        "greentic_setup_version": "1.0.0",
        "platform_setup": {
            "deployment_targets": [],
            "static_routes": {
                "default_route_prefix_policy": "pack_declared",
                "public_base_url": ctx.webchat_url,
                "public_surface_policy": "enabled",
                "public_web_enabled": true,
                "tenant_path_policy": "pack_declared"
            },
            "tunnel": {"mode": "off"}
        },
        "setup_answers": {
            "messaging-webchat-gui": {
                "base_url": ctx.webchat_url,
                "jwt_signing_key": "sorx-manager-local-signing-key-0123456789abcdef",
                "mode": "local_queue",
                "nav_links": [
                    {
                        "id": "sorx-manager",
                        "label": "Sorx Manager",
                        "url": format!("{}/v1/sorx/manager", ctx.sorx_url.base)
                    },
                    {
                        "id": "sorx-dashboard-card",
                        "label": "Dashboard Card",
                        "url": format!("{}/v1/sorx/manager/cards/dashboard", ctx.sorx_url.base)
                    }
                ],
                "presentation_mode": "standalone",
                "public_base_url": ctx.webchat_url,
                "route": "webchat",
                "skin": "default",
                "tenant_channel_id": "demo:webchat",
                "text_input_enabled": false
            }
        },
        "team": "default",
        "tenant": "demo"
    });
    write_json(&ctx.setup_answers, &value)
}

fn install_sorx_handoff_pack(ctx: &TestContext) -> Result<(), String> {
    let pack_path = ctx.bundle_dir.join("packs/default.gtpack");
    let work_dir = ctx.work_dir.join("fake-app-pack");
    if !pack_path.is_file() {
        return Err(format!(
            "default app pack was not generated: {}",
            pack_path.display()
        ));
    }
    extract_zip_to_dir(&pack_path, &work_dir)?;
    let cards_dir = work_dir.join("assets/cards");
    fs::create_dir_all(&cards_dir)
        .map_err(|err| format!("failed to create {}: {err}", cards_dir.display()))?;
    let welcome = welcome_card(ctx);
    write_json(&cards_dir.join("welcome_card.json"), &welcome)?;
    write_json(&cards_dir.join("welcome.json"), &welcome)?;
    let dashboard = placeholder_dashboard_card(&ctx.options.locale);
    write_json(&cards_dir.join("sorx_dashboard.json"), &dashboard)?;
    for role in roles_or_selected(ctx) {
        write_json(
            &cards_dir.join(format!("{}.json", role_card_id(&role, "dashboard"))),
            &dashboard,
        )?;
    }
    write_card_i18n(&cards_dir, &ctx.options.locale)?;
    pack_dir(&work_dir, &pack_path)?;
    println!("Installed Sorx handoff default.gtpack");
    Ok(())
}

fn start_sorx_runtime(ctx: &TestContext) -> Result<Child, String> {
    println!("Starting Sorx runtime on {}", ctx.sorx_url.base);
    let exe = std::env::current_exe()
        .map_err(|err| format!("failed to resolve current executable: {err}"))?;
    Command::new(exe)
        .arg("start")
        .arg(&ctx.pack_abs)
        .arg("--answers")
        .arg(&ctx.sorx_answers)
        .stdin(Stdio::null())
        .spawn()
        .map_err(|err| format!("failed to start Sorx runtime: {err}"))
}

fn wait_for_manager_card(
    ctx: &TestContext,
    target: &str,
    timeout: Duration,
    child: &mut Child,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    while Instant::now() < deadline {
        if let Some(status) = child
            .try_wait()
            .map_err(|err| format!("failed to inspect Sorx runtime process: {err}"))?
        {
            return Err(format!(
                "Sorx runtime exited before it became ready: {status}"
            ));
        }
        match manager_card(ctx, target, &ctx.selected_role) {
            Ok(_) => return Ok(()),
            Err(err) => last_error = Some(err),
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(format!(
        "Sorx dashboard card endpoint did not become ready: {}",
        last_error.unwrap_or_else(|| "unknown error".to_string())
    ))
}

fn refresh_sorx_dashboard_card(ctx: &TestContext) -> Result<(), String> {
    let pack_path = ctx.bundle_dir.join("packs/default.gtpack");
    let work_dir = ctx.work_dir.join("fake-app-pack-live");
    extract_zip_to_dir(&pack_path, &work_dir)?;
    let cards_dir = work_dir.join("assets/cards");
    fs::create_dir_all(&cards_dir)
        .map_err(|err| format!("failed to create {}: {err}", cards_dir.display()))?;

    for role in roles_or_selected(ctx) {
        let dashboard = match manager_card(ctx, "dashboard", &role) {
            Ok(mut card) => {
                normalize_card_for_webchat(&mut card, ctx, &role);
                card
            }
            Err(err) => {
                eprintln!("  [skip] manager dashboard unavailable for role={role}: {err}");
                continue;
            }
        };
        write_json(
            &cards_dir.join(format!("{}.json", role_card_id(&role, "dashboard"))),
            &dashboard,
        )?;
        if role == ctx.selected_role {
            write_json(&cards_dir.join("sorx_dashboard.json"), &dashboard)?;
        }

        let mut queue = VecDeque::from(collect_navigable_targets(&dashboard));
        let mut seen = BTreeSet::new();
        while let Some(target) = queue.pop_front() {
            if seen.len() >= 50 || !seen.insert(target.clone()) {
                continue;
            }
            let Ok(mut card) = manager_card(ctx, &target, &role) else {
                continue;
            };
            normalize_card_for_webchat(&mut card, ctx, &role);
            write_json(
                &cards_dir.join(format!("{}.json", role_card_id(&role, &target))),
                &card,
            )?;
            for next in collect_navigable_targets(&card) {
                if !seen.contains(&next) {
                    queue.push_back(next);
                }
            }
            if target.starts_with("records/") && target.matches('/').count() == 1 {
                let record = target.trim_start_matches("records/");
                if record.contains('?') {
                    continue;
                }
                let create_target = format!("records/{record}/create");
                match manager_card(ctx, &create_target, &role) {
                    Ok(mut create_card) => {
                        normalize_card_for_webchat(&mut create_card, ctx, &role);
                        write_json(
                            &cards_dir
                                .join(format!("{}.json", role_card_id(&role, &create_target))),
                            &create_card,
                        )?;
                    }
                    Err(err) if err.contains("HTTP 404") => {
                        eprintln!(
                            "  [skip] Optional manager card not found for role={role}: {}/v1/sorx/manager/cards/{create_target}",
                            ctx.sorx_url.base
                        );
                    }
                    Err(err) => return Err(err),
                }
            }
        }
    }
    write_card_i18n(&cards_dir, &ctx.options.locale)?;
    pack_dir(&work_dir, &pack_path)?;
    println!("Injected live Sorx manager cards into default.gtpack");
    Ok(())
}

fn manager_card(ctx: &TestContext, target: &str, role: &str) -> Result<Value, String> {
    let path = format!("/v1/sorx/manager/cards/{target}");
    http_get_json(ctx, &path, role)
}

fn http_get_json(ctx: &TestContext, path: &str, role: &str) -> Result<Value, String> {
    let mut stream = TcpStream::connect(ctx.sorx_url.bind_addr())
        .map_err(|err| format!("failed to connect to {}: {err}", ctx.sorx_url.bind_addr()))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|err| format!("failed to set HTTP read timeout: {err}"))?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nX-Greentic-Tenant-Id: demo\r\nX-Greentic-Caller-Id: local-test\r\nX-Greentic-Caller-Role: {role}\r\nX-Greentic-Channel: webchat\r\nX-Greentic-Locale: {}\r\nAccept-Language: {}\r\nConnection: close\r\n\r\n",
        ctx.sorx_url.host, ctx.options.locale, ctx.options.locale
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("failed to write HTTP request: {err}"))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|err| format!("failed to read HTTP response: {err}"))?;
    parse_http_json_response(&response)
}

fn parse_http_json_response(bytes: &[u8]) -> Result<Value, String> {
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "HTTP response did not include headers".to_string())?;
    let headers = String::from_utf8_lossy(&bytes[..split]);
    let status_line = headers
        .lines()
        .next()
        .ok_or_else(|| "HTTP response was empty".to_string())?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| format!("invalid HTTP status line: {status_line}"))?;
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status}: {status_line}"));
    }
    serde_json::from_slice(&bytes[split + 4..])
        .map_err(|err| format!("HTTP response body is invalid JSON: {err}"))
}

fn ensure_port_available(url: &ParsedHttpUrl) -> Result<(), String> {
    TcpListener::bind(url.bind_addr())
        .map(|_| ())
        .map_err(|err| {
            format!(
                "cannot start Sorx runtime on {}: address is already in use or unavailable ({err})",
                url.bind_addr()
            )
        })
}

fn run_command(command: &mut Command, label: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|err| format!("failed to run {label}: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} exited with status {status}"))
    }
}

fn terminate_child(child: &mut Child) {
    if let Ok(None) = child.try_wait() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|err| format!("failed to encode JSON for {}: {err}", path.display()))?;
    let mut bytes = bytes;
    bytes.push(b'\n');
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    fs::write(path, bytes).map_err(|err| format!("failed to write {}: {err}", path.display()))
}

fn extract_zip_to_dir(pack_path: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        fs::remove_dir_all(target)
            .map_err(|err| format!("failed to clean {}: {err}", target.display()))?;
    }
    fs::create_dir_all(target)
        .map_err(|err| format!("failed to create {}: {err}", target.display()))?;
    let file = fs::File::open(pack_path)
        .map_err(|err| format!("failed to open {}: {err}", pack_path.display()))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|err| format!("failed to read zip {}: {err}", pack_path.display()))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| format!("failed to read zip entry {index}: {err}"))?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = entry.enclosed_name() else {
            return Err(format!("unsafe zip entry path `{}`", entry.name()));
        };
        let out_path = target.join(name);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        let mut out = fs::File::create(&out_path)
            .map_err(|err| format!("failed to create {}: {err}", out_path.display()))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|err| format!("failed to extract {}: {err}", out_path.display()))?;
    }
    Ok(())
}

fn pack_dir(source: &Path, pack_path: &Path) -> Result<(), String> {
    let tmp = pack_path.with_extension("gtpack.tmp");
    let file = fs::File::create(&tmp)
        .map_err(|err| format!("failed to create {}: {err}", tmp.display()))?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    for path in sorted_files(source)? {
        let rel = path
            .strip_prefix(source)
            .map_err(|err| format!("failed to compute relative zip path: {err}"))?
            .to_string_lossy()
            .replace('\\', "/");
        writer
            .start_file(rel, options)
            .map_err(|err| format!("failed to start zip entry: {err}"))?;
        let bytes =
            fs::read(&path).map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        writer
            .write_all(&bytes)
            .map_err(|err| format!("failed to write zip entry: {err}"))?;
    }
    writer
        .finish()
        .map_err(|err| format!("failed to finish {}: {err}", tmp.display()))?;
    fs::rename(&tmp, pack_path).map_err(|err| {
        format!(
            "failed to replace {} with {}: {err}",
            pack_path.display(),
            tmp.display()
        )
    })
}

fn sorted_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    collect_files(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in
        fs::read_dir(root).map_err(|err| format!("failed to read {}: {err}", root.display()))?
    {
        let entry = entry.map_err(|err| format!("failed to read directory entry: {err}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files)?;
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn welcome_card(ctx: &TestContext) -> Value {
    let actions = roles_or_selected(ctx)
        .into_iter()
        .map(|role| {
            let card_id = role_card_id(&role, "dashboard");
            json!({
                "type": "Action.Submit",
                "title": format!("Open as {}", humanize(&role)),
                "data": {
                    "routeToCardId": card_id,
                    "cardId": card_id,
                    "action": card_id,
                    "sorx_role": role
                }
            })
        })
        .collect::<Vec<_>>();
    json!({
        "type": "AdaptiveCard",
        "version": "1.5",
        "lang": ctx.options.locale,
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "metadata": {"locale": ctx.options.locale},
        "body": [
            {"type": "TextBlock", "text": humanize(&ctx.pack_id), "size": "Large", "weight": "Bolder", "wrap": true},
            {"type": "TextBlock", "text": "Continue to the manager dashboard card to inspect records and card navigation.", "wrap": true}
        ],
        "actions": actions
    })
}

fn placeholder_dashboard_card(locale: &str) -> Value {
    json!({
        "type": "AdaptiveCard",
        "version": "1.5",
        "lang": locale,
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "metadata": {"locale": locale},
        "body": [
            {"type": "TextBlock", "text": "Sorx dashboard is starting", "size": "Large", "weight": "Bolder", "wrap": true},
            {"type": "TextBlock", "text": "The live dashboard card will be injected here after the Sorx runtime is ready.", "wrap": true}
        ],
        "actions": []
    })
}

fn normalize_card_for_webchat(card: &mut Value, ctx: &TestContext, role: &str) {
    normalize_actions(card, ctx, role);
    normalize_card_items(card);
}

fn normalize_actions(value: &mut Value, ctx: &TestContext, role: &str) {
    match value {
        Value::Object(map) => {
            let is_submit = map
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|value| value == "Action.Submit");
            if is_submit && let Some(data) = map.get_mut("data").and_then(Value::as_object_mut) {
                if data.get("action").and_then(Value::as_str) == Some("manager_submit") {
                    data.entry("manager_submit_url".to_string())
                        .or_insert_with(|| {
                            json!(format!("{}/v1/sorx/manager/submit", ctx.sorx_url.base))
                        });
                    if let Some(record) = data
                        .get("record")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned)
                    {
                        let target = format!("records/{record}");
                        let card_id = role_card_id(role, &target);
                        data.entry("manager_target".to_string())
                            .or_insert_with(|| json!(target));
                        data.entry("manager_cards_base_url".to_string())
                            .or_insert_with(|| {
                                json!(format!("{}/v1/sorx/manager/cards", ctx.sorx_url.base))
                            });
                        data.entry("routeToCardId".to_string())
                            .or_insert_with(|| json!(card_id));
                        data.entry("cardId".to_string())
                            .or_insert_with(|| json!(role_card_id(role, &target)));
                        data.entry("step".to_string())
                            .or_insert_with(|| json!("submit"));
                        data.entry("sorx_role".to_string())
                            .or_insert_with(|| json!(role));
                    }
                }
                if let Some(target) = data
                    .get("manager_target")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                {
                    let card_id = role_card_id(role, &target);
                    data.entry("manager_cards_base_url".to_string())
                        .or_insert_with(|| {
                            json!(format!("{}/v1/sorx/manager/cards", ctx.sorx_url.base))
                        });
                    data.insert("routeToCardId".to_string(), json!(card_id));
                    data.entry("cardId".to_string())
                        .or_insert_with(|| json!(role_card_id(role, &target)));
                    data.entry("step".to_string())
                        .or_insert_with(|| json!("open"));
                    data.entry("action".to_string())
                        .or_insert_with(|| json!(role_card_id(role, &target)));
                    data.entry("sorx_role".to_string())
                        .or_insert_with(|| json!(role));
                }
            }
            for child in map.values_mut() {
                normalize_actions(child, ctx, role);
            }
        }
        Value::Array(items) => {
            for child in items {
                normalize_actions(child, ctx, role);
            }
        }
        _ => {}
    }
}

fn normalize_card_items(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("TextBlock") {
                for key in ["size", "weight"] {
                    if let Some(text) = map.get(key).and_then(Value::as_str) {
                        let mut chars = text.chars();
                        if let Some(first) = chars.next() {
                            *map.get_mut(key).unwrap() = Value::String(format!(
                                "{}{}",
                                first.to_uppercase(),
                                chars.as_str()
                            ));
                        }
                    }
                }
            }
            if map.get("type").and_then(Value::as_str) == Some("Input.Text") {
                let label = map
                    .get("label")
                    .or_else(|| map.get("placeholder"))
                    .or_else(|| map.get("id"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                if let Some(label) = label {
                    map.entry("label".to_string())
                        .or_insert_with(|| json!(label));
                    map.entry("placeholder".to_string())
                        .or_insert_with(|| json!(label));
                    if map.get("isRequired").and_then(Value::as_bool) == Some(true) {
                        map.entry("errorMessage".to_string())
                            .or_insert_with(|| json!(format!("{label} is required.")));
                    }
                }
            }
            for child in map.values_mut() {
                normalize_card_items(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                normalize_card_items(child);
            }
        }
        _ => {}
    }
}

fn collect_navigable_targets(card: &Value) -> Vec<String> {
    let mut targets = BTreeSet::new();
    collect_targets(card, &mut targets);
    targets
        .into_iter()
        .filter(|target| {
            target == "metrics"
                || target.starts_with("metrics/")
                || (target.starts_with("records/") && !target.ends_with("/create"))
        })
        .collect()
}

fn collect_targets(value: &Value, targets: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("Action.Submit")
                && let Some(target) = map
                    .get("data")
                    .and_then(Value::as_object)
                    .and_then(|data| data.get("manager_target"))
                    .and_then(Value::as_str)
            {
                targets.insert(target.to_string());
            }
            for child in map.values() {
                collect_targets(child, targets);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_targets(child, targets);
            }
        }
        _ => {}
    }
}

fn write_card_i18n(cards_dir: &Path, locale: &str) -> Result<(), String> {
    let i18n_dir = cards_dir
        .parent()
        .ok_or_else(|| format!("cards directory has no parent: {}", cards_dir.display()))?
        .join("i18n");
    fs::create_dir_all(&i18n_dir)
        .map_err(|err| format!("failed to create {}: {err}", i18n_dir.display()))?;
    let mut en = serde_json::Map::new();
    for file in sorted_files(cards_dir)? {
        if file.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(card_name) = file.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        let Ok(value) = fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .ok_or(())
        else {
            continue;
        };
        collect_i18n(card_name, &value, Vec::new(), &mut en);
    }
    write_json(
        &i18n_dir.join("_manifest.json"),
        &json!({"locales": locale_codes(locale)}),
    )?;
    let en_value = Value::Object(en);
    write_json(&i18n_dir.join("en.json"), &en_value)?;
    for code in locale_codes(locale) {
        if code != "en" {
            write_json(&i18n_dir.join(format!("{code}.json")), &en_value)?;
        }
    }
    Ok(())
}

fn collect_i18n(
    card_name: &str,
    value: &Value,
    path: Vec<String>,
    out: &mut serde_json::Map<String, Value>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if ["text", "title", "label", "placeholder", "errorMessage"].contains(&key.as_str())
                {
                    if let Some(text) = child.as_str() {
                        out.insert(
                            format!("cards.{card_name}.{}.{}", path.join("."), key),
                            json!(text),
                        );
                    }
                } else {
                    let mut next = path.clone();
                    next.push(key.clone());
                    collect_i18n(card_name, child, next, out);
                }
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let mut next = path.clone();
                next.push(format!("i{index}"));
                collect_i18n(card_name, child, next, out);
            }
        }
        _ => {}
    }
}

fn locale_codes(locale: &str) -> Vec<String> {
    let mut codes = vec!["en".to_string(), "es".to_string()];
    if !codes.iter().any(|value| value == locale) {
        codes.push(locale.to_string());
    }
    if let Some(language) = locale.split('-').next()
        && !language.is_empty()
        && !codes.iter().any(|value| value == language)
    {
        codes.push(language.to_string());
    }
    codes
}

fn roles_or_selected(ctx: &TestContext) -> Vec<String> {
    if ctx.available_roles.is_empty() {
        vec![ctx.selected_role.clone()]
    } else {
        ctx.available_roles.clone()
    }
}

fn role_card_id(role: &str, target: &str) -> String {
    route_card_id(&format!("roles/{role}/{target}"))
}

fn route_card_id(target: &str) -> String {
    if target == "dashboard" {
        return "sorx_dashboard".to_string();
    }
    target
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn sanitize_id(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn humanize(value: &str) -> String {
    let mut out = String::new();
    let mut previous_was_space = true;
    for ch in value.replace(['_', '-'], " ").chars() {
        if ch.is_whitespace() {
            if !previous_was_space {
                out.push(' ');
            }
            previous_was_space = true;
        } else if previous_was_space {
            out.extend(ch.to_uppercase());
            previous_was_space = false;
        } else {
            out.push(ch);
        }
    }
    out.trim().to_string()
}

fn default_collection_name(entity: &str) -> String {
    let mut chars = entity.chars();
    match chars.next() {
        Some(first) => format!("{}{}s", first.to_lowercase(), chars.as_str()),
        None => "records".to_string(),
    }
}

fn absolutize(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map_err(|err| format!("failed to resolve current directory: {err}"))
            .map(|cwd| cwd.join(path))
    }
}

const MANAGER_HOOK_JS: &str = r#"

// Sorx manager Adaptive Cards are rendered as static WebChat card assets, but
// their submit buttons need to persist through the live manager API.
var __greenticManagerSubmitHook = true;

function isGreenticManagerSubmitAction(action) {
  var activity = action && action.payload && action.payload.activity;
  var value = activity && activity.value;
  return action && action.type === 'DIRECT_LINE/POST_ACTIVITY' &&
    value && value.action === 'manager_submit';
}

function isGreenticManagerOpenAction(action) {
  var activity = action && action.payload && action.payload.activity;
  var value = activity && activity.value;
  return action && action.type === 'DIRECT_LINE/POST_ACTIVITY' &&
    value && value.manager_target && value.action !== 'manager_submit';
}

function greenticHeaderValue(value, fallback) {
  return value == null || value === '' ? fallback : String(value);
}

function greenticManagerHeaders(value) {
  var locale = document.documentElement.getAttribute('lang') ||
    document.querySelector('[data-webchat-locale]')?.getAttribute('data-webchat-locale') ||
    'en-GB';
  return {
    'Content-Type': 'application/json',
    'Accept': 'application/json',
    'X-Greentic-Tenant-Id': greenticHeaderValue(window.__TENANT__, 'demo'),
    'X-Greentic-Caller-Id': greenticHeaderValue(window.__GUEST_ID__, 'webchat-user'),
    'X-Greentic-Caller-Role': greenticHeaderValue(value.sorx_role, 'admin'),
    'X-Greentic-Channel': 'webchat',
    'X-Greentic-Locale': locale,
    'Accept-Language': locale
  };
}

function greenticManagerInput(value) {
  var input = Object.assign({}, value.input || {});
  Object.keys(value || {}).forEach(function (key) {
    if (/^(action|cardId|routeToCardId|step|manager_|sorx_role|_)/.test(key)) return;
    if (key === 'endpoint_id' || key === 'operation_id' || key === 'record') return;
    if (input[key] === undefined) input[key] = value[key];
  });
  return input;
}

function greenticManagerCardsBase(value) {
  if (value.manager_cards_base_url) {
    window.__GREENTIC_MANAGER_CARDS_BASE_URL__ = value.manager_cards_base_url;
    return value.manager_cards_base_url;
  }
  if (value.manager_submit_url) {
    var derived = String(value.manager_submit_url).replace(/\/submit(?:[?#].*)?$/, '/cards');
    window.__GREENTIC_MANAGER_CARDS_BASE_URL__ = derived;
    return derived;
  }
  return window.__GREENTIC_MANAGER_CARDS_BASE_URL__ || null;
}

function greenticManagerSubmitUrl(value) {
  if (value.manager_submit_url) return value.manager_submit_url;
  var base = greenticManagerCardsBase(value);
  return base ? String(base).replace(/\/cards\/?$/, '/submit') : null;
}

function greenticManagerSearchValue(value) {
  var inputId = value.manager_search_input;
  if (!inputId) return null;
  var raw = value[inputId];
  if ((raw == null || raw === '') && value.input) raw = value.input[inputId];
  if (raw == null) return null;
  var text = String(raw).trim();
  return text === '' ? null : text;
}

function greenticManagerTarget(value) {
  var target = value.manager_target || (value.record ? 'records/' + value.record : 'dashboard');
  var search = greenticManagerSearchValue(value);
  if (search) {
    target += (String(target).indexOf('?') === -1 ? '?' : '&') + 'q=' + encodeURIComponent(search);
  }
  return target;
}

function greenticManagerCardUrl(value) {
  var base = greenticManagerCardsBase(value);
  var target = greenticManagerTarget(value);
  return base ? String(base).replace(/\/+$/, '') + '/' + String(target) : null;
}

function greenticIncomingCardActivity(card) {
  return {
    type: 'message',
    id: 'greentic-manager-' + Date.now(),
    timestamp: new Date().toISOString(),
    from: { id: 'sorx-manager', name: 'Sorx Manager', role: 'bot' },
    attachments: [{ contentType: 'application/vnd.microsoft.card.adaptive', content: card }]
  };
}

function greenticIncomingTextActivity(text) {
  return {
    type: 'message',
    id: 'greentic-manager-error-' + Date.now(),
    timestamp: new Date().toISOString(),
    from: { id: 'sorx-manager', name: 'Sorx Manager', role: 'bot' },
    text: text
  };
}

async function greenticManagerErrorMessage(response, fallback) {
  var message = fallback;
  try {
    var body = await response.clone().json();
    message =
      (body && body.error && body.error.message) ||
      (body && body.message) ||
      message;
  } catch (err) {
    try {
      var text = await response.text();
      if (text) message = text;
    } catch (ignored) {}
  }
  return message;
}

async function handleGreenticManagerSubmit(store, activity) {
  var value = Object.assign({}, activity && activity.value || {});
  var submitUrl = greenticManagerSubmitUrl(value);
  if (!submitUrl) return;
  greenticManagerCardsBase(value);
  var headers = greenticManagerHeaders(value);
  var body = Object.assign({}, value, { input: greenticManagerInput(value) });
  try {
    var submitResponse = await fetch(submitUrl, {
      method: 'POST',
      headers: headers,
      body: JSON.stringify(body)
    });
    if (!submitResponse.ok) {
      throw new Error(await greenticManagerErrorMessage(submitResponse, 'manager submit failed with HTTP ' + submitResponse.status));
    }
    var target = value.manager_target || (value.record ? 'records/' + value.record : 'dashboard');
    var cardResponse = await fetch(String(submitUrl).replace(/\/submit(?:[?#].*)?$/, '/cards/' + target), {
      method: 'GET',
      headers: headers
    });
    if (!cardResponse.ok) {
      throw new Error(await greenticManagerErrorMessage(cardResponse, 'manager card reload failed with HTTP ' + cardResponse.status));
    }
    store.dispatch({
      type: 'DIRECT_LINE/INCOMING_ACTIVITY',
      payload: { activity: greenticIncomingCardActivity(await cardResponse.json()) }
    });
  } catch (err) {
    console.error('[manager-submit]', err);
    var detail = err && err.message ? ' ' + err.message : '';
    store.dispatch({
      type: 'DIRECT_LINE/INCOMING_ACTIVITY',
      payload: { activity: greenticIncomingTextActivity('Unable to submit this manager form.' + detail) }
    });
  }
}

async function handleGreenticManagerOpen(store, activity) {
  var value = Object.assign({}, activity && activity.value || {});
  var cardUrl = greenticManagerCardUrl(value);
  if (!cardUrl) return;
  try {
    var cardResponse = await fetch(cardUrl, {
      method: 'GET',
      headers: greenticManagerHeaders(value)
    });
    if (!cardResponse.ok) throw new Error('manager card load failed with HTTP ' + cardResponse.status);
    store.dispatch({
      type: 'DIRECT_LINE/INCOMING_ACTIVITY',
      payload: { activity: greenticIncomingCardActivity(await cardResponse.json()) }
    });
  } catch (err) {
    console.error('[manager-open]', err);
    store.dispatch({
      type: 'DIRECT_LINE/INCOMING_ACTIVITY',
      payload: { activity: greenticIncomingTextActivity('Unable to open this manager card. Please try again.') }
    });
  }
}
"#;
