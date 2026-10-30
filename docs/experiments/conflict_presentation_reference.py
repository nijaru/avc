"""Private conflict-presentation observations over protected canonical source.

The caller supplies trusted destination/generation identities, registered manifests
and observed File/Absent/Unknown values. These are NOT filesystem attestations.
No scanner, manifest persistence, marker writes, durable receipts or authority CAS
is implemented. In particular, preparing a manifest does not make it safe to write.
The optional flat-text renderer and its SHA-256 checksum are fixture conventions,
not canonical source encoding or a proposed stable editor format.

Capture keeps the original source plus exact private observations. Pending bytes
never become File replacements implicitly. Confirmation constructs only one exact
whole-entry resolution; root/directory construction belongs to other references.
"""
from dataclasses import dataclass
from hashlib import sha256
from types import MappingProxyType

from codec_reference import Builder, Ref, K_STATE
from codec_records import Edit, Observation, Path, Transform
from codec_verify import Verifier, entry_ref
from operation_reference import Evaluation, ObservationFacts, qualify_authoring
from protected_inputs import ProtectedInputs, Refused


class NoPresentation(Refused):
    pass


@dataclass(frozen=True)
class Context:
    state: Ref
    destination: bytes
    work: bytes
    selection_generation: int
    materialization_generation: int


@dataclass(frozen=True)
class File:
    data: bytes
    executable: bool = False


@dataclass(frozen=True)
class Absent:
    pass


@dataclass(frozen=True)
class Unknown:
    reason: str


@dataclass(frozen=True)
class Manifest:
    context: Context
    path: Path
    conflict: Ref
    codec: str
    generated: File
    checksum: bytes


@dataclass(frozen=True)
class Capture:
    context: Context
    path: Path
    unit: Path
    entry: bytes | None  # original opaque source value, never inferred from markers
    observed: File | Absent | Unknown
    disposition: str  # literal | conflict | pending
    reason: str
    manifest: Manifest | None
    objects: MappingProxyType  # private base closure; observations/manifests inline


@dataclass(frozen=True)
class Resolution:
    state: Ref
    provenance: Ref
    objects: MappingProxyType  # public Provenance/result closure, not private Capture


def locate(inputs, context, path):
    if (context.state.kind != K_STATE
            or type(context.destination) is not bytes or not context.destination
            or type(context.work) is not bytes or not context.work
            or any(type(n) is not int or not 0 <= n < 2**64 for n in
                   (context.selection_generation, context.materialization_generation))):
        raise Refused('invalid trusted presentation context')
    path = Verifier.checked_path(path)
    entry = inputs.payload(context.state)
    verifier = Verifier(inputs.objects)
    verifier.state(context.state)
    for index, name in enumerate(path):
        if entry is None or entry[0] != 3:
            return path[:index], entry, verifier
        entry = verifier.maps[entry_ref(entry)].get(name)
    return path, entry, verifier


def prepare(inputs: ProtectedInputs, context: Context, path: Path) -> Manifest:
    """Generate an optional projection; do not persist it or write marker files."""
    path = Verifier.checked_path(path)
    unit, entry, verifier = locate(inputs, context, path)
    if unit != path or entry is None or entry[0] != 4:
        raise Refused('presentation requires an exact whole Conflict Entry')
    conflict = entry_ref(entry)
    value = verifier.conflicts[conflict]
    terms = (value.base, *value.alternatives)
    if value.region != 0 or any(term is None or term[0] != 1 for term in terms):
        raise NoPresentation('no flat regular-file presentation')
    if len({term[-1] for term in terms}) != 1:
        raise NoPresentation('no flat regular-file presentation')
    contents = [inputs.payload(entry_ref(term)) for term in terms]
    for data in contents:
        if len(data) > 1024 * 1024 or b'\0' in data:
            raise NoPresentation('no bounded text presentation')
        try:
            data.decode('utf-8', 'strict')
        except UnicodeDecodeError as error:
            raise NoPresentation('no bounded text presentation') from error
    chunks = [b'AVC fixture conflict ' + conflict.digest.hex().encode() + b'\n']
    for label, data in zip((b'BASE', b'ALTERNATIVE 1', b'ALTERNATIVE 2'), contents):
        chunks.extend((b'<<<<<<< ' + label + b'\n', data,
                       b'' if data.endswith((b'\n', b'\r')) else b'\n'))
    chunks.append(b'>>>>>>> END\n')
    generated = File(b''.join(chunks), bool(terms[0][-1]))
    return Manifest(context, path, conflict, 'fixture-flat-text-1', generated,
                    sha256(generated.data).digest())


