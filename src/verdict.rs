use crate::{
    jev::{self, Answer},
    questions::Questions,
};
use serde_json::{Value, json};

/// How sure the model is of its answer, from 0 to 1, and which source gave the number. `jeff` counts a sure yes and
/// a sure no alike for `noul`, and uses the normalized entropy of `probabilities` otherwise.
pub fn confidence(answer: &Answer, source: &str) -> (f64, &'static str) {
    let (probs, provided): (Vec<f64>, f64) = match answer {
        Answer::Noul(a) => return ((2.0 * a.noul - 1.0).abs(), "jeff"),
        Answer::Choice(a) => (a.probabilities.values().copied().collect(), a.confidence),
        Answer::Score(a) => (a.probabilities.values().copied().collect(), a.confidence),
    };
    if source == "provider" {
        return (provided, "provider");
    }
    let k = probs.len();
    if k < 2 {
        return (1.0, "jeff");
    }
    let h: f64 = probs
        .iter()
        .filter(|p| **p > 0.0)
        .map(|p| -p * p.ln())
        .sum();
    ((1.0 - h / (k as f64).ln()).clamp(0.0, 1.0), "jeff")
}

const SOFT: &str = "confidence_soft_cap";
const HARD: &str = "confidence_hard_cap";

pub fn verdict(answer: &Answer, question: &Value) -> Value {
    let source = question
        .get("confidence_source")
        .and_then(Value::as_str)
        .unwrap_or("jeff");
    let (c, used) = confidence(answer, source);
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
    v
}

/// Adds `verdict` to every answer in `response` that one of `questions` asked; `typed` is the same response, read.
pub fn apply(response: &mut Value, typed: &jev::Response, questions: &Questions) {
    for (key, answer) in &typed.answers {
        if let Some(q) = questions.get(key)
            && let Some(obj) = response["answers"][key].as_object_mut()
        {
            obj.insert("verdict".into(), verdict(answer, q));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// One answer read the way `forward` reads a provider's response.
    fn read(answer: Value) -> Answer {
        let body = json!({"model": "m", "answers": {"q": answer},
            "usage": {"input_tokens": 0, "output_tokens": 0}});
        let mut r = jev::parse(&serde_json::to_vec(&body).unwrap()).unwrap();
        r.answers.remove("q").unwrap()
    }

    fn choice(probs: Value, confidence: f64) -> Answer {
        read(json!({"choice": "a", "confidence": confidence, "probabilities": probs}))
    }

    #[test]
    fn jeff_confidence_matches_openjev() {
        let a = choice(
            json!({"120 words": 0.0007616466220741751, "15 words": 0.0061794831192532465,
            "30 words": 0.855626768666195, "300 words": 0.0004921765548989722,
            "5 words": 0.0002882554682521148, "50 words": 0.07728563919190698,
            "90 words": 0.05890993994416244, "Unlimited": 0.00003213464363223489,
            "Yes/No only": 0.00003547371546969253, "insufficient_context": 0.00016975864041433232,
            "uncertain": 0.00021872343374085176}),
            0.7726711895383636,
        );
        let (c, s) = confidence(&a, "jeff");
        assert!((c - 0.7726711895383636).abs() < 1e-12 && s == "jeff");
        let score = read(
            json!({"score": 2.0, "confidence": 0.98, "legend": {"0": "a", "1": "b", "2": "c"},
            "probabilities": {"0": 0.0002131869375836218, "1": 0.0032810850259026047, "2": 0.9965057280365138}}),
        );
        assert!((confidence(&score, "jeff").0 - 0.9781026173054966).abs() < 1e-12);
    }

    #[test]
    fn noul_counts_both_ways() {
        for (p, want) in [(0.5, 0.0), (0.03, 0.94), (1.0, 1.0), (0.0, 1.0)] {
            let (c, _) = confidence(&read(json!({"noul": p})), "jeff");
            assert!((c - want).abs() < 1e-12, "{p}");
        }
    }

    #[test]
    fn provider_source_uses_the_providers_confidence() {
        let a = choice(json!({"a": 0.9, "b": 0.1}), 0.42);
        assert_eq!(confidence(&a, "provider"), (0.42, "provider"));
    }

    #[test]
    fn noul_always_uses_jeff() {
        let (c, s) = confidence(&read(json!({"noul": 0.03, "confidence": 0.2})), "provider");
        assert!((c - 0.94).abs() < 1e-12 && s == "jeff");
    }

    #[test]
    fn a_zero_probability_is_not_nan() {
        let (c, _) = confidence(&choice(json!({"a": 1.0, "b": 0.0}), 1.0), "jeff");
        assert_eq!(c, 1.0);
    }

    #[test]
    fn verdict_rules_are_strict() {
        let a = read(json!({"noul": 0.75})); // confidence 0.5
        let q = |s: f64, h: f64| json!({"confidence_soft_cap": s, "confidence_hard_cap": h});
        assert_eq!(verdict(&a, &q(0.5, 0.25))["result"], "pass");
        assert_eq!(verdict(&a, &q(0.51, 0.25))["result"], "warn");
        assert_eq!(verdict(&a, &q(0.75, 0.51))["result"], "fail");
        assert_eq!(
            verdict(&a, &q(0.75, 0.25)),
            json!({"result": "warn", "confidence": 0.5, "source": "jeff",
                "confidence_soft_cap": 0.75, "confidence_hard_cap": 0.25})
        );
        assert_eq!(
            verdict(&a, &json!({})),
            json!({"result": "pass", "confidence": 0.5, "source": "jeff"})
        );
    }

    #[test]
    fn apply_judges_only_the_questions_asked() {
        let body = json!({"model": "m", "answers": {"a": {"noul": 0.9}, "c": {"noul": 0.1}},
            "usage": {"input_tokens": 0, "output_tokens": 0}});
        let typed = jev::parse(&serde_json::to_vec(&body).unwrap()).unwrap();
        let qs: Questions = json!({"a": {}}).as_object().unwrap().clone();
        let mut r = body.clone();
        apply(&mut r, &typed, &qs);
        assert!(r["answers"]["a"]["verdict"].is_object());
        assert!(r["answers"]["c"].get("verdict").is_none());
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
                .map(|(i, p)| (i.to_string(), json!((p / sum).min(1.0))))
                .collect();
            let a = choice(Value::Object(probs), 0.5);
            let (c, _) = confidence(&a, "jeff");
            assert!((0.0..=1.0).contains(&c), "{c}");
            let mut failed = false;
            for step in 0..=20 {
                let hard = step as f64 * 0.05;
                let now = verdict(&a, &json!({"confidence_hard_cap": hard}))["result"] == "fail";
                assert!(!failed || now, "fail turned back at hard {hard}");
                failed = now;
            }
        }
    }
}
