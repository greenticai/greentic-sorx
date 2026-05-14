use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{SorxError, SorxResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OntologyConceptNode {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OntologyRelationshipEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypePath {
    pub concepts: Vec<String>,
    pub relationships: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OntologyGraphService {
    concepts: BTreeMap<String, OntologyConceptNode>,
    relationships: BTreeMap<String, OntologyRelationshipEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WalkFrame {
    current: String,
    remaining_depth: u8,
    concepts: Vec<String>,
    relationships: Vec<String>,
}

impl OntologyGraphService {
    pub fn new(
        concepts: Vec<OntologyConceptNode>,
        relationships: Vec<OntologyRelationshipEdge>,
    ) -> SorxResult<Self> {
        let concepts = concepts
            .into_iter()
            .map(|concept| (concept.id.clone(), concept))
            .collect::<BTreeMap<_, _>>();
        let relationships = relationships
            .into_iter()
            .map(|relationship| (relationship.id.clone(), relationship))
            .collect::<BTreeMap<_, _>>();
        for relationship in relationships.values() {
            if !concepts.contains_key(&relationship.from) {
                return Err(SorxError::new(
                    "ontology_concept_missing",
                    format!(
                        "relationship `{}` references unknown from concept `{}`",
                        relationship.id, relationship.from
                    ),
                ));
            }
            if !concepts.contains_key(&relationship.to) {
                return Err(SorxError::new(
                    "ontology_concept_missing",
                    format!(
                        "relationship `{}` references unknown to concept `{}`",
                        relationship.id, relationship.to
                    ),
                ));
            }
        }
        Ok(Self {
            concepts,
            relationships,
        })
    }

    pub fn concepts(&self) -> Vec<OntologyConceptNode> {
        self.concepts.values().cloned().collect()
    }

    pub fn relationships(&self) -> Vec<OntologyRelationshipEdge> {
        self.relationships.values().cloned().collect()
    }

    pub fn concept(&self, id: &str) -> Option<OntologyConceptNode> {
        self.concepts.get(id).cloned()
    }

    pub fn relationships_from(&self, concept: &str) -> Vec<OntologyRelationshipEdge> {
        self.relationships
            .values()
            .filter(|relationship| relationship.from == concept)
            .cloned()
            .collect()
    }

    pub fn relationships_to(&self, concept: &str) -> Vec<OntologyRelationshipEdge> {
        self.relationships
            .values()
            .filter(|relationship| relationship.to == concept)
            .cloned()
            .collect()
    }

    pub fn find_type_paths(
        &self,
        from: &str,
        to: &str,
        max_depth: u8,
    ) -> SorxResult<Vec<TypePath>> {
        self.require_concept(from)?;
        self.require_concept(to)?;
        if max_depth == 0 {
            return Ok(Vec::new());
        }
        let mut paths = Vec::new();
        let mut visited = BTreeSet::from([from.to_string()]);
        self.walk(
            to,
            &mut visited,
            &mut paths,
            WalkFrame {
                current: from.to_string(),
                remaining_depth: max_depth,
                concepts: vec![from.to_string()],
                relationships: Vec::new(),
            },
        );
        paths.sort_by(|left, right| {
            left.relationships
                .cmp(&right.relationships)
                .then_with(|| left.concepts.cmp(&right.concepts))
        });
        Ok(paths)
    }

    pub fn neighbors(
        &self,
        entity_type: &str,
        depth: u8,
    ) -> SorxResult<Vec<OntologyRelationshipEdge>> {
        self.require_concept(entity_type)?;
        if depth == 0 {
            return Ok(Vec::new());
        }
        let mut frontier = BTreeSet::from([entity_type.to_string()]);
        let mut seen_concepts = frontier.clone();
        let mut seen_relationships = BTreeMap::new();
        for _ in 0..depth {
            let mut next = BTreeSet::new();
            for concept in &frontier {
                for relationship in self.relationships.values().filter(|relationship| {
                    relationship.from == *concept || relationship.to == *concept
                }) {
                    seen_relationships.insert(relationship.id.clone(), relationship.clone());
                    for candidate in [&relationship.from, &relationship.to] {
                        if seen_concepts.insert(candidate.clone()) {
                            next.insert(candidate.clone());
                        }
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        Ok(seen_relationships.into_values().collect())
    }

    fn walk(
        &self,
        target: &str,
        visited: &mut BTreeSet<String>,
        paths: &mut Vec<TypePath>,
        frame: WalkFrame,
    ) {
        if frame.remaining_depth == 0 {
            return;
        }
        for relationship in self.relationships_from(&frame.current) {
            if visited.contains(&relationship.to) {
                continue;
            }
            let mut next_concepts = frame.concepts.clone();
            next_concepts.push(relationship.to.clone());
            let mut next_relationships = frame.relationships.clone();
            next_relationships.push(relationship.id.clone());
            if relationship.to == target {
                paths.push(TypePath {
                    concepts: next_concepts,
                    relationships: next_relationships,
                });
                continue;
            }
            visited.insert(relationship.to.clone());
            self.walk(
                target,
                visited,
                paths,
                WalkFrame {
                    current: relationship.to.clone(),
                    remaining_depth: frame.remaining_depth - 1,
                    concepts: next_concepts,
                    relationships: next_relationships,
                },
            );
            visited.remove(&relationship.to);
        }
    }

    fn require_concept(&self, concept: &str) -> SorxResult<()> {
        if self.concepts.contains_key(concept) {
            Ok(())
        } else {
            Err(SorxError::new(
                "ontology_concept_missing",
                format!("ontology concept `{concept}` does not exist"),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> OntologyGraphService {
        OntologyGraphService::new(
            vec![
                concept("Payment"),
                concept("Tenant"),
                concept("Lease"),
                concept("Cycle"),
            ],
            vec![
                relationship("tenant_has_lease", "Tenant", "Lease"),
                relationship("lease_has_payment", "Lease", "Payment"),
                relationship("cycle_back", "Payment", "Tenant"),
            ],
        )
        .unwrap()
    }

    fn concept(id: &str) -> OntologyConceptNode {
        OntologyConceptNode {
            id: id.to_string(),
            label: None,
        }
    }

    fn relationship(id: &str, from: &str, to: &str) -> OntologyRelationshipEdge {
        OntologyRelationshipEdge {
            id: id.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            label: None,
        }
    }

    #[test]
    fn ontology_graph_lists_concepts_deterministically() {
        let ids = service()
            .concepts()
            .into_iter()
            .map(|concept| concept.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["Cycle", "Lease", "Payment", "Tenant"]);
    }

    #[test]
    fn ontology_graph_lists_relationships_deterministically() {
        let ids = service()
            .relationships()
            .into_iter()
            .map(|relationship| relationship.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec!["cycle_back", "lease_has_payment", "tenant_has_lease"]
        );
    }

    #[test]
    fn ontology_graph_finds_depth_one_path() {
        let paths = service().find_type_paths("Tenant", "Lease", 2).unwrap();
        assert_eq!(paths[0].relationships, vec!["tenant_has_lease"]);
    }

    #[test]
    fn ontology_graph_finds_depth_two_path() {
        let paths = service().find_type_paths("Tenant", "Payment", 2).unwrap();
        assert_eq!(
            paths[0].relationships,
            vec!["tenant_has_lease", "lease_has_payment"]
        );
    }

    #[test]
    fn ontology_graph_prevents_cycles() {
        let paths = service().find_type_paths("Tenant", "Tenant", 4).unwrap();
        assert!(paths.is_empty());
    }

    #[test]
    fn ontology_graph_unknown_concept_fails() {
        let err = service()
            .find_type_paths("Tenant", "Missing", 2)
            .unwrap_err();
        assert_eq!(err.code, "ontology_concept_missing");
    }
}
