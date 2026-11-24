#!/usr/bin/env python3
"""Bounded HTTP probes of links actually offered by a book detail page.

Input: JSON [{name, url}]. Saves latency, content type, and a small response
sample; a responsive HTML page is not counted as a successful file download.
"""
import concurrent.futures
import json
from pathlib import Path
import sys
import time
import urllib.request

def probe(item):
    started = time.monotonic()
    try:
        request = urllib.request.Request(item['url'], headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(request, timeout=15) as response:
            data = response.read1(65536)
            return dict(item, status=response.status, seconds=round(time.monotonic()-started, 3),
                        content_type=response.headers.get('Content-Type'), bytes=len(data),
                        file=data.startswith((b'%PDF-', b'PK\x03\x04')),
                        final_url=response.url, sample=data.decode('utf8', 'replace') if 'html' in (response.headers.get('Content-Type') or '') else '')
    except Exception as error:
        return dict(item, error=str(error), seconds=round(time.monotonic()-started, 3))

items = json.loads(Path(sys.argv[1]).read_text())
with concurrent.futures.ThreadPoolExecutor(max_workers=5) as pool:
    results = list(pool.map(probe, items))
Path(sys.argv[2]).write_text(json.dumps(results, indent=2))
for result in results:
    print({k: v for k, v in result.items() if k not in ['sample', 'url', 'final_url']})
