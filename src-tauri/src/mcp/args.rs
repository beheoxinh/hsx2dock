//! Argument extraction and validation for MCP tool calls.
//!
//! MCP hands tools an untyped `serde_json::Map`. Validating that by hand in
//! every tool would be repetitive and easy to get subtly wrong, so extraction
//! goes through [`Args`], which reports *which* argument was wrong so the agent
//! can correct itself instead of guessing.
//!
//! Pure logic only — no engine, no sockets — so the whole module is unit-tested
//! without infrastructure.

use serde_json::{Map, Value};

/// A validated view over a tool call's arguments.
pub struct Args {
    raw: Map<String, Value>,
}

/// Why an argument could not be read. The message names the argument and the
/// expected shape, because it is fed straight back to the model.
#[derive(Debug, PartialEq, Eq)]
pub struct ArgError {
    pub message: String,
}

impl ArgError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl Args {
    pub fn new(raw: Option<Map<String, Value>>) -> Self {
        Self {
            raw: raw.unwrap_or_default(),
        }
    }

    /// The raw map, for tools whose argument shape is too open to type here.
    pub fn raw(&self) -> &Map<String, Value> {
        &self.raw
    }

    fn get(&self, key: &str) -> Option<&Value> {
        // `null` is treated as absent: clients routinely send explicit nulls for
        // optional fields, and rejecting that would fail valid calls.
        match self.raw.get(key) {
            None | Some(Value::Null) => None,
            Some(v) => Some(v),
        }
    }

    /// A required string. Blank strings are rejected — they are always a
    /// mistake, never a meaningful value.
    pub fn req_str(&self, key: &str) -> Result<String, ArgError> {
        match self.get(key) {
            None => Err(ArgError::new(format!(
                "missing required argument '{key}' (expected a non-empty string)"
            ))),
            Some(Value::String(s)) if s.trim().is_empty() => {
                Err(ArgError::new(format!("argument '{key}' must not be empty")))
            }
            Some(Value::String(s)) => Ok(s.clone()),
            Some(other) => Err(ArgError::new(format!(
                "argument '{key}' must be a string, got {}",
                type_name(other)
            ))),
        }
    }

