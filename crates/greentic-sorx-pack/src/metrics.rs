use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricCatalog {
    pub schema: String,
    pub package: MetricPackage,
    #[serde(default)]
    pub metrics: Vec<MetricDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricPackage {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricDefinition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<MetricSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measure: Option<MetricMeasure>,
    #[serde(default)]
    pub dimensions: Vec<MetricDimension>,
    #[serde(default)]
    pub filters: Vec<MetricFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<MetricTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<MetricWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<MetricTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<MetricFormula>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<MetricCache>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricSource {
    pub entity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricMeasure {
    pub aggregate: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricDimension {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub sensitive: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricFilter {
    pub field: String,
    pub operator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricTime {
    pub field: String,
    #[serde(default)]
    pub grains: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricWindow {
    pub grain: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricTarget {
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricFormula {
    pub expression: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricCache {
    pub ttl_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricAssets {
    pub catalog_json: Value,
    pub catalog: MetricCatalog,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MetricInspectSummary {
    pub present: bool,
    pub count: usize,
    pub names: Vec<String>,
    pub required_capabilities: Vec<String>,
}

impl MetricAssets {
    pub fn inspect_summary(&self) -> MetricInspectSummary {
        let names = self
            .catalog
            .metrics
            .iter()
            .map(|metric| metric.name.clone())
            .collect::<Vec<_>>();
        MetricInspectSummary {
            present: true,
            count: self.catalog.metrics.len(),
            names,
            required_capabilities: vec!["metrics.query".to_string()],
        }
    }
}

impl MetricInspectSummary {
    pub fn missing() -> Self {
        Self {
            present: false,
            count: 0,
            names: Vec::new(),
            required_capabilities: Vec::new(),
        }
    }
}

pub fn validate_metrics(catalog: &MetricCatalog) -> Vec<String> {
    let mut errors = Vec::new();
    if catalog.schema != "greentic.sorla.metrics.v1" {
        errors.push(format!(
            "assets/sorla/metrics.json has unsupported schema `{}`",
            catalog.schema
        ));
    }
    if catalog.package.name.trim().is_empty() || catalog.package.version.trim().is_empty() {
        errors.push("assets/sorla/metrics.json package name and version are required".to_string());
    }

    let mut names = BTreeSet::new();
    let mut by_name = BTreeMap::new();
    for metric in &catalog.metrics {
        if metric.name.trim().is_empty() {
            errors.push("assets/sorla/metrics.json metric name is required".to_string());
            continue;
        }
        if !names.insert(metric.name.as_str()) {
            errors.push(format!(
                "assets/sorla/metrics.json repeats metric `{}`",
                metric.name
            ));
        }
        by_name.insert(metric.name.as_str(), metric);
        validate_metric(metric, &mut errors);
    }

    for metric in &catalog.metrics {
        if let Some(formula) = &metric.formula {
            for dependency in &formula.dependencies {
                if !by_name.contains_key(dependency.as_str()) {
                    errors.push(format!(
                        "metric `{}` formula dependency `{dependency}` does not exist",
                        metric.name
                    ));
                }
            }
        }
    }
    for metric in &catalog.metrics {
        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        if has_cycle(metric.name.as_str(), &by_name, &mut visiting, &mut visited) {
            errors.push(format!(
                "metric `{}` formula dependencies contain a cycle",
                metric.name
            ));
        }
    }

    errors
}

fn validate_metric(metric: &MetricDefinition, errors: &mut Vec<String>) {
    let has_measure = metric.measure.is_some();
    let has_formula = metric.formula.is_some();
    if has_measure == has_formula {
        errors.push(format!(
            "metric `{}` must define exactly one of measure or formula",
            metric.name
        ));
    }
    if has_measure && metric.source.is_none() {
        errors.push(format!("metric `{}` source is required", metric.name));
    }
    if let Some(measure) = &metric.measure
        && !supported_aggregate(&measure.aggregate)
    {
        errors.push(format!(
            "metric `{}` uses unsupported aggregate `{}`",
            metric.name, measure.aggregate
        ));
    }
    if let Some(time) = &metric.time {
        for grain in &time.grains {
            if !supported_grain(grain) {
                errors.push(format!(
                    "metric `{}` uses unsupported time grain `{grain}`",
                    metric.name
                ));
            }
        }
    }
    if let Some(window) = &metric.window
        && !supported_grain(&window.grain)
    {
        errors.push(format!(
            "metric `{}` uses unsupported window grain `{}`",
            metric.name, window.grain
        ));
    }
    if let Some(formula) = &metric.formula
        && formula.expression.trim().is_empty()
    {
        errors.push(format!(
            "metric `{}` formula expression is required",
            metric.name
        ));
    }
}

fn has_cycle<'a>(
    name: &'a str,
    by_name: &BTreeMap<&'a str, &'a MetricDefinition>,
    visiting: &mut BTreeSet<&'a str>,
    visited: &mut BTreeSet<&'a str>,
) -> bool {
    if visited.contains(name) {
        return false;
    }
    if !visiting.insert(name) {
        return true;
    }
    if let Some(metric) = by_name.get(name)
        && let Some(formula) = &metric.formula
    {
        for dependency in &formula.dependencies {
            if has_cycle(dependency, by_name, visiting, visited) {
                return true;
            }
        }
    }
    visiting.remove(name);
    visited.insert(name);
    false
}

fn supported_aggregate(value: &str) -> bool {
    matches!(
        value,
        "count" | "sum" | "avg" | "min" | "max" | "distinct_count"
    )
}

fn supported_grain(value: &str) -> bool {
    matches!(value, "minute" | "hour" | "day" | "week" | "month")
}

fn is_false(value: &bool) -> bool {
    !*value
}
