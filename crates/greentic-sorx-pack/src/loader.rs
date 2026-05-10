use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{Cursor, Read, Seek};
use std::path::{Component, Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::inspect::{SorxInspectPack, SorxInspectReport, SorxInspectSorla, SorxInspectSorx};
use crate::manifest::{PackLock, PackManifest};

const SORX_RUNTIME_EXTENSION_ID: &str = "greentic.sorx.runtime.v1";
const REQUIRED_ENTRIES: &[&str] = &[
    "pack.cbor",
    "assets/sorla/model.cbor",
    "assets/sorla/agent-gateway.json",
    "assets/sorx/start.schema.json",
];

#[derive(Debug, Clone, PartialEq)]
pub struct LoadedSorlaPack {
    pub pack_path: PathBuf,
    pub pack_name: String,
    pub pack_version: String,
    pub pack_digest: Option<String>,
    pub manifest: PackManifest,
    pub lock: Option<PackLock>,
    pub sorla_assets: SorlaAssets,
    pub sorx_assets: SorxAssets,
    pub validation_suite_status: ValidationSuiteStatus,
    pub entries: BTreeSet<String>,
    pub doctor_errors: Vec<String>,
    pub doctor_warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SorlaAssets {
    pub model_cbor: Vec<u8>,
    pub agent_gateway_json: Value,
    pub openapi_overlay_yaml: Option<String>,
    pub arazzo_yaml: Option<String>,
    pub mcp_tools_json: Option<Value>,
    pub llms_txt_fragment: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SorxAssets {
    pub start_schema_json: Value,
    pub start_questions_cbor: Option<Vec<u8>>,
    pub runtime_template_yaml: Option<String>,
    pub provider_bindings_template_yaml: Option<String>,
    pub validation_suite_cbor: Option<Vec<u8>>,
    pub validation_suite_json: Option<Value>,
    pub validation_fixture_paths: Vec<String>,
    pub validation_fixtures_json: BTreeMap<String, Value>,
    pub validation_openapi_expected_json: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationSuiteStatus {
    Missing,
    Present,
    Invalid,
}

impl ValidationSuiteStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Present => "present",
            Self::Invalid => "invalid",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SorxPackError {
    code: &'static str,
    message: String,
}

impl SorxPackError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for SorxPackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SorxPackError {}

pub fn load_sorla_pack(path: &Path) -> Result<LoadedSorlaPack, SorxPackError> {
    validate_input_path(path)?;
    let pack_digest = Some(format!("sha256:{}", sha256_file(path)?));
    let mut archive = open_gtpack(path)?;
    let entries = zip_entry_names(&mut archive)?;
    validate_entry_paths(&entries)?;

    for required in REQUIRED_ENTRIES {
        if !entries.contains(*required) {
            return Err(SorxPackError::new(
                "missing_required_entry",
                format!("gtpack is missing required entry `{required}`"),
            ));
        }
    }

    let manifest = read_manifest(&mut archive)?;
    validate_sorx_extension(&manifest)?;
    validate_manifest_asset_paths(&manifest)?;

    let lock = if entries.contains("pack.lock.cbor") {
        Some(read_lock(&mut archive)?)
    } else {
        None
    };
    if let Some(lock) = &lock {
        validate_lock(&mut archive, &entries, lock)?;
    }

    validate_extension_references(&manifest, &entries)?;

    let sorla_assets = read_sorla_assets(&mut archive, &entries)?;
    let (sorx_assets, validation_suite_status, validation_errors) =
        read_sorx_assets(&mut archive, &entries)?;

    let mut doctor_errors = validation_errors;
    doctor_errors.extend(validate_mcp_tools(&sorla_assets));
    let mut doctor_warnings = Vec::new();
    if lock.is_none() {
        doctor_warnings.push("pack.lock.cbor is missing; lock validation was skipped".to_string());
    }
    doctor_warnings.extend(secret_warnings(&sorx_assets));

    Ok(LoadedSorlaPack {
        pack_path: path.to_path_buf(),
        pack_name: manifest.pack.name.clone(),
        pack_version: manifest.pack.version.clone(),
        pack_digest,
        manifest,
        lock,
        sorla_assets,
        sorx_assets,
        validation_suite_status,
        entries,
        doctor_errors,
        doctor_warnings,
    })
}

pub fn inspect_sorla_pack(path: &Path) -> Result<SorxInspectReport, SorxPackError> {
    let pack = load_sorla_pack(path)?;
    Ok(SorxInspectReport {
        schema: "greentic.sorx.inspect.v1".to_string(),
        pack: SorxInspectPack {
            name: pack.pack_name,
            version: pack.pack_version,
            digest: pack.pack_digest,
        },
        sorla: SorxInspectSorla {
            has_model: true,
            has_agent_gateway: true,
            has_openapi_overlay: pack.sorla_assets.openapi_overlay_yaml.is_some(),
            has_arazzo: pack.sorla_assets.arazzo_yaml.is_some(),
            has_mcp_tools: pack.sorla_assets.mcp_tools_json.is_some(),
            has_llms_fragment: pack.sorla_assets.llms_txt_fragment.is_some(),
        },
        sorx: SorxInspectSorx {
            has_start_schema: true,
            has_runtime_template: pack.sorx_assets.runtime_template_yaml.is_some(),
            has_provider_bindings_template: pack
                .sorx_assets
                .provider_bindings_template_yaml
                .is_some(),
            validation_suite_status: pack.validation_suite_status.as_str().to_string(),
            has_validation_suite_cbor: pack.sorx_assets.validation_suite_cbor.is_some(),
            has_validation_suite_json: pack.sorx_assets.validation_suite_json.is_some(),
        },
    })
}

fn validate_input_path(path: &Path) -> Result<(), SorxPackError> {
    if !path.exists() {
        return Err(SorxPackError::new(
            "path_missing",
            format!("input path does not exist: {}", path.display()),
        ));
    }
    if path.extension().and_then(|value| value.to_str()) != Some("gtpack") {
        return Err(SorxPackError::new(
            "not_gtpack",
            format!("input path must have .gtpack extension: {}", path.display()),
        ));
    }
    Ok(())
}

fn open_gtpack(path: &Path) -> Result<ZipArchive<fs::File>, SorxPackError> {
    let file = fs::File::open(path).map_err(|err| {
        SorxPackError::new(
            "open_failed",
            format!("failed to open {}: {err}", path.display()),
        )
    })?;
    ZipArchive::new(file).map_err(|err| {
        SorxPackError::new(
            "invalid_archive",
            format!("failed to read gtpack archive {}: {err}", path.display()),
        )
    })
}

fn zip_entry_names<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<BTreeSet<String>, SorxPackError> {
    let mut names = BTreeSet::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|err| {
            SorxPackError::new(
                "archive_entry_failed",
                format!("failed to inspect gtpack entry {index}: {err}"),
            )
        })?;
        if !entry.is_dir() {
            names.insert(entry.name().to_string());
        }
    }
    Ok(names)
}

fn validate_entry_paths(entries: &BTreeSet<String>) -> Result<(), SorxPackError> {
    for entry in entries {
        validate_relative_pack_path(entry)?;
    }
    Ok(())
}

fn validate_relative_pack_path(path: &str) -> Result<(), SorxPackError> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return Err(SorxPackError::new(
            "unsafe_path",
            format!("unsafe pack entry path `{path}`"),
        ));
    }
    let path = Path::new(path);
    for component in path.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(SorxPackError::new(
                "unsafe_path",
                format!("unsafe pack entry path `{}`", path.display()),
            ));
        }
    }
    Ok(())
}

