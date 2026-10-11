# jeff HTTP ↔ CLI map

Confirm flags with `jeff <cmd> --help`. Paths below are relative to the host in
`JEFF_URL` (API, default `:8080`) or `JEFF_ADMIN_URL` (management UI, default
`:8081`). The UI port serves both `/api/*` and the same `/v1/*` routes for
browser use.

Auth: when any key exists, send `Authorization: Bearer <key>`. **client** →
`/v1/*`; **admin** → `/api/*` (and `/v1/*`).

## Client `/v1/*` (classification and read)

| HTTP | CLI equivalent | Notes |
| --- | --- | --- |
| `GET /health` | curl only | No auth; liveness |
| `GET /v1/models` | curl only | Providers and models |
| `POST /v1/systemone` | `jeff ask …`, `jeff ask --request` | Body: `state`, `questions` (array of keys or map), optional `model` |
| `GET /v1/questions` | `jeff questions list` (admin URL) | Same JSON as API |
| `GET /v1/questions/{key}` | `jeff questions get KEY` | |
| `GET /v1/classifiers` | `jeff classifiers list` | |
| `GET /v1/classifiers/{key}` | `jeff classifiers get KEY` | |
| `POST /v1/classifiers/{key}` | `jeff ask --classifier KEY --state …` | Body: `state`, optional `model` |
| `POST /v1/classifiers` | no dedicated CLI | Body: `classifier`, `state`, optional `model` — try unsaved classifier |

## Admin `/api/*` (mutate config)

| HTTP | CLI equivalent |
| --- | --- |
| `GET /api/status` | curl (config ok / invalid) |
| `GET /api/config` | Admin UI / curl |
| `PUT /api/config` | Admin UI |
| `POST /api/questions` | `jeff questions add FILE` |
| `PUT /api/questions/{key}` | `jeff questions update KEY FILE` |
| `DELETE /api/questions/{key}` | `jeff questions remove KEY` |
| `POST /api/classifiers` | `jeff classifiers add …` or `add --file` |
| `PUT /api/classifiers/{key}` | `jeff classifiers update …` or `update --file` |
| `DELETE /api/classifiers/{key}` | `jeff classifiers remove KEY` |
| `GET/POST /api/keys`, `DELETE /api/keys/{id}` | Admin UI (not `jeff keys`) |
| `GET/POST /api/clm…` | `jeff install clm`, UI Install |

`jeff keys *` writes `jeff.json` directly (bootstrap before `/api` works).

## Question JSON (for add / `--questions`)

```json
{
  "urgent": {
    "type": "noul",
    "instructions": "The message conveys urgency."
  },
  "department": {
    "type": "choice",
    "instructions": "Which team should handle this?",
    "criteria": {
      "billing": "Charges and refunds",
      "technical": "Bugs and outages"
    }
  },
  "anger": {
    "type": "score",
    "instructions": "How angry is the customer?",
    "criteria": ["Calm", "Annoyed", "Very angry"]
  }
}
```

Optional `"model": "provider/model"` per question.

## Classifier JSON

```json
{
  "support-triage": {
    "description": "Routes incoming tickets",
    "questions": ["urgent", "department", "anger"],
    "model": "clm/clm-latest"
  }
}
```

`model` overrides every question for that classifier’s runs only.

## `/v1/systemone` response shapes (per question key)

- **noul**: `{ "noul": 0.97 }`
- **choice**: `{ "choice": "billing", "confidence": 0.85, "probabilities": { … } }`
- **score**: `{ "score": 1.2, "confidence": …, "legend": […], "probabilities": […] }`

Classifier run: same map under `"answers"` (exact top-level shape matches jeff
version — inspect one `jeff ask -o` sample when wiring hooks).