    /// An optional string. Blank is normalised to `None` so callers do not have
    /// to distinguish `""` from absent.
    pub fn opt_str(&self, key: &str) -> Result<Option<String>, ArgError> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(other) => Err(ArgError::new(format!(
                "argument '{key}' must be a string, got {}",
                type_name(other)
            ))),
        }
    }

    /// A required boolean.
    pub fn req_bool(&self, key: &str) -> Result<bool, ArgError> {
        match self.get(key) {
            None => Err(ArgError::new(format!(
                "missing required argument '{key}' (expected true or false)"
            ))),
            Some(Value::Bool(b)) => Ok(*b),
            Some(other) => Err(ArgError::new(format!(
                "argument '{key}' must be a boolean, got {}",
                type_name(other)
            ))),
        }
    }

    /// A boolean with a default, for flags whose documented default is part of
    /// the tool's contract.
    pub fn bool_or(&self, key: &str, default: bool) -> Result<bool, ArgError> {
        match self.get(key) {
            None => Ok(default),
            Some(Value::Bool(b)) => Ok(*b),
            Some(other) => Err(ArgError::new(format!(
                "argument '{key}' must be a boolean, got {}",
                type_name(other)
            ))),
        }
    }

    /// A required integer that must be zero or greater.
    pub fn req_u64(&self, key: &str) -> Result<u64, ArgError> {
        match self.get(key) {
            None => Err(ArgError::new(format!(
                "missing required argument '{key}' (expected a non-negative integer)"
            ))),
            Some(Value::Number(n)) => n.as_u64().ok_or_else(|| {
                ArgError::new(format!(
                    "argument '{key}' must be a non-negative integer, got {n}"
                ))
            }),
            Some(other) => Err(ArgError::new(format!(
                "argument '{key}' must be an integer, got {}",
                type_name(other)
            ))),
        }
    }

    /// An optional integer that must be zero or greater.
    pub fn opt_u64(&self, key: &str) -> Result<Option<u64>, ArgError> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Number(n)) => n.as_u64().map(Some).ok_or_else(|| {
                ArgError::new(format!(
                    "argument '{key}' must be a non-negative integer, got {n}"
                ))
            }),
            Some(other) => Err(ArgError::new(format!(
                "argument '{key}' must be an integer, got {}",
                type_name(other)
            ))),
        }
    }

    /// An integer with a default. Used for counts where the tool documents a
    /// fallback (e.g. log tail lines).
    pub fn u64_or(&self, key: &str, default: u64) -> Result<u64, ArgError> {
        Ok(self.opt_u64(key)?.unwrap_or(default))
    }

    /// A required array of strings; an empty array is rejected because every
    /// caller of this means "a non-empty list of things to act on".
    pub fn req_str_vec(&self, key: &str) -> Result<Vec<String>, ArgError> {
        match self.get(key) {
            None => Err(ArgError::new(format!(
                "missing required argument '{key}' (expected a non-empty array of strings)"
            ))),
            Some(Value::Array(items)) => {
                let mut out = Vec::with_capacity(items.len());
                for (i, item) in items.iter().enumerate() {
                    match item {
                        Value::String(s) if !s.trim().is_empty() => out.push(s.clone()),
                        Value::String(_) => {
                            return Err(ArgError::new(format!(
                                "argument '{key}[{i}]' must not be empty"
                            )))
                        }
                        other => {
                            return Err(ArgError::new(format!(
                                "argument '{key}[{i}]' must be a string, got {}",
                                type_name(other)
                            )))
                        }
                    }
                }
                if out.is_empty() {
                    return Err(ArgError::new(format!(
                        "argument '{key}' must contain at least one entry"
                    )));
                }
                Ok(out)
            }
            Some(other) => Err(ArgError::new(format!(
                "argument '{key}' must be an array of strings, got {}",
                type_name(other)
            ))),
        }
    }

    /// A required string→string map, for labels, build args and driver options.
    pub fn req_str_map(
        &self,
        key: &str,
    ) -> Result<std::collections::HashMap<String, String>, ArgError> {
        match self.get(key) {
            None => Ok(std::collections::HashMap::new()),
            Some(Value::Object(map)) => {
                let mut out = std::collections::HashMap::with_capacity(map.len());
                for (k, v) in map {
                    match v {
                        // Values are stringified: a label of `{"port": 8080}` is
                        // what a model naturally writes, and it is unambiguous.
                        Value::String(s) => {
                            out.insert(k.clone(), s.clone());
                        }
                        Value::Number(n) => {
                            out.insert(k.clone(), n.to_string());
                        }
                        Value::Bool(b) => {
                            out.insert(k.clone(), b.to_string());
                        }
                        Value::Null => {
                            out.insert(k.clone(), String::new());
                        }
                        other => {
                            return Err(ArgError::new(format!(
                                "argument '{key}.{k}' must be a string, number, or boolean, got {}",
                                type_name(other)
                            )))
                        }
                    }
                }
                Ok(out)
            }
            Some(other) => Err(ArgError::new(format!(
                "argument '{key}' must be an object of string values, got {}",
                type_name(other)
            ))),
        }
    }

    /// An optional raw JSON value, returned verbatim.
    pub fn opt_value(&self, key: &str) -> Option<Value> {
        self.get(key).cloned()
    }

    /// A required raw JSON object. Used for the container/image run and build
    /// specs, whose shape is large and defined by the app's own types.
    pub fn req_object(&self, key: &str) -> Result<Value, ArgError> {
        match self.get(key) {
            None => Err(ArgError::new(format!(
                "missing required argument '{key}' (expected an object)"
            ))),
            Some(v @ Value::Object(_)) => Ok(v.clone()),
            Some(other) => Err(ArgError::new(format!(
                "argument '{key}' must be an object, got {}",
                type_name(other)
            ))),
        }
    }
}

