"""Read-only inputs shared by opt-in studies of a pinned public Git release.

No clone, checkout, source execution or lazy object acquisition. Supply a local
repository containing git/git v2.48.1 and its bounded history. Object and path
bytes are data; output artifacts contain metadata/hashes, never source blobs.
"""
from collections import Counter
from hashlib import sha256
from pathlib import Path
import subprocess

from text_merge_study import environment

UPSTREAM = 'https://github.com/git/git.git'
REVISION = 'f93ff170b93a1782659637824b25923245ac9dd1'
TAG = 'v2.48.1'


class PublicGit:
    def __init__(self, repository):
        self.repository = Path(repository).resolve()
        grafts = Path(self.run('rev-parse', '--git-path', 'info/grafts').strip().decode())
        if not grafts.is_absolute():
            grafts = self.repository / grafts
        if grafts.exists():
            raise ValueError('grafted history is not a public study input')
        if self.run('rev-parse', f'{REVISION}^{{commit}}').strip().decode() != REVISION:
            raise ValueError('the pinned public commit is required')
        self.shallow = set()
        if self.run('rev-parse', '--is-shallow-repository').strip() == b'true':
            path = Path(self.run('rev-parse', '--git-path', 'shallow').strip().decode())
            if not path.is_absolute():
                path = self.repository / path
            self.shallow = set(path.read_bytes().split())

    def run(self, *args, check=True):
        env = environment()
        env['GIT_NO_LAZY_FETCH'] = '1'
        result = subprocess.run(['git', '--no-pager', '--no-replace-objects',
                                 '-c', 'core.hooksPath=/dev/null',
                                 '-C', str(self.repository), *args], env=env,
                                capture_output=True, timeout=30, check=check)
        return result.stdout if check else result

    @staticmethod
    def tree_records(data):
        result = {}
        for row in data.split(b'\0'):
            if row:
                metadata, path = row.split(b'\t', 1)
                mode, kind, oid = metadata.split()
                result[path] = (mode, kind, oid)
        return result

    def snapshot(self):
        return self.tree_records(self.run('ls-tree', '-r', '-z', REVISION))

    def entry(self, revision, path):
        rows = self.tree_records(self.run('--literal-pathspecs', 'ls-tree', '-z',
                                          revision, '--', path))
        return rows.get(path)

    def parents(self, commit):
        header = self.run('cat-file', 'commit', commit).split(b'\n\n', 1)[0]
        return [line[7:] for line in header.splitlines() if line.startswith(b'parent ')]

    def changes(self, base, side):
        rows = self.run('diff-tree', '--no-commit-id', '--no-ext-diff', '--no-renames',
                        '--raw', '--no-abbrev', '-r', '-z', base, side).split(b'\0')
        changes = {}
        for index in range(0, len(rows) - 1, 2):
            oldmode, mode, oldoid, oid, status = rows[index][1:].split()
            changes[rows[index + 1]] = (oldmode, mode, oldoid, oid, status)
        return changes

    def single_base(self, left, right):
        result = self.run('merge-base', '--all', left, right, check=False)
        if result.returncode not in (0, 1):
            raise RuntimeError(result.stderr.decode(errors='replace'))
        bases = result.stdout.split()
        if len(bases) != 1:
            return None, 'no-known-base' if not bases else 'multiple-bases'
        base = bases[0]
        # A shallow boundary outside the known base's ancestry could conceal
        # another maximal base. Never interpret truncated history as absence.
        remainder = set(self.run('rev-list', left, right, '--not', base).split())
        if remainder & self.shallow:
            return None, 'incomplete-frontier'
        return base, None

    def blob(self, oid, limit=1_048_576):
        size = int(self.run('cat-file', '-s', oid))
        return None if size > limit else self.run('cat-file', 'blob', oid)

    def metadata(self):
        return {'upstream': UPSTREAM, 'tag': TAG, 'revision': REVISION,
                'git_version': self.run('--version').decode().strip(),
                'reachable_commits': int(self.run('rev-list', '--count', REVISION)),
                'shallow_boundaries': len(self.shallow),
                'snapshot_modes': dict(Counter(row[0].decode() for row in self.snapshot().values()))}


def fingerprints(*paths):
    return {Path(path).name: sha256(Path(path).read_bytes()).hexdigest() for path in paths}
