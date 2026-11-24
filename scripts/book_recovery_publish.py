#!/usr/bin/env python3
"""Verify recovery receipts and publish books without overwriting existing files."""
import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
from urllib.parse import quote
import xml.etree.ElementTree as ET
import zipfile
from book_recovery_common import batch_lock, digest, read_items, write_json


def require(condition, message):
    if not condition:
        raise ValueError(message)


def verify_container(source, ext, request):
    if ext == 'epub':
        with zipfile.ZipFile(source) as archive:
            require(archive.testzip() is None, 'EPUB ZIP integrity failure')
            require(archive.read('mimetype').strip() == b'application/epub+zip', 'Not an EPUB')
            container = ET.fromstring(archive.read('META-INF/container.xml'))
            opf = container.find('.//{*}rootfile').attrib['full-path']
            package = ET.fromstring(archive.read(opf))
            metadata = {field: [e.text or '' for e in package.findall('.//{*}metadata/{*}' + field)]
                        for field in ['title', 'creator', 'date', 'publisher', 'identifier', 'source']}
            expected = request.get('expected_identifiers', [])
            if expected:
                identifiers = [re.sub(r'[^0-9Xx]', '', value) for value in metadata['identifier'] + metadata['source']]
                require(any(str(isbn) in identifiers for isbn in expected), 'Requested edition ISBN not confirmed')
            return metadata
    if ext == 'pdf':
        with source.open('rb') as stream:
            require(stream.read(5) == b'%PDF-', 'Not a PDF')
        info = subprocess.run(['pdfinfo', str(source)], check=True, capture_output=True, text=True).stdout
        pages = int(re.search(r'^Pages:\s+(\d+)', info, re.M).group(1))
        require(pages >= request.get('min_pdf_pages', 20), 'Possible review scan: PDF is too short')
        subprocess.run(['pdftotext', str(source), '-'], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        # An edition requirement needs evidence inside the PDF, not its index label.
        if request.get('expected_identifiers'):
            text = subprocess.run(['pdftotext', '-f', '1', '-l', '15', str(source), '-'],
                                  check=True, capture_output=True, text=True).stdout
            compact = re.sub(r'[^0-9Xx]', '', text)
            require(any(str(isbn) in compact for isbn in request['expected_identifiers']),
                    'Requested edition ISBN not confirmed in the PDF front matter')
        return {'pages': pages, 'pdf_info': info}
    raise ValueError('Unverified container type: ' + str(ext))


def publish(evidence, destination, report_name):
    requests = {item['key']: item for item in read_items(evidence / 'requests.json')}
    destination.mkdir(parents=True, exist_ok=True)
    report = {}
    for key, request in requests.items():
        receipt = evidence / f'{key}.get.json'
        if not receipt.exists():
            report[key] = {'status': 'pending'}
            continue
        try:
            result = json.loads(receipt.read_text())
            if result.get('status') != 'downloaded':
                report[key] = {'status': result.get('status', 'failed')}
                continue
            require(len(result['files']) == 1, 'Expected exactly one file')
            item = result['files'][0]
            source = Path(item['path'])
            require(source.stat().st_size == item['bytes'], 'Receipt size mismatch')
            require(digest(source) == item['sha256'], 'Receipt hash mismatch')
            ext = result['book']['ext']
            metadata = verify_container(source, ext, request)
            name = request.get('filename') or request['title'] + ' - ' + request['author']
            name = re.sub(r'[\\/:*?"<>|]', '', name).strip()
            require(name not in ['', '.', '..'], 'Invalid destination filename')
            target = destination / (name + '.' + ext)
            if target.exists() and digest(target) != item['sha256']:
                target = target.with_stem(target.stem + ' - ' + item['sha256'][:8])
            if not target.exists():
                with target.open('xb') as output:
                    try:
                        with source.open('rb') as stream:
                            shutil.copyfileobj(stream, output)
                    except BaseException:
                        target.unlink(missing_ok=True)
                        raise
            require(digest(target) == item['sha256'], 'Published file hash mismatch')
            report[key] = {'status': 'verified', 'path': str(target), 'bytes': item['bytes'],
                           'sha256': item['sha256'], 'book': result['book'], 'metadata': metadata}
            print(key, 'verified', target.name, flush=True)
        except Exception as error:
            report[key] = {'status': 'verification_failed', 'error': str(error)}
            print(key, 'verification failed:', error, flush=True)
    if (evidence / 'unavailable.json').exists():
        for key, value in json.loads((evidence / 'unavailable.json').read_text()).items():
            if key in requests and report[key]['status'] != 'verified':
                report[key] = value
    write_json(evidence / 'verification.json', report)
    verified = sum(item['status'] == 'verified' for item in report.values())
    lines = ['# Book recovery — ' + evidence.name, '',
             f'{verified} of {len(requests)} titles verified. Existing files were preserved.', '',
             'Checks: receipt size and SHA-256; EPUB ZIP integrity and metadata; full PDF parsing and page count. Edition requirements come from requests.json. Container checks alone do not establish the correct title or author: review the metadata and title pages.', '']
    if (evidence / 'batch-notes.md').exists():
        lines += [(evidence / 'batch-notes.md').read_text().strip(), '']
    lines += ['| Book | Status | File |', '|---|---|---|']
    for key, request in requests.items():
        item = report[key]
        target = item.get('path')
        link = '[' + Path(target).name + '](' + quote(Path(target).name) + ')' if target else '—'
        if not target and item.get('purchase_url'):
            link = '[Access option](' + item['purchase_url'] + ')'
        title = request['title'].replace('|', '\\|')
        lines.append(f'| {title} | {item["status"]} | {link} |')
    (destination / report_name).write_text('\n'.join(lines) + '\n')
    return 1 if any(item['status'] == 'verification_failed' for item in report.values()) else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence', type=Path)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--report-name', help='Report filename; defaults to BOOK-RECOVERY-<evidence folder>.md')
    args = parser.parse_args()
    report_name = args.report_name or 'BOOK-RECOVERY-' + args.evidence.resolve().name + '.md'
    require(Path(report_name).name == report_name, 'Report name must be a filename')
    with batch_lock(args.evidence.resolve()):
        return publish(args.evidence.resolve(), args.destination.resolve(), report_name)


if __name__ == '__main__':
    raise SystemExit(main())
