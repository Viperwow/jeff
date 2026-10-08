use serde_json::Value;

/// Saved questions by key, in the shape `/v1/systemone` takes, plus an optional `model`.
pub type Questions = serde_json::Map<String, Value>;

/// Checks the fields jeff relies on; any other field passes through to the provider.
pub fn validate(key: &str, q: &Value) -> Result<(), String> {
    check(key, q).map_err(|e| format!("question '{key}': {e}"))
}

fn check(key: &str, q: &Value) -> Result<(), &'static str> {
    // The key appears in URL paths.
    if key.is_empty()
        || key.len() > 64
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("key must be 1-64 characters of A-Z, a-z, 0-9, '_' and '-'");
    }
    let Some(q) = q.as_object() else {
        return Err("must be an object");
    };
    if !q
        .get("instructions")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty())
    {
        return Err("instructions must be a non-empty string");
    }
    let criteria = q.get("criteria");
    match q.get("type").and_then(Value::as_str) {
        Some("noul") => {}
        Some("choice") => {
            if !criteria
                .and_then(Value::as_object)
                .is_some_and(|c| c.len() >= 2)
            {
                return Err("choice criteria must be an object with at least two options");
            }
        }
        Some("score") => {
            if !criteria
                .and_then(Value::as_array)
                .is_some_and(|c| c.len() >= 2)
            {
                return Err("score criteria must be an array with at least two levels");
            }
        }
        _ => return Err("type must be noul, choice or score"),
    }
    if q.get("model")
        .is_some_and(|m| m.as_str().is_none_or(|m| m.trim().is_empty()))
    {
        return Err("model must be a non-empty string");
    }
    Ok(())
}

pub fn validate_all(input: &Questions) -> Result<(), String> {
    if input.is_empty() {
        return Err("no questions given".into());
    }
    input.iter().try_for_each(|(k, q)| validate(k, q))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_native_questions() {
        assert!(
            validate(
                "urgency",
                &json!({"type": "noul", "instructions": "Urgent?"})
            )
            .is_ok()
        );
        assert!(
            validate(
                "dept",
                &json!({"type": "choice", "instructions": "Team?",
                    "criteria": {"a": "A", "b": "B"}, "model": "clm/clm-latest", "extra": 1})
            )
            .is_ok()
        );
        assert!(
            validate(
                "anger",
                &json!({"type": "score", "instructions": "How?", "criteria": ["Calm", "Angry"]})
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_broken_questions() {
        let long = "k".repeat(65);
        let bad = [
            ("a/b", json!({"type": "noul", "instructions": "x"})),
            ("a b", json!({"type": "noul", "instructions": "x"})),
            ("", json!({"type": "noul", "instructions": "x"})),
            (long.as_str(), json!({"type": "noul", "instructions": "x"})),
            ("k", json!({"type": "maybe", "instructions": "x"})),
            ("k", json!({"type": "noul", "instructions": " "})),
            (
                "k",
                json!({"type": "choice", "instructions": "x", "criteria": {"a": "A"}}),
            ),
            (
                "k",
                json!({"type": "choice", "instructions": "x", "criteria": ["a", "b"]}),
            ),
            (
                "k",
                json!({"type": "score", "instructions": "x", "criteria": ["one"]}),
            ),
            (
                "k",
                json!({"type": "noul", "instructions": "x", "model": ""}),
            ),
            ("k", json!("not an object")),
        ];
        for (key, q) in bad {
            let e = validate(key, &q).unwrap_err();
            assert!(e.starts_with(&format!("question '{key}': ")), "{e}");
        }
    }

    #[test]
    fn validate_all_rejects_empty_input() {
        assert_eq!(
            validate_all(&Questions::new()).unwrap_err(),
            "no questions given"
        );
    }
}
