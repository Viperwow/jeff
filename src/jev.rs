//! What a Jev-compatible provider returns from `/v1/systemone`, typed field for field after TypeSafe's API reference
//! (https://docs.typesafe.ai/api). Fields a provider adds beyond it, such as `type`, are ignored. Every field the
//! reference requires is here even when jeff does not read it, so reading a response checks its whole shape.
#![allow(dead_code)]
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct Response {
    pub model: String,
    pub answers: BTreeMap<String, Answer>,
    pub usage: TokenUsage,
}

#[derive(Debug, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Told apart by the field only its kind carries: `noul`, `choice` or `score`.
#[derive(Debug)]
pub enum Answer {
    Score(ScoreAnswer),
    Choice(ChoiceAnswer),
    Noul(NoulAnswer),
}

#[derive(Debug, Deserialize)]
pub struct NoulAnswer {
    pub noul: f64,
}

#[derive(Debug, Deserialize)]
pub struct ChoiceAnswer {
    pub choice: String,
    pub confidence: f64,
    pub probabilities: BTreeMap<String, f64>,
}

#[derive(Debug, Deserialize)]
pub struct ScoreAnswer {
    pub score: f64,
    pub confidence: f64,
    pub legend: BTreeMap<u32, Value>,
    pub probabilities: BTreeMap<u32, f64>,
}

/// The reference shape with each answer still raw, so a mismatch can name the question it belongs to.
#[derive(Deserialize)]
struct Raw {
    model: String,
    answers: BTreeMap<String, Value>,
    usage: TokenUsage,
}

/// Reads a provider's answer; the error says what does not match the reference, for jeff.log.
pub fn parse(bytes: &[u8]) -> Result<Response, String> {
    let raw: Raw = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let mut answers = BTreeMap::new();
    for (key, v) in raw.answers {
        let typed = if v.get("noul").is_some() {
            NoulAnswer::deserialize(v).map(Answer::Noul)
        } else if v.get("choice").is_some() {
            ChoiceAnswer::deserialize(v).map(Answer::Choice)
        } else if v.get("score").is_some() {
            ScoreAnswer::deserialize(v).map(Answer::Score)
        } else {
            return Err(format!("answer '{key}' has no noul, choice or score field"));
        };
        let a = typed.map_err(|e| format!("answer '{key}': {e}"))?;
        a.check().map_err(|e| format!("answer '{key}': {e}"))?;
        answers.insert(key, a);
    }
    Ok(Response {
        model: raw.model,
        answers,
        usage: raw.usage,
    })
}

/// Providers compute these numbers in floating point, so a value a hair past 0 or 1 is rounding, not a broken answer.
const NOISE: f64 = 1e-9;

fn unit(name: &str, v: f64) -> Result<(), String> {
    match (-NOISE..=1.0 + NOISE).contains(&v) {
        true => Ok(()),
        false => Err(format!("{name} {v} is outside 0 to 1")),
    }
}

impl Answer {
    fn check(&self) -> Result<(), String> {
        match self {
            Answer::Noul(a) => unit("noul", a.noul),
            Answer::Choice(a) => {
                unit("confidence", a.confidence)?;
                a.probabilities
                    .values()
                    .try_for_each(|p| unit("probability", *p))
            }
            Answer::Score(a) => {
                unit("confidence", a.confidence)?;
                a.probabilities
                    .values()
                    .try_for_each(|p| unit("probability", *p))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parsed(v: Value) -> Result<Response, String> {
        parse(&serde_json::to_vec(&v).unwrap())
    }

    #[test]
    fn reads_openjev_and_h2o_answers() {
        let r = parsed(
            json!({"model": "openjev-0.1", "probability_source": "logits",
            "answers": {
                "n": {"type": "noul", "noul": 0.0010450819778616118},
                "c": {"type": "choice", "choice": "30 words", "confidence": 0.77,
                    "probabilities": {"30 words": 0.86, "50 words": 0.14}},
                "s": {"type": "score", "score": 1.99, "confidence": 0.97,
                    "legend": {"0": "yes", "1": "maybe", "2": "no"},
                    "probabilities": {"0": 0.0002, "1": 0.0033, "2": 0.9965}}},
            "usage": {"input_tokens": 538, "output_tokens": 0, "latency_ms": 12}}),
        )
        .unwrap();
        assert!(matches!(r.answers["n"], Answer::Noul(_)));
        assert!(matches!(r.answers["c"], Answer::Choice(_)));
        let Answer::Score(s) = &r.answers["s"] else {
            panic!()
        };
        assert_eq!(
            s.probabilities.keys().copied().collect::<Vec<_>>(),
            [0, 1, 2]
        );
    }

    #[test]
    fn rounding_noise_at_the_ends_is_not_off_the_reference() {
        let r = parsed(
            json!({"model": "m", "usage": {"input_tokens": 1, "output_tokens": 0},
            "answers": {"q": {"choice": "a", "confidence": -2.220446049250313e-16,
                "probabilities": {"a": 0.2, "b": 0.2, "c": 0.2, "d": 0.2, "e": 0.2}},
                "n": {"noul": 1.0000000000000002}}}),
        );
        assert!(r.is_ok(), "{r:?}");
    }

    #[test]
    fn refuses_answers_off_the_reference() {
        let ok_usage = json!({"input_tokens": 1, "output_tokens": 0});
        for (answer, why) in [
            (
                json!({"choice": "a", "confidence": 0.5, "probabilities": {"a": 0.5, "b": "0.5"}}),
                "a probability as text",
            ),
            (json!({"noul": 1.5}), "noul above 1"),
            (json!({"noul": -0.1}), "noul below 0"),
            (
                json!({"choice": "a", "confidence": 2.0, "probabilities": {"a": 1.0}}),
                "confidence above 1",
            ),
            (
                json!({"score": 1.0, "confidence": 0.5, "legend": {"x": "?"}, "probabilities": {"x": 1.0}}),
                "a level that is not a number",
            ),
            (json!({"something": 1}), "no known answer shape"),
        ] {
            let e = parsed(json!({"model": "m", "answers": {"q": answer}, "usage": ok_usage}))
                .unwrap_err();
            assert!(e.contains("'q'"), "{why}: {e}");
        }
        for body in [
            json!({"answers": {}, "usage": ok_usage}),
            json!({"model": "m", "usage": ok_usage}),
            json!({"model": "m", "answers": {}}),
        ] {
            assert!(parsed(body.clone()).is_err(), "{body}");
        }
        assert!(parse(b"not json").is_err());
    }
}
