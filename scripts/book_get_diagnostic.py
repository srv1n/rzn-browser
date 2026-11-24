#!/usr/bin/env python3
"""Reproduce a book/get transition and print the reached page, without downloading.

Run after `make build-rust`:
  python3 scripts/book_get_diagnostic.py --param query='On Directing Film' \
      --param base_url=https://annas-archive.gl/
"""
import json
from pathlib import Path
import subprocess
import sys
from book_recovery_common import cli, generated_workflow

root = Path(__file__).resolve().parents[1]
workflow = json.loads((root / 'workflows/book/book-get.json').read_text())
probe_start = next(i for i, step in enumerate(workflow['steps']) if step['id'].startswith('download_probe_'))
workflow['steps'] = workflow['steps'][:probe_start] + [workflow['steps'][-1]]
workflow['steps'][-1]['action']['inputs']['script'] = r"""
return {url: location.href, title: document.title,
  page_text: document.querySelector('main')?.innerText.slice(-6000),
  state: JSON.parse(sessionStorage.getItem('__rzn_book_selection') || 'null'),
  links: Array.from(document.querySelectorAll('main a[href]'))
    .filter(a => /slow_download|fast_download|get\.php/.test(a.href) || /download now/i.test(a.innerText))
    .map(a => ({text: a.innerText, href: a.href}))};
"""
with generated_workflow(workflow, root / 'test-results/book-diagnostics') as generated:
    result = subprocess.run([cli(), 'run', str(generated), *sys.argv[1:]], cwd=root)
sys.exit(result.returncode)
