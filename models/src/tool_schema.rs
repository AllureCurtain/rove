use std::collections::BTreeSet;

use serde_json::{Map, Value};
use thiserror::Error;

use crate::{MAX_TOOL_NAME_BYTES, ModelToolSchema};

pub const MAX_MODEL_TOOLS: usize = 128;
pub const MAX_TOOL_DESCRIPTION_BYTES: usize = 4 * 1024;
pub const MAX_TOOL_SCHEMA_BYTES: usize = 64 * 1024;
pub const MAX_TOOL_SCHEMA_DEPTH: usize = 16;
pub const MAX_TOOL_SCHEMA_NODES: usize = 1_024;
pub const MAX_TOOL_SCHEMA_PROPERTIES: usize = 256;
pub const MAX_TOOL_SCHEMA_REQUIRED_FIELDS: usize = 256;
pub const MAX_TOOL_SCHEMA_ENUM_VALUES: usize = 128;
pub const MAX_TOOL_SCHEMA_PROPERTY_NAME_BYTES: usize = 256;

const MAX_SCHEMA_BOUND: u64 = 1024 * 1024;
const ALLOWED_KEYWORDS: &[&str] = &[
    "additionalProperties",
    "default",
    "description",
    "enum",
    "items",
    "maxItems",
    "maxLength",
    "maximum",
    "minItems",
    "minLength",
    "minimum",
    "properties",
    "required",
    "type",
];
const SUPPORTED_TYPES: &[&str] = &[
    "array", "boolean", "integer", "null", "number", "object", "string",
];

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ToolSchemaValidationError {
    #[error("too many model tools; maximum is {max}")]
    TooManyTools { max: usize },
    #[error("tool name must not be empty")]
    EmptyName,
    #[error("tool name exceeds {max} bytes")]
    NameTooLarge { max: usize },
    #[error("tool name `{name}` contains unsupported characters")]
    InvalidName { name: String },
    #[error("duplicate tool name `{name}`")]
    DuplicateName { name: String },
    #[error("tool `{name}` description exceeds {max} bytes")]
    DescriptionTooLarge { name: String, max: usize },
    #[error("tool `{name}` schema exceeds {max} bytes")]
    SchemaTooLarge { name: String, max: usize },
    #[error("tool `{name}` schema root must be an object schema")]
    RootMustBeObject { name: String },
    #[error("tool `{name}` schema at {path} must be a JSON object")]
    SchemaNodeMustBeObject { name: String, path: String },
    #[error("tool `{name}` schema exceeds maximum depth {max} at {path}")]
    SchemaTooDeep {
        name: String,
        path: String,
        max: usize,
    },
    #[error("tool `{name}` schema exceeds maximum node count {max}")]
    TooManySchemaNodes { name: String, max: usize },
    #[error("tool `{name}` schema uses unsupported keyword `{keyword}` at {path}")]
    UnsupportedKeyword {
        name: String,
        path: String,
        keyword: String,
    },
    #[error("tool `{name}` schema has invalid `{keyword}` at {path}: {reason}")]
    InvalidKeyword {
        name: String,
        path: String,
        keyword: &'static str,
        reason: String,
    },
}

impl ModelToolSchema {
    pub fn validate(&self) -> Result<(), ToolSchemaValidationError> {
        validate_name(&self.name)?;
        if self.description.len() > MAX_TOOL_DESCRIPTION_BYTES {
            return Err(ToolSchemaValidationError::DescriptionTooLarge {
                name: self.name.clone(),
                max: MAX_TOOL_DESCRIPTION_BYTES,
            });
        }
        let encoded = serde_json::to_vec(&self.parameters).map_err(|_| {
            ToolSchemaValidationError::SchemaTooLarge {
                name: self.name.clone(),
                max: MAX_TOOL_SCHEMA_BYTES,
            }
        })?;
        if encoded.len() > MAX_TOOL_SCHEMA_BYTES {
            return Err(ToolSchemaValidationError::SchemaTooLarge {
                name: self.name.clone(),
                max: MAX_TOOL_SCHEMA_BYTES,
            });
        }
        if self.parameters.get("type").and_then(Value::as_str) != Some("object") {
            return Err(ToolSchemaValidationError::RootMustBeObject {
                name: self.name.clone(),
            });
        }

        let mut budget = SchemaBudget::default();
        validate_schema_node(&self.name, &self.parameters, "parameters", 1, &mut budget)
    }
}

