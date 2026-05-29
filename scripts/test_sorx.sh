#!/usr/bin/env bash
# Build a local Greentic bundle from a SORX .gtpack plus the WebChat GUI provider,
# run setup non-interactively, and start it for manual manager-card testing.
#
# Usage:
#   scripts/test_sorx.sh <pack.gtpack> [--sorx-answers answers.json]
#                        [--answers setup-answers.json] [--bundle-dir DIR]
#                        [--sorx-url http://127.0.0.1:8787]
#                        [--webchat-url http://127.0.0.1:8080]
#                        [--role ROLE] [--locale LOCALE] [--force] [--no-start]
#
# Environment:
#   SORX_TEST_BUNDLE_DIR      Default bundle workspace directory.
#   SORX_TEST_SETUP_ANSWERS   Existing gtc setup answers file.
#   SORX_TEST_RUNTIME_ANSWERS Existing greentic-sorx startup answers file.
#   SORX_TEST_ROLE            Principal role id used for manager requests.
#   SORX_TEST_LOCALE          Initial WebChat locale.
#   SORX_TEST_BASE_CARD_LOCALE Locale used for generated base card assets.
#   SORX_BIN                  Optional greentic-sorx binary path.
#   SORX_TEST_NO_START=1      Stop after bundle creation and setup.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

WEBCHAT_REF="oci://ghcr.io/greenticai/packs/messaging/messaging-webchat-gui:stable"
SORX_URL="${SORX_URL:-http://127.0.0.1:8787}"
WEBCHAT_URL="${WEBCHAT_URL:-http://127.0.0.1:8080}"
BUNDLE_DIR="${SORX_TEST_BUNDLE_DIR:-}"
SETUP_ANSWERS="${SORX_TEST_SETUP_ANSWERS:-}"
SORX_ANSWERS="${SORX_TEST_RUNTIME_ANSWERS:-}"
SELECTED_ROLE="${SORX_TEST_ROLE:-}"
SELECTED_LOCALE="${SORX_TEST_LOCALE:-en-GB}"
BASE_CARD_LOCALE="${SORX_TEST_BASE_CARD_LOCALE:-en-GB}"
START_BUNDLE=1
FORCE=0
PACK_PATH=""

usage() {
  sed -n '2,18p' "$0" >&2
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 127
  fi
}

json_escape() {
  python3 -c 'import json, sys; print(json.dumps(sys.argv[1]))' "$1"
}

inspect_pack_json() {
  local pack_path="$1"
  if [ -n "${SORX_BIN:-}" ]; then
    "${SORX_BIN}" inspect "${pack_path}" --json
  else
    cargo run --quiet --bin greentic-sorx -- inspect "${pack_path}" --json
  fi
}

generate_sorx_answers() {
  local target="$1"
  python3 - "${INSPECT_JSON}" "${PACK_ABS}" "${target}" "${SORX_URL%/}" <<'PY'
import json
import sys
import zipfile
from pathlib import Path
from urllib.parse import urlparse

inspect_path = Path(sys.argv[1])
pack_path = Path(sys.argv[2])
target = Path(sys.argv[3])
base_url = sys.argv[4].rstrip("/")
parsed = urlparse(base_url)
if parsed.scheme != "http" or not parsed.hostname or not parsed.port:
    raise SystemExit("--sorx-url must look like http://host:port")

inspect = json.loads(inspect_path.read_text(encoding="utf-8"))
pack_name = inspect.get("pack", {}).get("name") or pack_path.stem
with zipfile.ZipFile(pack_path, "r") as archive:
    gateway = json.loads(archive.read("assets/sorla/agent-gateway.json").decode("utf-8"))

entities = {}
for endpoint in gateway.get("endpoints", []):
    if not isinstance(endpoint, dict):
        continue
    entity = endpoint.get("entity") or endpoint.get("record")
    if not isinstance(entity, str) or not entity:
        continue
    collection = endpoint.get("collection")
    if not isinstance(collection, str) or not collection:
        collection = entity[:1].lower() + entity[1:] + "s"
    entities.setdefault(entity, {"provider": "store", "collection": collection})

answers = {
    "tenant": {
        "tenant_id": "demo",
        "environment": "local",
    },
    "server": {
        "bind": f"{parsed.hostname}:{parsed.port}",
        "public_base_url": base_url,
        "auth": {"mode": "none"},
    },
    "mcp": {
        "enabled": False,
        "bind": "127.0.0.1:8790",
    },
    "providers": {
        "store": {
            "kind": "memory",
            "config_ref": "providers.memory.local",
        }
    },
    "bindings": {
        "entities": entities,
    },
    "policy": {
        "approvals": {
            "low": "auto",
            "medium": "auto",
            "high": "require_approval",
            "critical": "deny",
        }
    },
    "audit": {
        "sink": "stdout",
    },
    "deployment": {
        "tenant_id": "demo",
        "sor_name": pack_name,
        "environment": "local",
        "deployment_mode": "local_single",
        "api_version_label": "local",
        "base_path": "/",
    },
    "exposure": {},
    "ghcr": {},
}
target.write_text(json.dumps(answers, indent=2, sort_keys=True) + "\n", encoding="utf-8")
print(f"Generated local memory Sorx startup answers: {target}")
PY
}