fn zip_bytes<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>, SorxPackError> {
    let mut entry = archive.by_name(name).map_err(|err| {
        SorxPackError::new(
            "missing_entry",
            format!("gtpack is missing `{name}`: {err}"),
        )
    })?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).map_err(|err| {
        SorxPackError::new(
            "read_entry_failed",
            format!("failed to read `{name}`: {err}"),
        )
    })?;
    Ok(bytes)
}

fn zip_text<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<String, SorxPackError> {
    let bytes = zip_bytes(archive, name)?;
    String::from_utf8(bytes)
        .map_err(|err| SorxPackError::new("invalid_utf8", format!("`{name}` is not UTF-8: {err}")))
}

fn optional_zip_bytes<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    entries: &BTreeSet<String>,
    name: &str,
) -> Result<Option<Vec<u8>>, SorxPackError> {
    if entries.contains(name) {
        zip_bytes(archive, name).map(Some)
    } else {
        Ok(None)
    }
}

fn optional_zip_text<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    entries: &BTreeSet<String>,
    name: &str,
) -> Result<Option<String>, SorxPackError> {
    if entries.contains(name) {
        zip_text(archive, name).map(Some)
    } else {
        Ok(None)
    }
}

fn read_manifest<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<PackManifest, SorxPackError> {
    let bytes = zip_bytes(archive, "pack.cbor")?;
    ciborium::de::from_reader(Cursor::new(bytes)).map_err(|err| {
        SorxPackError::new(
            "invalid_manifest",
            format!("pack.cbor is not a valid SoRLa pack manifest: {err}"),
        )
    })
}

