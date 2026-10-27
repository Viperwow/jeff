# Question caps and verdict

## Goal

A question can carry two optional thresholds, `confidence_soft_cap` and `confidence_hard_cap`, on how confident the model
is in its answer. jeff adds a verdict to every answer it returns: `fail` when the confidence is below
the hard cap, `warn` when it is below the soft cap, `pass` otherwise. The verdict tells a client
whether to use the answer (`pass`), ask a person (`warn`) or not trust it (`fail`), the way an
autorater escalates low-confidence ratings to a human.

The caps guard against an unsure model. They do not judge whether the answer itself is good: a
confident "Very angry" passes.

## Scope

In:

- Two optional caps and a confidence source on every question type: `noul`, `choice`, `score`.
- A `verdict` object on every answer in the `/v1/systemone` response, including classifier calls.
- The caps in the question editor, the CLI and the docs.

Out:

- Caps on the answer's value. ROADMAP "Thresholds" keeps `soft_cap` and `hard_cap` for them; the
  confidence caps here take the `confidence_` prefix so the two never share a name.
- Confidence caps per model. A confidence cap belongs to the question only.
- One-sided caps for `noul` (four caps on the P(yes) scale), in ROADMAP.
- Removing the duplicate number when the source is `provider`, in ROADMAP.
- Rules stored per project and versioned with it. The caps move there unchanged when that lands.

## Approaches

1. **Verdict after the answers are joined (chosen).** `answer()` in `src/main.rs` adds the verdict to
   each answer once all providers have answered and their parts are merged. One place, and request
   overrides work through the existing `expand`. Cost: the single-provider path, which forwards the
   provider's bytes untouched today, now parses and re-serializes the body.
2. **Verdict inside each provider call.** Keeps the raw single-provider path, but every call has to
   know the question rules, and the verdict is computed in several places. More code, same result.
3. **A separate verdict endpoint.** Dropped: the client would make two requests for one answer.

The user asked for the option that keeps waiting time lowest at equal quality. The client waits for
every provider either way, and the verdict costs microseconds next to a model call, so both 1 and 2
are equally fast; 1 is simpler.

## Data

A saved question gains three optional fields:

| Field | Type | Meaning |
|---|---|---|
| `confidence_soft_cap` | number, 0–1 | Below it the verdict is `warn`. |
| `confidence_hard_cap` | number, 0–1 | Below it the verdict is `fail`. |
| `confidence_source` | `"jeff"` or `"provider"` | Which confidence the caps compare against. Absent means `"jeff"`. |

Validation, in `questions::validate` with the existing two-sentence messages:

- Each cap is a number from 0 to 1.
- When both are set, `confidence_hard_cap < confidence_soft_cap`. Equal caps are refused.
- `confidence_source` is `"jeff"` or `"provider"`.

The server does not check a cap against the lowest confidence a question can reach: that floor
depends on the options, and editing the options must never turn a saved question invalid.

Existing questions have none of the fields, so they load unchanged and every answer gets `pass`. No
migration is needed.

A request may override the fields for one call the way it overrides `model`:
`{"questions": {"urgent": {"confidence_soft_cap": 0.9}}}`. Calls made while designing (marked as such):

- `null` removes a saved field for that call: `{"confidence_hard_cap": null}` asks without the hard cap.
- An override that breaks the rules, such as a soft cap below the saved hard cap, is refused with
  `400` and the validation message, as a broken override is today. Nothing is clamped.
- An inline question with its own `type` gets the same cap checks; today such questions skip
  validation, so the cap checks run on their own for them.

## Confidence

| Type | `jeff` (default) | `provider` |
|---|---|---|
| `noul` | `\|2P − 1\|`, where P is `noul` | No provider sends one, so `jeff` is used. |
| `choice` | `1 − H(p) / ln k` over `probabilities` | The provider's `confidence`. |
| `score` | `1 − H(p) / ln k` over `probabilities` | The provider's `confidence`. |

`H(p) = −Σ pᵢ ln pᵢ`, with 0 · ln 0 = 0, and k is the number of entries in `probabilities`,
including provider-added ones such as `insufficient_context`. Every value lies in 0–1: 0 for an even
spread, 1 when all probability sits on one answer.

For `noul` the confidence counts both ways: a sure yes and a sure no both score high. P = 0.03 gives
0.94. The editor says so next to the caps.

When the source is `provider` and the answer has no numeric `confidence`, jeff computes its own and
reports `source: "jeff"`.

Measured on 2026-10-27 over 120 answers: OpenJev's `confidence` equals `1 − H/ln k` to 1e-16;
H2O-Lightning's equals the normalized maximum `(max p − 1/k) / (1 − 1/k)`, per its shim. The two
disagree on the verdict for up to 8 of 36 answers at a cap of 0.9, so `provider` caps depend on the
provider.