install_sorx_handoff_pack() {
  local pack_path="${BUNDLE_DIR}/packs/default.gtpack"
  local fake_app_dir="${WORK_DIR}/fake-app-pack"
  if [ ! -f "${pack_path}" ]; then
    echo "default app pack was not generated: ${pack_path}" >&2
    exit 1
  fi

  rm -rf "${fake_app_dir}"
  mkdir -p "${fake_app_dir}"
  python3 - "${pack_path}" "${fake_app_dir}" "${DASHBOARD_CARD_URL}" "${MANAGER_URL}" "${SELECTED_ROLE}" "${AVAILABLE_ROLES_JSON}" "${SELECTED_LOCALE}" "${BASE_CARD_LOCALE}" <<'PY'
import json
import shutil
import sys
import zipfile
from pathlib import Path

pack_path = Path(sys.argv[1])
work_dir = Path(sys.argv[2])
dashboard_url = sys.argv[3]
selected_role = sys.argv[5]
available_roles = json.loads(sys.argv[6])
selected_locale = sys.argv[7]
base_card_locale = sys.argv[8]

def humanize(value):
    out = []
    prev = ""
    for ch in value.replace("_", "-"):
        if ch == "-":
            out.append(" ")
        elif ch.isupper() and prev and prev != " ":
            out.append(" ")
            out.append(ch)
        else:
            out.append(ch)
        prev = out[-1] if out else ""
    return " ".join("".join(out).split()).title()

def route_card_id(target):
    if target == "dashboard":
        return "sorx_dashboard"
    return "".join(ch if ch.isalnum() or ch in "_-" else "_" for ch in target)

def role_card_id(role, target):
    return route_card_id(f"roles/{role}/{target}")

pack_title = humanize(pack_path.stem.removesuffix(".gtpack"))

with zipfile.ZipFile(pack_path, "r") as src:
    src.extractall(work_dir)

cards = work_dir / "assets" / "cards"
cards.mkdir(parents=True, exist_ok=True)

welcome = {
    "type": "AdaptiveCard",
    "version": "1.5",
    "lang": base_card_locale,
    "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
    "metadata": {
        "locale": base_card_locale,
    },
    "body": [
        {"type": "TextBlock", "text": pack_title, "size": "Large", "weight": "Bolder", "wrap": True},
        {"type": "TextBlock", "text": "Continue to the manager dashboard card to inspect records and card navigation.", "wrap": True},
    ],
    "actions": [
        {
            "type": "Action.Submit",
            "title": f"Open as {humanize(role)}",
            "data": {
                "routeToCardId": role_card_id(role, "dashboard"),
                "cardId": role_card_id(role, "dashboard"),
                "action": role_card_id(role, "dashboard"),
                "sorx_role": role,
            },
        }
        for role in (available_roles or [selected_role])
    ],
}

def card_i18n_key(card_name, path, field):
    return f"cards.{card_name}.{'.'.join(path)}.{field}"

def translate_manager_text(value, locale):
    if not isinstance(value, str):
        return value
    language = locale.split("-", 1)[0].lower()
    if language != "es":
        return value
    exact = {
        "Continue to the manager dashboard card to inspect records and card navigation.": "Continua a la tarjeta del panel de gestion para revisar registros y navegar por las tarjetas.",
        "Sorx dashboard is starting": "El panel de Sorx se esta iniciando",
        "The live dashboard card will be injected here after the Sorx runtime is ready.": "La tarjeta del panel en vivo se insertara aqui cuando el runtime de Sorx este listo.",
        "Create": "Crear",
        "Dashboard": "Panel",
        "Submit": "Enviar",
        "Search": "Buscar",
        "Search and dropdown choices will appear here when records are available.": "La busqueda y las opciones desplegables apareceran aqui cuando haya registros disponibles.",
        "Metric": "Metrica",
        "Metric not found.": "No se encontro la metrica.",
        "Landlord Tenant Sor": "SOR de arrendadores e inquilinos",
        "This package exposes handoff metadata for business-safe agent endpoints.": "Este paquete expone metadatos de traspaso para endpoints de agentes empresariales seguros.",
        "Building": "Edificio",
        "Buildings": "Edificios",
        "buildings": "edificios",
        "Landlord": "Arrendador",
        "Landlords": "Arrendadores",
        "landlords": "arrendadores",
        "Maintenance Request": "Solicitud de mantenimiento",
        "Maintenance Requests": "Solicitudes de mantenimiento",
        "MaintenanceRequest": "Solicitud de mantenimiento",
        "maintenance_requests": "solicitudes_de_mantenimiento",
        "Payment": "Pago",
        "Payments": "Pagos",
        "payments": "pagos",
        "Tenancy": "Arrendamiento",
        "Tenancies": "Arrendamientos",
        "tenancies": "arrendamientos",
        "Tenant": "Inquilino",
        "Tenants": "Inquilinos",
        "tenants": "inquilinos",
        "Unit": "Unidad",
        "Units": "Unidades",
        "units": "unidades",
        "Address": "Direccion",
        "Amount": "Importe",
        "Building Id": "ID de edificio",
        "Completed At": "Completado el",
        "Created At": "Creado el",
        "Description": "Descripcion",
        "Due Date": "Fecha de vencimiento",
        "Email": "Correo electronico",
        "Failed": "Fallido",
        "Full Name": "Nombre completo",
        "Lease End": "Fin del contrato",
        "Lease Start": "Inicio del contrato",
        "Landlord Id": "ID de arrendador",
        "Notes": "Notas",
        "Paid At": "Pagado el",
        "Patch Json": "JSON de parche",
        "Payment Id": "ID de pago",
        "Pending": "Pendiente",
        "Reason": "Motivo",
        "Record Id": "ID de registro",
        "Record Name": "Nombre del registro",
        "Refunded": "Reembolsado",
        "Rent Amount": "Importe del alquiler",
        "Settled": "Liquidado",
        "Status": "Estado",
        "Summary": "Resumen",
        "Tenant Id": "ID de inquilino",
        "Tenancy Id": "ID de arrendamiento",
        "Unit Id": "ID de unidad",
    }
    if value in exact:
        return exact[value]
    if value.startswith("Open as "):
        return "Abrir como " + value.removeprefix("Open as ")
    return value

def collect_card_i18n(card_name, value, locale, path=None, en=None, translated=None):
    path = path or []
    en = en if en is not None else {}
    translated = translated if translated is not None else {}
    if isinstance(value, dict):
        for key, child in value.items():
            if key in ("text", "title", "label", "placeholder", "errorMessage") and isinstance(child, str):
                i18n_key = card_i18n_key(card_name, path, key)
                en[i18n_key] = child
                translated[i18n_key] = translate_manager_text(child, locale)
            else:
                collect_card_i18n(card_name, child, locale, path + [key], en, translated)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            collect_card_i18n(card_name, child, locale, path + [f"i{index}"], en, translated)
    return en, translated

def write_card(path, card):
    path.write_text(json.dumps(card, indent=2) + "\n", encoding="utf-8")

def write_card_i18n(cards_dir, locale):
    i18n_dir = cards_dir.parent / "i18n"
    i18n_dir.mkdir(parents=True, exist_ok=True)
    en = {}
    translated = {}
    for card_path in sorted(cards_dir.glob("*.json")):
        try:
            card = json.loads(card_path.read_text(encoding="utf-8"))
        except Exception:
            continue
        collect_card_i18n(card_path.stem, card, "es", en=en, translated=translated)
    locale_codes = ["en", "es"]
    language = locale.split("-", 1)[0]
    if locale not in locale_codes:
        locale_codes.append(locale)
    if language and language not in locale_codes:
        locale_codes.append(language)
    (i18n_dir / "_manifest.json").write_text(json.dumps({"locales": locale_codes}, indent=2) + "\n", encoding="utf-8")
    (i18n_dir / "en.json").write_text(json.dumps(en, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    for code in locale_codes:
        if code != "en":
            locale_map = translated if code.split("-", 1)[0].lower() == "es" else en
            (i18n_dir / f"{code}.json").write_text(json.dumps(locale_map, indent=2, sort_keys=True) + "\n", encoding="utf-8")

dashboard = {
    "type": "AdaptiveCard",
    "version": "1.5",
    "lang": base_card_locale,
    "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
    "metadata": {
        "locale": base_card_locale,
    },
    "body": [
        {"type": "TextBlock", "text": "Sorx dashboard is starting", "size": "Large", "weight": "Bolder", "wrap": True},
        {"type": "TextBlock", "text": "The live dashboard card will be injected here after the Sorx runtime is ready.", "wrap": True},
    ],
    "actions": [],
}

for name in ("welcome_card", "welcome"):
    write_card(cards / f"{name}.json", welcome)
write_card(cards / "sorx_dashboard.json", dashboard)
for role in (available_roles or [selected_role]):
    write_card(cards / f"{role_card_id(role, 'dashboard')}.json", dashboard)
write_card_i18n(cards, selected_locale)

tmp_pack = pack_path.with_suffix(".gtpack.tmp")
with zipfile.ZipFile(tmp_pack, "w", compression=zipfile.ZIP_DEFLATED) as dst:
    for path in sorted(work_dir.rglob("*")):
        if path.is_file():
            dst.write(path, path.relative_to(work_dir).as_posix())
shutil.move(tmp_pack, pack_path)
PY
  echo "Installed Sorx handoff default.gtpack"
}

patch_webchat_manager_submit_hook() {
  local provider_pack="${BUNDLE_DIR}/providers/messaging/messaging-webchat-gui.gtpack"
  local provider_dir="${WORK_DIR}/webchat-provider-pack"
  local hooks_path="${provider_dir}/assets/webchat-gui/skins/default/webchat/hooks.js"
  if [ ! -f "${provider_pack}" ]; then
    echo "WebChat provider pack was not generated: ${provider_pack}" >&2
    exit 1
  fi

  rm -rf "${provider_dir}"
  mkdir -p "${provider_dir}"
  python3 - "${provider_pack}" "${provider_dir}" "${hooks_path}" <<'PY'
import shutil
import sys
import zipfile
from pathlib import Path

pack_path = Path(sys.argv[1])
provider_dir = Path(sys.argv[2])
hooks_path = Path(sys.argv[3])

with zipfile.ZipFile(pack_path, "r") as src:
    src.extractall(provider_dir)

source = hooks_path.read_text(encoding="utf-8")
if "__greenticManagerSubmitHook" not in source:
    needle = "    const result = next(action);\n"
    replacement = """    if (isGreenticManagerSubmitAction(action)) {
      handleGreenticManagerSubmit(store, action.payload.activity);
      return;
    }
    if (isGreenticManagerOpenAction(action)) {
      handleGreenticManagerOpen(store, action.payload.activity);
      return;
    }

    const result = next(action);
"""
    if needle not in source:
        raise SystemExit("Unable to find WebChat hook middleware insertion point")
    source = source.replace(needle, replacement, 1)
    source += r'''

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

function greenticManagerCardsUrl(submitUrl, target) {
  return String(submitUrl).replace(/\/submit(?:[?#].*)?$/, '/cards/' + String(target || 'dashboard'));
}

function greenticManagerDefaultTarget(value) {
  return value.manager_target || (value.record ? 'records/' + value.record : 'dashboard');
}

function greenticManagerCardsBase(value) {
  if (value.manager_cards_base_url) {
    window.__GREENTIC_MANAGER_CARDS_BASE_URL__ = value.manager_cards_base_url;
    return value.manager_cards_base_url;
  }
  if (value.manager_submit_url) {
    var derived = greenticManagerCardsUrl(value.manager_submit_url, '').replace(/\/$/, '');
    window.__GREENTIC_MANAGER_CARDS_BASE_URL__ = derived;
    return derived;
  }
  return window.__GREENTIC_MANAGER_CARDS_BASE_URL__ || null;
}

function greenticManagerSubmitUrl(value) {
  if (value.manager_submit_url) return value.manager_submit_url;
  var base = greenticManagerCardsBase(value);
  if (!base) return null;
  return String(base).replace(/\/cards\/?$/, '/submit');
}

function greenticManagerCardUrl(value) {
  var base = greenticManagerCardsBase(value);
  if (!base) return null;
  return String(base).replace(/\/+$/, '') + '/' + String(greenticManagerDefaultTarget(value));
}

function greenticIncomingCardActivity(card) {
  return {
    type: 'message',
    id: 'greentic-manager-' + Date.now(),
    timestamp: new Date().toISOString(),
    from: { id: 'sorx-manager', name: 'Sorx Manager', role: 'bot' },
    attachments: [{
      contentType: 'application/vnd.microsoft.card.adaptive',
      content: card
    }]
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

async function handleGreenticManagerSubmit(store, activity) {
  var value = Object.assign({}, activity && activity.value || {});
  var submitUrl = greenticManagerSubmitUrl(value);
  if (!submitUrl) {
    console.warn('[manager-submit] manager submit URL is not available');
    return;
  }
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
      throw new Error('manager submit failed with HTTP ' + submitResponse.status);
    }
    var cardResponse = await fetch(greenticManagerCardsUrl(submitUrl, greenticManagerDefaultTarget(value)), {
      method: 'GET',
      headers: headers
    });
    if (!cardResponse.ok) {
      throw new Error('manager card reload failed with HTTP ' + cardResponse.status);
    }
    var card = await cardResponse.json();
    store.dispatch({
      type: 'DIRECT_LINE/INCOMING_ACTIVITY',
      payload: { activity: greenticIncomingCardActivity(card) }
    });
  } catch (err) {
    console.error('[manager-submit]', err);
    store.dispatch({
      type: 'DIRECT_LINE/INCOMING_ACTIVITY',
      payload: { activity: greenticIncomingTextActivity('Unable to submit this manager form. Please try again.') }
    });
  }
}

async function handleGreenticManagerOpen(store, activity) {
  var value = Object.assign({}, activity && activity.value || {});
  var cardUrl = greenticManagerCardUrl(value);
  if (!cardUrl) {
    console.warn('[manager-open] manager cards base URL is not available');
    return;
  }
  try {
    var cardResponse = await fetch(cardUrl, {
      method: 'GET',
      headers: greenticManagerHeaders(value)
    });
    if (!cardResponse.ok) {
      throw new Error('manager card load failed with HTTP ' + cardResponse.status);
    }
    var card = await cardResponse.json();
    store.dispatch({
      type: 'DIRECT_LINE/INCOMING_ACTIVITY',
      payload: { activity: greenticIncomingCardActivity(card) }
    });
  } catch (err) {
    console.error('[manager-open]', err);
    store.dispatch({
      type: 'DIRECT_LINE/INCOMING_ACTIVITY',
      payload: { activity: greenticIncomingTextActivity('Unable to open this manager card. Please try again.') }
    });
  }
}
'''
    hooks_path.write_text(source, encoding="utf-8")

tmp_pack = pack_path.with_suffix(".gtpack.tmp")
with zipfile.ZipFile(tmp_pack, "w", compression=zipfile.ZIP_DEFLATED) as dst:
    for path in sorted(provider_dir.rglob("*")):
        if path.is_file():
            dst.write(path, path.relative_to(provider_dir).as_posix())
shutil.move(tmp_pack, pack_path)
PY
  echo "Patched WebChat manager submit hook"
}

refresh_sorx_dashboard_card() {
  local pack_path="${BUNDLE_DIR}/packs/default.gtpack"
  local fake_app_dir="${WORK_DIR}/fake-app-pack-live"
  rm -rf "${fake_app_dir}"
  mkdir -p "${fake_app_dir}"
  python3 - "${pack_path}" "${fake_app_dir}" "${DASHBOARD_CARD_URL}" "${SELECTED_ROLE}" "${AVAILABLE_ROLES_JSON}" "${SELECTED_LOCALE}" "${BASE_CARD_LOCALE}" <<'PY'
import json
import shutil
import sys
import urllib.error
import urllib.request
import zipfile
from pathlib import Path

pack_path = Path(sys.argv[1])
work_dir = Path(sys.argv[2])
dashboard_url = sys.argv[3]
selected_role = sys.argv[4]
available_roles = json.loads(sys.argv[5])
selected_locale = sys.argv[6]
base_card_locale = sys.argv[7]
base_url = dashboard_url.rsplit("/cards/dashboard", 1)[0]

def route_card_id(target):
    if target == "dashboard":
        return "sorx_dashboard"
    return "".join(ch if ch.isalnum() or ch in "_-" else "_" for ch in target)

def role_card_id(role, target):
    return route_card_id(f"roles/{role}/{target}")

def target_from_card_id(card_id):
    if card_id == "sorx_dashboard":
        return "dashboard"
    if card_id.startswith("records_"):
        return card_id.replace("_", "/")
    return card_id

def request_json(url, role):
    req = urllib.request.Request(url, headers={
        "X-Greentic-Tenant-Id": "demo",
        "X-Greentic-Caller-Id": "local-test",
        "X-Greentic-Caller-Role": role,
        "X-Greentic-Channel": "webchat",
        "X-Greentic-Locale": base_card_locale,
        "Accept-Language": base_card_locale,
    })
    with urllib.request.urlopen(req, timeout=5) as resp:
        return json.loads(resp.read().decode("utf-8"))

def request_optional_json(url, role):
    try:
        return request_json(url, role)
    except urllib.error.HTTPError as err:
        if err.code == 404:
            print(f"  [skip] Optional manager card not found for role={role}: {url}", file=sys.stderr)
            return None
        raise

def normalize_actions(value, role):
    if isinstance(value, dict):
        data = value.get("data")
        if value.get("type") == "Action.Submit" and isinstance(data, dict):
            if data.get("action") == "manager_submit":
                data.setdefault("manager_submit_url", f"{base_url}/submit")
            target = data.get("manager_target")
            if isinstance(target, str) and target:
                data.setdefault("manager_cards_base_url", f"{base_url}/cards")
                card_id = role_card_id(role, target)
                data["routeToCardId"] = card_id
                data.setdefault("cardId", card_id)
                data.setdefault("step", "open")
                data.setdefault("action", card_id)
                data.setdefault("sorx_role", role)
        for child in value.values():
            normalize_actions(child, role)
    elif isinstance(value, list):
        for child in value:
            normalize_actions(child, role)

def collect_manager_targets(value, targets=None):
    targets = targets if targets is not None else []
    if isinstance(value, dict):
        data = value.get("data")
        if value.get("type") == "Action.Submit" and isinstance(data, dict):
            target = data.get("manager_target")
            if isinstance(target, str) and target:
                targets.append(target)
        for child in value.values():
            collect_manager_targets(child, targets)
    elif isinstance(value, list):
        for child in value:
            collect_manager_targets(child, targets)
    return targets

def normalize_card_for_webchat(card, role):
    normalize_actions(card, role)
    for item in list(card.get("body", [])):
        normalize_card_item_for_webchat(item)

def translate_card_in_place(value, locale):
    if isinstance(value, dict):
        for key, child in list(value.items()):
            if key in ("text", "title", "label", "placeholder", "errorMessage") and isinstance(child, str):
                value[key] = translate_manager_text(child, locale)
            else:
                translate_card_in_place(child, locale)
    elif isinstance(value, list):
        for child in value:
            translate_card_in_place(child, locale)

def normalize_card_item_for_webchat(item):
    if not isinstance(item, dict):
        return
    if item.get("type") == "TextBlock":
        size = item.get("size")
        if isinstance(size, str):
            item["size"] = size[:1].upper() + size[1:]
        weight = item.get("weight")
        if isinstance(weight, str):
            item["weight"] = weight[:1].upper() + weight[1:]
    if item.get("type") == "Input.Text":
        label = item.get("label") or item.get("placeholder") or item.get("id")
        if isinstance(label, str) and label:
            item.setdefault("label", label)
            item.setdefault("placeholder", label)
            if item.get("isRequired") is True:
                item.setdefault("errorMessage", f"{label} is required.")
    for key in ("items", "columns"):
        for child in item.get(key, []) or []:
            normalize_card_item_for_webchat(child)

def card_i18n_key(card_name, path, field):
    return f"cards.{card_name}.{'.'.join(path)}.{field}"

def translate_manager_text(value, locale):
    if not isinstance(value, str):
        return value
    language = locale.split("-", 1)[0].lower()
    if language != "es":
        return value
    exact = {
        "Continue to the manager dashboard card to inspect records and card navigation.": "Continua a la tarjeta del panel de gestion para revisar registros y navegar por las tarjetas.",
        "Sorx dashboard is starting": "El panel de Sorx se esta iniciando",
        "The live dashboard card will be injected here after the Sorx runtime is ready.": "La tarjeta del panel en vivo se insertara aqui cuando el runtime de Sorx este listo.",
        "Create": "Crear",
        "Dashboard": "Panel",
        "Submit": "Enviar",
        "Search": "Buscar",
        "⌕ Search": "⌕ Buscar",
        "Add": "Anadir",
        "Cancel": "Cancelar",
        "< Main Menu": "< Menu principal",
        "Metrics": "Metricas",
        "Select a metric to inspect or query.": "Selecciona una metrica para inspeccionar o consultar.",
        "No metrics are declared.": "No se han declarado metricas.",
        "Search and dropdown choices will appear here when records are available.": "La busqueda y las opciones desplegables apareceran aqui cuando haya registros disponibles.",
        "Landlord Tenant Sor": "SOR de arrendadores e inquilinos",
        "This package exposes handoff metadata for business-safe agent endpoints.": "Este paquete expone metadatos de traspaso para endpoints de agentes empresariales seguros.",
        "Building": "Edificio",
        "Buildings": "Edificios",
        "buildings": "edificios",
        "Landlord": "Arrendador",
        "Landlords": "Arrendadores",
        "landlords": "arrendadores",
        "Maintenance Request": "Solicitud de mantenimiento",
        "Maintenance Requests": "Solicitudes de mantenimiento",
        "MaintenanceRequest": "Solicitud de mantenimiento",
        "maintenance_requests": "solicitudes_de_mantenimiento",
        "Payment": "Pago",
        "Payments": "Pagos",
        "payments": "pagos",
        "Tenancy": "Arrendamiento",
        "Tenancies": "Arrendamientos",
        "tenancies": "arrendamientos",
        "Tenant": "Inquilino",
        "Tenants": "Inquilinos",
        "tenants": "inquilinos",
        "Unit": "Unidad",
        "Units": "Unidades",
        "units": "unidades",
        "Address": "Direccion",
        "Amount": "Importe",
        "Building Id": "ID de edificio",
        "Completed At": "Completado el",
        "Created At": "Creado el",
        "Description": "Descripcion",
        "Due Date": "Fecha de vencimiento",
        "Email": "Correo electronico",
        "Failed": "Fallido",
        "Full Name": "Nombre completo",
        "Lease End": "Fin del contrato",
        "Lease Start": "Inicio del contrato",
        "Landlord Id": "ID de arrendador",
        "Notes": "Notas",
        "Paid At": "Pagado el",
        "Patch Json": "JSON de parche",
        "Payment Id": "ID de pago",
        "Pending": "Pendiente",
        "Reason": "Motivo",
        "Record Id": "ID de registro",
        "Record Name": "Nombre del registro",
        "Refunded": "Reembolsado",
        "Rent Amount": "Importe del alquiler",
        "Settled": "Liquidado",
        "Status": "Estado",
        "Summary": "Resumen",
        "Tenant Id": "ID de inquilino",
        "Tenancy Id": "ID de arrendamiento",
        "Unit Id": "ID de unidad",
    }
    if value in exact:
        return exact[value]
    if value.startswith("Open as "):
        return "Abrir como " + value.removeprefix("Open as ")
    if value.startswith("Select "):
        return "Seleccionar " + value.removeprefix("Select ")
    if value.startswith("Search "):
        return "Buscar " + value.removeprefix("Search ")
    if value.startswith("⌕ Search "):
        return "⌕ Buscar " + value.removeprefix("⌕ Search ")
    if value.startswith("Add "):
        return "Anadir " + translate_manager_text(value.removeprefix("Add "), locale)
    if value.endswith(" is required."):
        return "Se requiere " + value.removesuffix(" is required.") + "."
    return value

def collect_card_i18n(card_name, value, locale, path=None, en=None, translated=None):
    path = path or []
    en = en if en is not None else {}
    translated = translated if translated is not None else {}
    if isinstance(value, dict):
        for key, child in value.items():
            if key in ("text", "title", "label", "placeholder", "errorMessage") and isinstance(child, str):
                i18n_key = card_i18n_key(card_name, path, key)
                en[i18n_key] = child
                translated[i18n_key] = translate_manager_text(child, locale)
            else:
                collect_card_i18n(card_name, child, locale, path + [key], en, translated)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            collect_card_i18n(card_name, child, locale, path + [f"i{index}"], en, translated)
    return en, translated

def write_card(path, card):
    path.write_text(json.dumps(card, indent=2) + "\n", encoding="utf-8")

def write_card_i18n(cards_dir, locale):
    i18n_dir = cards_dir.parent / "i18n"
    i18n_dir.mkdir(parents=True, exist_ok=True)
    en = {}
    translated = {}
    for card_path in sorted(cards_dir.glob("*.json")):
        try:
            card = json.loads(card_path.read_text(encoding="utf-8"))
        except Exception:
            continue
        collect_card_i18n(card_path.stem, card, "es", en=en, translated=translated)
    locale_codes = ["en", "es"]
    language = locale.split("-", 1)[0]
    if locale not in locale_codes:
        locale_codes.append(locale)
    if language and language not in locale_codes:
        locale_codes.append(language)
    (i18n_dir / "_manifest.json").write_text(json.dumps({"locales": locale_codes}, indent=2) + "\n", encoding="utf-8")
    (i18n_dir / "en.json").write_text(json.dumps(en, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    for code in locale_codes:
        if code != "en":
            locale_map = translated if code.split("-", 1)[0].lower() == "es" else en
            (i18n_dir / f"{code}.json").write_text(json.dumps(locale_map, indent=2, sort_keys=True) + "\n", encoding="utf-8")

with zipfile.ZipFile(pack_path, "r") as src:
    src.extractall(work_dir)

cards = work_dir / "assets" / "cards"
cards.mkdir(parents=True, exist_ok=True)

def enhance_create_card(card, record, role, create_actions):
    create_card_id = role_card_id(role, f"records/{record}/create")
    list_target = f"records/{record}"
    list_card_id = role_card_id(role, list_target)
    action_meta = create_actions.get(record, {})
    for action in iter_submit_actions(card):
        data = action.get("data")
        if data.get("record") != record or data.get("manager_target"):
            continue
        data.setdefault("endpoint_id", action_meta.get("endpoint_id", ""))
        data.setdefault("operation_id", action_meta.get("operation_id", data.get("endpoint_id", "")))
        data["action"] = "manager_submit"
        data["cardId"] = create_card_id
        data["step"] = "submit"
        data["manager_target"] = list_target
        data["routeToCardId"] = list_card_id
        data["sorx_role"] = role
        data["manager_submit_url"] = f"{base_url}/submit"

def iter_submit_actions(value):
    if isinstance(value, dict):
        data = value.get("data")
        if value.get("type") == "Action.Submit" and isinstance(data, dict):
            yield value
        for child in value.values():
            yield from iter_submit_actions(child)
    elif isinstance(value, list):
        for child in value:
            yield from iter_submit_actions(child)

for role in (available_roles or [selected_role]):
    dashboard = request_json(dashboard_url, role)
    normalize_card_for_webchat(dashboard, role)
    view = request_json(f"{base_url}/view", role)
    create_actions = {}
    for action in view.get("actions", []):
        record = action.get("record")
        label_key = action.get("label_key", "")
        endpoint_id = action.get("endpoint_id", "")
        operation_id = action.get("operation_id", endpoint_id)
        if (
            isinstance(record, str)
            and (
                label_key.endswith(".create.label")
                or ".create" in endpoint_id
                or ".create" in operation_id
                or "_create" in endpoint_id
                or "_create" in operation_id
                or "-create" in endpoint_id
                or "-create" in operation_id
                or endpoint_id.startswith("create_")
                or operation_id.startswith("create_")
            )
        ):
            create_actions[record] = {
                "endpoint_id": endpoint_id,
                "operation_id": operation_id,
            }
    dashboard_id = role_card_id(role, "dashboard")
    write_card(cards / f"{dashboard_id}.json", dashboard)
    if role == selected_role:
        write_card(cards / "sorx_dashboard.json", dashboard)

    targets = []
    for target in collect_manager_targets(dashboard):
        if isinstance(target, str) and (
            target == "metrics"
            or (target.startswith("records/") and target.count("/") == 1)
        ):
            targets.append(target)

    seen_targets = set()
    queue = sorted(set(targets))
    while queue and len(seen_targets) < 50:
        target = queue.pop(0)
        if target in seen_targets:
            continue
        seen_targets.add(target)
        card = request_json(f"{base_url}/cards/{target}", role)
        normalize_card_for_webchat(card, role)
        write_card(cards / f"{role_card_id(role, target)}.json", card)
        for child_target in collect_manager_targets(card):
            if not isinstance(child_target, str):
                continue
            if not (
                child_target == "metrics"
                or child_target.startswith("metrics/")
                or child_target.startswith("records/")
            ):
                continue
            if child_target.endswith("/create"):
                continue
            if child_target not in seen_targets and child_target not in queue:
                queue.append(child_target)

        if not target.startswith("records/") or target.count("/") != 1:
            continue
        record = target.split("/", 1)[1].split("?", 1)[0]
        if "/" in record:
            continue
        create_target = f"records/{record}/create"
        card = request_optional_json(f"{base_url}/cards/{create_target}", role)
        if card is None:
            continue
        normalize_card_for_webchat(card, role)
        enhance_create_card(card, record, role, create_actions)
        write_card(cards / f"{role_card_id(role, create_target)}.json", card)

write_card_i18n(cards, selected_locale)

tmp_pack = pack_path.with_suffix(".gtpack.tmp")
with zipfile.ZipFile(tmp_pack, "w", compression=zipfile.ZIP_DEFLATED) as dst:
    for path in sorted(work_dir.rglob("*")):
        if path.is_file():
            dst.write(path, path.relative_to(work_dir).as_posix())
shutil.move(tmp_pack, pack_path)
PY
  echo "Injected live Sorx manager cards into default.gtpack"
}

while [ $# -gt 0 ]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --answers)
      shift
      SETUP_ANSWERS="${1:-}"
      ;;
    --answers=*)
      SETUP_ANSWERS="${1#--answers=}"
      ;;
    --sorx-answers)
      shift
      SORX_ANSWERS="${1:-}"
      ;;
    --sorx-answers=*)
      SORX_ANSWERS="${1#--sorx-answers=}"
      ;;
    --bundle-dir)
      shift
      BUNDLE_DIR="${1:-}"
      ;;
    --bundle-dir=*)
      BUNDLE_DIR="${1#--bundle-dir=}"
      ;;
    --sorx-url)
      shift
      SORX_URL="${1:-}"
      ;;
    --sorx-url=*)
      SORX_URL="${1#--sorx-url=}"
      ;;
    --webchat-url)
      shift
      WEBCHAT_URL="${1:-}"
      ;;
    --webchat-url=*)
      WEBCHAT_URL="${1#--webchat-url=}"
      ;;
    --role)
      shift
      SELECTED_ROLE="${1:-}"
      ;;
    --role=*)
      SELECTED_ROLE="${1#--role=}"
      ;;
    --locale)
      shift
      SELECTED_LOCALE="${1:-}"
      ;;
    --locale=*)
      SELECTED_LOCALE="${1#--locale=}"
      ;;
    --no-docker-pull)
      echo "warning: --no-docker-pull is deprecated; WebChat OCI packs are resolved by greentic-bundle/gtc" >&2
      ;;
    --force)
      FORCE=1
      ;;
    --no-start)
      START_BUNDLE=0
      ;;
    -*)
      echo "unknown argument: $1" >&2
      usage
      exit 2
      ;;
    *)
      if [ -n "${PACK_PATH}" ]; then
        echo "only one <pack.gtpack> argument is supported" >&2
        usage
        exit 2
      fi
      PACK_PATH="$1"
      ;;
  esac
  shift
