"""Runs every prompt through `claude -p`, without the mod and with it, 3 times; one JSON row per run in results.jsonl.

Usage: python run.py [none|mod]   (both by default). Then: python summarize.py
"""
import json
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).parent
MOD = str(HERE.parent.resolve())
WORK = Path(tempfile.mkdtemp(prefix='jeff-bench-'))
TASKS = [
    ('mermaid', ['mermaid-diagrams'], 'Draw a mermaid sequence diagram of an OAuth login'),
    ('commit', ['caveman:caveman-commit'], 'Write a Conventional Commits message for this change: added retries with exponential backoff to the HTTP client'),
    ('rust', ['rust-best-practices'], 'Review this Rust function for idiomatic issues: fn first(v: &Vec<String>) -> String { v[0].clone() }'),
    ('jeff', ['jeff'], 'Which saved classifiers does my jeff have? Use the jeff CLI.'),
    ('find', ['find-skills'], 'Is there an installable agent skill that can help me automate Slack? Find one for me.'),
    ('slop', ['kill-ai-slop', 'humanizer'], "Rewrite this landing page line so it does not sound AI-generated: 'Not just a tool - it's a revolution. Unlock seamless synergy.'"),
    ('slidev', ['slidev'], 'Make a 3-slide Slidev deck about Rust ownership; just print the slides.md content.'),
    ('monad', [], 'What is a monad? Answer in two sentences.'),
    ('tcp', [], 'Explain the difference between TCP and UDP in three bullets.'),
    ('french', [], "Translate 'good morning, see you tomorrow' into French and Spanish."),
]
ARMS = {'none': [], 'mod': ['--plugin-dir', MOD]}


def run(arm, task):
    name, expected, prompt = task
    cmd = ['claude', '-p', prompt, '--output-format', 'stream-json', '--verbose', *ARMS[arm], '--allowedTools', 'Bash(jeff:*)']
    started = time.time()
    out = subprocess.run(cmd, cwd=WORK, capture_output=True, text=True, encoding='utf-8', timeout=600).stdout
    row = {'arm': arm, 'task': name, 'expected': expected, 'wall_s': round(time.time() - started, 1), 'skills': [], 'mod': None}
    for line in out.splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get('subtype') == 'ui_log' and m.get('plugin') == 'jeff-skill-suggestion':
            row['mod'] = m.get('text')
        if m.get('type') == 'assistant':
            for c in m['message']['content']:
                if c.get('type') == 'tool_use' and c['name'] == 'Skill':
                    row['skills'].append(c['input'].get('skill'))
        if m.get('type') == 'result':
            u = m.get('usage') or {}
            row.update(duration_ms=m.get('duration_ms'), cost=m.get('total_cost_usd'), turns=m.get('num_turns'),
                       input=u.get('input_tokens'), cache_write=u.get('cache_creation_input_tokens'),
                       cache_read=u.get('cache_read_input_tokens'), output=u.get('output_tokens'))
    return row


if __name__ == '__main__':
    with open(HERE / 'results.jsonl', 'a', encoding='utf-8') as f:
        for attempt in range(3):
          for task in TASKS:
            for arm in sys.argv[1:] or ARMS:
                row = run(arm, task)
                row['attempt'] = attempt
                f.write(json.dumps(row) + '\n')
                f.flush()
                print(row, flush=True)