fn read_lock<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<PackLock, SorxPackError> {
    let bytes = zip_bytes(archive, "pack.lock.cbor")?;
    ciborium::de::from_reader(Cursor::new(bytes)).map_err(|err| {
        SorxPackError::new(
            "invalid_lock",
            format!("pack.lock.cbor is not valid CBOR: {err}"),
        )
    })
}

fn validate_sorx_extension(manifest: &PackManifest) -> Result<(), SorxPackError> {
    let extension = manifest
        .extension
        .get("extension")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            SorxPackError::new(
                "missing_sorx_extension",
                "pack.cbor is missing SORX runtime extension metadata",
            )
        })?;
    if extension != SORX_RUNTIME_EXTENSION_ID {
        return Err(SorxPackError::new(
            "unsupported_sorx_extension",
            format!("unsupported SORX extension `{extension}`"),
        ));
    }
    Ok(())
}

fn validate_manifest_asset_paths(manifest: &PackManifest) -> Result<(), SorxPackError> {
    for asset in &manifest.assets {
        validate_allowed_asset_path(asset)?;
    }
    Ok(())
}

fn validate_allowed_asset_path(path: &str) -> Result<(), SorxPackError> {
    validate_relative_pack_path(path)?;
    if !(path.starts_with("assets/sorla/")
        || path.starts_with("assets/sorx/")
        || matches!(
            path,
            "pack.cbor" | "pack.lock.cbor" | "manifest.cbor" | "manifest.json"
        ))
    {
        return Err(SorxPackError::new(
            "disallowed_asset_path",
            format!("manifest references disallowed asset path `{path}`"),
        ));
    }
    Ok(())
}

fn validate_extension_references(
    manifest: &PackManifest,
    entries: &BTreeSet<String>,
) -> Result<(), SorxPackError> {
    for section in ["sorla", "sorx"] {
        let Some(map) = manifest.extension.get(section).and_then(Value::as_object) else {
            continue;
        };
        for value in map.values() {
            let Some(path) = value.as_str() else {
                continue;
            };
            validate_allowed_asset_path(path)?;
            if !entries.contains(path) {
                return Err(SorxPackError::new(
                    "missing_extension_asset",
                    format!("SORX extension references missing asset `{path}`"),
                ));
            }
        }
    }
    Ok(())
}

fn validate_lock<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    entries: &BTreeSet<String>,
    lock: &PackLock,
) -> Result<(), SorxPackError> {
    for (path, expected) in &lock.entries {
        validate_relative_pack_path(path)?;
        if !entries.contains(path) {
            return Err(SorxPackError::new(
                "lock_missing_entry",
                format!("pack.lock.cbor references missing entry `{path}`"),
            ));
        }
        let bytes = zip_bytes(archive, path)?;
        if expected.size != bytes.len() as u64 {
            return Err(SorxPackError::new(
                "lock_size_mismatch",
                format!("pack.lock.cbor size mismatch for `{path}`"),
            ));
        }
        let actual = sha256_hex(&bytes);
        if expected.sha256 != actual {
            return Err(SorxPackError::new(
                "lock_digest_mismatch",
                format!("pack.lock.cbor digest mismatch for `{path}`"),
            ));
        }
    }
    Ok(())
}