done

if [ -z "${PACK_PATH}" ]; then
  usage
  exit 2
fi
if [ ! -f "${PACK_PATH}" ]; then
  echo "SORX pack not found: ${PACK_PATH}" >&2
  exit 1
fi
if [ -z "${SORX_URL}" ] || [ -z "${WEBCHAT_URL}" ] || [ -z "${SELECTED_LOCALE}" ]; then
  echo "--sorx-url, --webchat-url and --locale require non-empty values" >&2
  exit 2
fi

require_cmd python3
require_cmd greentic-bundle
require_cmd gtc
if [ -z "${SORX_BIN:-}" ]; then
  require_cmd cargo
elif [ ! -x "${SORX_BIN}" ]; then
  echo "SORX_BIN is not executable: ${SORX_BIN}" >&2
  exit 1
fi

if [ "${SORX_TEST_NO_START:-0}" = "1" ]; then
  START_BUNDLE=0
fi

PACK_ABS="$(cd "$(dirname "${PACK_PATH}")" && pwd)/$(basename "${PACK_PATH}")"
PACK_BASE="$(basename "${PACK_ABS}")"
PACK_ID="${PACK_BASE%.gtpack}"
PACK_ID="${PACK_ID//[^A-Za-z0-9._-]/-}"
BUNDLE_ID="sorx-manager-${PACK_ID}"

