use crate::questions::Questions;
use serde_json::{Value, json};

/// How sure the model is of its answer, from 0 to 1, and which source gave the number. `jeff` counts a sure yes and
/// a sure no alike for `noul`, and uses the normalized entropy of `probabilities` otherwise.
pub fn confidence(answer: &Value, source: &str) -> Option<(f64, &'static str)> {
    if source == "provider"
        && let Some(c) = answer.get("confidence").and_then(Value::as_f64)
    {
        return Some((c, "provider"));
    }
    if let Some(p) = answer.get("noul").and_then(Value::as_f64) {
        return Some(((2.0 * p - 1.0).abs().min(1.0), "jeff"));
    }
    let probs: Vec<f64> = answer
        .get("probabilities")?
        .as_object()?
        .values()
        .filter_map(Value::as_f64)
        .collect();
    match probs.len() {
        0 => None,
        1 => Some((1.0, "jeff")),
        k => {
            let h: f64 = probs
                .iter()
                .filter(|p| **p > 0.0)
                .map(|p| -p * p.ln())
                .sum();
            Some(((1.0 - h / (k as f64).ln()).clamp(0.0, 1.0), "jeff"))
        }
    }
}

const SOFT: &str = "confidence_soft_cap";
const HARD: &str = "confidence_hard_cap";

pub fn verdict(answer: &Value, question: &Value) -> Option<Value> {
    let source = question
        .get("confidence_source")
        .and_then(Value::as_str)
        .unwrap_or("jeff");
    let (c, used) = confidence(answer, source)?;
    let soft = question.get(SOFT).and_then(Value::as_f64);
    let hard = question.get(HARD).and_then(Value::as_f64);
    let result = if hard.is_some_and(|h| c < h) {
        "fail"
    } else if soft.is_some_and(|s| c < s) {
        "warn"
    } else {
        "pass"
    };
    let mut v = json!({"result": result, "confidence": c, "source": used});
    if let Some(s) = soft {
        v[SOFT] = json!(s);
    }
    if let Some(h) = hard {
        v[HARD] = json!(h);
    }
    Some(v)
}

/// Adds `verdict` to every answer of `response` asked by one of `questions`.
pub fn apply(response: &mut Value, questions: &Questions) {
    let Some(answers) = response.get_mut("answers").and_then(Value::as_object_mut) else {
        return;
    };
    for (key, answer) in answers {
        let Some(q) = questions.get(key) else {
            continue;
        };
        if let Some(v) = verdict(answer, q)
            && let Some(obj) = answer.as_object_mut()
        {
            obj.insert("verdict".into(), v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jeff_confidence_matches_openjev() {
        let choice = json!({"type": "choice", "probabilities": {
            "120 words": 0.0007616466220741751, "15 words": 0.0061794831192532465,
            "30 words": 0.855626768666195, "300 words": 0.0004921765548989722,
            "5 words": 0.0002882554682521148, "50 words": 0.07728563919190698,
            "90 words": 0.05890993994416244, "Unlimited": 0.00003213464363223489,
            "Yes/No only": 0.00003547371546969253, "insufficient_context": 0.00016975864041433232,
            "uncertain": 0.00021872343374085176}, "confidence": 0.7726711895383636});
        let (c, s) = confidence(&choice, "jeff").unwrap();
        assert!((c - 0.7726711895383636).abs() < 1e-12 && s == "jeff");
        let score = json!({"type": "score", "probabilities": {"0": 0.0002131869375836218,
            "1": 0.0032810850259026047, "2": 0.9965057280365138}});
        assert!((confidence(&score, "jeff").unwrap().0 - 0.9781026173054966).abs() < 1e-12);
    }

    #[test]
    fn noul_counts_both_ways_even_without_type() {
        for (p, want) in [(0.5, 0.0), (0.03, 0.94), (1.0, 1.0), (0.0, 1.0)] {
            let (c, _) = confidence(&json!({"noul": p}), "jeff").unwrap();
            assert!((c - want).abs() < 1e-12, "{p}");
        }
    }

    #[test]
    fn provider_source_falls_back_to_jeff() {
        let a =
            json!({"type": "choice", "probabilities": {"a": 0.9, "b": 0.1}, "confidence": 0.42});
        assert_eq!(confidence(&a, "provider"), Some((0.42, "provider")));
        assert_eq!(
            confidence(&json!({"noul": 0.9}), "provider").unwrap().1,
            "jeff"
        );
    }

    #[test]
    fn a_zero_probability_is_not_nan() {
        let (c, _) = confidence(&json!({"probabilities": {"a": 1.0, "b": 0.0}}), "jeff").unwrap();
        assert_eq!(c, 1.0);
    }

    #[test]
    fn verdict_rules_are_strict() {
        let a = json!({"noul": 0.75}); // confidence 0.5
        let q = |s: f64, h: f64| json!({"confidence_soft_cap": s, "confidence_hard_cap": h});
        assert_eq!(verdict(&a, &q(0.5, 0.25)).unwrap()["result"], "pass");
        assert_eq!(verdict(&a, &q(0.51, 0.25)).unwrap()["result"], "warn");
        assert_eq!(verdict(&a, &q(0.75, 0.51)).unwrap()["result"], "fail");
        assert_eq!(
            verdict(&a, &q(0.75, 0.25)).unwrap(),
            json!({"result": "warn", "confidence": 0.5, "source": "jeff",
                "confidence_soft_cap": 0.75, "confidence_hard_cap": 0.25})
        );
        assert_eq!(
            verdict(&a, &json!({})).unwrap(),
            json!({"result": "pass", "confidence": 0.5, "source": "jeff"})
        );
    }

    #[test]
    fn apply_skips_unreadable_and_unknown_answers() {
        let qs: Questions = json!({"a": {}, "b": {}}).as_object().unwrap().clone();
        let mut r = json!({"answers": {"a": {"noul": 0.9}, "b": {"oops": 1}, "c": {"noul": 0.1}}, "usage": {}});
        apply(&mut r, &qs);
        assert!(r["answers"]["a"]["verdict"].is_object());
        assert!(r["answers"]["b"].get("verdict").is_none());
        assert!(r["answers"]["c"].get("verdict").is_none());
        let mut s = json!("not an object");
        apply(&mut s, &qs);
        assert_eq!(s, json!("not an object"));
    }

    /// A seeded generator keeps the property run the same on every machine.
    fn lcg(seed: &mut u64) -> f64 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (*seed >> 11) as f64 / (1u64 << 53) as f64
    }

    #[test]
    fn confidence_stays_in_range_and_a_higher_hard_cap_never_helps() {
        let mut seed = 42;
        for _ in 0..1000 {
            let k = 2 + (lcg(&mut seed) * 11.0) as usize;
            let raw: Vec<f64> = (0..k).map(|_| lcg(&mut seed).powi(3)).collect();
            let sum: f64 = raw.iter().sum::<f64>().max(f64::MIN_POSITIVE);
            let probs: serde_json::Map<String, Value> = raw
                .iter()
                .enumerate()
                .map(|(i, p)| (i.to_string(), json!(p / sum)))
                .collect();
            let a = json!({ "probabilities": probs });
            let (c, _) = confidence(&a, "jeff").unwrap();
            assert!((0.0..=1.0).contains(&c), "{c}");
            let mut failed = false;
            for step in 0..=20 {
                let hard = step as f64 * 0.05;
                let r = verdict(&a, &json!({"confidence_hard_cap": hard})).unwrap();
                let now = r["result"] == "fail";
                assert!(!failed || now, "fail turned back at hard {hard}");
                failed = now;
            }
        }
    }
}
