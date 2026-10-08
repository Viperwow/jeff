use serde_json::{Map, Value, json};

use crate::questions::{self, Change, Questions};

/// Saved classifiers by key: `{"questions": [keys], "model"?: "provider/model"}`.
pub type Classifiers = Map<String, Value>;

pub fn validate(key: &str, c: &Value) -> Result<(), String> {
    check(key, c).map_err(|e| format!("classifier '{key}': {e}"))
}

fn check(key: &str, c: &Value) -> Result<(), &'static str> {
    questions::check_key(key)?;
    let Some(c) = c.as_object() else {
        return Err("must be an object");
    };
    let Some(keys) = c
        .get("questions")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
    else {
        return Err("questions must be a non-empty array of question keys");
    };
    for (i, k) in keys.iter().enumerate() {
        if !k.is_string() {
            return Err("questions must be a non-empty array of question keys");
        }
        if keys[..i].contains(k) {
            return Err("questions must not repeat a key");
        }
    }
    if c.get("model")
        .is_some_and(|m| m.as_str().is_none_or(|m| m.trim().is_empty()))
    {
        return Err("model must be a non-empty string");
    }
    Ok(())
}

fn not_found(key: &str) -> Change {
    Change::NotFound(format!("unknown classifier '{key}'"))
}

/// Adds new classifiers; one invalid or existing key refuses them all.
pub fn create(saved: &mut Classifiers, input: Classifiers) -> Result<(), Change> {
    if input.is_empty() {
        return Err(Change::Invalid("no classifiers given".into()));
    }
    for (k, c) in &input {
        validate(k, c).map_err(Change::Invalid)?;
    }
    if let Some(key) = input.keys().find(|k| saved.contains_key(*k)) {
        return Err(Change::Exists(format!("classifier '{key}' already exists")));
    }
    saved.extend(input);
    Ok(())
}

pub fn update(saved: &mut Classifiers, key: &str, c: Value) -> Result<(), Change> {
    validate(key, &c).map_err(Change::Invalid)?;
    let old = saved.get_mut(key).ok_or_else(|| not_found(key))?;
    *old = c;
    Ok(())
}

pub fn remove(saved: &mut Classifiers, key: &str) -> Result<(), Change> {
    saved.remove(key).map(|_| ()).ok_or_else(|| not_found(key))
}

fn keys(c: &Value) -> impl Iterator<Item = &str> {
    c["questions"].as_array().into_iter().flatten().filter_map(Value::as_str)
}

/// Question keys the classifier names but cannot ask: deleted, renamed or invalid.
pub fn skipped(c: &Value, saved: &Questions) -> Vec<String> {
    keys(c)
        .filter(|k| saved.get(*k).is_none_or(|q| questions::validate(k, q).is_err()))
        .map(str::to_owned)
        .collect()
}

/// Turns a classifier into the `questions` input of `/v1/systemone`, with the keys it skips. The override goes
/// into the request only; the saved questions keep their own model.
pub fn resolve(key: &str, c: &Value, saved: &Questions) -> Result<(Value, Vec<String>), String> {
    validate(key, c)?;
    let skipped = skipped(c, saved);
    let working: Vec<&str> = keys(c).filter(|k| !skipped.iter().any(|s| s == k)).collect();
    if working.is_empty() {
        return Err(format!("classifier '{key}' has no working questions"));
    }
    let input = match c.get("model") {
        Some(model) => Value::Object(
            working
                .iter()
                .map(|k| ((*k).to_owned(), json!({ "model": model })))
                .collect(),
        ),
        None => json!(working),
    };
    Ok((input, skipped))
}

