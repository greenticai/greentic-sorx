//! Validate model CRUD against its own compiled record, never a command input.
use super::{ManagerModelRecord, SorxError, SorxResult, Value};

pub(super) fn validate(record: &ManagerModelRecord, input: &Value) -> SorxResult<()> {
    let object = input
        .as_object()
        .ok_or_else(|| SorxError::new("invalid_input", "record must be an object"))?;
    let invalid = |field: &str, reason: &str| SorxError::at_path("invalid_input", reason, field);
    for field in &record.fields {
        let value = object.get(&field.name).filter(|value| !value.is_null());
        let Some(value) = value else {
            if field.required && !field.generated {
                return Err(invalid(&field.name, "a value is required"));
            }
            continue;
        };
        let valid = match field
            .json_type
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some(
                "string" | "uuid" | "date" | "datetime" | "timestamp" | "time" | "email" | "url"
                | "enum",
            ) => value.is_string(),
            Some("integer" | "int" | "u32") => value.is_i64() || value.is_u64(),
            Some("number" | "decimal" | "float" | "double") => value.is_number(),
            Some("boolean" | "bool") => value.is_boolean(),
            Some("array") => value.is_array(),
            Some("object") => value.is_object(),
            _ => true,
        };
        if !valid {
            return Err(invalid(
                &field.name,
                "value has the wrong type for this record field",
            ));
        }
    }
    Ok(())
}

/// Check client-supplied fields before the runtime adds generated values.
pub(super) fn validate_supplied(
    record: &ManagerModelRecord,
    input: &Value,
    body: &serde_json::Map<String, Value>,
) -> SorxResult<()> {
    for name in input
        .as_object()
        .into_iter()
        .flat_map(|object| object.keys())
    {
        let Some(field) = record.fields.iter().find(|field| field.name == *name) else {
            return Err(SorxError::at_path(
                "invalid_input",
                "field is not present in this record model",
                name,
            ));
        };
        if field.read_only {
            return Err(SorxError::at_path(
                "invalid_input",
                "field is read only",
                name,
            ));
        }
    }
    for field in &record.fields {
        if field.read_only && body.contains_key(&field.name) {
            return Err(SorxError::at_path(
                "invalid_input",
                "field is read only",
                &field.name,
            ));
        }
    }
    Ok(())
}
