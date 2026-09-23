#!/usr/bin/env python3
"""Seeded repair fixture for finite Slotbook checks; not model-generated output."""
from datetime import datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import sqlite3
import threading
from urllib.parse import parse_qs, urlsplit
import uuid

config = json.loads(Path('/config/application.json').read_text())
resource, capacity = config['resource'], config['capacity']
if not isinstance(capacity, int) or isinstance(capacity, bool) or capacity <= 0:
    raise ValueError('positive integer capacity required')
token = Path('/config/token').read_text().strip()
lock = threading.Lock()
db = sqlite3.connect('/data/bookings.sqlite', check_same_thread=False)
db.execute('CREATE TABLE IF NOT EXISTS reservations (id TEXT PRIMARY KEY, name TEXT, start TEXT, end TEXT, quantity INTEGER, cancelled INTEGER)')
db.commit()


def timestamp(value):
    if not isinstance(value, str) or not value.endswith('Z') or len(value) > 32:
        raise ValueError('UTC timestamp required')
    return datetime.fromisoformat(value[:-1] + '+00:00')


def rows():
    return [dict(zip(('id', 'name', 'start', 'end', 'quantity'), row)) for row in
            db.execute('SELECT id, name, start, end, quantity FROM reservations WHERE cancelled=0 ORDER BY id')]


def available(a, b):
    events = []
    for row in rows():
        lo, hi = max(a, timestamp(row['start'])), min(b, timestamp(row['end']))
        if lo < hi:
            events.extend(((lo, row['quantity']), (hi, -row['quantity'])))
    current, peak = 0, 0
    for _, delta in sorted(events):
        current += delta
        peak = max(current, peak)
    return capacity - peak


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def reply(self, code, body):
        content = json.dumps(body, separators=(',', ':')).encode()
        self.send_response(code)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(content)))
        self.end_headers()
        self.wfile.write(content)

    def authorized(self):
        if self.headers.get('Authorization') != 'Bearer ' + token:
            self.reply(401, {'error': 'authorization required'})
            return False
        return True

    def do_GET(self):
        path = urlsplit(self.path)
        if path.path == '/health':
            return self.reply(200, {'status': 'ready'})
        if path.path == '/reservations':
            if not self.authorized():
                return
            with lock:
                return self.reply(200, rows())
        if path.path != '/availability':
            return self.reply(404, {'error': 'absent'})
        try:
            query = parse_qs(path.query)
            start, end = query['start'][0], query['end'][0]
            a, b = timestamp(start), timestamp(end)
            if a >= b:
                raise ValueError()
            with lock:
                return self.reply(200, {'resource': resource, 'start': start, 'end': end, 'remaining': available(a, b)})
        except (ValueError, KeyError, IndexError):
            return self.reply(400, {'error': 'invalid interval'})

    def do_POST(self):
        if not self.authorized():
            return
        if self.path != '/reservations':
            return self.reply(404, {'error': 'absent'})
        try:
            length = int(self.headers.get('Content-Length', '0'))
            if not 0 < length <= 8192:
                raise ValueError()
            value = json.loads(self.rfile.read(length))
            name, start, end, quantity = (value[k] for k in ('name', 'start', 'end', 'quantity'))
            a, b = timestamp(start), timestamp(end)
            if a >= b or not isinstance(name, str) or not 0 < len(name) <= 128 or type(quantity) is not int or quantity <= 0:
                raise ValueError()
            with lock:
                if available(a, b) < quantity:
                    return self.reply(409, {'error': 'insufficient capacity'})
                identity = uuid.uuid4().hex
                db.execute('INSERT INTO reservations VALUES (?, ?, ?, ?, ?, 0)', (identity, name, start, end, quantity))
                db.commit()
                return self.reply(201, {'id': identity})
        except (ValueError, KeyError, TypeError):
            return self.reply(400, {'error': 'invalid reservation'})

    def do_DELETE(self):
        if not self.authorized():
            return
        if not self.path.startswith('/reservations/'):
            return self.reply(404, {'error': 'absent'})
        identity = self.path.removeprefix('/reservations/')
        with lock:
            row = db.execute('SELECT start, cancelled FROM reservations WHERE id=?', (identity,)).fetchone()
            if row is None:
                return self.reply(404, {'error': 'absent'})
            if not row[1]:
                if timestamp(Path('/config/clock').read_text().strip()) >= timestamp(row[0]):
                    return self.reply(409, {'error': 'cancellation cutoff reached'})
                db.execute('UPDATE reservations SET cancelled=1 WHERE id=?', (identity,))
                db.commit()
            return self.reply(200, {'cancelled': True})


ThreadingHTTPServer(('0.0.0.0', 8080), Handler).serve_forever()
