"""Canonical-object fixtures shared by recorded-move contract tests."""
from codec_reference import Builder
from codec_verify import Verifier
from operation_reference import replay
from protected_inputs import ProtectedInputs


class MoveFixture:
    def setUp(self):
        self.b = Builder()

    def file(self, data, executable=False):
        return self.b.file(self.b.blob(data), executable)

    def directory(self, tree):
        return self.b.directory(self.b.map({name: self.directory(value) if isinstance(value, dict)
                                          else value for name, value in tree.items()}))

    def state(self, tree):
        return self.b.state(self.directory(tree))

    def protect(self, *roots, authorized=None):
        return ProtectedInputs(self.b.store, set(roots) if authorized is None else authorized, set(roots))

    def batch_evidence(self, base, side, moves, edits=(), copies=()):
        structural = tuple(self.b.move_op(*move) for move in moves) + tuple(copies)
        record = self.b.transform_record(base, structural, edits)
        return self.b.provenance(side, [], [], record)

    def evidence(self, base, side, source, destination, old, occupied=None, edits=(), copies=()):
        return self.batch_evidence(base, side, ((source, destination, old, occupied),), edits, copies)

    def claim(self, base, left, right, result, moves, **options):
        return self.b.provenance(result, [], [], self.b.merge_record(
            source=(base, left, right), moves=moves, **options))

    def conflict(self, base, left, right, region=1):
        return self.b.conflict_entry(self.b.conflict(region, base, [left, right]))

    def root_conflict(self, base, left, right):
        return self.b.state(self.conflict(*(self.b.store[s] for s in (base, left, right))))

    def revision(self, state, parents=()):
        return self.b.revision(state, list(parents), len(self.b.store).to_bytes(16, 'big'),
                               b'fixture', b'', 0)

    def check(self, base, left, right, expected, moves, extra=(), **options):
        # Source results, including canonical conflict grouping, must be symmetric.
        for a, b, evidence in ((left, right, moves), (right, left, moves[::-1])):
            claim = self.claim(base, a, b, expected, evidence, **options)
            roots = (claim, base, a, b, *(e for e in evidence if e is not None), *extra)
            before = dict(self.b.store)
            result = replay(claim, self.protect(*roots), (base, a, b))
            self.assertEqual(result.state, expected)
            self.assertEqual(self.b.store, before)
            verifier = Verifier(result.objects)
            verifier.state(expected)
            self.assertEqual(set(result.objects), verifier.closure)
        return result