pub fn validate_model_tools(tools: &[ModelToolSchema]) -> Result<(), ToolSchemaValidationError> {
    if tools.len() > MAX_MODEL_TOOLS {
        return Err(ToolSchemaValidationError::TooManyTools {
            max: MAX_MODEL_TOOLS,
        });
    }
    let mut names = BTreeSet::new();
    for tool in tools {
        tool.validate()?;
        if !names.insert(tool.name.as_str()) {
            return Err(ToolSchemaValidationError::DuplicateName {
                name: tool.name.clone(),
            });
        }
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), ToolSchemaValidationError> {
    if name.trim().is_empty() {
        return Err(ToolSchemaValidationError::EmptyName);
    }
    if name.len() > MAX_TOOL_NAME_BYTES {
        return Err(ToolSchemaValidationError::NameTooLarge {
            max: MAX_TOOL_NAME_BYTES,
        });
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(ToolSchemaValidationError::InvalidName {
            name: name.to_string(),
        });
    }
    Ok(())
}

#[derive(Default)]
struct SchemaBudget {
    nodes: usize,
    properties: usize,
    required: usize,
    enum_values: usize,
}

fn validate_schema_node(
    tool_name: &str,
    schema: &Value,
    path: &str,
    depth: usize,
    budget: &mut SchemaBudget,
) -> Result<(), ToolSchemaValidationError> {
    if depth > MAX_TOOL_SCHEMA_DEPTH {
        return Err(ToolSchemaValidationError::SchemaTooDeep {
            name: tool_name.to_string(),
            path: path.to_string(),
            max: MAX_TOOL_SCHEMA_DEPTH,
        });
    }
    budget.nodes += 1;
    if budget.nodes > MAX_TOOL_SCHEMA_NODES {
        return Err(ToolSchemaValidationError::TooManySchemaNodes {
            name: tool_name.to_string(),
            max: MAX_TOOL_SCHEMA_NODES,
        });
    }
    let object =
        schema
            .as_object()
            .ok_or_else(|| ToolSchemaValidationError::SchemaNodeMustBeObject {
                name: tool_name.to_string(),
                path: path.to_string(),
            })?;
    for keyword in object.keys() {
        if !ALLOWED_KEYWORDS.contains(&keyword.as_str()) {
            return Err(ToolSchemaValidationError::UnsupportedKeyword {
                name: tool_name.to_string(),
                path: path.to_string(),
                keyword: keyword.clone(),
            });
        }
    }

    let schema_type = required_type(tool_name, object, path)?;
    validate_description(tool_name, object, path)?;
    validate_enum(tool_name, object, path, schema_type, budget)?;
    validate_default(tool_name, object, path, schema_type)?;

    match schema_type {
        "object" => validate_object_schema(tool_name, object, path, depth, budget),
        "array" => validate_array_schema(tool_name, object, path, depth, budget),
        "string" => validate_string_schema(tool_name, object, path),
        "number" | "integer" => validate_number_schema(tool_name, object, path),
        "boolean" | "null" => reject_keywords_for_scalar(tool_name, object, path),
        _ => unreachable!("required_type accepts only supported types"),
    }
}

fn required_type<'a>(
    tool_name: &str,
    object: &'a Map<String, Value>,
    path: &str,
) -> Result<&'a str, ToolSchemaValidationError> {
    let Some(schema_type) = object.get("type").and_then(Value::as_str) else {
        return invalid_keyword(tool_name, path, "type", "must be a supported string");
    };
    if !SUPPORTED_TYPES.contains(&schema_type) {
        return invalid_keyword(tool_name, path, "type", "is not supported");
    }
    Ok(schema_type)
}

fn validate_description(
    tool_name: &str,
    object: &Map<String, Value>,
    path: &str,
) -> Result<(), ToolSchemaValidationError> {
    if let Some(description) = object.get("description") {
        let Some(description) = description.as_str() else {
            return invalid_keyword(tool_name, path, "description", "must be a string");
        };
        if description.len() > MAX_TOOL_DESCRIPTION_BYTES {
            return invalid_keyword(tool_name, path, "description", "exceeds the supported size");
        }
    }
    Ok(())
}

