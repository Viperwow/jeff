use std::collections::HashSet;

use serde_json::{Map, Value, json};

use crate::questions::{self, Change, Questions};

/// Saved answers by key: `{"text": "...", "types": ["choice", "score"], "default"?: bool}`.
pub type Answers = Map<String, Value>;

/// Fields beyond `text`, `types` and `default` are refused, so a typo such as `txt` does not pass silently.
/// A `questions` field is ignored: it is the computed usage that `with_uses` adds.
pub fn validate(key: &str, a: &Value) -> Result<(), String> {
    check(key, a).map_err(|e| format!("Answer '{key}': {e}"))?;
    let unknown = a
        .as_object()
        .into_iter()
        .flat_map(|o| o.keys())
        .find(|f| !["text", "types", "default", "questions"].contains(&f.as_str()));
    match unknown {
        Some(f) => Err(format!(
            "Answer '{key}': Unknown field '{f}'. Keep only text, types and default."
        )),
        None => Ok(()),
    }
}

fn check(key: &str, a: &Value) -> Result<(), &'static str> {
    questions::check_key(key)?;
    let Some(a) = a.as_object() else {
        return Err("It is not an object. Send the answer as a JSON object.");
    };
    if a.get("text")
        .and_then(Value::as_str)
        .is_none_or(|t| t.trim().is_empty())
    {
        return Err("The text is empty. Write the answer text.");
    }
    const TYPES: &str = "types is not a non-empty array of choice and score. List each type once.";
    let Some(types) = a
        .get("types")
        .and_then(Value::as_array)
        .filter(|t| !t.is_empty())
    else {
        return Err(TYPES);
    };
    let mut seen = HashSet::new();
    for t in types {
        match t.as_str() {
            Some(t @ ("choice" | "score")) if seen.insert(t) => {}
            _ => return Err(TYPES),
        }
    }
    if a.get("default").is_some_and(|d| !d.is_boolean()) {
        return Err("default is not true or false. Use true or false, or remove the field.");
    }
    Ok(())
}

fn text(a: &Value) -> &str {
    a["text"].as_str().unwrap_or_default().trim()
}

fn has_type(a: &Value, t: &str) -> bool {
    a["types"]
        .as_array()
        .is_some_and(|ts| ts.iter().any(|x| x == t))
}

/// Each answer is valid and no two answers that allow `score` share a text.
pub fn validate_all(all: &Answers) -> Result<(), String> {
    let mut scores: Vec<(&str, &str)> = Vec::new();
    for (k, a) in all {
        validate(k, a)?;
        if has_type(a, "score") {
            if let Some((first, _)) = scores.iter().find(|(_, t)| *t == text(a)) {
                return Err(format!(
                    "Answer '{k}' has the same text as '{first}'. Change one of the texts."
                ));
            }
            scores.push((k, text(a)));
        }
    }
    Ok(())
}

