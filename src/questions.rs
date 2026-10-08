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

/// Why a change to the saved questions was refused; each carries the message for the user.
#[derive(Debug, PartialEq)]
pub enum Change {
    NotFound(String),
    Exists(String),
    Invalid(String),
}

fn not_found(key: &str) -> Change {
    Change::NotFound(format!("unknown question '{key}'"))
}

/// Adds new questions; one invalid or existing key refuses them all.
pub fn create(saved: &mut Questions, input: Questions) -> Result<(), Change> {
    validate_all(&input).map_err(Change::Invalid)?;
    if let Some(key) = input.keys().find(|k| saved.contains_key(*k)) {
        return Err(Change::Exists(format!("question '{key}' already exists")));
    }
    saved.extend(input);
    Ok(())
}

pub fn update(saved: &mut Questions, key: &str, q: Value) -> Result<(), Change> {
    validate(key, &q).map_err(Change::Invalid)?;
    let old = saved.get_mut(key).ok_or_else(|| not_found(key))?;
    *old = q;
    Ok(())
}

pub fn remove(saved: &mut Questions, key: &str) -> Result<(), Change> {
    saved.remove(key).map(|_| ()).ok_or_else(|| not_found(key))
}

/// Turns the request's `questions` into a map of full questions: an array names saved questions, and a map entry
/// without `type` overrides a saved one.
pub fn expand(input: &Value, saved: &Questions) -> Result<Questions, String> {
    let out = match input {
        Value::Object(map) => {
            let mut out = Questions::new();
            for (key, q) in map {
                // An entry without `type` reuses the saved question, with its own fields on top for this request.
                let q = match q.as_object() {
                    Some(fields) if !fields.contains_key("type") => {
                        let mut base = saved
                            .get(key)
                            .and_then(Value::as_object)
                            .cloned()
                            .ok_or_else(|| format!("unknown question '{key}'"))?;
                        base.extend(fields.clone());
                        Value::Object(base)
                    }
                    _ => q.clone(),
                };
                out.insert(key.clone(), q);
            }
            out
        }
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
        let model = match q.as_object_mut().and_then(|o| o.remove("model")) {
            None => request_model.to_owned(),
            Some(Value::String(m)) if !m.trim().is_empty() => m,
            Some(_) => {
                return Err(format!(
                    "question '{key}': model must be a non-empty string"
                ));
            }
        };
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
                        let sum = match (usage.get(k), v) {
                            (None, _) => v.clone(),
                            (Some(Value::Number(a)), Value::Number(b)) => {
                                match (a.as_u64(), b.as_u64()) {
                                    (Some(a), Some(b)) => Value::from(a + b),
                                    _ => Value::from(
                                        a.as_f64().unwrap_or(0.0) + b.as_f64().unwrap_or(0.0),
                                    ),
                                }
                            }
                            // Only counts add up; any other field keeps the first group's value.
                            (Some(first), _) => first.clone(),
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
    fn overrides_saved_questions() {
        let saved = map(json!({"u": {"type": "noul", "instructions": "x"}}));
        assert_eq!(
            Value::Object(expand(&json!({"u": {"model": "p/m"}}), &saved).unwrap()),
            json!({"u": {"type": "noul", "instructions": "x", "model": "p/m"}})
        );
        assert_eq!(
            Value::Object(expand(&json!({"u": {}}), &saved).unwrap()),
            json!({"u": {"type": "noul", "instructions": "x"}})
        );
        assert_eq!(
            expand(&json!({"nope": {}}), &saved).unwrap_err(),
            "unknown question 'nope'"
        );
        let inline = json!({"u": {"type": "score", "instructions": "y", "criteria": ["a", "b"]}});
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
    fn group_rejects_a_model_that_is_not_text() {
        for model in [json!(5), json!(""), json!(null)] {
            let qs = map(json!({"a": {"type": "noul", "instructions": "x", "model": model}}));
            assert_eq!(
                group(qs, "p/m").unwrap_err(),
                "question 'a': model must be a non-empty string"
            );
        }
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
    fn merge_sums_only_numbers_in_usage() {
        let merged = merge(vec![
            json!({"usage": {"input_tokens": 10, "details": {"cached": 1}, "unit": "tok"}}),
            json!({"usage": {"input_tokens": 5, "details": {"cached": 2}, "cost": 0.5, "note": "x"}}),
        ]);
        assert_eq!(
            merged["usage"],
            json!({"input_tokens": 15, "details": {"cached": 1}, "unit": "tok", "cost": 0.5, "note": "x"})
        );
    }

    #[test]
    fn create_rejects_existing_and_invalid() {
        let mut saved = map(json!({"u": {"type": "noul", "instructions": "x"}}));
        let before = saved.clone();
        let fresh = json!({"type": "noul", "instructions": "y"});
        assert_eq!(
            create(&mut saved, map(json!({"v": fresh, "u": fresh}))).unwrap_err(),
            Change::Exists("question 'u' already exists".into())
        );
        assert!(matches!(
            create(&mut saved, map(json!({"a b": fresh}))),
            Err(Change::Invalid(_))
        ));
        assert_eq!(saved, before);
        create(&mut saved, map(json!({"v": fresh}))).unwrap();
        assert_eq!(saved["v"], fresh);
    }

    #[test]
    fn update_and_remove_need_existing_key() {
        let mut saved = map(json!({"u": {"type": "noul", "instructions": "x"}}));
        let q = json!({"type": "noul", "instructions": "y"});
        let missing = Change::NotFound("unknown question 'nope'".into());
        assert_eq!(update(&mut saved, "nope", q.clone()).unwrap_err(), missing);
        assert!(matches!(
            update(&mut saved, "u", json!({"type": "noul"})),
            Err(Change::Invalid(_))
        ));
        update(&mut saved, "u", q.clone()).unwrap();
        assert_eq!(saved["u"], q);
        assert_eq!(remove(&mut saved, "nope").unwrap_err(), missing);
        remove(&mut saved, "u").unwrap();
        assert!(saved.is_empty());
    }

    #[test]
    fn validate_all_rejects_empty_input() {
        assert_eq!(
            validate_all(&Questions::new()).unwrap_err(),
            "no questions given"
        );
    }
}