fn validate_object_schema(
    tool_name: &str,
    object: &Map<String, Value>,
    path: &str,
    depth: usize,
    budget: &mut SchemaBudget,
) -> Result<(), ToolSchemaValidationError> {
    reject_present(
        tool_name,
        object,
        path,
        &[
            "items",
            "minItems",
            "maxItems",
            "minLength",
            "maxLength",
            "minimum",
            "maximum",
        ],
    )?;
    let properties = match object.get("properties") {
        Some(Value::Object(properties)) => Some(properties),
        Some(_) => return invalid_keyword(tool_name, path, "properties", "must be an object"),
        None => None,
    };
    budget.properties = budget
        .properties
        .saturating_add(properties.map_or(0, Map::len));
    if budget.properties > MAX_TOOL_SCHEMA_PROPERTIES {
        return invalid_keyword(
            tool_name,
            path,
            "properties",
            "exceeds the catalog property limit",
        );
    }
    if let Some(properties) = properties {
        for (name, child_schema) in properties {
            if name.is_empty() || name.len() > MAX_TOOL_SCHEMA_PROPERTY_NAME_BYTES {
                return invalid_keyword(
                    tool_name,
                    path,
                    "properties",
                    "contains an empty or oversized property name",
                );
            }
            validate_schema_node(
                tool_name,
                child_schema,
                &format!("{path}.properties.{name}"),
                depth + 1,
                budget,
            )?;
        }
    }

    let mut required_names = BTreeSet::new();
    if let Some(required) = object.get("required") {
        let Some(required) = required.as_array() else {
            return invalid_keyword(tool_name, path, "required", "must be an array");
        };
        budget.required = budget.required.saturating_add(required.len());
        if budget.required > MAX_TOOL_SCHEMA_REQUIRED_FIELDS {
            return invalid_keyword(
                tool_name,
                path,
                "required",
                "exceeds the catalog required-field limit",
            );
        }
        for field in required {
            let Some(field) = field.as_str() else {
                return invalid_keyword(tool_name, path, "required", "must contain only strings");
            };
            if !properties.is_some_and(|properties| properties.contains_key(field)) {
                return invalid_keyword(
                    tool_name,
                    path,
                    "required",
                    "must reference declared properties",
                );
            }
            if !required_names.insert(field) {
                return invalid_keyword(tool_name, path, "required", "contains duplicates");
            }
        }
    }
    if let Some(additional) = object.get("additionalProperties")
        && !additional.is_boolean()
    {
        return invalid_keyword(tool_name, path, "additionalProperties", "must be a boolean");
    }
    Ok(())
}

fn validate_array_schema(
    tool_name: &str,
    object: &Map<String, Value>,
    path: &str,
    depth: usize,
    budget: &mut SchemaBudget,
) -> Result<(), ToolSchemaValidationError> {
    reject_present(
        tool_name,
        object,
        path,
        &[
            "properties",
            "required",
            "additionalProperties",
            "minLength",
            "maxLength",
            "minimum",
            "maximum",
        ],
    )?;
    let items = object
        .get("items")
        .ok_or_else(|| ToolSchemaValidationError::InvalidKeyword {
            name: tool_name.to_string(),
            path: path.to_string(),
            keyword: "items",
            reason: "is required for array schemas".to_string(),
        })?;
    validate_schema_node(
        tool_name,
        items,
        &format!("{path}.items"),
        depth + 1,
        budget,
    )?;
    validate_u64_bounds(tool_name, object, path, "minItems", "maxItems")
}

fn validate_string_schema(
    tool_name: &str,
    object: &Map<String, Value>,
    path: &str,
) -> Result<(), ToolSchemaValidationError> {
    reject_present(
        tool_name,
        object,
        path,
        &[
            "properties",
            "required",
            "additionalProperties",
            "items",
            "minItems",
            "maxItems",
            "minimum",
            "maximum",
        ],
    )?;
    validate_u64_bounds(tool_name, object, path, "minLength", "maxLength")
}