fn read_sorla_assets<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    entries: &BTreeSet<String>,
) -> Result<SorlaAssets, SorxPackError> {
    let model_cbor = zip_bytes(archive, "assets/sorla/model.cbor")?;
    let agent_gateway_json = parse_json(archive, "assets/sorla/agent-gateway.json")?;
    let openapi_overlay_yaml = optional_zip_text(
        archive,
        entries,
        "assets/sorla/agent-endpoints.openapi.overlay.yaml",
    )?;
    if let Some(yaml) = &openapi_overlay_yaml {
        parse_yaml("assets/sorla/agent-endpoints.openapi.overlay.yaml", yaml)?;
    }
    let arazzo_yaml =
        optional_zip_text(archive, entries, "assets/sorla/agent-workflows.arazzo.yaml")?;
    if let Some(yaml) = &arazzo_yaml {
        parse_yaml("assets/sorla/agent-workflows.arazzo.yaml", yaml)?;
    }
    let mcp_tools_json = if entries.contains("assets/sorla/mcp-tools.json") {
        Some(parse_json(archive, "assets/sorla/mcp-tools.json")?)
    } else {
        None
    };
    let llms_txt_fragment = optional_zip_text(archive, entries, "assets/sorla/llms.txt.fragment")?;

    Ok(SorlaAssets {
        model_cbor,
        agent_gateway_json,
        openapi_overlay_yaml,
        arazzo_yaml,
        mcp_tools_json,
        llms_txt_fragment,
    })
}

fn read_sorx_assets<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    entries: &BTreeSet<String>,
) -> Result<(SorxAssets, ValidationSuiteStatus, Vec<String>), SorxPackError> {
    let start_schema_json = parse_json(archive, "assets/sorx/start.schema.json")?;
    let start_questions_cbor =
        optional_zip_bytes(archive, entries, "assets/sorx/start.questions.cbor")?;
    let runtime_template_yaml =
        optional_zip_text(archive, entries, "assets/sorx/runtime.template.yaml")?;
    if let Some(yaml) = &runtime_template_yaml {
        parse_yaml("assets/sorx/runtime.template.yaml", yaml)?;
    }
    let provider_bindings_template_yaml = optional_zip_text(
        archive,
        entries,
        "assets/sorx/provider-bindings.template.yaml",
    )?;
    if let Some(yaml) = &provider_bindings_template_yaml {
        parse_yaml("assets/sorx/provider-bindings.template.yaml", yaml)?;
    }

    let validation_suite_cbor =
        optional_zip_bytes(archive, entries, "assets/sorx/validation-suite.cbor")?;
    let mut validation_errors = Vec::new();
    let validation_suite_json = if entries.contains("assets/sorx/validation-suite.json") {
        match parse_json(archive, "assets/sorx/validation-suite.json") {
            Ok(value) => {
                validation_errors.extend(validate_validation_suite_json(&value));
                Some(value)
            }
            Err(err) => {
                validation_errors.push(err.to_string());
                None
            }
        }
    } else {
        None
    };
    let validation_fixture_paths = entries
        .iter()
        .filter(|entry| entry.starts_with("assets/sorx/validation-fixtures/"))
        .cloned()
        .collect::<Vec<_>>();
    let mut validation_fixtures_json = BTreeMap::new();
    for path in validation_fixture_paths
        .iter()
        .filter(|path| path.ends_with(".json"))
    {
        match parse_json(archive, path) {
            Ok(value) => {
                validation_fixtures_json.insert(path.clone(), value);
            }
            Err(err) => validation_errors.push(err.to_string()),
        }
    }
    let validation_openapi_expected_json =
        if entries.contains("assets/sorx/validation-openapi.expected.json") {
            match parse_json(archive, "assets/sorx/validation-openapi.expected.json") {
                Ok(value) => Some(value),
                Err(err) => {
                    validation_errors.push(err.to_string());
                    None
                }
            }
        } else {
            None
        };

    let validation_suite_status = if !validation_errors.is_empty() {
        ValidationSuiteStatus::Invalid
    } else if validation_suite_cbor.is_some() || validation_suite_json.is_some() {
        ValidationSuiteStatus::Present
    } else {
        ValidationSuiteStatus::Missing
    };

    Ok((
        SorxAssets {
            start_schema_json,
            start_questions_cbor,
            runtime_template_yaml,
            provider_bindings_template_yaml,
            validation_suite_cbor,
            validation_suite_json,
            validation_fixture_paths,
            validation_fixtures_json,
            validation_openapi_expected_json,
        },
        validation_suite_status,
        validation_errors,
    ))
}

