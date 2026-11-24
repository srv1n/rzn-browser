#!/usr/bin/env python3
"""Inspect actual offered mirror links in the existing browser session."""
import json
from pathlib import Path
import subprocess
import sys
from book_recovery_common import cli, generated_workflow

root = Path(__file__).resolve().parents[1]
workflow = json.loads((root / 'workflows/book/book-search.json').read_text())
workflow['steps'] = [workflow['steps'][0], {
    'id': 'settle', 'timeout_ms': 15000 if '/search?' in sys.argv[1] else 12000 if '/download/' in sys.argv[1] else 5000,
    'action': {'kind': 'wait_for_timeout', 'inputs': {}, 'side_effects': ['read_only']},
}, workflow['steps'][-1]]
workflow['steps'][0]['action']['inputs']['url'] = sys.argv[1]
workflow['steps'][-1]['action']['inputs']['script'] = """
if (document.querySelector('#ddg-captcha')) throw new Error('site_challenge: complete manual verification in the browser');
return {url: location.href, title: document.title,
 text: (document.querySelector('main') || document.body).innerText.slice(-16000),
 download_controls: Array.from(document.querySelectorAll('*')).filter(e=>e.textContent.trim()==='Download').slice(-8).map(e=>e.outerHTML.slice(0,2000)),
 controls: Array.from(document.querySelectorAll('button, input, [role=button]')).map(e=>e.outerHTML.slice(0,2000)),
 links: Array.from(document.querySelectorAll('a[href]')).map(a => ({text:a.innerText,href:a.href}))};
"""
if len(sys.argv) > 3:
    workflow['steps'][2:2] = [
        {'id':'wait_download', 'timeout_ms':20000, 'action':{'kind':'wait_for_element', 'target':{'selector':':is(' + sys.argv[3] + '):not([disabled])'}, 'inputs':{}, 'side_effects':['read_only']}},
        {'id':'open_download', 'timeout_ms':10000, 'action':{'kind':'click_element', 'target':{'selector':sys.argv[3]}, 'inputs':{}, 'side_effects':['browser_state']}},
        {'id':'download_settle', 'timeout_ms':5000, 'action':{'kind':'wait_for_timeout', 'inputs':{}, 'side_effects':['read_only']}},
    ]
with generated_workflow(workflow, Path(sys.argv[2]).resolve().parent) as generated:
    result = subprocess.run([cli(), 'run', str(generated), '--param', 'query=mirror inspection'], capture_output=True, text=True)
    Path(sys.argv[2]).write_text(result.stdout + '\n' + result.stderr)
    print(result.stdout[-30000:] if result.returncode == 0 else result.stderr[-1500:])
sys.exit(result.returncode)
