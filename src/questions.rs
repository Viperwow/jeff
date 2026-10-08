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

/// Turns the request's `questions` into a map: a map passes through, an array names saved questions.
pub fn expand(input: &Value, saved: &Questions) -> Result<Questions, String> {
    let out = match input {
        Value::Object(map) => map.clone(),
        Value::Array(keys) => {
            let mut out = Questions::new();
            for key in keys {
                let Some(key) = key.as_str() else {
                    return Err("saved question keys must be strings".into());
                };
                let q = saved
                    .get(key)
                    .ok_or_else(|| format!("unknown question '{key}'"))?;
                out.insert(key.to_owned(), q.clone());
            }
            out
        }
        _ => return Err("questions must be a map or an array of saved keys".into()),
    };
    if out.is_empty() {
        return Err("questions must not be empty".into());
    }
    Ok(out)
}

/// Each model costs one upstream call, buffered in full, so a request cannot fan out without bound.
pub const MAX_MODELS: usize = 8;

/// Splits questions by the model that answers them; providers never see the `model` field.
pub fn group(
    questions: Questions,
    request_model: &str,
) -> Result<Vec<(String, Questions)>, String> {
    let mut groups: Vec<(String, Questions)> = Vec::new();
    for (key, mut q) in questions {
        let model = q
            .as_object_mut()
            .and_then(|o| o.remove("model"))
            .and_then(|m| m.as_str().map(str::to_owned))
            .unwrap_or_else(|| request_model.to_owned());
        match groups.iter_mut().find(|(m, _)| *m == model) {
            Some((_, qs)) => {
                qs.insert(key, q);
            }
            None => groups.push((model, Questions::from_iter([(key, q)]))),
        }
    }
    if groups.len() > MAX_MODELS {
        return Err(format!(
            "questions use {} models; one request may use at most {MAX_MODELS}",
            groups.len()
        ));
    }
    Ok(groups)
}

/// Joins the responses of several groups into one, as if a single provider had answered.
pub fn merge(parts: Vec<Value>) -> Value {
    let mut parts = parts.into_iter();
    let mut out = parts
        .next()
        .unwrap_or_else(|| Value::Object(Questions::new()));
    let Some(obj) = out.as_object_mut() else {
        return out;
    };
    for part in parts {
        let Value::Object(part) = part else { continue };
        for (field, value) in part {
            match (field.as_str(), obj.get_mut(&field)) {
                ("answers", Some(Value::Object(answers))) => {
                    if let Value::Object(more) = value {
                        answers.extend(more);
                    }
                }
                ("usage", Some(Value::Object(usage))) => {
                    for (k, v) in value.as_object().into_iter().flatten() {
                        let sum = usage.get(k).and_then(Value::as_f64).unwrap_or(0.0)
                            + v.as_f64().unwrap_or(0.0);
                        let sum = match v.is_u64() && usage.get(k).is_none_or(Value::is_u64) {
                            true => Value::from(sum as u64),
                            false => Value::from(sum),
                        };
                        usage.insert(k.clone(), sum);
                    }
                }
                ("model", Some(Value::String(model))) => {
                    if let Some(m) = value.as_str() {
                        model.push_str(", ");
                        model.push_str(m);
                    }
                }
                (_, None) => {
                    obj.insert(field, value);
                }
                _ => {}
            }
        }
    }
    out
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

    fn map(v: Value) -> Questions {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn expands_saved_keys() {
        let saved = map(json!({"u": {"type": "noul", "instructions": "x"}}));
        assert_eq!(expand(&json!(["u", "u"]), &saved).unwrap(), saved);
        assert_eq!(
            expand(&json!(["nope"]), &saved).unwrap_err(),
            "unknown question 'nope'"
        );
        assert_eq!(
            expand(&json!([]), &saved).unwrap_err(),
            "questions must not be empty"
        );
        assert_eq!(
            expand(&json!({}), &saved).unwrap_err(),
            "questions must not be empty"
        );
        assert!(expand(&json!("u"), &saved).is_err());
        assert!(expand(&json!([1]), &saved).is_err());
        let inline = json!({"i": {"type": "noul", "instructions": "y"}});
        assert_eq!(Value::Object(expand(&inline, &saved).unwrap()), inline);
    }

    #[test]
    fn groups_by_model_and_strips_it() {
        let qs = map(json!({
            "a": {"type": "noul", "instructions": "x"},
            "b": {"type": "noul", "instructions": "x", "model": "typesafe/jev-latest"},
            "c": {"type": "noul", "instructions": "x", "model": "clm/clm-latest"},
        }));
        let groups = group(qs, "clm/clm-latest").unwrap();
        let names: Vec<_> = groups
            .iter()
            .map(|(m, q)| (m.as_str(), q.keys().cloned().collect::<Vec<_>>()))
            .collect();
        assert_eq!(
            names,
            [
                ("clm/clm-latest", vec!["a".to_owned(), "c".to_owned()]),
                ("typesafe/jev-latest", vec!["b".to_owned()]),
            ]
        );
        assert!(
            groups
                .iter()
                .all(|(_, q)| q.values().all(|v| v.get("model").is_none()))
        );
    }

    #[test]
    fn limits_models_per_request() {
        let many = |n: usize| {
            (0..n)
                .map(|i| {
                    let q =
                        json!({"type": "noul", "instructions": "x", "model": format!("p/m{i}")});
                    (format!("q{i}"), q)
                })
                .collect::<Questions>()
        };
        assert_eq!(group(many(MAX_MODELS), "p/m0").unwrap().len(), MAX_MODELS);
        assert_eq!(
            group(many(MAX_MODELS + 1), "p/m0").unwrap_err(),
            format!(
                "questions use {} models; one request may use at most {MAX_MODELS}",
                MAX_MODELS + 1
            )
        );
    }

    #[test]
    fn merges_group_answers() {
        let merged = merge(vec![
            json!({"model": "clm-latest", "answers": {"a": {"noul": 0.9}},
                "usage": {"input_tokens": 10}, "id": "x"}),
            json!({"model": "jev-latest", "answers": {"b": {"noul": 0.1}},
                "usage": {"input_tokens": 5}}),
        ]);
        assert_eq!(
            merged,
            json!({"model": "clm-latest, jev-latest", "id": "x",
                "answers": {"a": {"noul": 0.9}, "b": {"noul": 0.1}},
                "usage": {"input_tokens": 15}})
        );
    }

    #[test]
    fn validate_all_rejects_empty_input() {
        assert_eq!(
            validate_all(&Questions::new()).unwrap_err(),
            "no questions given"
        );
    }
}