fn validate_validation_suite_json(value: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    if value.get("schema").and_then(Value::as_str) != Some("greentic.sorx.validation-suite.v1") {
        errors.push(
            "assets/sorx/validation-suite.json has unsupported or missing schema".to_string(),
        );
    }
    if !value.get("tests").is_some_and(Value::is_array) {
        errors.push("assets/sorx/validation-suite.json must contain a tests array".to_string());
    }
    if let Some(tests) = value.get("tests").and_then(Value::as_array) {
        let mut ids = BTreeSet::new();
        for (index, test) in tests.iter().enumerate() {
            let path = format!("assets/sorx/validation-suite.json tests[{index}]");
            let Some(id) = test.get("id").and_then(Value::as_str) else {
                errors.push(format!("{path} is missing id"));
                continue;
            };
            if !ids.insert(id) {
                errors.push(format!("duplicate validation test id `{id}`"));
            }
            if test.get("kind").and_then(Value::as_str).is_none() {
                errors.push(format!("validation test `{id}` is missing kind"));
            }
        }
    }
    errors
}

fn validate_mcp_tools(sorla_assets: &SorlaAssets) -> Vec<String> {
    let mut errors = Vec::new();
    let Some(mcp_tools) = &sorla_assets.mcp_tools_json else {
        return errors;
    };
    let endpoints = sorla_assets
        .agent_gateway_json
        .get("endpoints")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let endpoint_ids = endpoints
        .iter()
        .filter_map(|endpoint| {
            endpoint
                .get("endpoint_id")
                .or_else(|| endpoint.get("id"))
                .and_then(Value::as_str)
        })
        .collect::<BTreeSet<_>>();
    let operation_ids = endpoints
        .iter()
        .filter_map(|endpoint| {
            endpoint
                .get("operation_id")
                .or_else(|| endpoint.get("operationId"))
                .and_then(Value::as_str)
        })
        .collect::<BTreeSet<_>>();

    let Some(tools) = mcp_tools.get("tools").and_then(Value::as_array) else {
        errors.push("assets/sorla/mcp-tools.json must contain a tools array".to_string());
        return errors;
    };

    let mut names = BTreeSet::new();
    for (index, tool) in tools.iter().enumerate() {
        let path = format!("assets/sorla/mcp-tools.json tools[{index}]");
        let Some(name) = tool.get("name").and_then(Value::as_str) else {
            errors.push(format!("{path} is missing name"));
            continue;
        };
        if !names.insert(name) {
            errors.push(format!("duplicate MCP tool name `{name}`"));
        }
        let endpoint_id = tool
            .get("endpoint_id")
            .or_else(|| tool.get("endpointId"))
            .and_then(Value::as_str);
        let operation_id = tool
            .get("operation_id")
            .or_else(|| tool.get("operationId"))
            .and_then(Value::as_str);
        if endpoint_id.is_none() && operation_id.is_none() {
            errors.push(format!(
                "MCP tool `{name}` must reference endpoint_id or operation_id"
            ));
        }
        if let Some(endpoint_id) = endpoint_id
            && !endpoint_ids.contains(endpoint_id)
        {
            errors.push(format!(
                "MCP tool `{name}` references unknown endpoint `{endpoint_id}`"
            ));
        }
        if let Some(operation_id) = operation_id
            && !operation_ids.contains(operation_id)
        {
            errors.push(format!(
                "MCP tool `{name}` references unknown operation `{operation_id}`"
            ));
        }
        if let Some(input_schema) = tool.get("input_schema")
            && !input_schema.is_object()
        {
            errors.push(format!("MCP tool `{name}` input_schema must be an object"));
        }
    }
    errors
}

fn parse_json<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<Value, SorxPackError> {
    let text = zip_text(archive, name)?;
    serde_json::from_str(&text).map_err(|err| {
        SorxPackError::new("invalid_json", format!("`{name}` is invalid JSON: {err}"))
    })
}

fn parse_yaml(name: &str, text: &str) -> Result<Value, SorxPackError> {
    serde_yaml::from_str(text).map_err(|err| {
        SorxPackError::new("invalid_yaml", format!("`{name}` is invalid YAML: {err}"))
    })
}

