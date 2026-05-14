use std::path::Path;

use serde::Serialize;

use crate::loader::{LoadedSorlaPack, load_sorla_pack, load_sorla_pack_from_bytes};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SorxDoctorReport {
    pub ok: bool,
    pub errors: Vec<SorxDoctorIssue>,
    pub warnings: Vec<SorxDoctorIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SorxDoctorIssue {
    pub level: SorxDoctorIssueLevel,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SorxDoctorIssueLevel {
    Error,
    Warning,
}

impl SorxDoctorIssue {
    pub(crate) fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            level: SorxDoctorIssueLevel::Error,
            code: code.into(),
            message: message.into(),
        }
    }

    pub(crate) fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            level: SorxDoctorIssueLevel::Warning,
            code: code.into(),
            message: message.into(),
        }
    }
}

pub fn doctor_sorla_pack(path: &Path) -> SorxDoctorReport {
    match load_sorla_pack(path) {
        Ok(pack) => doctor_sorla_loaded_pack(&pack),
        Err(err) => SorxDoctorReport {
            ok: false,
            errors: vec![SorxDoctorIssue::error(err.code(), err.to_string())],
            warnings: Vec::new(),
        },
    }
}

pub fn doctor_sorla_pack_from_bytes(bytes: &[u8]) -> SorxDoctorReport {
    match load_sorla_pack_from_bytes(bytes) {
        Ok(pack) => doctor_sorla_loaded_pack(&pack),
        Err(err) => SorxDoctorReport {
            ok: false,
            errors: vec![SorxDoctorIssue::error(err.code(), err.to_string())],
            warnings: Vec::new(),
        },
    }
}

pub fn doctor_sorla_loaded_pack(pack: &LoadedSorlaPack) -> SorxDoctorReport {
    let errors = pack
        .doctor_errors
        .iter()
        .cloned()
        .map(|message| SorxDoctorIssue::error("validation_suite_invalid", message))
        .collect::<Vec<_>>();
    let warnings = pack
        .doctor_warnings
        .iter()
        .cloned()
        .map(|message| SorxDoctorIssue::warning("warning", message))
        .collect::<Vec<_>>();
    SorxDoctorReport {
        ok: errors.is_empty(),
        errors,
        warnings,
    }
}