## Verdict

| Condition | `result` |
|---|---|
| `confidence_hard_cap` set and confidence < `confidence_hard_cap` | `fail` |
| else `confidence_soft_cap` set and confidence < `confidence_soft_cap` | `warn` |
| else, including no caps | `pass` |

The comparison is strict: a confidence equal to a cap does not break it.

Every answer gains one object, added next to the provider's fields, which stay untouched:

```json
"answer-length": {
  "type": "choice", "choice": "30 words", "probabilities": {"...": 0.1}, "confidence": 0.77,
  "verdict": {"result": "warn", "confidence": 0.77, "source": "jeff", "confidence_soft_cap": 0.8, "confidence_hard_cap": 0.5}
}
```

- `verdict` is present on every answer, caps or not.
- `confidence_soft_cap` and `confidence_hard_cap` appear only when set.
- An answer jeff cannot read (no `noul` number, no `probabilities`) gets no `verdict`; the rest of the
  response is returned as is.

## Flow

1. `expand` builds the full questions as today.
2. `answer()` keeps the expanded questions' caps and source by key, then `group()` strips
   `confidence_soft_cap`, `confidence_hard_cap` and `confidence_source` along with `model`, so providers never see them and
   the answer cache does not depend on them. Changing a cap therefore needs no new model call.
3. After the parts are merged, every answer whose key has a kept question gets its `verdict`. The
   single-provider path parses the body for this too.
4. A request without `questions` is forwarded as today with no verdict: jeff knows no rules for it.
5. The re-serialized body keeps the provider's fields and their values; key order may differ from
   the provider's bytes. Design call: acceptable, JSON clients do not depend on key order.

Classifier calls go through the same path, so each of their answers gets a `verdict` too, and the
Classifiers page Try shows the badges through the shared answer renderer. A verdict for the
classifier as a whole is out of scope (design call): it needs the classifier score from ROADMAP
"Classifier score" first.

## API

`/v1/systemone` gains `verdict` on each answer. `POST /api/questions` and `PUT /api/questions/{key}`
accept and return the three fields. `GET /v1/questions` returns them.

## CLI

No new flags. `jeff questions add` and `update` take the fields in the question JSON; `jeff ask`
prints the response with `verdict`. Document both.

## UI

Question editor, below **Model** (mockup: `.superpowers/brainstorm/872-1793091596/content/caps-slider-v4.html`):

- **Confidence** select: "Jeff: from the probabilities, the same for every provider" and "Provider:
  the confidence the model returns". For `noul`, a note says the confidence counts both ways.
- **Caps**: one 0–1 bar in three thin segments with gaps, fail red, warn amber, pass green, matching
  the provider install step icons. Each cap is a small triangle in the neutral colour of the Try
  confidence dot, its tip touching the bar: soft above pointing down, hard below pointing up, each
  labelled with its value. Lines drag by pointer and move by 0.01 with the arrow keys; hard always stays
  at least 0.01 left of soft.
- Hovering a segment shows a tooltip of at most 15 words, such as "Warn from 0.50 to 0.80: ask a
  person to check." Cap lines and the last Try's confidence dot have tooltips too.
- To the right of the bar, facing their lines: **Soft cap** and **Hard cap** number inputs (step 0.01)
  with a remove icon button each, tooltip "Remove soft cap". A removed cap shows "+ Soft cap".
- **Try** shows a verdict badge next to each answer, then the confidence, its source and the caps.

Every icon and icon-only button has a tooltip and the same text as its accessible name.

## Docs

README: the three fields, the confidence table, the verdict rules and the `verdict` object, with one
example. ROADMAP: the two follow-ups above.

## Testing

- Unit, `src/questions.rs`: validation accepts and refuses caps and sources; `group()` strips the
  three fields.
- Unit, verdict: the confidence formulas on the measured OpenJev answers (`answer-length` gives
  0.7726711895383636, `test-score-question` 0.9781026173054966); `noul` at P = 0.5, 0.03 and 1;
  strict comparison at a cap; the `provider` fallback; an unreadable answer gets no verdict.
- Property: for any probabilities the confidence stays in 0–1, and the verdict never gets better as a
  cap rises.
- End to end on a throwaway jeff: save a question with caps, ask it, read `verdict`; change a cap and
  ask again from cache with a new verdict; check the editor in the browser.

## Left to the builder

- Where the confidence and verdict code lives (a new module or `questions.rs`).
- Exact UI markup within the existing `ui/app.js` helpers.
- The README wording.