fn secret_warnings(assets: &SorxAssets) -> Vec<String> {
    const MARKERS: &[&str] = &[
        "BEGIN PRIVATE KEY",
        "api_key:",
        "access_token:",
        "refresh_token:",
        "client_secret:",
        "password:",
    ];
    let mut warnings = Vec::new();
    for (name, value) in [
        (
            "assets/sorx/runtime.template.yaml",
            &assets.runtime_template_yaml,
        ),
        (
            "assets/sorx/provider-bindings.template.yaml",
            &assets.provider_bindings_template_yaml,
        ),
    ] {
        let Some(text) = value else {
            continue;
        };
        for marker in MARKERS {
            if text.contains(marker) {
                warnings.push(format!("`{name}` contains secret-like marker `{marker}`"));
            }
        }
    }
    warnings
}

fn sha256_file(path: &Path) -> Result<String, SorxPackError> {
    let bytes = fs::read(path).map_err(|err| {
        SorxPackError::new(
            "read_failed",
            format!("failed to read {}: {err}", path.display()),
        )
    })?;
    Ok(sha256_hex(&bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs::File;
    use std::io::Write;

    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    use super::*;
    use crate::doctor::doctor_sorla_pack;

    fn valid_entries() -> BTreeMap<String, Vec<u8>> {
        let extension = serde_json::json!({
            "extension": SORX_RUNTIME_EXTENSION_ID,
            "sorla": {
                "model": "assets/sorla/model.cbor",
                "agent_gateway": "assets/sorla/agent-gateway.json",
                "mcp_tools": "assets/sorla/mcp-tools.json"
            },
            "sorx": {
                "start_schema": "assets/sorx/start.schema.json",
                "runtime_template": "assets/sorx/runtime.template.yaml"
            }
        });
        let manifest = PackManifest {
            schema: "greentic.gtpack.manifest.sorla.v1".to_string(),
            pack: crate::manifest::PackIdentity {
                name: "landlord-tenant-sor".to_string(),
                version: "0.1.0".to_string(),
                kind: Some("application".to_string()),
            },
            extension,
            integrity: None,
            assets: vec![
                "assets/sorla/model.cbor".to_string(),
                "assets/sorla/agent-gateway.json".to_string(),
                "assets/sorla/mcp-tools.json".to_string(),
                "assets/sorx/start.schema.json".to_string(),
                "assets/sorx/runtime.template.yaml".to_string(),
            ],
        };
        let mut entries = BTreeMap::new();
        entries.insert("pack.cbor".to_string(), encode_cbor(&manifest));
        entries.insert("manifest.cbor".to_string(), encode_cbor(&manifest));
        entries.insert(
            "manifest.json".to_string(),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        );
        entries.insert(
            "assets/sorla/model.cbor".to_string(),
            vec![0xa1, 0x61, 0x78, 0x01],
        );
        entries.insert(
            "assets/sorla/agent-gateway.json".to_string(),
            br#"{"schema":"greentic.sorla.agent-gateway.v1","endpoints":[]}"#.to_vec(),
        );
        entries.insert(
            "assets/sorla/mcp-tools.json".to_string(),
            br#"{"schema":"greentic.sorla.mcp-tools.v1","tools":[]}"#.to_vec(),
        );
        entries.insert(
            "assets/sorx/start.schema.json".to_string(),
            br#"{"schema":"greentic.sorx.start.answers.v1","required":["tenant.tenant_id"]}"#
                .to_vec(),
        );
        entries.insert(
            "assets/sorx/runtime.template.yaml".to_string(),
            b"schema: greentic.sorx.runtime.template.v1\n".to_vec(),
        );
        let lock = lock_for_entries(&entries);
        entries.insert("pack.lock.cbor".to_string(), encode_cbor(&lock));
        entries
    }

    fn encode_cbor<T: serde::Serialize>(value: &T) -> Vec<u8> {
        let mut out = Vec::new();
        ciborium::ser::into_writer(value, &mut out).unwrap();
        out
    }

    fn lock_for_entries(entries: &BTreeMap<String, Vec<u8>>) -> PackLock {
        PackLock {
            schema: "greentic.gtpack.lock.sorla.v1".to_string(),
            entries: entries
                .iter()
                .filter(|(path, _)| path.as_str() != "pack.lock.cbor")
                .map(|(path, bytes)| {
                    (
                        path.clone(),
                        crate::manifest::PackLockEntry {
                            size: bytes.len() as u64,
                            sha256: sha256_hex(bytes),
                        },
                    )
                })
                .collect(),
        }
    }

    fn refresh_lock(entries: &mut BTreeMap<String, Vec<u8>>) {
        entries.remove("pack.lock.cbor");
        entries.insert(
            "pack.lock.cbor".to_string(),
            encode_cbor(&lock_for_entries(entries)),
        );
    }

    fn write_pack(entries: BTreeMap<String, Vec<u8>>) -> (TempDir, PathBuf) {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("pack.gtpack");
        let file = File::create(&path).unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .unix_permissions(0o644);
        for (name, bytes) in entries {
            writer.start_file(name, options).unwrap();
            writer.write_all(&bytes).unwrap();
        }
        writer.finish().unwrap();
        (temp, path)
    }

    #[test]
    fn valid_pack_passes_doctor() {
        let (_temp, path) = write_pack(valid_entries());
        let report = doctor_sorla_pack(&path);
        assert!(report.ok, "{report:?}");
    }

    #[test]
    fn missing_required_asset_fails_doctor() {
        let mut entries = valid_entries();
        entries.remove("assets/sorla/model.cbor");
        let (_temp, path) = write_pack(entries);
        let report = doctor_sorla_pack(&path);
        assert!(!report.ok);
        assert!(report.errors[0].message.contains("model.cbor"));
    }

    #[test]
    fn invalid_gateway_json_fails_doctor() {
        let mut entries = valid_entries();
        entries.insert(
            "assets/sorla/agent-gateway.json".to_string(),
            b"not-json".to_vec(),
        );
        refresh_lock(&mut entries);
        let (_temp, path) = write_pack(entries);
        let report = doctor_sorla_pack(&path);
        assert!(!report.ok);
        assert!(report.errors[0].message.contains("invalid JSON"));
    }

    #[test]
    fn inspect_output_is_stable() {
        let (_temp, path) = write_pack(valid_entries());
        let report = inspect_sorla_pack(&path).unwrap();
        let json = serde_json::to_string_pretty(&report).unwrap();
        assert!(json.contains("\"schema\": \"greentic.sorx.inspect.v1\""));
        assert!(json.contains("\"name\": \"landlord-tenant-sor\""));
        assert!(json.contains("\"has_mcp_tools\": true"));
    }

    #[test]
    fn path_traversal_entry_fails_doctor() {
        let mut entries = valid_entries();
        entries.insert("../escape".to_string(), b"bad".to_vec());
        let (_temp, path) = write_pack(entries);
        let report = doctor_sorla_pack(&path);
        assert!(!report.ok);
        assert_eq!(report.errors[0].code, "unsafe_path");
    }

    #[test]
    fn validation_suite_status_is_present_when_manifest_exists() {
        let mut entries = valid_entries();
        entries.insert(
            "assets/sorx/validation-suite.json".to_string(),
            br#"{"schema":"greentic.sorx.validation-suite.v1","tests":[]}"#.to_vec(),
        );
        refresh_lock(&mut entries);
        let (_temp, path) = write_pack(entries);
        let report = inspect_sorla_pack(&path).unwrap();
        assert_eq!(report.sorx.validation_suite_status, "present");
        assert!(report.sorx.has_validation_suite_json);
    }

    #[test]
    fn validation_suite_status_is_invalid_when_manifest_is_broken() {
        let mut entries = valid_entries();
        entries.insert(
            "assets/sorx/validation-suite.json".to_string(),
            b"not-json".to_vec(),
        );
        refresh_lock(&mut entries);
        let (_temp, path) = write_pack(entries);
        let report = inspect_sorla_pack(&path).unwrap();
        assert_eq!(report.sorx.validation_suite_status, "invalid");
        let doctor = doctor_sorla_pack(&path);
        assert!(!doctor.ok);
        assert!(doctor.errors[0].message.contains("validation-suite.json"));
    }

    #[test]
    fn validation_suite_status_is_invalid_when_schema_is_wrong() {
        let mut entries = valid_entries();
        entries.insert(
            "assets/sorx/validation-suite.json".to_string(),
            br#"{"schema":"wrong","tests":[{"id":"doctor","kind":"doctor"}]}"#.to_vec(),
        );
        refresh_lock(&mut entries);
        let (_temp, path) = write_pack(entries);
        let report = inspect_sorla_pack(&path).unwrap();
        assert_eq!(report.sorx.validation_suite_status, "invalid");
        let doctor = doctor_sorla_pack(&path);
        assert!(!doctor.ok);
        assert!(
            doctor.errors[0]
                .message
                .contains("unsupported or missing schema")
        );
    }

    #[test]
    fn missing_optional_mcp_tools_is_allowed_when_not_referenced() {
        let mut entries = valid_entries();
        entries.remove("assets/sorla/mcp-tools.json");
        let mut manifest: PackManifest =
            ciborium::de::from_reader(Cursor::new(entries.get("pack.cbor").unwrap().clone()))
                .unwrap();
        manifest
            .assets
            .retain(|path| path != "assets/sorla/mcp-tools.json");
        manifest
            .extension
            .get_mut("sorla")
            .and_then(Value::as_object_mut)
            .unwrap()
            .remove("mcp_tools");
        entries.insert("pack.cbor".to_string(), encode_cbor(&manifest));
        entries.insert("manifest.cbor".to_string(), encode_cbor(&manifest));
        entries.insert(
            "manifest.json".to_string(),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        );
        refresh_lock(&mut entries);
        let (_temp, path) = write_pack(entries);
        let report = doctor_sorla_pack(&path);
        assert!(report.ok, "{report:?}");
    }

    #[test]
    fn duplicate_mcp_tool_name_fails_doctor() {
        let mut entries = valid_entries();
        entries.insert(
            "assets/sorla/agent-gateway.json".to_string(),
            br#"{"schema":"greentic.sorla.agent-gateway.v1","endpoints":[{"endpoint_id":"tenant.create","operation_id":"tenant.create"}]}"#.to_vec(),
        );
        entries.insert(
            "assets/sorla/mcp-tools.json".to_string(),
            br#"{"schema":"greentic.sorla.mcp-tools.v1","tools":[{"name":"create","endpoint_id":"tenant.create"},{"name":"create","endpoint_id":"tenant.create"}]}"#.to_vec(),
        );
        refresh_lock(&mut entries);
        let (_temp, path) = write_pack(entries);
        let report = doctor_sorla_pack(&path);
        assert!(!report.ok);
        assert!(report.errors[0].message.contains("duplicate MCP tool"));
    }

    #[test]
    fn invalid_mcp_tool_reference_fails_doctor() {
        let mut entries = valid_entries();
        entries.insert(
            "assets/sorla/agent-gateway.json".to_string(),
            br#"{"schema":"greentic.sorla.agent-gateway.v1","endpoints":[{"endpoint_id":"tenant.create","operation_id":"tenant.create"}]}"#.to_vec(),
        );
        entries.insert(
            "assets/sorla/mcp-tools.json".to_string(),
            br#"{"schema":"greentic.sorla.mcp-tools.v1","tools":[{"name":"missing","endpoint_id":"tenant.missing"}]}"#.to_vec(),
        );
        refresh_lock(&mut entries);
        let (_temp, path) = write_pack(entries);
        let report = doctor_sorla_pack(&path);
        assert!(!report.ok);
        assert!(report.errors[0].message.contains("unknown endpoint"));
    }

    #[test]
    fn future_integrity_fields_are_parsed_from_manifest() {
        let mut entries = valid_entries();
        let mut manifest: PackManifest =
            ciborium::de::from_reader(Cursor::new(entries.get("pack.cbor").unwrap().clone()))
                .unwrap();
        manifest.integrity = Some(crate::manifest::PackIntegrity {
            digest: Some("sha256:abc123".to_string()),
            signature: None,
            signature_ref: Some("sigstore:bundle-ref".to_string()),
        });
        entries.insert("pack.cbor".to_string(), encode_cbor(&manifest));
        refresh_lock(&mut entries);
        let (_temp, path) = write_pack(entries);
        let pack = load_sorla_pack(&path).unwrap();
        let integrity = pack.manifest.integrity.unwrap();
        assert_eq!(integrity.digest.as_deref(), Some("sha256:abc123"));
        assert_eq!(
            integrity.signature_ref.as_deref(),
            Some("sigstore:bundle-ref")
        );
    }
}
