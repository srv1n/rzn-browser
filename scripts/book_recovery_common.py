"""Shared local file handling for the foreground book recovery tools."""
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import tempfile

ROOT = Path(__file__).resolve().parents[1]

def book_key(value):
    if not isinstance(value, str) or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_-]{0,79}', value):
        raise ValueError('Use a short key containing letters, digits, underscores or hyphens')
    return value

def read_items(path):
    items = json.loads(Path(path).read_text())
    keys = [book_key(item['key']) for item in items]
    if len(keys) != len(set(keys)):
        raise ValueError('Duplicate book keys')
    return items

def digest(path, algorithm='sha256'):
    result = hashlib.new(algorithm)
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            result.update(block)
    return result.hexdigest()

def receipt_files_exist(receipt):
    files = receipt.get('files', [])
    return bool(files) and all(
        Path(item['path']).is_file()
        and Path(item['path']).stat().st_size == item['bytes']
        and digest(item['path']) == item['sha256'] for item in files)

def write_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix='.' + path.name, dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as stream:
            json.dump(value, stream, ensure_ascii=False, indent=2, default=str)
            stream.write('\n')
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)

@contextmanager
def batch_lock(evidence):
    evidence.mkdir(parents=True, exist_ok=True)
    with (evidence / '.recovery.lock').open('a') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise SystemExit('This evidence folder already has an active recovery writer')
        yield

@contextmanager
def generated_workflow(workflow, folder):
    folder.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix='.book-workflow-', suffix='.json', dir=folder)
    try:
        with os.fdopen(fd, 'w') as stream:
            json.dump(workflow, stream)
        yield Path(name)
    finally:
        Path(name).unlink(missing_ok=True)

def cli():
    binary = os.environ.get('RZN_BOOK_CLI')
    if binary:
        return binary
    debug = ROOT / 'target/debug/rzn-browser'
    return str(debug) if debug.is_file() else (shutil.which('rzn-browser') or 'rzn-browser')