def capture(inputs: ProtectedInputs, context: Context, path: Path,
            observed: File | Absent | Unknown, manifest: Manifest | None = None) -> Capture:
    """Classify one trusted observation without changing any canonical source."""
    if (not isinstance(observed, (File, Absent, Unknown))
            or isinstance(observed, File) and
               (type(observed.data) is not bytes or type(observed.executable) is not bool)
            or isinstance(observed, Unknown) and type(observed.reason) is not str):
        raise Refused('expected an immutable File, Absent or Unknown observation')
    path = Verifier.checked_path(path)
    unit, entry, verifier = locate(inputs, context, path)
    retained = None
    disposition, reason = 'pending', 'opaque, missing or unsupported parent/unit'
    if isinstance(observed, Unknown):
        reason = 'observation unknown; not absence'
    elif unit == path and (entry is None or entry[0] == 1) and path:
        # Marker-looking bytes have no special meaning in ordinary source files.
        disposition, reason = 'literal', 'ordinary observed value'
    elif unit == path and entry is not None and entry[0] == 4:
        try:
            expected = prepare(inputs, context, path)
        except NoPresentation:
            expected = None  # typed/binary/nested conflicts need no presentation
        if expected is not None and manifest == expected:
            retained = manifest
            if observed == manifest.generated:
                disposition, reason = 'conflict', 'unchanged exact bound presentation'
            else:
                reason = 'changed or missing presentation; explicit resolution required'
        else:
            reason = 'missing, stale, incomplete or unsupported presentation binding'
    return Capture(context, path, unit, entry, observed, disposition, reason, retained,
                   MappingProxyType({ref: inputs.objects[ref] for ref in verifier.closure}))


def resolve(captured: Capture, current: Context, confirmed: File | Absent) -> Resolution:
    """Construct the exact confirmed File/deletion, never later editor bytes.

    Context equality is a fixture check, not a transactional native Selection CAS.
    ObservationFacts come from this trusted capture/confirmation boundary; reading
    an arbitrary canonical Observation would not establish those facts.
    """
    if current != captured.context:
        raise Refused('stale destination, work or generation')
    if (not isinstance(confirmed, (File, Absent)) or confirmed != captured.observed
            or isinstance(confirmed, File) and
               (type(confirmed.data) is not bytes or type(confirmed.executable) is not bool)):
        raise Refused('confirmation must bind the exact known captured observation')
    base, path = current.state, captured.path
    inputs = ProtectedInputs(captured.objects, {base}, {base})
    unit, before, _ = locate(inputs, current, path)
    if not path or unit != path or before is None or before[0] != 4:
        raise Refused('fixture requires a whole nonroot Conflict Entry')
    evaluation = Evaluation(inputs)
    builder = evaluation.builder
    chosen = (builder.file(builder.blob(confirmed.data), confirmed.executable)
              if isinstance(confirmed, File) else None)
    # Newly constructed replacement bytes also need explicit protection for replay.
    if chosen is not None:
        blob = entry_ref(chosen)
        inputs = ProtectedInputs(builder.store, {base, blob}, {base, blob})
        evaluation = Evaluation(inputs)
        builder = evaluation.builder
    record = Transform(base, (), (Edit(path, before, chosen),))
    state = evaluation.complete(evaluation.transform(record))
    resolutions = ((path, entry_ref(before), chosen),)
    facts = ObservationFacts(state, Observation(base, None, current.selection_generation,
                             0, None, False, 0, (path,), (path,), (), (), resolutions))
    observation = builder.observation_record(base,
        selection_generation=current.selection_generation, scope=(path,), covered=(path,),
        resolutions=resolutions)
    provenance = builder.provenance(state, [], [],
        builder.transform_record(base, edits=((path, before, chosen),)), observation=observation)
    checked = ProtectedInputs(builder.store, {base, provenance}, {base, provenance})
    qualify_authoring(provenance, checked, facts, (base,), authoring_base=base)
    verifier = Verifier(builder.store)
    verifier.generic(provenance)
    return Resolution(state, provenance,
                      MappingProxyType({ref: builder.store[ref] for ref in verifier.closure}))
