use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ProviderNamespace, SorxError, SorxResult};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeMetricCatalog {
    pub metrics: Vec<RuntimeMetric>,
}

impl RuntimeMetricCatalog {
    pub fn new(metrics: Vec<RuntimeMetric>) -> SorxResult<Self> {
        let catalog = Self { metrics };
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn metric(&self, name: &str) -> SorxResult<&RuntimeMetric> {
        self.metrics
            .iter()
            .find(|metric| metric.name == name)
            .ok_or_else(|| {
                SorxError::new("metric_missing", format!("metric `{name}` does not exist"))
            })
    }

    pub fn validate(&self) -> SorxResult<()> {
        let mut names = BTreeSet::new();
        for metric in &self.metrics {
            if !names.insert(metric.name.as_str()) {
                return Err(SorxError::new(
                    "metric_duplicate",
                    format!("metric `{}` is defined more than once", metric.name),
                ));
            }
        }
        for metric in &self.metrics {
            if let RuntimeMetricKind::Formula { dependencies, .. } = &metric.kind {
                for dependency in dependencies {
                    if !names.contains(dependency.as_str()) {
                        return Err(SorxError::new(
                            "metric_dependency_missing",
                            format!(
                                "metric `{}` depends on missing metric `{dependency}`",
                                metric.name
                            ),
                        ));
                    }
                }
            }
        }
        for metric in &self.metrics {
            let mut visiting = BTreeSet::new();
            let mut visited = BTreeSet::new();
            if self.has_cycle(&metric.name, &mut visiting, &mut visited) {
                return Err(SorxError::new(
                    "metric_dependency_cycle",
                    format!("metric `{}` has cyclic dependencies", metric.name),
                ));
            }
        }
        Ok(())
    }

    fn has_cycle<'a>(
        &'a self,
        name: &'a str,
        visiting: &mut BTreeSet<&'a str>,
        visited: &mut BTreeSet<&'a str>,
    ) -> bool {
        if visited.contains(name) {
            return false;
        }
        if !visiting.insert(name) {
            return true;
        }
        if let Ok(metric) = self.metric(name)
            && let RuntimeMetricKind::Formula { dependencies, .. } = &metric.kind
        {
            for dependency in dependencies {
                if self.has_cycle(dependency, visiting, visited) {
                    return true;
                }
            }
        }
        visiting.remove(name);
        visited.insert(name);
        false
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeMetric {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub kind: RuntimeMetricKind,
    #[serde(default)]
    pub dimensions: Vec<RuntimeMetricDimension>,
    #[serde(default)]
    pub filters: Vec<MetricQueryFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<RuntimeMetricCache>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeMetricKind {
    Aggregate {
        source_entity: String,
        collection: String,
        aggregate: MetricAggregate,
        field: Option<String>,
    },
    Formula {
        expression: String,
        dependencies: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricAggregate {
    Count,
    Sum,
    Avg,
    Min,
    Max,
    DistinctCount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeMetricDimension {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub sensitive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeMetricCache {
    pub ttl_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricQuery {
    pub namespace: ProviderNamespace,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain: Option<String>,
    #[serde(default)]
    pub dimensions: Vec<String>,
    #[serde(default)]
    pub filters: Vec<MetricQueryFilter>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricQueryFilter {
    pub field: String,
    pub operator: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricQueryResult {
    pub metric: String,
    pub rows: Vec<MetricResultRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricResultRow {
    #[serde(default)]
    pub dimensions: BTreeMap<String, Value>,
    pub value: f64,
}

pub trait MetricRuntimeProvider {
    fn query_metric(
        &self,
        definition: &RuntimeMetric,
        query: &MetricQuery,
    ) -> SorxResult<MetricQueryResult>;
}

pub struct MetricRuntime<'a> {
    catalog: RuntimeMetricCatalog,
    provider: &'a dyn MetricRuntimeProvider,
}

impl<'a> MetricRuntime<'a> {
    pub fn new(catalog: RuntimeMetricCatalog, provider: &'a dyn MetricRuntimeProvider) -> Self {
        Self { catalog, provider }
    }

    pub fn catalog(&self) -> &RuntimeMetricCatalog {
        &self.catalog
    }

    pub fn query(&self, metric_name: &str, query: MetricQuery) -> SorxResult<MetricQueryResult> {
        let metric = self.catalog.metric(metric_name)?;
        self.query_definition(metric, &query, &mut BTreeSet::new())
    }

    fn query_definition(
        &self,
        metric: &RuntimeMetric,
        query: &MetricQuery,
        stack: &mut BTreeSet<String>,
    ) -> SorxResult<MetricQueryResult> {
        if !stack.insert(metric.name.clone()) {
            return Err(SorxError::new(
                "metric_dependency_cycle",
                format!("metric `{}` has cyclic dependencies", metric.name),
            ));
        }
        let result = match &metric.kind {
            RuntimeMetricKind::Aggregate { .. } => self.provider.query_metric(metric, query),
            RuntimeMetricKind::Formula {
                expression,
                dependencies,
            } => {
                let mut values = BTreeMap::new();
                for dependency in dependencies {
                    let dependency_metric = self.catalog.metric(dependency)?;
                    let result = self.query_definition(dependency_metric, query, stack)?;
                    let value = result.rows.first().map(|row| row.value).ok_or_else(|| {
                        SorxError::new(
                            "metric_dependency_empty",
                            format!("metric `{dependency}` returned no rows"),
                        )
                    })?;
                    values.insert(dependency.clone(), value);
                }
                let value = evaluate_formula(expression, &values)?;
                Ok(MetricQueryResult {
                    metric: metric.name.clone(),
                    rows: vec![MetricResultRow {
                        dimensions: BTreeMap::new(),
                        value,
                    }],
                })
            }
        };
        stack.remove(&metric.name);
        result
    }
}

fn evaluate_formula(expression: &str, values: &BTreeMap<String, f64>) -> SorxResult<f64> {
    let mut parser = FormulaParser {
        chars: expression.chars().collect(),
        index: 0,
        values,
    };
    let value = parser.expression()?;
    parser.skip_ws();
    if parser.index != parser.chars.len() {
        return Err(SorxError::new(
            "metric_formula_unsupported",
            format!("unsupported formula token near `{}`", parser.remaining()),
        ));
    }
    Ok(value)
}

struct FormulaParser<'a> {
    chars: Vec<char>,
    index: usize,
    values: &'a BTreeMap<String, f64>,
}

impl FormulaParser<'_> {
    fn expression(&mut self) -> SorxResult<f64> {
        let mut value = self.term()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('+') => {
                    self.index += 1;
                    value += self.term()?;
                }
                Some('-') => {
                    self.index += 1;
                    value -= self.term()?;
                }
                _ => return Ok(value),
            }
        }
    }

    fn term(&mut self) -> SorxResult<f64> {
        let mut value = self.factor()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('*') => {
                    self.index += 1;
                    value *= self.factor()?;
                }
                Some('/') => {
                    self.index += 1;
                    let divisor = self.factor()?;
                    if divisor == 0.0 {
                        return Err(SorxError::new(
                            "metric_formula_divide_by_zero",
                            "metric formula attempted division by zero",
                        ));
                    }
                    value /= divisor;
                }
                _ => return Ok(value),
            }
        }
    }

    fn factor(&mut self) -> SorxResult<f64> {
        self.skip_ws();
        match self.peek() {
            Some('(') => {
                self.index += 1;
                let value = self.expression()?;
                self.skip_ws();
                if self.peek() != Some(')') {
                    return Err(SorxError::new(
                        "metric_formula_unsupported",
                        "metric formula has an unclosed parenthesis",
                    ));
                }
                self.index += 1;
                Ok(value)
            }
            Some(char) if char.is_ascii_digit() => self.number(),
            Some(char) if char.is_ascii_alphabetic() || char == '_' => self.identifier(),
            _ => Err(SorxError::new(
                "metric_formula_unsupported",
                format!("unsupported formula token near `{}`", self.remaining()),
            )),
        }
    }

    fn number(&mut self) -> SorxResult<f64> {
        let start = self.index;
        while self
            .peek()
            .is_some_and(|char| char.is_ascii_digit() || char == '.')
        {
            self.index += 1;
        }
        self.chars[start..self.index]
            .iter()
            .collect::<String>()
            .parse::<f64>()
            .map_err(|err| SorxError::new("metric_formula_unsupported", err.to_string()))
    }

    fn identifier(&mut self) -> SorxResult<f64> {
        let start = self.index;
        while self
            .peek()
            .is_some_and(|char| char.is_ascii_alphanumeric() || char == '_' || char == '-')
        {
            self.index += 1;
        }
        let name = self.chars[start..self.index].iter().collect::<String>();
        self.values.get(&name).copied().ok_or_else(|| {
            SorxError::new(
                "metric_formula_dependency_missing",
                format!("formula references unknown dependency `{name}`"),
            )
        })
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.index += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.index).copied()
    }

    fn remaining(&self) -> String {
        self.chars[self.index..].iter().collect()
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[derive(Default)]
    struct FakeMetricProvider {
        values: HashMap<String, f64>,
    }

    impl MetricRuntimeProvider for FakeMetricProvider {
        fn query_metric(
            &self,
            definition: &RuntimeMetric,
            _query: &MetricQuery,
        ) -> SorxResult<MetricQueryResult> {
            let value = self.values.get(&definition.name).copied().ok_or_else(|| {
                SorxError::new(
                    "metric_unsupported",
                    format!(
                        "metric `{}` is not supported by fake provider",
                        definition.name
                    ),
                )
            })?;
            Ok(MetricQueryResult {
                metric: definition.name.clone(),
                rows: vec![MetricResultRow {
                    dimensions: BTreeMap::new(),
                    value,
                }],
            })
        }
    }

    #[test]
    fn aggregate_metric_delegates_to_provider() {
        let provider = FakeMetricProvider {
            values: HashMap::from([("daily_clicks".to_string(), 42.0)]),
        };
        let runtime = MetricRuntime::new(catalog().unwrap(), &provider);
        let result = runtime.query("daily_clicks", query()).unwrap();
        assert_eq!(result.metric, "daily_clicks");
        assert_eq!(result.rows[0].value, 42.0);
    }

    #[test]
    fn formula_metric_resolves_dependencies() {
        let provider = FakeMetricProvider {
            values: HashMap::from([
                ("monthly_revenue".to_string(), 100.0),
                ("monthly_cost".to_string(), 35.0),
            ]),
        };
        let runtime = MetricRuntime::new(catalog().unwrap(), &provider);
        let result = runtime.query("gross_margin", query()).unwrap();
        assert_eq!(result.rows[0].value, 65.0);
    }

    #[test]
    fn unsupported_formula_returns_clear_error() {
        let err =
            evaluate_formula("a && b", &BTreeMap::from([("a".to_string(), 1.0)])).unwrap_err();
        assert_eq!(err.code, "metric_formula_unsupported");
    }

    #[test]
    fn catalog_rejects_dependency_cycles() {
        let err = RuntimeMetricCatalog::new(vec![
            formula("a", "b + 1", &["b"]),
            formula("b", "a + 1", &["a"]),
        ])
        .unwrap_err();
        assert_eq!(err.code, "metric_dependency_cycle");
    }

    fn catalog() -> SorxResult<RuntimeMetricCatalog> {
        RuntimeMetricCatalog::new(vec![
            aggregate("daily_clicks", MetricAggregate::Count, None),
            aggregate("monthly_revenue", MetricAggregate::Sum, Some("amount")),
            aggregate("monthly_cost", MetricAggregate::Sum, Some("amount")),
            formula(
                "gross_margin",
                "monthly_revenue - monthly_cost",
                &["monthly_revenue", "monthly_cost"],
            ),
        ])
    }

    fn aggregate(name: &str, aggregate: MetricAggregate, field: Option<&str>) -> RuntimeMetric {
        RuntimeMetric {
            name: name.to_string(),
            label: None,
            kind: RuntimeMetricKind::Aggregate {
                source_entity: "Event".to_string(),
                collection: "events".to_string(),
                aggregate,
                field: field.map(ToString::to_string),
            },
            dimensions: Vec::new(),
            filters: Vec::new(),
            cache: None,
        }
    }

    fn formula(name: &str, expression: &str, dependencies: &[&str]) -> RuntimeMetric {
        RuntimeMetric {
            name: name.to_string(),
            label: None,
            kind: RuntimeMetricKind::Formula {
                expression: expression.to_string(),
                dependencies: dependencies
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
            },
            dimensions: Vec::new(),
            filters: Vec::new(),
            cache: None,
        }
    }

    fn query() -> MetricQuery {
        MetricQuery {
            namespace: ProviderNamespace {
                tenant_id: "tenant-a".to_string(),
                sor_name: "commerce".to_string(),
            },
            from: None,
            to: None,
            grain: None,
            dimensions: Vec::new(),
            filters: Vec::new(),
        }
    }
}
