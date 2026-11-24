#!/usr/bin/env python3
"""Foreground, resumable search/get batches. See --help and the recovery SOP."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
from book_recovery_common import ROOT, batch_lock, cli, read_items, receipt_files_exist, write_json


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=['search', 'get'])
    parser.add_argument('source', type=Path, help='Reviewed requests or exact-ID selections JSON')
    parser.add_argument('evidence', type=Path)
    parser.add_argument('--base-url', default='https://libgen.li/')
    parser.add_argument('--mirror-urls', default='https://annas-archive.gl/')
    parser.add_argument('--sort', choices=['relevance', 'year_desc', 'year_asc'], default='relevance')
    parser.add_argument('--max-attempts', type=int, choices=[1, 2, 3], default=3)
    parser.add_argument('--limit', type=int, default=25)
    parser.add_argument('--refresh', action='store_true', help='Refresh search results; get receipts stay protected')
    args = parser.parse_args()
    items = read_items(args.source)
    evidence = args.evidence.resolve()
    env = dict(os.environ, RZN_BUILTIN_WORKFLOWS_DIR=str(ROOT / 'workflows'))
    failures = False
    with batch_lock(evidence):
        for item in items:
            key = item['key']
            output = evidence / f'{key}.{args.mode}.json'
            if output.exists():
                saved = json.loads(output.read_text())
                if saved.get('success') is True:
                    if args.mode == 'get':
                        if receipt_files_exist(saved):
                            print(key, 'already downloaded; receipt checked', flush=True)
                        else:
                            print(key, 'saved receipt has a missing or changed file; inspect before retry', flush=True)
                            failures = True
                        continue
                    if not args.refresh and time.time() - output.stat().st_mtime < 24 * 3600:
                        print(key, 'using recent search results', flush=True)
                        continue
            if args.mode == 'search':
                params = ['query=' + item['query'], 'base_url=' + args.base_url,
                          'mirror_urls=' + args.mirror_urls, 'sort=' + args.sort,
                          'max_attempts=' + str(args.max_attempts), 'limit=' + str(args.limit)]
            else:
                params = ['id=' + item['id']]
            command = [cli(), 'run', 'book', args.mode, '--agent-output']
            for param in params:
                command += ['--param', param]
            if args.mode == 'get':
                command += ['--download-dir', str(evidence / key)]
            print(key, 'starting', flush=True)
            result = subprocess.run(command, env=env, capture_output=True, text=True)
            try:
                value = json.loads(result.stdout)
                if not isinstance(value, dict):
                    raise ValueError('Expected result object')
            except ValueError:
                value = {'success': False, 'status': 'invalid_runner_output'}
            if result.returncode and value.get('success'):
                value = {'success': False, 'status': 'runner_failed'}
            write_json(output, value)
            print(key, value.get('status'), flush=True)
            failures |= value.get('success') is not True
            if value.get('status') in ['human_action_required', 'browser_unavailable']:
                print('Batch stopped: browser needs attention.', flush=True)
                return 2
    return 1 if failures else 0


if __name__ == '__main__':
    raise SystemExit(main())