if [ -z "${BUNDLE_DIR}" ]; then
  BUNDLE_DIR="${TMPDIR:-/tmp}/${BUNDLE_ID}-bundle"
fi
BUNDLE_DIR="$(mkdir -p "$(dirname "${BUNDLE_DIR}")" && cd "$(dirname "${BUNDLE_DIR}")" && pwd)/$(basename "${BUNDLE_DIR}")"

WORK_DIR="${BUNDLE_DIR}/.test-sorx"
MARKER="${WORK_DIR}/created-by-test-sorx"
CREATE_ANSWERS="${WORK_DIR}/create-answers.json"
GENERATED_SETUP_ANSWERS="${WORK_DIR}/setup-answers.json"
GENERATED_SORX_ANSWERS="${WORK_DIR}/sorx-answers.json"
SORX_METADATA_DIR="${BUNDLE_DIR}/sorx"
DASHBOARD_CARD_URL="${SORX_URL%/}/v1/sorx/manager/cards/dashboard"
MANAGER_URL="${SORX_URL%/}/v1/sorx/manager"

if [ -e "${BUNDLE_DIR}" ] && [ ! -f "${MARKER}" ] && [ "${FORCE}" -ne 1 ]; then
  echo "bundle directory already exists and was not created by this script: ${BUNDLE_DIR}" >&2
  echo "pass --force to replace it" >&2
  exit 1
