"""Real SQLite boundary probes, NOT an AVC object store or power-loss test.

Single-object toy closure; identifiers are fixture labels, not native ObjectIDs.
Exercise bounded staging beyond SQLite's row limit, atomic root/receipt admission,
current-pin GC exclusion and self-contained online backup. Native canonical graph
validation, sync qualification, throughput and multi-object GC remain engine gates.
"""
from contextlib import closing
from hashlib import sha256
from pathlib import Path
import sqlite3
from tempfile import TemporaryDirectory
import unittest


class SQLiteCustodyQualification(unittest.TestCase):
    def setUp(self):
        self.temp = TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / 'live.db'
        self.db = self.connect(self.path)
        self.db.executescript('''
            CREATE TABLE objects(id TEXT PRIMARY KEY, size INTEGER, verified INTEGER);
            CREATE TABLE parts(object TEXT REFERENCES objects(id) ON DELETE CASCADE,
                seq INTEGER, data BLOB, PRIMARY KEY(object,seq));
            CREATE TABLE pins(id TEXT PRIMARY KEY, object TEXT REFERENCES objects(id));
            CREATE TABLE roots(name TEXT PRIMARY KEY, object TEXT REFERENCES objects(id));
            CREATE TABLE receipts(key TEXT PRIMARY KEY, object TEXT REFERENCES objects(id));
            CREATE TABLE outbox(event TEXT PRIMARY KEY);
        ''')

    def connect(self, path):
        db = sqlite3.connect(path)
        self.addCleanup(db.close)
        self.assertEqual(db.execute('PRAGMA journal_mode=WAL').fetchone()[0], 'wal')
        db.execute('PRAGMA synchronous=FULL')
        db.execute('PRAGMA foreign_keys=ON')
        self.assertEqual(db.execute('PRAGMA synchronous').fetchone()[0], 2)
        self.assertEqual(db.execute('PRAGMA foreign_keys').fetchone()[0], 1)
        return db

    def count(self, table):
        return self.db.execute(f'SELECT count(*) FROM {table}').fetchone()[0]

    def test_segmented_staging_atomic_acknowledgement_and_backup(self):
        payload = bytes(range(256)) * 100  # 25,600 bytes, above this connection's row limit
        self.db.setlimit(sqlite3.SQLITE_LIMIT_LENGTH, 8192)
        with self.db:
            self.db.execute('INSERT INTO objects VALUES(?,?,0)', ('blob', len(payload)))
            self.db.execute('INSERT INTO pins VALUES(?,?)', ('stage', 'blob'))
        with self.assertRaises(sqlite3.DataError), self.db:
            self.db.execute('INSERT INTO parts VALUES(?,?,?)', ('blob', 0, payload))
        for seq, start in enumerate(range(0, len(payload), 1024)):
            with self.db:  # earlier bounded commits are not root acknowledgements
                self.db.execute('INSERT INTO parts VALUES(?,?,?)',
                                ('blob', seq, payload[start:start + 1024]))
            self.assertEqual((self.count('roots'), self.count('receipts')), (0, 0))
        rows = self.db.execute('SELECT seq,data FROM parts ORDER BY seq').fetchall()
        self.assertEqual([i for i, _ in rows], list(range(len(rows))))
        recovered = b''.join(data for _, data in rows)
        self.assertEqual(len(recovered), len(payload))
        self.assertEqual(sha256(recovered).digest(), sha256(payload).digest())

        def admission():
            self.db.execute('UPDATE objects SET verified=1 WHERE id=?', ('blob',))
            self.db.execute('INSERT INTO roots VALUES(?,?)', ('authored', 'blob'))
            self.db.execute('INSERT INTO receipts VALUES(?,?)', ('request', 'blob'))
            self.db.execute('INSERT INTO outbox VALUES(?)', ('accepted',))
            self.db.execute('DELETE FROM pins WHERE id=?', ('stage',))

        with self.assertRaisesRegex(RuntimeError, 'interrupted'), self.db:
            admission()
            raise RuntimeError('interrupted before commit')
        self.assertEqual((self.count('roots'), self.count('receipts'), self.count('outbox')), (0, 0, 0))
        self.assertEqual(self.count('pins'), 1)
        self.assertEqual(self.db.execute('SELECT verified FROM objects').fetchone()[0], 0)
        with self.db:
            admission()
        destination = Path(self.temp.name) / 'backup.db'
        with closing(sqlite3.connect(destination)) as backup:
            self.db.backup(backup, pages=1)
        # Remove the live source; reconstruction must depend only on the snapshot.
        self.db.close()
        self.path.unlink()
        with closing(sqlite3.connect(destination)) as backup:
            self.assertEqual(backup.execute('PRAGMA quick_check').fetchone()[0], 'ok')
            root = backup.execute('SELECT object FROM roots WHERE name=?', ('authored',)).fetchone()[0]
            self.assertEqual(backup.execute('SELECT verified FROM objects WHERE id=?', (root,)).fetchone()[0], 1)
            data = b''.join(row[0] for row in backup.execute(
                'SELECT data FROM parts WHERE object=? ORDER BY seq', (root,)))
            self.assertEqual(data, payload)
            self.assertEqual(backup.execute('SELECT object FROM receipts').fetchone()[0], root)

    def test_gc_rechecks_current_pins_and_rejects_late_pin(self):
        with self.db:
            self.db.execute('INSERT INTO objects VALUES(?,?,1)', ('old', 0))
        # Advisory scan before another authority connection protects the object.
        candidate = self.db.execute('SELECT id FROM objects').fetchone()[0]
        other = self.connect(self.path)
        with other:
            other.execute('INSERT INTO pins VALUES(?,?)', ('reader', candidate))
        statement = '''DELETE FROM objects WHERE id=?
            AND NOT EXISTS(SELECT 1 FROM pins WHERE object=objects.id)
            AND NOT EXISTS(SELECT 1 FROM roots WHERE object=objects.id)'''
        with self.db:
            self.assertEqual(self.db.execute(statement, (candidate,)).rowcount, 0)
        with other:
            other.execute('DELETE FROM pins WHERE id=?', ('reader',))
        with self.db:
            self.assertEqual(self.db.execute(statement, (candidate,)).rowcount, 1)
        # A check against the old scan cannot resurrect now-deleted bytes.
        with self.assertRaises(sqlite3.IntegrityError), other:
            other.execute('INSERT INTO pins VALUES(?,?)', ('late', candidate))


if __name__ == '__main__':
    unittest.main()
