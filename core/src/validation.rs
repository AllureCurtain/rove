use serde_json::Value;

use crate::ToolError;

/// Validate tool arguments against the bounded JSON Schema subset used by
/// first-party and embedded tools.
pub fn validate_tool_args(schema: &Value, value: &Value) -> Result<(), ToolError> {
    validate_value(schema, value, "arguments")
}

fn validate_value(schema: &Value, value: &Value, path: &str) -> Result<(), ToolError> {
    if let Some(expected) = schema.get("type").and_then(Value::as_str)
        && !matches_type(value, expected)
    {
        let reason = if path == "arguments" && expected == "object" {
            "tool arguments must be a JSON object".to_string()
        } else {
            format!("Argument {path} must be {expected}")
        };
        return invalid(reason);
    }

    if let Some(values) = schema.get("enum").and_then(Value::as_array)
        && !values.iter().any(|candidate| candidate == value)
    {
        return invalid(format!("Argument {path} must match one of the enum values"));
    }

    validate_scalar_bounds(schema, value, path)?;
    match schema.get("type").and_then(Value::as_str) {
        Some("object") => validate_object(schema, value, path),
        Some("array") => validate_array(schema, value, path),
        _ => Ok(()),
    }
}

fn validate_object(schema: &Value, value: &Value, path: &str) -> Result<(), ToolError> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for field in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(field) {
                return invalid(format!("Missing required argument: {}", child(path, field)));
            }
        }
    }

    let properties = schema.get("properties").and_then(Value::as_object);
    if schema.get("additionalProperties").and_then(Value::as_bool) == Some(false) {
        for field in object.keys() {
            if !properties.is_some_and(|properties| properties.contains_key(field)) {
                return invalid(format!(
                    "Argument {} is not allowed by additionalProperties=false",
                    child(path, field)
                ));
            }
        }
    }
    if let Some(properties) = properties {
        for (field, child_schema) in properties {
            if let Some(child_value) = object.get(field) {
                validate_value(child_schema, child_value, &child(path, field))?;
            }
        }
    }
    Ok(())
}

fn validate_array(schema: &Value, value: &Value, path: &str) -> Result<(), ToolError> {
    let Some(items) = value.as_array() else {
        return Ok(());
    };
    if let Some(minimum) = schema.get("minItems").and_then(Value::as_u64)
        && items.len() < minimum as usize
    {
        return invalid(format!(
            "Argument {path} must have at least {minimum} items"
        ));
    }
    if let Some(maximum) = schema.get("maxItems").and_then(Value::as_u64)
        && items.len() > maximum as usize
    {
        return invalid(format!("Argument {path} must have at most {maximum} items"));
    }
    if let Some(item_schema) = schema.get("items") {
        for (index, item) in items.iter().enumerate() {
            validate_value(item_schema, item, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

fn validate_scalar_bounds(schema: &Value, value: &Value, path: &str) -> Result<(), ToolError> {
    if let Some(text) = value.as_str() {
        let length = text.chars().count();
        if let Some(minimum) = schema.get("minLength").and_then(Value::as_u64)
            && length < minimum as usize
        {
            return invalid(format!(
                "Argument {path} must have at least {minimum} characters"
            ));
        }
        if let Some(maximum) = schema.get("maxLength").and_then(Value::as_u64)
            && length > maximum as usize
        {
            return invalid(format!(
                "Argument {path} must have at most {maximum} characters"
            ));
        }
    }
    if let Some(number) = value.as_f64() {
        if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64)
            && number < minimum
        {
            return invalid(format!(
                "Argument {path} must be greater than or equal to minimum {minimum}"
            ));
        }
        if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64)
            && number > maximum
        {
            return invalid(format!(
                "Argument {path} must be less than or equal to maximum {maximum}"
            ));
        }
    }
    Ok(())
}

fn child(path: &str, field: &str) -> String {
    if path == "arguments" {
        field.to_string()
    } else {
        format!("{path}.{field}")
    }
}

fn matches_type(value: &Value, expected: &str) -> bool {
    match expected {
        "array" => value.is_array(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "object" => value.is_object(),
        "string" => value.is_string(),
        _ => true,
    }
}

fn invalid<T>(reason: String) -> Result<T, ToolError> {
    Err(ToolError::InvalidArgs { reason })
}

#[cfg(test)]
mod tests {
    use super::validate_tool_args;

    #[test]
    fn rejects_nested_invalid_arguments() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "options": {
                    "type": "object",
                    "properties": { "level": { "type": "number", "maximum": 3.0 } },
                    "required": ["level"],
                    "additionalProperties": false
                }
            },
            "required": ["options"],
            "additionalProperties": false
        });

        let error =
            validate_tool_args(&schema, &serde_json::json!({"options":{"level":4.0}})).unwrap_err();

        assert!(error.to_string().contains("options.level"));
    }
}
