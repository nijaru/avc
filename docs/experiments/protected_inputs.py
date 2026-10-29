"""Authorized, protected full-closure snapshots for bounded reference readers.

This is not a sparse store, a permission system implementation or a GC lease.
An audit commitment never acquires access or retention implicitly.
"""
from types import MappingProxyType
from codec_reference import Ref, K_PROV
from codec_verify import Invalid, Verifier


class Refused(ValueError):
    pass


class NotAuthorized(Refused):
    pass


class EvidenceUnavailable(Refused):
    pass


class ProtectedInputs:
    """A trusted caller supplies grants and separately acquires full-root pins."""
    def __init__(self, objects: dict[Ref, bytes], authorized: set[Ref], pins: set[Ref]):
        self.authorized = frozenset(authorized)
        protected = {}
        for root in sorted(pins):
            if root not in authorized:
                raise NotAuthorized('input root not authorized')
            verifier = Verifier(objects)
            try:
                verifier.generic(root)
            except Invalid as error:
                if str(error) == 'missing required object':
                    raise EvidenceUnavailable('incomplete input closure') from error
                raise
            protected.update((ref, objects[ref]) for ref in verifier.closure)
        self.objects = MappingProxyType(protected)

    def payload(self, ref: Ref) -> bytes:
        if ref in self.objects:
            return self.objects[ref]
        if ref in self.authorized:
            raise EvidenceUnavailable('authorized input was not protected')
        raise NotAuthorized('audit commitment is not a read grant')

    def provenance(self, ref: Ref):
        if ref.kind != K_PROV:
            raise Refused('expected Provenance')
        self.payload(ref)
        verifier = Verifier(self.objects)
        verifier.generic(ref)
        return verifier.provenances[ref]