fn validate_number_schema(
    tool_name: &str,
    object: &Map<String, Value>,
    path: &str,
) -> Result<(), ToolSchemaValidationError> {
    reject_present(
        tool_name,
        object,
        path,
        &[
            "properties",
            "required",
            "additionalProperties",
            "items",
            "minItems",
            "maxItems",
            "minLength",
            "maxLength",
        ],
    )?;
    let minimum = optional_f64(tool_name, object, path, "minimum")?;
    let maximum = optional_f64(tool_name, object, path, "maximum")?;
    if minimum.zip(maximum).is_some_and(|(min, max)| min > max) {
        return invalid_keyword(tool_name, path, "minimum", "must not exceed maximum");
    }
    Ok(())
}

fn reject_keywords_for_scalar(
    tool_name: &str,
    object: &Map<String, Value>,
    path: &str,
) -> Result<(), ToolSchemaValidationError> {
    reject_present(
        tool_name,
        object,
        path,
        &[
            "properties",
            "required",
            "additionalProperties",
            "items",
            "minItems",
            "maxItems",
            "minLength",
            "maxLength",
            "minimum",
            "maximum",
        ],
    )
}

fn validate_enum(
    tool_name: &str,
    object: &Map<String, Value>,
    path: &str,
    schema_type: &str,
    budget: &mut SchemaBudget,
) -> Result<(), ToolSchemaValidationError> {
    let Some(values) = object.get("enum") else {
        return Ok(());
    };
    let Some(values) = values.as_array() else {
        return invalid_keyword(tool_name, path, "enum", "must be an array");
    };
    if values.is_empty() {
        return invalid_keyword(tool_name, path, "enum", "must not be empty");
    }
    budget.enum_values = budget.enum_values.saturating_add(values.len());
    if budget.enum_values > MAX_TOOL_SCHEMA_ENUM_VALUES {
        return invalid_keyword(tool_name, path, "enum", "exceeds the catalog enum limit");
    }
    let mut unique = BTreeSet::new();
    for value in values {
        if !matches_type(value, schema_type) {
            return invalid_keyword(
                tool_name,
                path,
                "enum",
                "contains a value of the wrong type",
            );
        }
        let encoded = serde_json::to_string(value).unwrap_or_default();
        if !unique.insert(encoded) {
            return invalid_keyword(tool_name, path, "enum", "contains duplicates");
        }
    }
    Ok(())
}

fn validate_default(
    tool_name: &str,
    object: &Map<String, Value>,
    path: &str,
    schema_type: &str,
) -> Result<(), ToolSchemaValidationError> {
    if let Some(value) = object.get("default")
        && !matches_type(value, schema_type)
    {
        return invalid_keyword(tool_name, path, "default", "has the wrong type");
    }
    Ok(())
}

fn validate_u64_bounds(
    tool_name: &str,
    object: &Map<String, Value>,
    path: &str,
    minimum_key: &'static str,
    maximum_key: &'static str,
) -> Result<(), ToolSchemaValidationError> {
    let minimum = optional_u64(tool_name, object, path, minimum_key)?;
    let maximum = optional_u64(tool_name, object, path, maximum_key)?;
    if minimum.zip(maximum).is_some_and(|(min, max)| min > max) {
        return invalid_keyword(tool_name, path, minimum_key, "must not exceed the maximum");
    }
    Ok(())
}

fn optional_u64(
    tool_name: &str,
    object: &Map<String, Value>,
    path: &str,
    key: &'static str,
) -> Result<Option<u64>, ToolSchemaValidationError> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_u64() else {
        return invalid_keyword(tool_name, path, key, "must be a non-negative integer");
    };
    if value > MAX_SCHEMA_BOUND {
        return invalid_keyword(tool_name, path, key, "exceeds the supported bound");
    }
    Ok(Some(value))
}

fn optional_f64(
    tool_name: &str,
    object: &Map<String, Value>,
    path: &str,
    key: &'static str,
) -> Result<Option<f64>, ToolSchemaValidationError> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    value
        .as_f64()
        .map(Some)
        .ok_or_else(|| ToolSchemaValidationError::InvalidKeyword {
            name: tool_name.to_string(),
            path: path.to_string(),
            keyword: key,
            reason: "must be a number".to_string(),
        })
}

