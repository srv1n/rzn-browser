"""Offline regression checks: python3 -m unittest discover -s scripts -p test_book_recovery.py."""
from contextlib import redirect_stdout
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import book_recovery
from book_recovery_common import (ROOT, batch_lock, book_key, digest,
                                  generated_workflow, receipt_files_exist, write_json)
from book_external_download import make_receipt
from book_recovery_publish import publish, verify_container


def epub(path, isbn='9781119854166'):
    with zipfile.ZipFile(path, 'w') as archive:
        archive.writestr('mimetype', 'application/epub+zip')
        archive.writestr('META-INF/container.xml', '<container><rootfile full-path="book.opf"/></container>')
        archive.writestr('book.opf', '<package><metadata><title>Example</title>'
                        '<creator>Author</creator><source>urn:isbn:' + isbn + '</source></metadata></package>')


class BookRecoveryTests(unittest.TestCase):
    def setUp(self):
        parent = ROOT / 'test-results/book-helper-tests'
        parent.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=parent)
        self.addCleanup(self.temporary.cleanup)
        self.folder = Path(self.temporary.name)

    def test_keys_cannot_escape_evidence_and_writers_are_exclusive(self):
        for key in ['../escape', '', '/absolute', 'a/b']:
            with self.assertRaises(ValueError):
                book_key(key)
        with batch_lock(self.folder):
            with self.assertRaises(SystemExit):
                with batch_lock(self.folder):
                    pass
        with batch_lock(self.folder):
            pass

    def test_generated_workflows_are_unique_and_cleaned_after_failure(self):
        with generated_workflow({'steps': []}, self.folder) as first:
            with generated_workflow({'steps': [1]}, self.folder) as second:
                self.assertNotEqual(first, second)
                self.assertEqual(json.loads(first.read_text())['steps'], [])
            self.assertFalse(second.exists())
        self.assertFalse(first.exists())

    def test_edition_is_checked_for_any_key_and_accepts_print_source_isbn(self):
        source = self.folder / 'book.epub'
        epub(source)
        self.assertEqual(verify_container(source, 'epub', {
            'expected_identifiers': ['9781119854166']})['source'], ['urn:isbn:9781119854166'])
        with self.assertRaisesRegex(ValueError, 'ISBN'):
            verify_container(source, 'epub', {'expected_identifiers': ['9781118905555']})

    def test_adoption_rejects_wrong_or_incomplete_files(self):
        source = self.folder / 'book.crdownload'
        source.write_bytes(b'%PDF-incomplete')
        selection = {'title': 'Example', 'ext': 'pdf', 'md5': digest(source, 'md5')}
        with self.assertRaisesRegex(ValueError, 'incomplete'):
            make_receipt(selection, source)
        complete = source.rename(self.folder / 'book.pdf')
        selection['md5'] = '0' * 32
        with self.assertRaisesRegex(ValueError, 'hash'):
            make_receipt(selection, complete)

    def test_publish_preserves_existing_books_and_rejects_modified_receipts(self):
        source = self.folder / 'book.epub'
        epub(source)
        request = {'key': 'new-key', 'title': 'Example', 'author': 'Author',
                   'expected_identifiers': ['9781119854166']}
        write_json(self.folder / 'requests.json', [request])
        receipt = make_receipt(dict(request, ext='epub', md5=digest(source, 'md5')), source)
        write_json(self.folder / 'new-key.get.json', receipt)
        destination = self.folder / 'books'
        destination.mkdir()
        original = destination / 'Example - Author.epub'
        original.write_bytes(b'existing unrelated edition')
        with redirect_stdout(io.StringIO()):
            self.assertEqual(publish(self.folder, destination, 'report.md'), 0)
        self.assertEqual(original.read_bytes(), b'existing unrelated edition')
        self.assertEqual(len(list(destination.glob('*.epub'))), 2)
        self.assertIn('1 of 1', (destination / 'report.md').read_text())
        source.write_bytes(b'changed')
        with redirect_stdout(io.StringIO()):
            self.assertEqual(publish(self.folder, destination, 'report.md'), 1)
        self.assertEqual(json.loads((self.folder / 'verification.json').read_text())['new-key']['status'],
                         'verification_failed')

    def test_resume_checks_file_and_does_not_launch_cli_for_changed_saved_download(self):
        source = self.folder / 'book.epub'
        epub(source)
        receipt = make_receipt({'title': 'Example', 'ext': 'epub', 'md5': digest(source, 'md5')}, source)
        self.assertTrue(receipt_files_exist(receipt))
        write_json(self.folder / 'one.get.json', receipt)
        write_json(self.folder / 'selections.json', [{'key': 'one', 'id': 'reviewed-id'}])
        args = ['book_recovery.py', 'get', str(self.folder / 'selections.json'), str(self.folder)]
        with patch.object(sys, 'argv', args), patch('book_recovery.subprocess.run') as run, redirect_stdout(io.StringIO()):
            self.assertEqual(book_recovery.main(), 0)
            source.write_bytes(b'changed')
            self.assertEqual(book_recovery.main(), 1)
            run.assert_not_called()

    def test_manual_verification_stops_before_the_next_book(self):
        write_json(self.folder / 'selections.json', [{'key': 'one', 'id': 'first'}, {'key': 'two', 'id': 'second'}])
        args = ['book_recovery.py', 'get', str(self.folder / 'selections.json'), str(self.folder)]
        result = subprocess.CompletedProcess([], 1, json.dumps({'success': False, 'status': 'human_action_required'}), '')
        with patch.object(sys, 'argv', args), patch('book_recovery.subprocess.run', return_value=result) as run, redirect_stdout(io.StringIO()):
            self.assertEqual(book_recovery.main(), 2)
            self.assertEqual(run.call_count, 1)
        self.assertFalse((self.folder / 'two.get.json').exists())


if __name__ == '__main__':
    unittest.main()
