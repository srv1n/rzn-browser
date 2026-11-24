#!/usr/bin/env python3
"""Download an observed LibGen detail link, or adopt a completed browser file by exact MD5."""
import argparse
import json
from pathlib import Path
import re
import subprocess
from urllib.parse import parse_qs, urlparse
from book_recovery_common import (ROOT, batch_lock, book_key, cli, digest,
                                  generated_workflow, read_items, receipt_files_exist, write_json)


def make_receipt(selection, source):
    if source.suffix.lower() in ['.crdownload', '.part', '.download']:
        raise ValueError('Browser download is incomplete')
    if digest(source, 'md5') != selection['md5'].lower():
        raise ValueError('Wrong exact file hash; receipt not published')
    book = {key: selection[key] for key in ['id', 'title', 'author', 'publisher', 'year',
            'language', 'pages', 'size', 'ext'] if key in selection}
    return {'success': True, 'status': 'downloaded', 'book': book, 'files': [{
        'path': str(source.resolve()), 'bytes': source.stat().st_size, 'sha256': digest(source)}]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence', type=Path)
    parser.add_argument('key', type=book_key)
    parser.add_argument('url', nargs='?', help='Observed detail URL containing the exact md5 query value')
    parser.add_argument('--selections', type=Path, help='Reviewed metadata; defaults to evidence/mirror-selections.json')
    parser.add_argument('--local-file', type=Path, help='Adopt a completed browser download instead of navigating')
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    selection = next(item for item in read_items(args.selections or evidence / 'mirror-selections.json')
                     if item['key'] == args.key)
    if not re.fullmatch(r'[a-fA-F0-9]{32}', selection['md5']):
        raise ValueError('Invalid exact MD5')
    if selection['ext'] not in ['epub', 'pdf'] or not selection.get('title'):
        raise ValueError('Reviewed title and PDF/EPUB format are required')
    if bool(args.url) == bool(args.local_file):
        parser.error('Provide either an observed detail URL or --local-file')
    receipt_path = evidence / f'{args.key}.get.json'
    with batch_lock(evidence):
        if receipt_path.exists():
            previous = json.loads(receipt_path.read_text())
            if previous.get('success'):
                if not receipt_files_exist(previous):
                    raise ValueError('Existing receipt has a missing or changed file; inspect before retry')
                print(args.key, 'already downloaded; receipt checked')
                return 0
        if args.local_file:
            source = args.local_file.resolve()
        else:
            url = urlparse(args.url)
            if url.scheme not in ['http', 'https'] or url.username or url.password:
                raise ValueError('Expected a public HTTP(S) detail URL')
            if parse_qs(url.query).get('md5', [''])[0].lower() != selection['md5'].lower():
                raise ValueError('Detail URL does not identify the selected exact file')
            workflow = json.loads((ROOT / 'workflows/book/book-get.json').read_text())
            workflow['params']['properties'] = {}
            workflow['steps'] = [workflow['steps'][0], {
                'id': 'settle', 'timeout_ms': 2000,
                'action': {'kind': 'wait_for_timeout', 'inputs': {}, 'side_effects': ['read_only']},
            }, workflow['steps'][-1]]
            workflow['steps'][0]['action']['inputs']['url'] = args.url
            script = "const book = " + json.dumps(selection) + ";\n" + r"""
if (document.querySelector('#ddg-captcha') || /DDoS-Guard|Just a moment/i.test(document.title))
  throw new Error('site_challenge: complete manual verification in the browser');
const link = Array.from(document.querySelectorAll('a[href]')).find(a => /^GET$/i.test(a.textContent.trim()));
if (!link) throw new Error('No observed GET link');
const filename = book.title.replace(/[\\/:*?"<>|]/g, ' ').slice(0,120) + '.' + book.ext;
return {attachment_urls:[{url:link.href, filename}]};
"""
            workflow['steps'][-1]['action']['inputs']['script'] = script
            with generated_workflow(workflow, evidence) as generated:
                result = subprocess.run([cli(), 'run', str(generated), '--download-dir', str(evidence / args.key)],
                                        capture_output=True, text=True)
            (evidence / f'{args.key}.external-run.txt').write_text(result.stdout + '\n' + result.stderr)
            if result.returncode:
                print(result.stderr[-1200:])
                return 2 if 'site_challenge' in result.stderr or 'bot challenge' in result.stderr else 1
            manifest = json.loads((evidence / args.key / 'manifest.json').read_text())
            if len(manifest['items']) != 1 or manifest['items'][0].get('error'):
                raise ValueError('Expected one successfully downloaded attachment')
            source = (evidence / args.key / manifest['items'][0]['path']).resolve()
            if not source.is_relative_to((evidence / args.key).resolve()):
                raise ValueError('Attachment path escaped the download folder')
        receipt = make_receipt(selection, source)
        write_json(receipt_path, receipt)
        print(args.key, 'downloaded and exact MD5 verified', receipt['files'][0]['bytes'])
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