fn levels(q: &Value) -> impl Iterator<Item = &str> {
    q["criteria"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
}

fn uses_one(key: &str, a: &Value, q: &Value) -> bool {
    match q["type"].as_str() {
        Some("choice") => {
            has_type(a, "choice")
                && q["criteria"]
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    == Some(text(a))
        }
        Some("score") => has_type(a, "score") && levels(q).any(|l| l == text(a)),
        _ => false,
    }
}

/// Question keys that hold the answer word for word, in question order.
pub fn uses(key: &str, a: &Value, qs: &Questions) -> Vec<String> {
    qs.iter()
        .filter(|(_, q)| uses_one(key, a, q))
        .map(|(k, _)| k.clone())
        .collect()
}

/// Copy of the library with a computed `questions` array on each answer.
pub fn with_uses(all: &Answers, qs: &Questions) -> Answers {
    all.iter()
        .map(|(k, a)| {
            let used = uses(k, a, qs);
            let mut a = a.clone();
            if let Some(o) = a.as_object_mut() {
                o.insert("questions".into(), json!(used));
            }
            (k.clone(), a)
        })
        .collect()
}

fn not_found(key: &str) -> Change {
    Change::NotFound(format!(
        "No answer '{key}'. Check the key or create the answer first."
    ))
}

/// Validates, then stores the text trimmed and without the computed `questions`.
fn clean(key: &str, a: Value) -> Result<Value, String> {
    validate(key, &a)?;
    let mut o = a.as_object().cloned().unwrap_or_default();
    o.remove("questions");
    let trimmed = text(&a).to_owned();
    o.insert("text".into(), Value::String(trimmed));
    Ok(Value::Object(o))
}

/// Adds new answers; one invalid or existing key refuses them all.
pub fn create(saved: &mut Answers, input: Answers) -> Result<(), Change> {
    if input.is_empty() {
        return Err(Change::Invalid(
            "No answers given. Send at least one answer.".into(),
        ));
    }
    let mut next = saved.clone();
    for (k, a) in input {
        let a = clean(&k, a).map_err(Change::Invalid)?;
        if next.contains_key(&k) {
            return Err(Change::Exists(format!(
                "Answer '{k}' already exists. Pick another key or update the saved one."
            )));
        }
        next.insert(k, a);
    }
    next.sort_keys();
    validate_all(&next).map_err(Change::Invalid)?;
    *saved = next;
    Ok(())
}

/// Replaces one answer and rewrites the questions that used it. Returns their keys; on error nothing is written.
pub fn update(
    saved: &mut Answers,
    qs: &mut Questions,
    key: &str,
    a: Value,
) -> Result<Vec<String>, Change> {
    let new = clean(key, a).map_err(Change::Invalid)?;
    let old = saved.get(key).ok_or_else(|| not_found(key))?.clone();
    let mut next = saved.clone();
    next.insert(key.to_owned(), new.clone());
    validate_all(&next).map_err(Change::Invalid)?;

    let (old_text, new_text) = (text(&old), text(&new));
    let rewritten: Vec<String> = uses(key, &old, qs)
        .into_iter()
        .filter(|k| {
            let t = qs[k]["type"].as_str().unwrap_or_default();
            has_type(&new, t)
        })
        .collect();
    for k in &rewritten {
        if qs[k]["type"] == "score" && old_text != new_text && levels(&qs[k]).any(|l| l == new_text)
        {
            return Err(Change::Invalid(format!(
                "Question '{k}' already has the level '{new_text}'. Change the text or edit {k}."
            )));
        }
    }
    for k in &rewritten {
        let q = &mut qs[k];
        if q["type"] == "choice" {
            q["criteria"][key] = json!(new_text);
        } else if let Some(ls) = q["criteria"].as_array_mut() {
            for l in ls
                .iter_mut()
                .filter(|l| l.as_str().map(str::trim) == Some(old_text))
            {
                *l = json!(new_text);
            }
        }
    }
    *saved = next;
    Ok(rewritten)
}

/// Removes the answer from the library only; questions keep the option as plain text.
pub fn remove(saved: &mut Answers, key: &str) -> Result<(), Change> {
    saved.remove(key).map(|_| ()).ok_or_else(|| not_found(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn lib() -> Answers {
        map(json!({"unsure": {"text": "Not sure.", "types": ["choice", "score"]}}))
    }

    #[test]
    fn matching_ignores_surrounding_spaces() {
        let mut qs = map(json!({
            "c": {"type": "choice", "criteria": {"unsure": " Not sure. ", "a": "A"}},
            "s": {"type": "score", "criteria": ["Low", "Not sure.\n", "High"]}
        }));
        assert_eq!(uses("unsure", &lib()["unsure"], &qs), ["c", "s"]);
        let mut saved = lib();
        let new = json!({"text": "Unsure.", "types": ["choice", "score"]});
        assert_eq!(
            update(&mut saved, &mut qs, "unsure", new).unwrap(),
            ["c", "s"]
        );
        assert_eq!(qs["c"]["criteria"]["unsure"], "Unsure.");
        assert_eq!(qs["s"]["criteria"], json!(["Low", "Unsure.", "High"]));
    }

    #[test]
    fn rejects_broken_answers() {
        let good = json!({"text": "x", "types": ["choice"]});
        assert!(validate("a", &good).is_ok());
        for (key, a) in [
            ("a", json!(["x"])),
            ("a", json!({"types": ["choice"]})),
            ("a", json!({"text": "", "types": ["choice"]})),
            ("a", json!({"text": "  ", "types": ["choice"]})),
            ("a", json!({"text": "x", "types": []})),
            ("a", json!({"text": "x", "types": ["noul"]})),
            ("a", json!({"text": "x", "types": ["choice", "choice"]})),
            ("a", json!({"text": "x", "types": ["choice"], "default": 1})),
            ("a b", good.clone()),
        ] {
            let e = validate(key, &a).unwrap_err();
            assert!(e.starts_with(&format!("Answer '{key}': ")), "{e}");
        }
    }

    #[test]
    fn refuses_unknown_fields_but_ignores_questions() {
        assert_eq!(
            validate("a", &json!({"text": "x", "types": ["choice"], "txt": 1})).unwrap_err(),
            "Answer 'a': Unknown field 'txt'. Keep only text, types and default."
        );
        assert!(
            validate(
                "a",
                &json!({"text": "x", "types": ["choice"], "questions": ["q"]})
            )
            .is_ok()
        );
    }

    #[test]
    fn score_texts_are_unique() {
        let all = map(json!({
            "a": {"text": "Same", "types": ["score"]},
            "b": {"text": "Same", "types": ["score", "choice"]}
        }));
        assert_eq!(
            validate_all(&all).unwrap_err(),
            "Answer 'b' has the same text as 'a'. Change one of the texts."
        );
        let ok = map(json!({
            "a": {"text": "Same", "types": ["choice"]},
            "b": {"text": "Same", "types": ["choice"]}
        }));
        assert!(validate_all(&ok).is_ok());
    }

    #[test]
    fn uses_matches_choice_by_key_and_text_and_score_by_text() {
        let qs = map(json!({
            "q1": {"type": "choice", "criteria": {"unsure": "Not sure.", "b": "B"}},
            "q2": {"type": "choice", "criteria": {"unsure": "Other"}},
            "q3": {"type": "score", "criteria": ["Low", "Not sure.", "High"]},
            "q4": {"type": "noul"}
        }));
        let a = json!({"text": "Not sure.", "types": ["choice", "score"]});
        assert_eq!(uses("unsure", &a, &qs), ["q1", "q3"]);
        let choice_only = json!({"text": "Not sure.", "types": ["choice"]});
        assert_eq!(uses("unsure", &choice_only, &qs), ["q1"]);
        let score_only = json!({"text": "Not sure.", "types": ["score"]});
        assert_eq!(uses("unsure", &score_only, &qs), ["q3"]);
        let all = with_uses(&lib(), &qs);
        assert_eq!(all["unsure"]["questions"], json!(["q1", "q3"]));
    }

    #[test]
    fn text_is_stored_trimmed() {
        let mut saved = Answers::new();
        create(
            &mut saved,
            map(json!({"a": {"text": "  Not sure.  ", "types": ["choice"]}})),
        )
        .unwrap();
        assert_eq!(saved["a"]["text"], "Not sure.");
    }

    #[test]
    fn create_strips_questions_and_refuses_existing() {
        let mut saved = Answers::new();
        let a = json!({"a": {"text": "x", "types": ["choice"], "questions": ["q"]}});
        create(&mut saved, map(a.clone())).unwrap();
        assert!(saved["a"].get("questions").is_none());
        assert_eq!(
            create(&mut saved, map(a)),
            Err(Change::Exists(
                "Answer 'a' already exists. Pick another key or update the saved one.".into()
            ))
        );
        assert!(matches!(
            create(&mut saved, Answers::new()),
            Err(Change::Invalid(_))
        ));
    }

    #[test]
    fn update_rewrites_only_matching_questions() {
        let mut saved = lib();
        let mut qs = map(json!({
            "q1": {"type": "choice", "criteria": {"unsure": "Not sure.", "b": "B"}},
            "q2": {"type": "choice", "criteria": {"unsure": "Hand edited"}},
            "q3": {"type": "score", "criteria": ["Low", "Not sure."]}
        }));
        let keys = update(
            &mut saved,
            &mut qs,
            "unsure",
            json!({"text": " Unclear. ", "types": ["choice", "score"]}),
        )
        .unwrap();
        assert_eq!(keys, ["q1", "q3"]);
        assert_eq!(qs["q1"]["criteria"]["unsure"], "Unclear.");
        assert_eq!(qs["q2"]["criteria"]["unsure"], "Hand edited");
        assert_eq!(qs["q3"]["criteria"], json!(["Low", "Unclear."]));
        assert_eq!(saved["unsure"]["text"], "Unclear.");
    }

    #[test]
    fn update_rewrites_every_equal_level() {
        let mut saved = lib();
        let mut qs =
            map(json!({"q": {"type": "score", "criteria": ["Not sure.", "Low", "Not sure."]}}));
        update(
            &mut saved,
            &mut qs,
            "unsure",
            json!({"text": "Unclear.", "types": ["score"]}),
        )
        .unwrap();
        assert_eq!(qs["q"]["criteria"], json!(["Unclear.", "Low", "Unclear."]));
    }

    #[test]
    fn update_narrowing_types_leaves_questions() {
        let mut saved = lib();
        let mut qs = map(json!({
            "c": {"type": "choice", "criteria": {"unsure": "Not sure."}},
            "s": {"type": "score", "criteria": ["Low", "Not sure."]}
        }));
        let keys = update(
            &mut saved,
            &mut qs,
            "unsure",
            json!({"text": "Unclear.", "types": ["score"]}),
        )
        .unwrap();
        assert_eq!(keys, ["s"]);
        assert_eq!(qs["c"]["criteria"]["unsure"], "Not sure.");
    }

    #[test]
    fn update_refuses_a_level_clash() {
        let mut saved = lib();
        let mut qs = map(json!({
            "ok": {"type": "choice", "criteria": {"unsure": "Not sure."}},
            "q": {"type": "score", "criteria": ["Not sure.", "Maybe"]}
        }));
        let before = (saved.clone(), qs.clone());
        assert_eq!(
            update(
                &mut saved,
                &mut qs,
                "unsure",
                json!({"text": "Maybe", "types": ["choice", "score"]})
            ),
            Err(Change::Invalid(
                "Question 'q' already has the level 'Maybe'. Change the text or edit q.".into()
            ))
        );
        assert_eq!((saved, qs), before);
    }

    #[test]
    fn update_skips_broken_questions() {
        let mut saved = lib();
        let mut qs =
            map(json!({"x": 5, "y": {"type": "choice"}, "z": {"type": "score", "criteria": 1}}));
        let keys = update(
            &mut saved,
            &mut qs,
            "unsure",
            json!({"text": "Unclear.", "types": ["choice", "score"]}),
        )
        .unwrap();
        assert!(keys.is_empty());
    }

    #[test]
    fn update_ignores_questions_field() {
        let mut saved = lib();
        update(
            &mut saved,
            &mut Questions::new(),
            "unsure",
            json!({"text": "x", "types": ["choice"], "questions": ["q"]}),
        )
        .unwrap();
        assert!(saved["unsure"].get("questions").is_none());
        assert!(matches!(
            update(
                &mut saved,
                &mut Questions::new(),
                "gone",
                json!({"text": "x", "types": ["choice"]})
            ),
            Err(Change::NotFound(_))
        ));
    }

    #[test]
    fn remove_leaves_questions() {
        let mut saved = lib();
        let qs = map(json!({"q": {"type": "choice", "criteria": {"unsure": "Not sure."}}}));
        remove(&mut saved, "unsure").unwrap();
        assert!(saved.is_empty());
        assert_eq!(qs["q"]["criteria"]["unsure"], "Not sure.");
        assert_eq!(
            remove(&mut saved, "x"),
            Err(Change::NotFound(
                "No answer 'x'. Check the key or create the answer first.".into()
            ))
        );
    }
}
