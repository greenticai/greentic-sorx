use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const ONTOLOGY_GRAPH_SCHEMA_V1: &str = "greentic.sorla.ontology.graph.v1";
pub const RETRIEVAL_BINDINGS_SCHEMA_V1: &str = "greentic.sorla.retrieval-bindings.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OntologyGraph {
    pub schema: String,
    #[serde(default)]
    pub concepts: Vec<OntologyConcept>,
    #[serde(default)]
    pub relationships: Vec<OntologyRelationship>,
    #[serde(default)]
    pub records: Vec<OntologyRecordRef>,
    #[serde(default)]
    pub ir_sha256: Option<String>,
    #[serde(default)]
    pub ontology_ir_sha256: Option<String>,
    #[serde(default)]
    pub ir_hash: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OntologyConcept {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub records: Vec<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OntologyRelationship {
    pub id: String,
    #[serde(default, alias = "source")]
    pub from: Option<String>,
    #[serde(default, alias = "target")]
    pub to: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OntologyRecordRef {
    pub id: String,
    #[serde(alias = "concept")]
    pub concept_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalBindings {
    pub schema: String,
    #[serde(default)]
    pub bindings: Vec<RetrievalBinding>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalBinding {
    pub id: String,
    #[serde(default)]
    pub concept_id: Option<String>,
    #[serde(default)]
    pub relationship_id: Option<String>,
    #[serde(default)]
    pub scope: Option<RetrievalBindingScope>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalBindingScope {
    #[serde(default)]
    pub concepts: Vec<String>,
    #[serde(default)]
    pub relationships: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OntologyAssets {
    pub graph_json: Value,
    pub graph: OntologyGraph,
    pub ir_cbor: Option<Vec<u8>>,
    pub retrieval_bindings_json: Option<Value>,
    pub retrieval_bindings: Option<RetrievalBindings>,
}

impl OntologyGraph {
    pub fn ir_hash_expectation(&self) -> Option<&str> {
        self.ir_sha256
            .as_deref()
            .or(self.ontology_ir_sha256.as_deref())
            .or(self.ir_hash.as_deref())
    }
}

pub fn validate_ontology_assets(assets: &OntologyAssets) -> Vec<String> {
    let mut errors = Vec::new();
    validate_graph(&assets.graph, &mut errors);
    if let Some(bindings) = &assets.retrieval_bindings {
        validate_retrieval_bindings(bindings, &assets.graph, &mut errors);
    }
    if let Some(ir_cbor) = &assets.ir_cbor
        && let Some(expected) = assets.graph.ir_hash_expectation()
    {
        let actual = hex::encode(Sha256::digest(ir_cbor));
        let expected = expected.strip_prefix("sha256:").unwrap_or(expected);
        if expected != actual {
            errors
                .push("ontology IR hash does not match assets/sorla/ontology.ir.cbor".to_string());
        }
    }
    collect_unsafe_values(
        "assets/sorla/ontology.graph.json",
        &assets.graph_json,
        &mut errors,
    );
    if let Some(value) = &assets.retrieval_bindings_json {
        collect_unsafe_values("assets/sorla/retrieval-bindings.json", value, &mut errors);
    }
    errors
}

fn validate_graph(graph: &OntologyGraph, errors: &mut Vec<String>) {
    if graph.schema != ONTOLOGY_GRAPH_SCHEMA_V1 {
        errors.push(format!(
            "assets/sorla/ontology.graph.json has unsupported schema `{}`",
            graph.schema
        ));
    }
    let concept_ids = collect_unique_ids(
        "ontology concept",
        graph.concepts.iter().map(|concept| concept.id.as_str()),
        errors,
    );
    let _relationship_ids = collect_unique_ids(
        "ontology relationship",
        graph
            .relationships
            .iter()
            .map(|relationship| relationship.id.as_str()),
        errors,
    );
    for relationship in &graph.relationships {
        match relationship.from.as_deref() {
            Some(from) if concept_ids.contains(from) => {}
            Some(from) => errors.push(format!(
                "ontology relationship `{}` references unknown from concept `{from}`",
                relationship.id
            )),
            None => errors.push(format!(
                "ontology relationship `{}` is missing from/source concept",
                relationship.id
            )),
        }
        match relationship.to.as_deref() {
            Some(to) if concept_ids.contains(to) => {}
            Some(to) => errors.push(format!(
                "ontology relationship `{}` references unknown to concept `{to}`",
                relationship.id
            )),
            None => errors.push(format!(
                "ontology relationship `{}` is missing to/target concept",
                relationship.id
            )),
        }
    }
    for record in &graph.records {
        if !concept_ids.contains(record.concept_id.as_str()) {
            errors.push(format!(
                "ontology record `{}` references unknown concept `{}`",
                record.id, record.concept_id
            ));
        }
    }
}

fn validate_retrieval_bindings(
    bindings: &RetrievalBindings,
    graph: &OntologyGraph,
    errors: &mut Vec<String>,
) {
    if bindings.schema != RETRIEVAL_BINDINGS_SCHEMA_V1 {
        errors.push(format!(
            "assets/sorla/retrieval-bindings.json has unsupported schema `{}`",
            bindings.schema
        ));
    }
    let concept_ids = graph
        .concepts
        .iter()
        .map(|concept| concept.id.as_str())
        .collect::<BTreeSet<_>>();
    let relationship_ids = graph
        .relationships
        .iter()
        .map(|relationship| relationship.id.as_str())
        .collect::<BTreeSet<_>>();
    collect_unique_ids(
        "retrieval binding",
        bindings.bindings.iter().map(|binding| binding.id.as_str()),
        errors,
    );
    for binding in &bindings.bindings {
        if let Some(concept_id) = binding.concept_id.as_deref()
            && !concept_ids.contains(concept_id)
        {
            errors.push(format!(
                "retrieval binding `{}` references unknown concept `{concept_id}`",
                binding.id
            ));
        }
        if let Some(relationship_id) = binding.relationship_id.as_deref()
            && !relationship_ids.contains(relationship_id)
        {
            errors.push(format!(
                "retrieval binding `{}` references unknown relationship `{relationship_id}`",
                binding.id
            ));
        }
        if let Some(scope) = &binding.scope {
            for concept_id in &scope.concepts {
                if !concept_ids.contains(concept_id.as_str()) {
                    errors.push(format!(
                        "retrieval binding `{}` scope references unknown concept `{concept_id}`",
                        binding.id
                    ));
                }
            }
            for relationship_id in &scope.relationships {
                if !relationship_ids.contains(relationship_id.as_str()) {
                    errors.push(format!(
                        "retrieval binding `{}` scope references unknown relationship `{relationship_id}`",
                        binding.id
                    ));
                }
            }
        }
    }
}

fn collect_unique_ids<'a>(
    label: &str,
    ids: impl Iterator<Item = &'a str>,
    errors: &mut Vec<String>,
) -> BTreeSet<&'a str> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if id.is_empty() {
            errors.push(format!("{label} id must not be empty"));
            continue;
        }
        if !seen.insert(id) {
            errors.push(format!("duplicate {label} id `{id}`"));
        }
    }
    seen
}

fn collect_unsafe_values(path: &str, value: &Value, errors: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            if is_secret_like(text) {
                errors.push(format!("`{path}` contains secret-like value"));
            }
            if is_absolute_local_path(text) {
                errors.push(format!("`{path}` contains absolute local path `{text}`"));
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_unsafe_values(path, value, errors);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_unsafe_values(path, value, errors);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn is_secret_like(text: &str) -> bool {
    const MARKERS: &[&str] = &[
        "BEGIN PRIVATE KEY",
        "api_key:",
        "access_token:",
        "refresh_token:",
        "client_secret:",
        "password:",
    ];
    MARKERS.iter().any(|marker| text.contains(marker))
}

fn is_absolute_local_path(text: &str) -> bool {
    text.starts_with('/')
        || text.starts_with("file:///")
        || (text.len() > 2
            && text.as_bytes()[1] == b':'
            && text.as_bytes()[2] == b'\\'
            && text.as_bytes()[0].is_ascii_alphabetic())
}