/// Adds `skipped` to a successful answer; anything that is not a JSON object passes through untouched.
pub fn with_skipped(bytes: Vec<u8>, skipped: &[String]) -> Vec<u8> {
    if skipped.is_empty() {
        return bytes;
    }
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(mut answer)) => {
            answer.insert("skipped".into(), json!(skipped));
            serde_json::to_vec(&answer).unwrap()
        }
        _ => bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn noul() -> Value {
        json!({"type": "noul", "instructions": "x"})
    }

    fn qs(v: Value) -> Questions {
        v.as_object().unwrap().clone()
    }

    fn cs(v: Value) -> Classifiers {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn rejects_broken_classifiers() {
        for (key, c) in [
            ("a b", json!({"questions": ["u"]})),
            ("k", json!({"questions": []})),
            ("k", json!({"questions": "u"})),
            ("k", json!({"questions": ["u", 1]})),
            ("k", json!({"questions": ["u", "u"]})),
            ("k", json!({"questions": ["u"], "model": ""})),
            ("k", json!({"questions": ["u"], "model": 3})),
            ("k", json!(["u"])),
        ] {
            assert!(validate(key, &c).is_err(), "{key} {c}");
        }
        assert!(validate("k", &json!({"questions": ["missing"], "model": "p/m"})).is_ok());
    }

    #[test]
    fn resolve_without_model_names_saved_keys() {
        let saved = qs(json!({"a": noul(), "b": noul()}));
        let (input, skipped) = resolve("c", &json!({"questions": ["b", "a"]}), &saved).unwrap();
        assert_eq!(input, json!(["b", "a"]));
        assert!(skipped.is_empty());
    }

    #[test]
    fn resolve_with_model_overrides_each_question() {
        let saved = qs(json!({"a": {"type": "noul", "instructions": "x", "model": "p/own"}}));
        let (input, _) =
            resolve("c", &json!({"questions": ["a"], "model": "p/m"}), &saved).unwrap();
        assert_eq!(input, json!({"a": {"model": "p/m"}}));
        assert_eq!(saved["a"]["model"], "p/own");
    }

    #[test]
    fn resolve_skips_missing_and_invalid() {
        let saved = qs(json!({"a": noul(), "bad": {"type": "maybe", "instructions": "x"}}));
        let (input, skipped) =
            resolve("c", &json!({"questions": ["gone", "a", "bad"]}), &saved).unwrap();
        assert_eq!(input, json!(["a"]));
        assert_eq!(skipped, ["gone", "bad"]);
    }

    #[test]
    fn resolve_needs_one_working_question() {
        let e = resolve("c", &json!({"questions": ["gone"]}), &Questions::new()).unwrap_err();
        assert_eq!(e, "classifier 'c' has no working questions");
    }

    #[test]
    fn with_skipped_adds_the_list() {
        let out = with_skipped(br#"{"answers":{}}"#.to_vec(), &["s".into()]);
        assert_eq!(
            serde_json::from_slice::<Value>(&out).unwrap(),
            json!({"answers": {}, "skipped": ["s"]})
        );
    }

    #[test]
    fn with_skipped_leaves_errors_and_non_objects() {
        for body in [&b"[1]"[..], b"not json"] {
            assert_eq!(with_skipped(body.to_vec(), &["s".into()]), body);
        }
        assert_eq!(with_skipped(b"{}".to_vec(), &[]), b"{}");
    }

    #[test]
    fn create_update_remove() {
        let mut saved = Classifiers::new();
        let c = json!({"questions": ["u"]});
        create(&mut saved, cs(json!({"k": c}))).unwrap();
        assert!(matches!(create(&mut saved, cs(json!({"k": c}))), Err(Change::Exists(_))));
        assert!(matches!(create(&mut saved, Classifiers::new()), Err(Change::Invalid(_))));
        assert!(matches!(update(&mut saved, "x", c.clone()), Err(Change::NotFound(_))));
        assert!(matches!(
            update(&mut saved, "k", json!({"questions": []})),
            Err(Change::Invalid(_))
        ));
        update(&mut saved, "k", json!({"questions": ["v"], "model": "p/m"})).unwrap();
        assert_eq!(saved["k"]["model"], "p/m");
        remove(&mut saved, "k").unwrap();
        assert_eq!(
            remove(&mut saved, "k"),
            Err(Change::NotFound("unknown classifier 'k'".into()))
        );
    }
}
