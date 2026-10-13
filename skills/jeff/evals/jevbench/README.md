# JevBench runs

Scripts that score jeff on the [JevBench](https://github.com/fstandhartinger/jevbench) public split: 48 easy, 111 hard and 72 original items. Each script downloads the split into `DATA_DIR` on first run and writes per-item results there as JSONL.

- `direct.py` asks every item through `jeff ask` and scores the label against `expected`.
- `wording.py` asks every item as published and with its option descriptions stripped, in one call.
- `companions.py` asks every item alone, then beside two neutral questions about the same text.

```sh
python direct.py DATA_DIR typesafe/openjev-latest
python wording.py DATA_DIR typesafe/openjev-latest 1.1
python companions.py DATA_DIR typesafe/openjev-latest 1.1
```

The pause keeps cloud providers under their rate limit. Use `0` for a local model.

## Results, 2026-10-13, jeff 0.1.0

Prices: TypeSafe Jev reference tariff, USD 0.042 per 1M input tokens, output free.

### Direct

| Model | easy | hard | original | total |
|---|---|---|---|---|
| typesafe/openjev-latest | 48/48 | 68/111 | 71/72 | 187/231 (81.0%) |

Average input 679 tokens, median latency 0.80 s, USD 0.0285 per 1,000 decisions. Weakest hard families: temporal_numeric 4/15, tradeoff 2/6, probability 4/10, long_policy 10/19, ambiguous 4/7, multi_hop 12/18.

### Option descriptions

| Model | with descriptions | without |
|---|---|---|
| typesafe/openjev-latest | 182/231 | 183/231 |
| clm/clm-latest | 89/231 | 102/231 |
| clm/clm-raw | 80/230 | 77/230 |

### Neighbour questions

| Model | alone | beside two neutral questions | answers changed |
|---|---|---|---|
| typesafe/openjev-latest | 190/231 | 184/231 | 13 |

The same answer moves between runs by about 0.1 and between batches far more: one urgency question on one message ranged from 0.03 to 1.00 depending on the questions beside it, through jeff and straight at the provider alike.
