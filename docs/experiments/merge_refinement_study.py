"""Replay the exact previously measured public triples, without re-sampling.

Run: python docs/experiments/merge_refinement_study.py --repository /path/to/git.git
The pinned baseline is recorded data, not executable historical source. No
clone/fetch, checkout, source execution or automatic Git/JJ rerun. Prior results
are reused only after exact public role/blob/content bindings are rechecked.
"""
import argparse
from array import array
from collections import Counter
from hashlib import sha256
import json
from pathlib import Path
import platform
from time import perf_counter
import tracemalloc

import git_study_inputs
from git_study_inputs import PublicGit, fingerprints
import history_merge_study
from history_merge_study import alignment_diagnostic, digest, verdict
import text_merge_reference as driver

BASELINE = Path(__file__).with_name('history_merge_results.json')
BASELINE_SHA256 = '44e77673b67b198c6cccf2d7d770ff79630c04466166b22f713e35ea3a999021'


def study(repository):
    raw = BASELINE.read_bytes()
    if sha256(raw).hexdigest() != BASELINE_SHA256:
        raise ValueError('the pinned public baseline has changed')
    baseline = json.loads(raw)
    source = PublicGit(repository)
    cases, samples = [], []
    for index, previous in enumerate(baseline['cases']):
        path = bytes.fromhex(previous['path_hex'])
        parents = [p.decode() for p in source.parents(previous['merge'])]
        assert parents == previous['parents']
        for revision, oid in zip([previous['base'], *parents], previous['input_git_oids']):
            assert source.entry(revision, path)[2].decode() == oid
        assert source.entry(previous['merge'], path)[2].decode() == previous['historical_git_oid']
        inputs = tuple(source.blob(oid) for oid in previous['input_git_oids'])
        historical = source.blob(previous['historical_git_oid'])
        assert list(map(digest, inputs)) == previous['input_sha256']
        assert digest(historical) == previous['historical_sha256']
        start = perf_counter()
        result = driver.merge_text(*inputs)
        elapsed = perf_counter() - start
        # An old clean result must not change or become a new budget conflict.
        if previous['outcomes']['candidate'] != 'conflict':
            assert digest(result) == previous['output_sha256']['candidate']
        cases.append({'baseline_case_index': index, 'outcome': verdict(result, historical),
                      'output_sha256': digest(result), 'alignment': alignment_diagnostic(inputs),
                      'python_seconds': elapsed})
        samples.append(inputs)
    index = baseline['python_allocation_probe']['case_index']
    tracemalloc.start()
    profiled = driver.merge_text(*samples[index])
    _, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    assert digest(profiled) == cases[index]['output_sha256']
    return {'source': source.metadata(), 'baseline_sha256': BASELINE_SHA256,
            'baseline_driver_sha256': baseline['driver_sha256'],
            'driver_sha256': digest(Path(driver.__file__).read_bytes()),
            'study_sources': fingerprints(__file__, git_study_inputs.__file__,
                                          history_merge_study.__file__),
            'python_version': platform.python_version(), 'machine': platform.machine(),
            'unsigned_array_item_bytes': array('I').itemsize,
            'scope': 'Exact same 64 public file triples; historical bytes are comparison '
                     'references, not a correctness oracle. Prior Git/JJ outcomes are reused, '
                     'not rerun. Allocation probes use the same case and resident-input baseline; '
                     'function timings are unpaired samples, not a throughput/speedup claim.',
            'summary': dict(Counter(row['outcome'] for row in cases)),
            'alignment_routes': dict(Counter(row['alignment']['route'] for row in cases)),
            'new_clean_cases': [i for i, row in enumerate(cases) if row['outcome'] != 'conflict'
                                and baseline['cases'][i]['outcomes']['candidate'] == 'conflict'],
            'python_allocation_probe': {'baseline_case_index': index,
                                        'baseline_incremental_peak_bytes': baseline['python_allocation_probe']['incremental_peak_bytes'],
                                        'incremental_peak_bytes': peak},
            'cases': cases}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', required=True)
    args = parser.parse_args()
    print(json.dumps(study(args.repository), indent=2))
