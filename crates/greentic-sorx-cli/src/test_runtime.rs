//! Driver for `greentic-sorx test`: builds a WebChat bundle around a SORX pack,
//! starts the runtime and injects live manager cards.
//!
//! Everything here spawns external processes, opens sockets or waits on a child,
//! so it is exercised end-to-end rather than by unit tests and is excluded from
//! the coverage policy. The pure logic it orchestrates lives in the submodules
//! below, each of which is unit-tested.

mod answers;
mod cards;
mod http;
mod ids;
mod packing;

use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use greentic_sorx_pack::{inspect_sorla_pack, load_sorla_pack};
use serde_json::Value;

use answers::{
    apply_server_overrides, build_entity_bindings, create_answers_value, setup_answers_value,
    sorx_answers_value,
};
use cards::{
    collect_navigable_targets, normalize_card_for_webchat, placeholder_dashboard_card,
    welcome_card, write_card_i18n,
};
use http::{ParsedHttpUrl, parse_http_json_response};
use ids::{role_card_id, sanitize_id};
use packing::{absolutize, extract_zip_to_dir, pack_dir, write_json};

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
    let value = create_answers_value(
        &ctx.options.locale,
        &ctx.bundle_id,
        &ctx.bundle_dir,
        WEBCHAT_REF,
    );
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
    apply_server_overrides(&mut value, &ctx.sorx_url.bind_addr(), &ctx.sorx_url.base)?;
    write_json(&ctx.sorx_answers, &value)
}

fn generate_sorx_answers(ctx: &TestContext) -> Result<PathBuf, String> {
    let pack = load_sorla_pack(&ctx.pack_abs)
        .map_err(|err| format!("failed to load pack for startup answer generation: {err}"))?;
    let entities = build_entity_bindings(&pack.sorla_assets.agent_gateway_json);
    let value = sorx_answers_value(
        &ctx.sorx_url.bind_addr(),
        &ctx.sorx_url.base,
        &pack.pack_name,
        entities,
    );
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
    let value = setup_answers_value(&ctx.webchat_url, &ctx.sorx_url.base);
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
    let welcome = welcome_card(&ctx.options.locale, &ctx.pack_id, &roles_or_selected(ctx));
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
                normalize_card_for_webchat(&mut card, &ctx.sorx_url.base, &role);
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
            normalize_card_for_webchat(&mut card, &ctx.sorx_url.base, &role);
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
                        normalize_card_for_webchat(&mut create_card, &ctx.sorx_url.base, &role);
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

fn roles_or_selected(ctx: &TestContext) -> Vec<String> {
    if ctx.available_roles.is_empty() {
        vec![ctx.selected_role.clone()]
    } else {
        ctx.available_roles.clone()
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