fi

rm -rf "${BUNDLE_DIR}"
mkdir -p "${WORK_DIR}"
touch "${MARKER}"
mkdir -p "${SORX_METADATA_DIR}"
cp "${PACK_ABS}" "${SORX_METADATA_DIR}/${PACK_BASE}"

INSPECT_JSON="${WORK_DIR}/inspect.json"
inspect_pack_json "${PACK_ABS}" > "${INSPECT_JSON}"
SELECTED_ROLE="$(python3 - "${INSPECT_JSON}" "${SELECTED_ROLE}" <<'PY'
import json
import sys
from pathlib import Path

inspect_path = Path(sys.argv[1])
requested = sys.argv[2].strip()
data = json.loads(inspect_path.read_text(encoding="utf-8"))
roles = [
    role.get("id", "")
    for role in data.get("sorla", {}).get("roles", [])
    if isinstance(role, dict) and role.get("id")
]
if requested:
    if roles and requested not in roles:
        raise SystemExit(
            f"selected role `{requested}` is not declared by the pack; available roles: {', '.join(roles)}"
        )
    print(requested)
elif roles:
    print(roles[0])
else:
    print("admin")
PY
)"
AVAILABLE_ROLES="$(python3 - "${INSPECT_JSON}" <<'PY'
import json
import sys
from pathlib import Path

data = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
roles = [
    role.get("id", "")
    for role in data.get("sorla", {}).get("roles", [])
    if isinstance(role, dict) and role.get("id")
]
print(", ".join(roles) if roles else "(none declared; using fallback role)")
PY
)"
AVAILABLE_ROLES_JSON="$(python3 - "${INSPECT_JSON}" "${SELECTED_ROLE}" <<'PY'
import json
import sys
from pathlib import Path

data = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
selected = sys.argv[2]
roles = [
    role.get("id", "")
    for role in data.get("sorla", {}).get("roles", [])
    if isinstance(role, dict) and role.get("id")
]
print(json.dumps(roles or [selected]))
PY
)"

echo "Preparing Sorx/WebChat test bundle"
echo "  SORX pack:        ${PACK_ABS}"
echo "  bundle workspace: ${BUNDLE_DIR}"
echo "  manager card URL: ${DASHBOARD_CARD_URL}"
echo "  WebChat URL:      ${WEBCHAT_URL%/}/webchat"
echo "  selected role:    ${SELECTED_ROLE}"
echo "  selected locale:  ${SELECTED_LOCALE}"
echo "  base card locale: ${BASE_CARD_LOCALE}"
echo "  available roles:  ${AVAILABLE_ROLES}"
echo "  WebChat pack ref: ${WEBCHAT_REF}"
echo "  OCI resolution:   greentic-bundle/gtc distributor-backed pack fetch"

