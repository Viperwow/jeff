"""Turns results.jsonl into the numbers BENCHMARK.md reports."""
import json
import re
import statistics
from pathlib import Path

ROWS = [json.loads(line) for line in open(Path(__file__).parent / 'results.jsonl', encoding='utf-8')]


def loaded(row):
    """Skills the run loaded: by the Skill tool, or by the mod's injection named in its log line."""
    skills = {name for name in row['skills'] if name}
    pick = re.match(r'(?:Skill: )?/(\S+)', row['mod'] or '')
    if pick:
        skills.add(pick.group(1))
    return skills


def right(row):
    """Right when an expected skill loaded; for a prompt that expects none, when nothing loaded."""
    return bool(loaded(row) & set(row['expected'])) if row['expected'] else not loaded(row)


def context(row):
    return row['input'] + row['cache_write'] + row['cache_read']


def change(without, with_mod):
    return f'{(with_mod - without) / without:+.0%}'


arms = {arm: [row for row in ROWS if row['arm'] == arm] for arm in ('none', 'mod')}
metrics = {
    'Right skill chosen': lambda rows: sum(right(row) for row in rows),
    'Right where a skill is needed': lambda rows: sum(right(row) for row in rows if row['expected']),
    'Cost, $': lambda rows: round(sum(row['cost'] for row in rows), 2),
    'Context of a one-turn run': lambda rows: round(statistics.mean(context(row) for row in rows if row['turns'] == 1)),
    'Tokens written to the cache, per run': lambda rows: round(statistics.mean(row['cache_write'] for row in rows)),
    'Model turns': lambda rows: sum(row['turns'] for row in rows),
    'Time, s': lambda rows: round(sum(row['duration_ms'] for row in rows) / 1000),
}
for name, metric in metrics.items():
    without, with_mod = metric(arms['none']), metric(arms['mod'])
    print(f'{name}: {without} without the mod, {with_mod} with it ({change(without, with_mod)})')