fn reject_present(
    tool_name: &str,
    object: &Map<String, Value>,
    path: &str,
    keywords: &[&'static str],
) -> Result<(), ToolSchemaValidationError> {
    if let Some(keyword) = keywords
        .iter()
        .find(|keyword| object.contains_key(**keyword))
    {
        return invalid_keyword(
            tool_name,
            path,
            keyword,
            "does not apply to this schema type",
        );
    }
    Ok(())
}

fn matches_type(value: &Value, schema_type: &str) -> bool {
    match schema_type {
        "array" => value.is_array(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "object" => value.is_object(),
        "string" => value.is_string(),
        _ => false,
    }
}

fn invalid_keyword<T>(
    tool_name: &str,
    path: &str,
    keyword: &'static str,
    reason: impl Into<String>,
) -> Result<T, ToolSchemaValidationError> {
    Err(ToolSchemaValidationError::InvalidKeyword {
        name: tool_name.to_string(),
        path: path.to_string(),
        keyword,
        reason: reason.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(parameters: Value) -> ModelToolSchema {
        ModelToolSchema {
            name: "inspect".to_string(),
            description: "Inspect the workspace".to_string(),
            parameters,
        }
    }

    #[test]
    fn accepts_the_executable_schema_subset() {
        schema(serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "minLength": 1, "maxLength": 200},
                "options": {
                    "type": "object",
                    "properties": {
                        "depth": {"type": "integer", "minimum": 0, "maximum": 8},
                        "mode": {"type": "string", "enum": ["fast", "full"], "default": "fast"},
                        "tags": {"type": "array", "items": {"type": "string"}, "maxItems": 8}
                    },
                    "required": ["mode"],
                    "additionalProperties": false
                }
            },
            "required": ["path"],
            "additionalProperties": false
        }))
        .validate()
        .unwrap();
    }

    #[test]
    fn rejects_unsupported_or_unenforceable_schema_shapes() {
        let unsupported = schema(serde_json::json!({
            "type": "object",
            "properties": {},
            "oneOf": [{"type": "object", "properties": {}}]
        }))
        .validate()
        .unwrap_err();
        assert!(matches!(
            unsupported,
            ToolSchemaValidationError::UnsupportedKeyword { .. }
        ));

        let undeclared = schema(serde_json::json!({
            "type": "object",
            "properties": {},
            "required": ["missing"]
        }))
        .validate()
        .unwrap_err();
        assert!(matches!(
            undeclared,
            ToolSchemaValidationError::InvalidKeyword {
                keyword: "required",
                ..
            }
        ));
    }

    #[test]
    fn enforces_depth_size_and_catalog_limits() {
        let mut nested = serde_json::json!({"type": "string"});
        for _ in 0..MAX_TOOL_SCHEMA_DEPTH {
            nested = serde_json::json!({
                "type": "object",
                "properties": {"next": nested}
            });
        }
        assert!(matches!(
            schema(nested).validate(),
            Err(ToolSchemaValidationError::SchemaTooDeep { .. })
        ));

        let mut oversized = schema(serde_json::json!({
            "type": "object",
            "properties": {}
        }));
        oversized.description = "x".repeat(MAX_TOOL_DESCRIPTION_BYTES + 1);
        assert!(matches!(
            oversized.validate(),
            Err(ToolSchemaValidationError::DescriptionTooLarge { .. })
        ));

        let tools = (0..=MAX_MODEL_TOOLS)
            .map(|index| ModelToolSchema {
                name: format!("tool_{index}"),
                description: String::new(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            validate_model_tools(&tools),
            Err(ToolSchemaValidationError::TooManyTools {
                max: MAX_MODEL_TOOLS
            })
        );
    }

    #[test]
    fn rejects_duplicate_or_nonportable_tool_names() {
        let first = schema(serde_json::json!({"type": "object", "properties": {}}));
        let duplicate = first.clone();
        assert!(matches!(
            validate_model_tools(&[first, duplicate]),
            Err(ToolSchemaValidationError::DuplicateName { .. })
        ));

        let mut invalid = schema(serde_json::json!({"type": "object", "properties": {}}));
        invalid.name = "mcp tool".to_string();
        assert!(matches!(
            invalid.validate(),
            Err(ToolSchemaValidationError::InvalidName { .. })
        ));
    }
}