cat > "${CREATE_ANSWERS}" <<JSON
{
  "wizard_id": "greentic-bundle.wizard.run",
  "schema_id": "greentic-bundle.wizard.answers",
  "schema_version": "1.0.0",
  "locale": $(json_escape "${SELECTED_LOCALE}"),
  "answers": {
    "access_rules": [
    ],
    "advanced_setup": false,
    "app_pack_entries": [
    ],
    "app_packs": [
    ],
    "bundle_id": $(json_escape "${BUNDLE_ID}"),
    "bundle_name": $(json_escape "${BUNDLE_ID}"),
    "export_intent": false,
    "extension_provider_entries": [
      {
        "detected_kind": "oci",
        "display_name": "Greentic Messaging WebChat GUI (stable)",
        "provider_id": "greentic.messaging.webchat-gui.stable",
        "reference": "${WEBCHAT_REF}",
        "version": "stable"
      }
    ],
    "extension_providers": [
      "${WEBCHAT_REF}"
    ],
    "mode": "create",
    "output_dir": $(json_escape "${BUNDLE_DIR}"),
    "remote_catalogs": [],
    "setup_answers": {},
    "setup_execution_intent": false,
    "setup_specs": {}
  }
}
JSON

echo "Creating bundle workspace"
greentic-bundle wizard apply --answers "${CREATE_ANSWERS}"
patch_webchat_manager_submit_hook
mkdir -p "${SORX_METADATA_DIR}"
cp "${PACK_ABS}" "${SORX_METADATA_DIR}/${PACK_BASE}"