/// The JSON type of a value, for error messages the model can act on.
fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(v: Value) -> Args {
        Args::new(v.as_object().cloned())
    }

    #[test]
    fn a_required_string_is_read() {
        let a = args(json!({"id": "abc"}));
        assert_eq!(a.req_str("id").unwrap(), "abc");
    }

    #[test]
    fn a_missing_required_string_names_the_argument() {
        let a = args(json!({}));
        let err = a.req_str("id").unwrap_err();
        assert!(err.message.contains("id"), "got: {}", err.message);
        assert!(
            err.message.contains("missing required"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn a_blank_required_string_is_rejected() {
        let a = args(json!({"id": "   "}));
        assert!(a.req_str("id").is_err());
    }

    #[test]
    fn a_wrongly_typed_string_reports_the_actual_type() {
        let a = args(json!({"id": 42}));
        let err = a.req_str("id").unwrap_err();
        assert!(
            err.message.contains("must be a string"),
            "got: {}",
            err.message
        );
        assert!(err.message.contains("number"), "got: {}", err.message);
    }

    #[test]
    fn an_explicit_null_reads_as_absent() {
        let a = args(json!({"id": null}));
        assert!(
            a.req_str("id").is_err(),
            "null must not satisfy a required arg"
        );
        assert_eq!(a.opt_str("id").unwrap(), None);
    }

    #[test]
    fn a_blank_optional_string_normalises_to_none() {
        let a = args(json!({"tag": ""}));
        assert_eq!(a.opt_str("tag").unwrap(), None);
    }

    #[test]
    fn booleans_honour_their_documented_default() {
        let a = args(json!({}));
        assert!(a.bool_or("force", true).unwrap());
        assert!(!a.bool_or("force", false).unwrap());
        let a = args(json!({"force": true}));
        assert!(a.bool_or("force", false).unwrap());
        let a = args(json!({"force": "yes"}));
        assert!(a.bool_or("force", false).is_err());
    }

    #[test]
    fn integers_are_read_and_negative_values_rejected() {
        let a = args(json!({"tail": 200}));
        assert_eq!(a.u64_or("tail", 50).unwrap(), 200);
        let a = args(json!({}));
        assert_eq!(a.u64_or("tail", 50).unwrap(), 50, "default applies");
        let a = args(json!({"tail": -1}));
        assert!(
            a.opt_u64("tail").is_err(),
            "a negative count is never valid"
        );
    }

    #[test]
    fn a_float_where_an_integer_is_expected_is_rejected() {
        let a = args(json!({"tail": 1.5}));
        let err = a.opt_u64("tail").unwrap_err();
        assert!(
            err.message.contains("non-negative integer"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn a_required_string_array_reads_and_rejects_an_empty_one() {
        let a = args(json!({"ids": ["a", "b"]}));
        assert_eq!(a.req_str_vec("ids").unwrap(), vec!["a", "b"]);
        let a = args(json!({"ids": []}));
        let err = a.req_str_vec("ids").unwrap_err();
        assert!(err.message.contains("at least one"), "got: {}", err.message);
    }

    #[test]
    fn a_bad_element_in_a_string_array_reports_its_index() {
        let a = args(json!({"ids": ["a", 7]}));
        let err = a.req_str_vec("ids").unwrap_err();
        assert!(err.message.contains("ids[1]"), "got: {}", err.message);
    }

    #[test]
    fn a_string_map_coerces_scalar_values() {
        let a = args(json!({"labels": {"port": 8080, "debug": true, "note": "x"}}));
        let map = a.req_str_map("labels").unwrap();
        assert_eq!(map.get("port").unwrap(), "8080");
        assert_eq!(map.get("debug").unwrap(), "true");
        assert_eq!(map.get("note").unwrap(), "x");
    }

    #[test]
    fn an_absent_map_is_empty_rather_than_an_error() {
        let a = args(json!({}));
        assert!(a.req_str_map("labels").unwrap().is_empty());
    }

    #[test]
    fn a_nested_value_in_a_string_map_is_rejected() {
        let a = args(json!({"labels": {"bad": {"nested": 1}}}));
        let err = a.req_str_map("labels").unwrap_err();
        assert!(err.message.contains("object"), "got: {}", err.message);
    }

    #[test]
    fn a_required_object_is_passed_through_verbatim() {
        let spec = json!({"image": "nginx", "ports": [{"host": 8080}]});
        let a = args(json!({"config": spec}));
        assert_eq!(a.req_object("config").unwrap(), spec);
    }

    #[test]
    fn a_missing_required_object_is_an_error() {
        let a = args(json!({}));
        assert!(a.req_object("config").is_err());
    }
}
