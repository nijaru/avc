"""Real SQLite boundary probes, NOT an AVC object store or power-loss test.

Toy single-object and shared-descendant fixtures use labels, not native ObjectIDs.
Exercise bounded staging, atomic root/receipt admission, online backup, protection
epochs, logical retirement and bounded segment cleanup. Known immutable manifests
abstract closure reservation; native decoding/traversal, process-crash recovery,
sync qualification and resource behavior remain engine gates.
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

    def seed_graph(self):
        # This independent SQL fixture does not import the state model. Decoded
        # edges have a parent FK only: an unrooted header can outlive a child.
        self.db.executescript('''
            ALTER TABLE objects ADD COLUMN state TEXT NOT NULL DEFAULT 'live'
                CHECK(state IN ('live','retired'));
            ALTER TABLE pins ADD COLUMN lease_version INTEGER NOT NULL DEFAULT 0;
            CREATE TABLE edges(parent TEXT REFERENCES objects(id) ON DELETE CASCADE,
                child TEXT, owning INTEGER NOT NULL, PRIMARY KEY(parent,child));
            CREATE TABLE protection(generation INTEGER NOT NULL
                CHECK(typeof(generation)='integer' AND generation>=0));
            INSERT INTO protection VALUES(0);
        ''')
        for table in ('roots', 'pins'):
            for operation in ('INSERT', 'UPDATE', 'DELETE'):
                self.db.execute(f'''CREATE TRIGGER {table}_{operation}
                    AFTER {operation} ON {table} BEGIN
                    UPDATE protection SET generation=generation+1; END''')
        with self.db:
            for obj in ('A', 'B', 'shared', 'a', 'b', 'audit'):
                parts = [(obj, seq, f'{obj}:{seq}'.encode())
                         for seq in range(5 if obj == 'b' else 1)]
                self.db.execute('INSERT INTO objects(id,size,verified) VALUES(?,?,1)',
                                (obj, sum(len(data) for _, _, data in parts)))
                self.db.executemany('INSERT INTO parts VALUES(?,?,?)', parts)
            self.db.executemany('INSERT INTO edges VALUES(?,?,?)',
                                [('A', 'shared', 1), ('A', 'a', 1), ('A', 'audit', 0),
                                 ('B', 'shared', 1), ('B', 'b', 1)])
            self.db.execute('INSERT INTO roots VALUES(?,?)', ('selected', 'A'))

    @staticmethod
    def epoch(db):
        return db.execute('SELECT generation FROM protection').fetchone()[0]

    def lease_manifest(self, db, key, root, manifest):
        # The tiny manifest is known independently, not discovered by traversing
        # an unprotected root. This is not a native graph-validation algorithm.
        with db:
            db.execute('BEGIN IMMEDIATE')
            placeholders = ','.join('?' for _ in manifest)
            resident = {r[0] for r in db.execute(f'''SELECT id FROM objects
                WHERE state='live' AND verified=1 AND id IN ({placeholders})''', tuple(manifest))}
            if resident != set(manifest):
                raise ValueError('unavailable closure')
            db.execute('INSERT INTO pins(id,object) VALUES(?,?)', (key, root))

    def scan(self):
        with self.db:
            self.db.execute('BEGIN IMMEDIATE')
            roots = {r[0] for r in self.db.execute('SELECT object FROM roots UNION SELECT object FROM pins')}
            self.db.executemany('INSERT INTO pins(id,object) VALUES(?,?)',
                                [(f'scan:{root}', root) for root in sorted(roots)])
            epoch = self.epoch(self.db)  # AFTER the collector's own lease.
        return roots, epoch

    def mark(self, roots):
        self.assertFalse(self.db.in_transaction)  # tracing is outside the writer
        values = ','.join('(?)' for _ in roots)
        return {r[0] for r in self.db.execute(f'''WITH RECURSIVE reach(id) AS (
            VALUES {values} UNION SELECT child FROM edges JOIN reach ON parent=reach.id
            WHERE owning=1) SELECT id FROM reach''', tuple(roots))}

    def retire(self, scan, marked, candidate):
        with self.db:
            self.db.execute('BEGIN IMMEDIATE')
            if self.epoch(self.db) != scan[1]:
                self.db.execute("DELETE FROM pins WHERE id LIKE 'scan:%'")
                return 'restart'
            if candidate in marked:
                return 'protected'
            changed = self.db.execute("UPDATE objects SET state='retired' WHERE id=? AND state='live'",
                                      (candidate,)).rowcount
            return 'retired' if changed else 'skip'

    def finish_scan(self):
        with self.db:
            self.db.execute("DELETE FROM pins WHERE id LIKE 'scan:%'")

    @staticmethod
    def clean_parts(db, obj):
        # The state guard also excludes an old cleanup task after key reuse.
        with db:
            db.execute('BEGIN IMMEDIATE')
            return db.execute('''DELETE FROM parts WHERE object=? AND seq IN (
                SELECT seq FROM parts WHERE object=? ORDER BY seq LIMIT 2)
                AND EXISTS(SELECT 1 FROM objects WHERE id=? AND state='retired')''',
                              (obj, obj, obj)).rowcount

    @staticmethod
    def delete_header(db, obj):
        with db:
            return db.execute('''DELETE FROM objects WHERE id=? AND state='retired'
                AND NOT EXISTS(SELECT 1 FROM parts WHERE object=objects.id)''', (obj,)).rowcount

    def test_closure_scan_restarts_after_stage_and_root_transfer(self):
        self.seed_graph()
        other = self.connect(self.path)
        scan = self.scan()
        marked = self.mark(scan[0])
        self.assertEqual(marked, {'A', 'shared', 'a'})
        self.lease_manifest(other, 'stage', 'B', {'B', 'shared', 'b'})
        self.assertEqual(self.retire(scan, marked, 'b'), 'restart')
        self.assertEqual(other.execute("SELECT state FROM objects WHERE id='b'").fetchone()[0], 'live')

        scan = self.scan()
        with other:
            other.execute("UPDATE roots SET object='B' WHERE name='selected'")
            other.execute("DELETE FROM pins WHERE id='stage'")
        self.assertEqual(self.db.execute("SELECT object FROM pins WHERE id='scan:A'").fetchone()[0], 'A')
        self.assertEqual(self.mark(scan[0]), {'A', 'B', 'shared', 'a', 'b'})
        self.assertEqual(self.retire(scan, self.mark(scan[0]), 'a'), 'restart')
        scan = self.scan()
        marked = self.mark(scan[0])
        self.assertEqual(marked, {'B', 'shared', 'b'})
        self.assertEqual(self.retire(scan, marked, 'shared'), 'protected')
        self.assertEqual(self.retire(scan, marked, 'A'), 'retired')
        self.assertEqual(self.retire(scan, marked, 'audit'), 'retired')
        self.finish_scan()

    def test_retirement_bounded_cleanup_and_late_closure_lease(self):
        self.seed_graph()
        other = self.connect(self.path)
        scan = self.scan()
        self.assertEqual(self.retire(scan, self.mark(scan[0]), 'b'), 'retired')
        self.finish_scan()
        self.assertEqual(other.execute("SELECT state FROM objects WHERE id='B'").fetchone()[0], 'live')
        for root, manifest in (('B', {'B', 'shared', 'b'}), ('b', {'b'})):
            with self.subTest(root=root), self.assertRaisesRegex(ValueError, 'unavailable closure'):
                self.lease_manifest(other, 'late', root, manifest)
        with self.assertRaises(sqlite3.IntegrityError), other:
            other.execute("INSERT INTO objects(id,size,verified) VALUES('b',0,1)")
        self.assertEqual(self.delete_header(self.db, 'b'), 0)
        self.assertEqual(self.clean_parts(self.db, 'b'), 2)
        # Reopen between committed cleanup batches; the retired header remains.
        other.close()
        other = self.connect(self.path)
        self.assertEqual(other.execute("SELECT state FROM objects WHERE id='b'").fetchone()[0], 'retired')
        self.assertEqual(self.clean_parts(other, 'b'), 2)
        self.assertEqual(self.delete_header(other, 'b'), 0)
        self.assertEqual(self.clean_parts(other, 'b'), 1)
        self.assertEqual(self.delete_header(other, 'b'), 1)
        # The B->b edge did not turn its unrooted target into an artificial GC
        # root. Restage the SAME fixture payload only after old cleanup finishes.
        self.assertEqual(other.execute("SELECT child FROM edges WHERE parent='B' AND child='b'").fetchone()[0], 'b')
        with other:
            other.execute('BEGIN IMMEDIATE')
            other.execute("INSERT INTO objects(id,size,verified) VALUES('b',15,1)")
            other.executemany('INSERT INTO parts VALUES(?,?,?)',
                              [('b', seq, f'b:{seq}'.encode()) for seq in range(5)])
            other.execute("INSERT INTO pins(id,object) VALUES('new-stage','b')")
        self.assertEqual(self.clean_parts(self.db, 'b'), 0)
        self.assertEqual(self.delete_header(self.db, 'b'), 0)
        self.assertEqual(other.execute("SELECT seq,data FROM parts WHERE object='b' ORDER BY seq").fetchall(),
                         [(seq, f'b:{seq}'.encode()) for seq in range(5)])
        self.lease_manifest(other, 'repaired-B', 'B', {'B', 'shared', 'b'})

    def test_lease_renewal_expiry_and_generation_exhaustion(self):
        self.seed_graph()
        self.lease_manifest(self.db, 'reader', 'A', {'A', 'shared', 'a'})
        before = self.epoch(self.db)
        with self.db:
            self.db.execute("UPDATE pins SET lease_version=lease_version+1 WHERE id='reader'")
        self.assertEqual(self.epoch(self.db), before + 1)
        with self.db:
            self.db.execute("DELETE FROM pins WHERE id='reader'")
        self.assertEqual(self.epoch(self.db), before + 2)
        with self.db:
            self.db.execute('UPDATE protection SET generation=?', (2**63 - 1,))
        # SQLite promotes overflowing addition to REAL. Reject it transactionally
        # rather than accepting a reused/rounded epoch or a half-created lease.
        with self.assertRaises(sqlite3.IntegrityError), self.db:
            self.db.execute("INSERT INTO pins(id,object) VALUES('overflow','A')")
        self.assertEqual(self.epoch(self.db), 2**63 - 1)
        self.assertEqual(self.db.execute("SELECT count(*) FROM pins WHERE id='overflow'").fetchone()[0], 0)


if __name__ == '__main__':
    unittest.main()
