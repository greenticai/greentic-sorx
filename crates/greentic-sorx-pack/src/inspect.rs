use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SorxInspectReport {
    pub schema: String,
    pub pack: SorxInspectPack,
    pub sorla: SorxInspectSorla,
    pub sorx: SorxInspectSorx,
    pub ontology: SorxInspectOntology,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SorxInspectPack {
    pub name: String,
    pub version: String,
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SorxInspectSorla {
    pub has_model: bool,
    pub has_agent_gateway: bool,
    pub has_openapi_overlay: bool,
    pub has_arazzo: bool,
    pub has_mcp_tools: bool,
    pub has_llms_fragment: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SorxInspectSorx {
    pub has_start_schema: bool,
    pub has_runtime_template: bool,
    pub has_provider_bindings_template: bool,
    pub validation_suite_status: String,
    pub has_validation_suite_cbor: bool,
    pub has_validation_suite_json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SorxInspectOntology {
    pub present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub concept_count: usize,
    pub relationship_count: usize,
    pub retrieval_bindings_present: bool,
}
