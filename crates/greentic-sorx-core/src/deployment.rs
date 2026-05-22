use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEPLOYMENT_REGISTRY_SCHEMA: &str = "greentic.sorx.deployment-registry.v1";
pub const DEPLOYMENT_ROUTE_TABLE_SCHEMA: &str = "greentic.sorx.deployment-routes.v1";
pub const DEPLOYMENT_PUBLIC_ROUTE_TABLE_SCHEMA: &str = "greentic.sorx.public-routes.v1";
pub const DEPLOYMENT_PROMOTION_STATUS_SCHEMA: &str = "greentic.sorx.promotion-status.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackArtifact {
    pub source: String,
    pub name: String,
    pub version: String,
    pub digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SorxDeployment {
    pub deployment_id: String,
    pub tenant_id: String,
    pub sor_name: String,
    pub pack_name: String,
    pub pack_version: String,
    pub pack_digest: String,
    pub environment: String,
    pub api_version_label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub view_version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub canonical_version: String,
    pub base_path: String,
    pub state_namespace: String,
    pub visibility: DeploymentVisibility,
    pub status: DeploymentStatus,
    pub state_mode: StateMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic: Option<TrafficSplit>,
    pub artifact: PackArtifact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentAlias {
    pub tenant_id: String,
    pub sor_name: String,
    pub alias: String,
    pub target_deployment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentRegistry {
    pub schema: String,
    pub deployments: Vec<SorxDeployment>,
    pub aliases: Vec<DeploymentAlias>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub webhook_deliveries: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation_reports: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub promotion_audit_events: Vec<PromotionAuditEvent>,
}

impl Default for DeploymentRegistry {
    fn default() -> Self {
        Self {
            schema: DEPLOYMENT_REGISTRY_SCHEMA.to_string(),
            deployments: Vec::new(),
            aliases: Vec::new(),
            webhook_deliveries: Vec::new(),
            validation_reports: Vec::new(),
            promotion_audit_events: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateDeploymentRequest {
    pub artifact: PackArtifact,
    pub tenant_id: String,
    pub sor_name: String,
    pub environment: String,
    pub api_version_label: String,
    pub base_path: String,
    pub visibility: DeploymentVisibility,
    pub state_mode: StateMode,
    pub state_namespace: Option<String>,
    pub deployment_id: Option<String>,
    pub allow_api_version_conflict: bool,
    pub allow_shared_state_conflict: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentStatus {
    Installed,
    Pending,
    Validating,
    Validated,
    ActivePrivate,
    ActivePublic,
    Failed,
    FailedPublicPromotion,
    Retired,
    RolledBack,
}

impl DeploymentStatus {
    pub fn is_routable(self) -> bool {
        matches!(
            self,
            Self::Validated | Self::ActivePrivate | Self::ActivePublic
        )
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::ActivePrivate | Self::ActivePublic)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentVisibility {
    Private,
    Internal,
    Public,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrafficMode {
    All,
    Percent,
    Header,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrafficHeaderMatch {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrafficSplit {
    pub mode: TrafficMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<TrafficHeaderMatch>,
}

impl Default for TrafficSplit {
    fn default() -> Self {
        Self {
            mode: TrafficMode::All,
            percent: None,
            header: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionAuditEvent {
    pub event: String,
    pub deployment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sor_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_target_deployment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_target_deployment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub actor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionStatus {
    pub schema: String,
    pub deployment_id: String,
    pub pack_digest: String,
    pub status: DeploymentStatus,
    pub visibility: DeploymentVisibility,
    pub validation_report_present: bool,
    pub validation_report_digest_matches: bool,
    pub validation_result: Option<String>,
    pub public_exposure_allowed: bool,
    pub promotable_private: bool,
    pub promotable_public: bool,
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gates: Vec<PromotionGateStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionGateStatus {
    pub id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackAliasRequest {
    pub tenant_id: String,
    pub sor_name: String,
    pub alias: String,
    pub to_deployment_id: String,
    pub reason: String,
    pub actor: String,
    pub automation_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidationGate {
    report_present: bool,
    digest_matches: bool,
    result: Option<String>,
    public_exposure_allowed: bool,
    private_ok: bool,
    ok: bool,
    reason: Option<String>,
    gates: Vec<PromotionGateStatus>,
}

impl DeploymentVisibility {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "private" => Some(Self::Private),
            "internal" => Some(Self::Internal),
            "public" => Some(Self::Public),
            _ => None,
        }
    }
}

impl fmt::Display for DeploymentVisibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Private => write!(f, "private"),
            Self::Internal => write!(f, "internal"),
            Self::Public => write!(f, "public"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateMode {
    Isolated,
    SharedCompatible,
    SharedRequiresMigration,
}

impl StateMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "isolated" => Some(Self::Isolated),
            "shared_compatible" => Some(Self::SharedCompatible),
            "shared_requires_migration" => Some(Self::SharedRequiresMigration),
            _ => None,
        }
    }
}

impl DeploymentRegistry {
    pub fn create_deployment(
        &mut self,
        request: CreateDeploymentRequest,
    ) -> Result<SorxDeployment, DeploymentRegistryError> {
        let deployment_id = request.deployment_id.clone().unwrap_or_else(|| {
            deployment_id(
                &request.tenant_id,
                &request.sor_name,
                &request.api_version_label,
                &request.artifact.version,
                &request.artifact.digest,
            )
        });
        if self.deployments.iter().any(|deployment| {
            deployment.deployment_id == deployment_id
                && !matches!(deployment.status, DeploymentStatus::Retired)
        }) {
            return Err(DeploymentRegistryError::new(
                "deployment_exists",
                format!("deployment `{deployment_id}` already exists"),
            ));
        }
        if request.state_mode == StateMode::Isolated
            && is_production_environment(&request.environment)
        {
            return Err(DeploymentRegistryError::new(
                "isolated_state_not_production",
                "production deployments must use shared canonical state",
            ));
        }

        let state_namespace = request
            .state_namespace
            .clone()
            .unwrap_or_else(|| default_state_namespace(&request));

        self.check_api_version_conflict(&request)?;
        self.check_base_path_conflict(&deployment_id, &request.base_path)?;
        self.check_state_namespace_conflict(
            &deployment_id,
            &state_namespace,
            request.state_mode,
            request.allow_shared_state_conflict,
        )?;

        let deployment = SorxDeployment {
            deployment_id,
            tenant_id: request.tenant_id,
            sor_name: request.sor_name,
            pack_name: request.artifact.name.clone(),
            pack_version: request.artifact.version.clone(),
            pack_digest: request.artifact.digest.clone(),
            environment: request.environment,
            api_version_label: request.api_version_label.clone(),
            view_version: request.api_version_label,
            canonical_version: request.artifact.version.clone(),
            base_path: normalize_base_path(&request.base_path),
            state_namespace,
            visibility: request.visibility,
            status: DeploymentStatus::Pending,
            state_mode: request.state_mode,
            traffic: Some(TrafficSplit::default()),
            artifact: request.artifact,
        };
        self.deployments.push(deployment.clone());
        self.deployments
            .sort_by(|left, right| left.deployment_id.cmp(&right.deployment_id));
        Ok(deployment)
    }

    pub fn deployment(&self, deployment_id: &str) -> Option<&SorxDeployment> {
        self.deployments
            .iter()
            .find(|deployment| deployment.deployment_id == deployment_id)
    }

    pub fn validate_deployment(
        &mut self,
        deployment_id: &str,
    ) -> Result<SorxDeployment, DeploymentRegistryError> {
        self.update_status(deployment_id, DeploymentStatus::Validated)
    }

    pub fn activate_private(
        &mut self,
        deployment_id: &str,
    ) -> Result<SorxDeployment, DeploymentRegistryError> {
        self.update_status(deployment_id, DeploymentStatus::ActivePrivate)
    }

    pub fn record_validation_report(
        &mut self,
        report: serde_json::Value,
    ) -> Result<(), DeploymentRegistryError> {
        let deployment_id = report
            .get("deployment_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                DeploymentRegistryError::new(
                    "validation_report_invalid",
                    "validation report must include deployment_id",
                )
            })?;
        self.deployment(deployment_id).ok_or_else(|| {
            DeploymentRegistryError::new(
                "deployment_missing",
                format!("deployment `{deployment_id}` does not exist"),
            )
        })?;
        self.validation_reports.push(report);
        Ok(())
    }

    pub fn latest_validation_report(&self, deployment_id: &str) -> Option<&serde_json::Value> {
        self.validation_reports.iter().rev().find(|report| {
            report
                .get("deployment_id")
                .and_then(serde_json::Value::as_str)
                == Some(deployment_id)
        })
    }

    pub fn promotion_status(
        &self,
        deployment_id: &str,
    ) -> Result<PromotionStatus, DeploymentRegistryError> {
        let deployment = self.deployment(deployment_id).ok_or_else(|| {
            DeploymentRegistryError::new(
                "deployment_missing",
                format!("deployment `{deployment_id}` does not exist"),
            )
        })?;
        let validation = self.validation_gate(deployment);
        Ok(PromotionStatus {
            schema: DEPLOYMENT_PROMOTION_STATUS_SCHEMA.to_string(),
            deployment_id: deployment.deployment_id.clone(),
            pack_digest: deployment.pack_digest.clone(),
            status: deployment.status,
            visibility: deployment.visibility,
            validation_report_present: validation.report_present,
            validation_report_digest_matches: validation.digest_matches,
            validation_result: validation.result,
            public_exposure_allowed: validation.public_exposure_allowed,
            promotable_private: validation.private_ok
                && matches!(
                    deployment.status,
                    DeploymentStatus::Pending
                        | DeploymentStatus::Validating
                        | DeploymentStatus::Validated
                        | DeploymentStatus::ActivePrivate
                ),
            promotable_public: validation.ok
                && matches!(
                    deployment.status,
                    DeploymentStatus::Validated | DeploymentStatus::ActivePrivate
                ),
            reason: validation.reason,
            gates: validation.gates,
        })
    }

    pub fn promote_private(
        &mut self,
        deployment_id: &str,
        actor: impl Into<String>,
        automation_source: Option<String>,
    ) -> Result<SorxDeployment, DeploymentRegistryError> {
        let deployment = self.deployment_required(deployment_id)?.clone();
        self.require_private_validation_gate(&deployment)?;
        let promoted = self.update_status(deployment_id, DeploymentStatus::ActivePrivate)?;
        self.record_promotion_audit(PromotionAuditEvent {
            event: "sorx.deployment.promoted_private".to_string(),
            deployment_id: deployment_id.to_string(),
            tenant_id: Some(promoted.tenant_id.clone()),
            sor_name: Some(promoted.sor_name.clone()),
            alias: None,
            old_target_deployment_id: None,
            new_target_deployment_id: Some(deployment_id.to_string()),
            reason: None,
            actor: actor.into(),
            automation_source,
        });
        Ok(promoted)
    }

    pub fn promote_public(
        &mut self,
        deployment_id: &str,
        actor: impl Into<String>,
        automation_source: Option<String>,
    ) -> Result<SorxDeployment, DeploymentRegistryError> {
        let deployment = self.deployment_required(deployment_id)?.clone();
        self.require_validation_gate(&deployment)?;
        let promoted = self.update_deployment(deployment_id, |deployment| {
            deployment.status = DeploymentStatus::ActivePublic;
            deployment.visibility = DeploymentVisibility::Public;
        })?;
        self.record_promotion_audit(PromotionAuditEvent {
            event: "sorx.deployment.promoted_public".to_string(),
            deployment_id: deployment_id.to_string(),
            tenant_id: Some(promoted.tenant_id.clone()),
            sor_name: Some(promoted.sor_name.clone()),
            alias: None,
            old_target_deployment_id: None,
            new_target_deployment_id: Some(deployment_id.to_string()),
            reason: None,
            actor: actor.into(),
            automation_source,
        });
        Ok(promoted)
    }

    pub fn promote_alias(
        &mut self,
        deployment_id: &str,
        alias: &str,
        public: bool,
        actor: impl Into<String>,
        automation_source: Option<String>,
    ) -> Result<DeploymentAlias, DeploymentRegistryError> {
        let actor = actor.into();
        if public {
            self.promote_public(deployment_id, actor.clone(), automation_source.clone())?;
        } else {
            self.promote_private(deployment_id, actor.clone(), automation_source.clone())?;
        }
        let target = self.deployment_required(deployment_id)?.clone();
        let old_target = self
            .aliases
            .iter()
            .find(|candidate| {
                candidate.tenant_id == target.tenant_id
                    && candidate.sor_name == target.sor_name
                    && candidate.alias == alias
            })
            .map(|existing| existing.target_deployment_id.clone());
        let next = self.set_alias(&target.tenant_id, &target.sor_name, alias, deployment_id)?;
        self.record_promotion_audit(PromotionAuditEvent {
            event: "sorx.deployment.alias_promoted".to_string(),
            deployment_id: deployment_id.to_string(),
            tenant_id: Some(target.tenant_id),
            sor_name: Some(target.sor_name),
            alias: Some(alias.to_string()),
            old_target_deployment_id: old_target,
            new_target_deployment_id: Some(deployment_id.to_string()),
            reason: None,
            actor,
            automation_source,
        });
        Ok(next)
    }

    pub fn rollback_alias(
        &mut self,
        request: RollbackAliasRequest,
    ) -> Result<DeploymentAlias, DeploymentRegistryError> {
        let target = self.deployment_required(&request.to_deployment_id)?.clone();
        if target.tenant_id != request.tenant_id || target.sor_name != request.sor_name {
            return Err(DeploymentRegistryError::new(
                "alias_scope_mismatch",
                "rollback target must have the same tenant and SOR name",
            ));
        }
        let old_target = self
            .aliases
            .iter()
            .find(|candidate| {
                candidate.tenant_id == request.tenant_id
                    && candidate.sor_name == request.sor_name
                    && candidate.alias == request.alias
            })
            .map(|existing| existing.target_deployment_id.clone());
        if let Some(old_target) = old_target.as_deref()
            && old_target != request.to_deployment_id
        {
            let _ = self.update_status(old_target, DeploymentStatus::RolledBack)?;
        }
        let next = self.set_alias(
            &request.tenant_id,
            &request.sor_name,
            &request.alias,
            &request.to_deployment_id,
        )?;
        self.record_promotion_audit(PromotionAuditEvent {
            event: "sorx.deployment.alias_rolled_back".to_string(),
            deployment_id: request.to_deployment_id.clone(),
            tenant_id: Some(request.tenant_id),
            sor_name: Some(request.sor_name),
            alias: Some(request.alias),
            old_target_deployment_id: old_target,
            new_target_deployment_id: Some(request.to_deployment_id),
            reason: Some(request.reason),
            actor: request.actor,
            automation_source: request.automation_source,
        });
        Ok(next)
    }

    pub fn retire_old(
        &mut self,
        tenant_id: &str,
        sor_name: &str,
        keep: usize,
    ) -> Result<Vec<SorxDeployment>, DeploymentRegistryError> {
        let mut active = self
            .deployments
            .iter()
            .filter(|deployment| {
                deployment.tenant_id == tenant_id
                    && deployment.sor_name == sor_name
                    && deployment.status.is_active()
            })
            .cloned()
            .collect::<Vec<_>>();
        active.sort_by(|left, right| right.deployment_id.cmp(&left.deployment_id));
        let mut retired = Vec::new();
        for deployment in active.into_iter().skip(keep) {
            retired.push(self.retire(&deployment.deployment_id)?);
        }
        Ok(retired)
    }

    pub fn retire(
        &mut self,
        deployment_id: &str,
    ) -> Result<SorxDeployment, DeploymentRegistryError> {
        let deployment = self.update_status(deployment_id, DeploymentStatus::Retired)?;
        self.aliases
            .retain(|alias| alias.target_deployment_id != deployment_id);
        Ok(deployment)
    }

    pub fn set_alias(
        &mut self,
        tenant_id: impl Into<String>,
        sor_name: impl Into<String>,
        alias: impl Into<String>,
        target_deployment_id: impl Into<String>,
    ) -> Result<DeploymentAlias, DeploymentRegistryError> {
        let tenant_id = tenant_id.into();
        let sor_name = sor_name.into();
        let alias = alias.into();
        let target_deployment_id = target_deployment_id.into();
        let target = self.deployment(&target_deployment_id).ok_or_else(|| {
            DeploymentRegistryError::new(
                "deployment_missing",
                format!("deployment `{target_deployment_id}` does not exist"),
            )
        })?;
        if target.tenant_id != tenant_id || target.sor_name != sor_name {
            return Err(DeploymentRegistryError::new(
                "alias_scope_mismatch",
                "alias target must have the same tenant and SOR name",
            ));
        }
        if !target.status.is_routable() {
            return Err(DeploymentRegistryError::new(
                "alias_target_not_routable",
                "alias target must be validated or active",
            ));
        }
        let next = DeploymentAlias {
            tenant_id,
            sor_name,
            alias,
            target_deployment_id,
        };
        self.aliases.retain(|existing| {
            !(existing.tenant_id == next.tenant_id
                && existing.sor_name == next.sor_name
                && existing.alias == next.alias)
        });
        self.aliases.push(next.clone());
        self.aliases.sort_by(|left, right| {
            (
                left.tenant_id.as_str(),
                left.sor_name.as_str(),
                left.alias.as_str(),
            )
                .cmp(&(
                    right.tenant_id.as_str(),
                    right.sor_name.as_str(),
                    right.alias.as_str(),
                ))
        });
        Ok(next)
    }

    pub fn public_deployments(&self) -> Vec<SorxDeployment> {
        self.deployments
            .iter()
            .filter(|deployment| deployment.status == DeploymentStatus::ActivePublic)
            .cloned()
            .collect()
    }

    pub fn aliases_for(
        &self,
        tenant_id: Option<&str>,
        sor_name: Option<&str>,
    ) -> Vec<DeploymentAlias> {
        self.aliases
            .iter()
            .filter(|alias| tenant_id.is_none_or(|tenant| alias.tenant_id == tenant))
            .filter(|alias| sor_name.is_none_or(|sor| alias.sor_name == sor))
            .cloned()
            .collect()
    }

    pub fn resolve_alias(
        &self,
        tenant_id: &str,
        sor_name: &str,
        alias: &str,
    ) -> Option<&SorxDeployment> {
        let target = self.aliases.iter().find(|candidate| {
            candidate.tenant_id == tenant_id
                && candidate.sor_name == sor_name
                && candidate.alias == alias
        })?;
        self.deployment(&target.target_deployment_id)
    }

    pub fn has_webhook_delivery(&self, delivery_id: &str) -> bool {
        self.webhook_deliveries
            .iter()
            .any(|existing| existing == delivery_id)
    }

    pub fn record_webhook_delivery(
        &mut self,
        delivery_id: impl Into<String>,
    ) -> Result<(), DeploymentRegistryError> {
        let delivery_id = delivery_id.into();
        if self.has_webhook_delivery(&delivery_id) {
            return Err(DeploymentRegistryError::new(
                "webhook_replay",
                format!("webhook delivery `{delivery_id}` was already processed"),
            ));
        }
        self.webhook_deliveries.push(delivery_id);
        self.webhook_deliveries.sort();
        Ok(())
    }

    fn update_status(
        &mut self,
        deployment_id: &str,
        status: DeploymentStatus,
    ) -> Result<SorxDeployment, DeploymentRegistryError> {
        self.update_deployment(deployment_id, |deployment| {
            deployment.status = status;
        })
    }

    fn update_deployment(
        &mut self,
        deployment_id: &str,
        update: impl FnOnce(&mut SorxDeployment),
    ) -> Result<SorxDeployment, DeploymentRegistryError> {
        let deployment = self
            .deployments
            .iter_mut()
            .find(|deployment| deployment.deployment_id == deployment_id)
            .ok_or_else(|| {
                DeploymentRegistryError::new(
                    "deployment_missing",
                    format!("deployment `{deployment_id}` does not exist"),
                )
            })?;
        update(deployment);
        Ok(deployment.clone())
    }

    fn deployment_required(
        &self,
        deployment_id: &str,
    ) -> Result<&SorxDeployment, DeploymentRegistryError> {
        self.deployment(deployment_id).ok_or_else(|| {
            DeploymentRegistryError::new(
                "deployment_missing",
                format!("deployment `{deployment_id}` does not exist"),
            )
        })
    }

    fn require_validation_gate(
        &self,
        deployment: &SorxDeployment,
    ) -> Result<(), DeploymentRegistryError> {
        let gate = self.validation_gate(deployment);
        if gate.ok {
            Ok(())
        } else {
            Err(DeploymentRegistryError::new(
                "promotion_blocked",
                gate.reason.unwrap_or_else(|| {
                    format!(
                        "deployment `{}` does not have a passing validation report",
                        deployment.deployment_id
                    )
                }),
            ))
        }
    }

    fn require_private_validation_gate(
        &self,
        deployment: &SorxDeployment,
    ) -> Result<(), DeploymentRegistryError> {
        let gate = self.validation_gate(deployment);
        if gate.private_ok {
            Ok(())
        } else {
            Err(DeploymentRegistryError::new(
                "promotion_blocked",
                gate.reason.unwrap_or_else(|| {
                    format!(
                        "deployment `{}` does not have a passing validation report",
                        deployment.deployment_id
                    )
                }),
            ))
        }
    }

    fn validation_gate(&self, deployment: &SorxDeployment) -> ValidationGate {
        let Some(report) = self.latest_validation_report(&deployment.deployment_id) else {
            return ValidationGate {
                report_present: false,
                digest_matches: false,
                result: None,
                public_exposure_allowed: false,
                private_ok: false,
                ok: false,
                reason: Some("validation report is required before promotion".to_string()),
                gates: vec![PromotionGateStatus {
                    id: "validation-report".to_string(),
                    status: "missing".to_string(),
                    reason: Some("validation report is required before promotion".to_string()),
                }],
            };
        };
        let report_digest = report
            .get("pack_digest")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                report
                    .get("pack")
                    .and_then(|pack| pack.get("digest"))
                    .and_then(serde_json::Value::as_str)
            });
        let digest_matches = report_digest == Some(deployment.pack_digest.as_str());
        let result = report
            .get("result")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string);
        let public_exposure_allowed = report
            .get("public_exposure_allowed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let mut gates = vec![
            PromotionGateStatus {
                id: "validation-report".to_string(),
                status: if result.as_deref() == Some("pass") && digest_matches {
                    "passed"
                } else {
                    "failed"
                }
                .to_string(),
                reason: if digest_matches {
                    None
                } else {
                    Some("validation report pack digest does not match deployment".to_string())
                },
            },
            PromotionGateStatus {
                id: "public-exposure".to_string(),
                status: if public_exposure_allowed {
                    "passed"
                } else {
                    "failed"
                }
                .to_string(),
                reason: if public_exposure_allowed {
                    None
                } else {
                    Some("validation report does not allow public exposure".to_string())
                },
            },
        ];
        let ontology_gates = ontology_promotion_gates(report);
        let override_allowed = report
            .get("local_operator_policy_override")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let ontology_ok =
            ontology_gates.iter().all(|gate| gate.status == "passed") || override_allowed;
        gates.extend(ontology_gates);
        if override_allowed {
            gates.push(PromotionGateStatus {
                id: "local-operator-override".to_string(),
                status: "passed".to_string(),
                reason: Some(
                    "local operator override allowed ontology public exposure".to_string(),
                ),
            });
        }
        let private_ok = digest_matches && result.as_deref() == Some("pass");
        let ok = private_ok && public_exposure_allowed && ontology_ok;
        let reason = if ok {
            None
        } else if !digest_matches {
            Some("validation report pack digest does not match deployment".to_string())
        } else if result.as_deref() != Some("pass") {
            Some("validation report did not pass required gates".to_string())
        } else if !ontology_ok {
            Some("ontology public exposure gates did not pass".to_string())
        } else {
            Some("validation report does not allow public exposure".to_string())
        };
        ValidationGate {
            report_present: true,
            digest_matches,
            result,
            public_exposure_allowed,
            private_ok,
            ok,
            reason,
            gates,
        }
    }

    fn record_promotion_audit(&mut self, event: PromotionAuditEvent) {
        self.promotion_audit_events.push(event);
    }

    fn check_base_path_conflict(
        &self,
        deployment_id: &str,
        base_path: &str,
    ) -> Result<(), DeploymentRegistryError> {
        let base_path = normalize_base_path(base_path);
        for existing in self
            .deployments
            .iter()
            .filter(|deployment| deployment.status != DeploymentStatus::Retired)
        {
            if existing.deployment_id != deployment_id && existing.base_path == base_path {
                return Err(DeploymentRegistryError::new(
                    "base_path_conflict",
                    format!(
                        "base path `{base_path}` conflicts with deployment `{}`",
                        existing.deployment_id
                    ),
                ));
            }
        }
        Ok(())
    }

    fn check_api_version_conflict(
        &self,
        request: &CreateDeploymentRequest,
    ) -> Result<(), DeploymentRegistryError> {
        if request.allow_api_version_conflict {
            return Ok(());
        }
        for existing in self.deployments.iter().filter(|deployment| {
            deployment.status != DeploymentStatus::Retired
                && deployment.tenant_id == request.tenant_id
                && deployment.sor_name == request.sor_name
                && deployment.api_version_label == request.api_version_label
        }) {
            if existing.pack_digest != request.artifact.digest {
                return Err(DeploymentRegistryError::new(
                    "api_version_conflict",
                    format!(
                        "API version `{}` already points to digest `{}`",
                        request.api_version_label, existing.pack_digest
                    ),
                ));
            }
        }
        Ok(())
    }

    fn check_state_namespace_conflict(
        &self,
        deployment_id: &str,
        state_namespace: &str,
        state_mode: StateMode,
        allow_shared_state_conflict: bool,
    ) -> Result<(), DeploymentRegistryError> {
        if allow_shared_state_conflict || state_mode == StateMode::SharedCompatible {
            return Ok(());
        }
        for existing in self
            .deployments
            .iter()
            .filter(|deployment| deployment.status != DeploymentStatus::Retired)
        {
            if existing.deployment_id != deployment_id
                && existing.state_namespace == state_namespace
                && existing.state_mode != StateMode::SharedCompatible
            {
                return Err(DeploymentRegistryError::new(
                    "state_namespace_conflict",
                    format!(
                        "state namespace `{state_namespace}` conflicts with deployment `{}`",
                        existing.deployment_id
                    ),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentRouteTable {
    pub schema: String,
    pub deployment_id: String,
    pub base_path: String,
    pub api_version_label: String,
    pub view_version: String,
    pub canonical_version: String,
    pub state_namespace: String,
    pub routes: Vec<DeploymentRoute>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentRoute {
    pub method: String,
    pub path: String,
    pub endpoint_id: String,
    pub operation_id: String,
}

impl DeploymentRouteTable {
    pub fn new(
        deployment: &SorxDeployment,
        routes: impl IntoIterator<Item = DeploymentRoute>,
    ) -> Self {
        Self {
            schema: DEPLOYMENT_ROUTE_TABLE_SCHEMA.to_string(),
            deployment_id: deployment.deployment_id.clone(),
            base_path: deployment.base_path.clone(),
            api_version_label: deployment.api_version_label.clone(),
            view_version: deployment.view_version.clone(),
            canonical_version: deployment.canonical_version.clone(),
            state_namespace: deployment.state_namespace.clone(),
            routes: routes.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDeploymentRegistryStore {
    path: PathBuf,
}

impl LocalDeploymentRegistryStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<DeploymentRegistry, DeploymentRegistryError> {
        if !self.path.exists() {
            return Ok(DeploymentRegistry::default());
        }
        let bytes = fs::read(&self.path).map_err(|err| {
            DeploymentRegistryError::new(
                "registry_read_failed",
                format!("failed to read {}: {err}", self.path.display()),
            )
        })?;
        let registry: DeploymentRegistry = serde_json::from_slice(&bytes).map_err(|err| {
            DeploymentRegistryError::new(
                "registry_decode_failed",
                format!("failed to decode {}: {err}", self.path.display()),
            )
        })?;
        if registry.schema != DEPLOYMENT_REGISTRY_SCHEMA {
            return Err(DeploymentRegistryError::new(
                "registry_schema_unsupported",
                format!("unsupported registry schema `{}`", registry.schema),
            ));
        }
        Ok(registry)
    }

    pub fn save(&self, registry: &DeploymentRegistry) -> Result<(), DeploymentRegistryError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                DeploymentRegistryError::new(
                    "registry_write_failed",
                    format!("failed to create {}: {err}", parent.display()),
                )
            })?;
        }
        let encoded = serde_json::to_vec_pretty(registry).map_err(|err| {
            DeploymentRegistryError::new(
                "registry_encode_failed",
                format!("failed to encode registry: {err}"),
            )
        })?;
        fs::write(&self.path, encoded).map_err(|err| {
            DeploymentRegistryError::new(
                "registry_write_failed",
                format!("failed to write {}: {err}", self.path.display()),
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentRegistryError {
    pub code: String,
    pub message: String,
}

impl DeploymentRegistryError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for DeploymentRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for DeploymentRegistryError {}

fn deployment_id(
    tenant_id: &str,
    sor_name: &str,
    api_version_label: &str,
    pack_version: &str,
    digest: &str,
) -> String {
    format!(
        "{}-{}-{}-{}-{}",
        clean_segment(tenant_id),
        clean_segment(sor_name),
        clean_segment(api_version_label),
        clean_segment(pack_version),
        digest_suffix(digest)
    )
}

fn default_state_namespace(request: &CreateDeploymentRequest) -> String {
    match request.state_mode {
        StateMode::Isolated => format!(
            "sorx/{}/{}/{}/{}",
            clean_segment(&request.tenant_id),
            clean_segment(&request.sor_name),
            clean_segment(&request.artifact.version),
            digest_suffix(&request.artifact.digest)
        ),
        StateMode::SharedCompatible | StateMode::SharedRequiresMigration => format!(
            "sorx/{}/{}",
            clean_segment(&request.tenant_id),
            clean_segment(&request.sor_name)
        ),
    }
}

fn digest_suffix(digest: &str) -> String {
    digest
        .strip_prefix("sha256:")
        .unwrap_or(digest)
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>()
}

fn ontology_promotion_gates(report: &serde_json::Value) -> Vec<PromotionGateStatus> {
    let Some(ontology) = report
        .get("ontology")
        .and_then(serde_json::Value::as_object)
    else {
        return Vec::new();
    };
    [
        ("ontology-static", "static_validation"),
        ("provider-compatibility", "provider_compatibility"),
        ("retrieval-bindings", "retrieval_bindings"),
        ("ontology-policy", "policy_validation"),
    ]
    .into_iter()
    .map(|(id, key)| {
        let passed = ontology
            .get(key)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|status| status == "passed" || status == "pass");
        PromotionGateStatus {
            id: id.to_string(),
            status: if passed { "passed" } else { "failed" }.to_string(),
            reason: if passed {
                None
            } else {
                Some(format!("ontology gate `{key}` did not pass"))
            },
        }
    })
    .collect()
}

fn clean_segment(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if cleaned.is_empty() {
        "unnamed".to_string()
    } else {
        cleaned
    }
}

fn is_production_environment(environment: &str) -> bool {
    matches!(
        environment.trim().to_ascii_lowercase().as_str(),
        "prod" | "production"
    )
}

fn normalize_base_path(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return "/".to_string();
    }
    format!("/{}", trimmed.trim_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(version: &str, digest: &str) -> PackArtifact {
        PackArtifact {
            source: format!("fixtures/landlord-{version}.gtpack"),
            name: "landlord-tenant-sor".to_string(),
            version: version.to_string(),
            digest: digest.to_string(),
            signature: None,
            signature_ref: None,
        }
    }

    fn request(version: &str, digest: &str, api_version_label: &str) -> CreateDeploymentRequest {
        CreateDeploymentRequest {
            artifact: artifact(version, digest),
            tenant_id: "acme".to_string(),
            sor_name: "landlord".to_string(),
            environment: "production".to_string(),
            api_version_label: api_version_label.to_string(),
            base_path: format!("/sorx/acme/landlord/{api_version_label}"),
            visibility: DeploymentVisibility::Private,
            state_mode: StateMode::SharedCompatible,
            state_namespace: None,
            deployment_id: None,
            allow_api_version_conflict: false,
            allow_shared_state_conflict: false,
        }
    }

    fn passing_report(deployment: &SorxDeployment) -> serde_json::Value {
        serde_json::json!({
            "schema": "greentic.sorx.validation-report.v1",
            "deployment_id": deployment.deployment_id,
            "pack_name": deployment.pack_name,
            "pack_version": deployment.pack_version,
            "pack_digest": deployment.pack_digest,
            "suite_id": "test",
            "started_at": "1970-01-01T00:00:00Z",
            "finished_at": "1970-01-01T00:00:00Z",
            "result": "pass",
            "public_exposure_allowed": true,
            "tests": []
        })
    }

    #[test]
    fn production_deployments_reject_isolated_state() {
        let mut registry = DeploymentRegistry::default();
        let mut request = request("1.0.0", "sha256:111", "v1");
        request.state_mode = StateMode::Isolated;
        let err = registry.create_deployment(request).unwrap_err();
        assert_eq!(err.code, "isolated_state_not_production");
    }

    #[test]
    fn creates_two_deployments_for_same_sor_with_different_versions() {
        let mut registry = DeploymentRegistry::default();
        let one = registry
            .create_deployment(request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        let two = registry
            .create_deployment(request("1.1.0", "sha256:222", "v1-1"))
            .unwrap();
        assert_ne!(one.deployment_id, two.deployment_id);
        assert_eq!(registry.deployments.len(), 2);
        assert_eq!(one.state_namespace, "sorx/acme/landlord");
        assert_eq!(two.state_namespace, "sorx/acme/landlord");
        assert_eq!(one.view_version, "v1");
        assert_eq!(one.canonical_version, "1.0.0");
    }

    #[test]
    fn same_api_version_different_digest_is_rejected() {
        let mut registry = DeploymentRegistry::default();
        registry
            .create_deployment(request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        let err = registry
            .create_deployment(request("1.0.0", "sha256:222", "v1"))
            .unwrap_err();
        assert_eq!(err.code, "api_version_conflict");
    }

    #[test]
    fn same_version_different_digest_can_use_different_api_label() {
        let mut registry = DeploymentRegistry::default();
        registry
            .create_deployment(request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        let created = registry
            .create_deployment(request("1.0.0", "sha256:222", "v1-preview"))
            .unwrap();
        assert_eq!(created.api_version_label, "v1-preview");
    }

    #[test]
    fn aliases_resolve_to_routable_deployment() {
        let mut registry = DeploymentRegistry::default();
        let created = registry
            .create_deployment(request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        registry
            .validate_deployment(&created.deployment_id)
            .unwrap();
        let alias = registry
            .set_alias("acme", "landlord", "stable", &created.deployment_id)
            .unwrap();
        assert_eq!(alias.target_deployment_id, created.deployment_id);
        assert_eq!(
            registry
                .resolve_alias("acme", "landlord", "stable")
                .unwrap()
                .deployment_id,
            created.deployment_id
        );
    }

    #[test]
    fn promotion_is_blocked_without_validation_report() {
        let mut registry = DeploymentRegistry::default();
        let created = registry
            .create_deployment(request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        let err = registry
            .promote_public(&created.deployment_id, "tester", None)
            .unwrap_err();
        assert_eq!(err.code, "promotion_blocked");
        assert!(err.message.contains("validation report"));
    }

    #[test]
    fn promotion_is_blocked_when_validation_digest_differs() {
        let mut registry = DeploymentRegistry::default();
        let created = registry
            .create_deployment(request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        let mut report = passing_report(&created);
        report["pack_digest"] = serde_json::json!("sha256:222");
        registry.record_validation_report(report).unwrap();
        let status = registry.promotion_status(&created.deployment_id).unwrap();
        assert!(status.validation_report_present);
        assert!(!status.validation_report_digest_matches);
        let err = registry
            .promote_public(&created.deployment_id, "tester", None)
            .unwrap_err();
        assert_eq!(err.code, "promotion_blocked");
    }

    #[test]
    fn promotion_is_blocked_when_required_validation_failed() {
        let mut registry = DeploymentRegistry::default();
        let created = registry
            .create_deployment(request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        let mut report = passing_report(&created);
        report["result"] = serde_json::json!("fail");
        report["public_exposure_allowed"] = serde_json::json!(false);
        registry.record_validation_report(report).unwrap();
        let err = registry
            .promote_public(&created.deployment_id, "tester", None)
            .unwrap_err();
        assert_eq!(err.code, "promotion_blocked");
    }

    #[test]
    fn ontology_public_promotion_requires_ontology_gates() {
        let mut registry = DeploymentRegistry::default();
        let created = registry
            .create_deployment(request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        let mut report = passing_report(&created);
        report["ontology"] = serde_json::json!({
            "static_validation": "passed",
            "provider_compatibility": "failed",
            "retrieval_bindings": "passed",
            "policy_validation": "passed"
        });
        registry.record_validation_report(report).unwrap();
        let status = registry.promotion_status(&created.deployment_id).unwrap();
        assert!(!status.promotable_public);
        assert!(
            status
                .gates
                .iter()
                .any(|gate| { gate.id == "provider-compatibility" && gate.status == "failed" })
        );
        let err = registry
            .promote_public(&created.deployment_id, "tester", None)
            .unwrap_err();
        assert_eq!(err.code, "promotion_blocked");
    }

    #[test]
    fn ontology_gate_failure_does_not_block_private_promotion() {
        let mut registry = DeploymentRegistry::default();
        let created = registry
            .create_deployment(request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        let mut report = passing_report(&created);
        report["public_exposure_allowed"] = serde_json::json!(false);
        report["ontology"] = serde_json::json!({
            "static_validation": "passed",
            "provider_compatibility": "failed",
            "retrieval_bindings": "passed",
            "policy_validation": "passed"
        });
        registry.record_validation_report(report).unwrap();
        let status = registry.promotion_status(&created.deployment_id).unwrap();
        assert!(status.promotable_private);
        assert!(!status.promotable_public);
        let private = registry
            .promote_private(&created.deployment_id, "tester", None)
            .unwrap();
        assert_eq!(private.status, DeploymentStatus::ActivePrivate);
    }

    #[test]
    fn local_operator_override_allows_ontology_gate_failure_and_is_audited() {
        let mut registry = DeploymentRegistry::default();
        let created = registry
            .create_deployment(request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        let mut report = passing_report(&created);
        report["local_operator_policy_override"] = serde_json::json!(true);
        report["ontology"] = serde_json::json!({
            "static_validation": "passed",
            "provider_compatibility": "failed",
            "retrieval_bindings": "passed",
            "policy_validation": "passed"
        });
        registry.record_validation_report(report).unwrap();
        registry
            .validate_deployment(&created.deployment_id)
            .unwrap();
        registry
            .promote_private(&created.deployment_id, "tester", None)
            .unwrap();
        let status = registry.promotion_status(&created.deployment_id).unwrap();
        assert!(status.promotable_public);
        assert!(
            status
                .gates
                .iter()
                .any(|gate| gate.id == "local-operator-override")
        );
        registry
            .promote_public(
                &created.deployment_id,
                "tester",
                Some("override-test".to_string()),
            )
            .unwrap();
        assert!(registry.promotion_audit_events.iter().any(|event| {
            event.deployment_id == created.deployment_id
                && event.automation_source.as_deref() == Some("override-test")
        }));
    }

    #[test]
    fn private_and_public_promotion_require_passing_validation() {
        let mut registry = DeploymentRegistry::default();
        let created = registry
            .create_deployment(request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        registry
            .record_validation_report(passing_report(&created))
            .unwrap();
        let private = registry
            .promote_private(&created.deployment_id, "tester", None)
            .unwrap();
        assert_eq!(private.status, DeploymentStatus::ActivePrivate);
        let public = registry
            .promote_public(&created.deployment_id, "tester", None)
            .unwrap();
        assert_eq!(public.status, DeploymentStatus::ActivePublic);
        assert_eq!(public.visibility, DeploymentVisibility::Public);
        assert_eq!(registry.public_deployments().len(), 1);
        assert_eq!(registry.promotion_audit_events.len(), 2);
    }

    #[test]
    fn alias_promotion_and_rollback_are_audited() {
        let mut registry = DeploymentRegistry::default();
        let one = registry
            .create_deployment(request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        registry
            .record_validation_report(passing_report(&one))
            .unwrap();
        registry
            .promote_alias(&one.deployment_id, "latest", true, "tester", None)
            .unwrap();
        let two = registry
            .create_deployment(request("1.1.0", "sha256:222", "v1-1"))
            .unwrap();
        registry
            .record_validation_report(passing_report(&two))
            .unwrap();
        registry
            .promote_alias(&two.deployment_id, "latest", true, "tester", None)
            .unwrap();
        assert_eq!(
            registry
                .resolve_alias("acme", "landlord", "latest")
                .unwrap()
                .deployment_id,
            two.deployment_id
        );
        registry
            .rollback_alias(RollbackAliasRequest {
                tenant_id: "acme".to_string(),
                sor_name: "landlord".to_string(),
                alias: "latest".to_string(),
                to_deployment_id: one.deployment_id.clone(),
                reason: "bad release".to_string(),
                actor: "tester".to_string(),
                automation_source: None,
            })
            .unwrap();
        assert_eq!(
            registry
                .resolve_alias("acme", "landlord", "latest")
                .unwrap()
                .deployment_id,
            one.deployment_id
        );
        assert_eq!(
            registry.deployment(&two.deployment_id).unwrap().status,
            DeploymentStatus::RolledBack
        );
        assert_eq!(
            registry.promotion_audit_events.last().unwrap().event,
            "sorx.deployment.alias_rolled_back"
        );
    }

    #[test]
    fn retiring_one_version_does_not_affect_another() {
        let mut registry = DeploymentRegistry::default();
        let one = registry
            .create_deployment(request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        let two = registry
            .create_deployment(request("1.1.0", "sha256:222", "v1-1"))
            .unwrap();
        registry.retire(&one.deployment_id).unwrap();
        assert_eq!(
            registry.deployment(&one.deployment_id).unwrap().status,
            DeploymentStatus::Retired
        );
        assert_eq!(
            registry.deployment(&two.deployment_id).unwrap().status,
            DeploymentStatus::Pending
        );
    }

    #[test]
    fn base_path_conflicts_are_rejected() {
        let mut registry = DeploymentRegistry::default();
        registry
            .create_deployment(request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        let mut second = request("1.1.0", "sha256:222", "v1-1");
        second.base_path = "/sorx/acme/landlord/v1".to_string();
        let err = registry.create_deployment(second).unwrap_err();
        assert_eq!(err.code, "base_path_conflict");
    }

    #[test]
    fn state_namespace_conflicts_are_rejected_by_default() {
        let mut registry = DeploymentRegistry::default();
        let mut first = request("1.0.0", "sha256:111", "v1");
        first.state_mode = StateMode::SharedRequiresMigration;
        let first = registry.create_deployment(first).unwrap();
        let mut second = request("1.1.0", "sha256:222", "v1-1");
        second.state_mode = StateMode::SharedRequiresMigration;
        second.state_namespace = Some(first.state_namespace);
        let err = registry.create_deployment(second).unwrap_err();
        assert_eq!(err.code, "state_namespace_conflict");
    }

    #[test]
    fn shared_compatible_state_namespace_can_be_reused() {
        let mut registry = DeploymentRegistry::default();
        let first = registry
            .create_deployment(request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        let mut second = request("1.1.0", "sha256:222", "v1-1");
        second.state_namespace = Some(first.state_namespace.clone());
        let second = registry.create_deployment(second).unwrap();
        assert_eq!(second.state_namespace, first.state_namespace);
    }

    #[test]
    fn local_registry_survives_restart() {
        let temp =
            std::env::temp_dir().join(format!("sorx-registry-test-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&temp);
        let store = LocalDeploymentRegistryStore::new(&temp);
        let mut registry = DeploymentRegistry::default();
        let created = registry
            .create_deployment(request("1.0.0", "sha256:111", "v1"))
            .unwrap();
        store.save(&registry).unwrap();
        let loaded = store.load().unwrap();
        let _ = std::fs::remove_file(&temp);
        assert_eq!(
            loaded
                .deployment(&created.deployment_id)
                .unwrap()
                .pack_digest,
            "sha256:111"
        );
    }
}