if [ -z "${SORX_ANSWERS}" ]; then
  if [ "${PACK_ID}" = "landlord-tenant-sor" ] && [ -f "crates/greentic-sorx-cli/tests/e2e/fixtures/landlord_tenant/answers.memory.json" ]; then
    SORX_ANSWERS="crates/greentic-sorx-cli/tests/e2e/fixtures/landlord_tenant/answers.memory.json"
  else
    generate_sorx_answers "${GENERATED_SORX_ANSWERS}"
    SORX_ANSWERS="${GENERATED_SORX_ANSWERS}"
  fi
fi
if [ ! -f "${SORX_ANSWERS}" ]; then
  echo "SORX startup answers not found: ${SORX_ANSWERS}" >&2
  exit 1
fi

python3 - "${SORX_ANSWERS}" "${GENERATED_SORX_ANSWERS}" "${SORX_URL%/}" <<'PY'
import json
import sys
from pathlib import Path
from urllib.parse import urlparse

source = Path(sys.argv[1])
target = Path(sys.argv[2])
base_url = sys.argv[3].rstrip("/")
parsed = urlparse(base_url)
if parsed.scheme != "http" or not parsed.hostname or not parsed.port:
    raise SystemExit("--sorx-url must look like http://host:port")

data = json.loads(source.read_text(encoding="utf-8"))
answers = data.get("answers", data)
server = answers.setdefault("server", {})
server["bind"] = f"{parsed.hostname}:{parsed.port}"
server["public_base_url"] = base_url
target.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

if [ -z "${SETUP_ANSWERS}" ]; then
  cat > "${GENERATED_SETUP_ANSWERS}" <<JSON
{
  "bundle_source": ".",
  "env": "dev",
  "greentic_setup_version": "1.0.0",
  "platform_setup": {
    "deployment_targets": [],
    "static_routes": {
      "default_route_prefix_policy": "pack_declared",
      "public_base_url": $(json_escape "${WEBCHAT_URL%/}"),
      "public_surface_policy": "enabled",
      "public_web_enabled": true,
      "tenant_path_policy": "pack_declared"
    },
    "tunnel": {
      "mode": "off"
    }
  },
  "setup_answers": {
    "messaging-webchat-gui": {
      "base_url": $(json_escape "${WEBCHAT_URL%/}"),
      "jwt_signing_key": "sorx-manager-local-signing-key-0123456789abcdef",
      "mode": "local_queue",
      "nav_links": [
        {
          "id": "sorx-manager",
          "label": "Sorx Manager",
          "url": $(json_escape "${MANAGER_URL}")
        },
        {
          "id": "sorx-dashboard-card",
          "label": "Dashboard Card",
          "url": $(json_escape "${DASHBOARD_CARD_URL}")
        }
      ],
      "presentation_mode": "standalone",
      "public_base_url": $(json_escape "${WEBCHAT_URL%/}"),
      "route": "webchat",
      "skin": "default",
      "tenant_channel_id": "demo:webchat",
      "text_input_enabled": false
    }
  },
  "team": "default",
  "tenant": "demo"
}
JSON
  SETUP_ANSWERS="${GENERATED_SETUP_ANSWERS}"
fi

echo "Running gtc setup"
gtc setup "${BUNDLE_DIR}" --no-ui --non-interactive --answers "${SETUP_ANSWERS}"
install_sorx_handoff_pack

echo
echo "Sorx manager card endpoint: ${DASHBOARD_CARD_URL}"
echo "WebChat route:               ${WEBCHAT_URL%/}/webchat"
echo "Sorx runtime answers:        ${GENERATED_SORX_ANSWERS}"
echo

if [ "${START_BUNDLE}" -eq 0 ]; then
  echo "Bundle is ready at ${BUNDLE_DIR}"
  exit 0
fi

SORX_PID=""
cleanup() {
  if [ -n "${SORX_PID}" ] && kill -0 "${SORX_PID}" >/dev/null 2>&1; then
    kill "${SORX_PID}" >/dev/null 2>&1 || true
    wait "${SORX_PID}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

echo "Starting Sorx runtime on ${SORX_URL%/}"
if [ -n "${SORX_BIN:-}" ]; then
  "${SORX_BIN}" start "${PACK_ABS}" --answers "${GENERATED_SORX_ANSWERS}" &
else
  cargo run --bin greentic-sorx -- start "${PACK_ABS}" --answers "${GENERATED_SORX_ANSWERS}" &
fi
SORX_PID="$!"

python3 - "${DASHBOARD_CARD_URL}" "${SELECTED_ROLE}" "${SELECTED_LOCALE}" <<'PY'
import sys
import time
import urllib.request

url = sys.argv[1]
selected_role = sys.argv[2]
selected_locale = sys.argv[3]
deadline = time.time() + 45
last_error = None
while time.time() < deadline:
    try:
        req = urllib.request.Request(
            url,
            headers={
                "X-Greentic-Tenant-Id": "demo",
                "X-Greentic-Caller-Id": "local-test",
                "X-Greentic-Caller-Role": selected_role,
                "X-Greentic-Channel": "webchat",
                "X-Greentic-Locale": selected_locale,
                "Accept-Language": selected_locale,
            },
        )
        with urllib.request.urlopen(req, timeout=2) as resp:
            if 200 <= resp.status < 300:
                raise SystemExit(0)
    except SystemExit:
        raise
    except Exception as err:
        last_error = err
    time.sleep(1)
raise SystemExit(f"Sorx dashboard card endpoint did not become ready: {last_error}")
PY
refresh_sorx_dashboard_card

echo "Starting WebChat bundle; press Ctrl-C to stop."
gtc start "${BUNDLE_DIR}"
