"""Sample actual public merge triples; historical output is NOT a truth oracle.

Supply a local bare/ordinary git/git repository (no implicit clone or fetch):
  python docs/experiments/history_merge_study.py --repository /path/to/git.git --jj
Only pinned public object bytes are read. Git/JJ run on owned temporary files;
no project source is executed. Python timings are not comparable to CLI costs.
"""
import argparse
from collections import Counter
from hashlib import sha256
import json
from pathlib import Path
import platform
import tempfile
from time import perf_counter
import tracemalloc

import git_study_inputs
from git_study_inputs import PublicGit, REVISION, fingerprints
import text_merge_reference as driver
import text_merge_study
from text_merge_study import git_merge, jj_merge, run


def digest(data):
    return None if data is None else sha256(data).hexdigest()


def verdict(data, historical):
    return ('conflict' if data is None else
            'clean-equal-historical' if data == historical else 'clean-other')


def alignment_diagnostic(inputs):
    base, left, right = inputs
    if left == right or left == base or right == base:
        return {'route': 'equality-shortcut'}
    try:
        for data in inputs:
            if b'\0' in data:
                return {'route': 'text-ineligible'}
            data.decode('utf-8')
    except UnicodeDecodeError:
        return {'route': 'text-ineligible'}
    try:
        indexes = []
        tokens = driver.lines(base)
        for side in (left, right):
            index = driver.AlignmentIndex(tokens, driver.lines(side))
            if index.count > driver.MAX_ALIGNMENTS:
                raise driver.AlignmentBudget('alignment variant budget exceeded')
            indexes.append(index)
        counts = [index.count for index in indexes]
        pairs = counts[0] * counts[1]
        refused = (pairs > driver.MAX_COMBINATIONS or
                   pairs * sum(map(len, inputs)) > driver.MAX_MERGE_BYTES)
        if not refused:
            # Validate enumeration work without retaining every full anchor set.
            for index in indexes:
                for _ in index.alignments():
                    pass
    except driver.AlignmentBudget as error:
        return {'route': 'alignment-budget', 'reason': str(error)}
    return {'route': 'combination-budget' if refused else 'alignments-qualified',
            'optimal_alignment_counts': counts, 'combinations': pairs}


def study(repository, merge_limit=40, case_limit=64, with_jj=False):
    source = PublicGit(repository)
    commits = source.run('rev-list', '--first-parent', '--merges',
                         f'--max-count={merge_limit}', REVISION).split()
    skipped, merges, records, samples = Counter(), [], [], []
    with tempfile.TemporaryDirectory(prefix='avc-real-history-') as temporary:
        for commit in commits:
            parents = source.parents(commit)
            merge = {'merge': commit.decode(), 'parents': [p.decode() for p in parents]}
            merges.append(merge)
            if commit in source.shallow:
                merge['skip'] = 'shallow-merge-boundary'
                continue
            if len(parents) != 2:
                merge['skip'] = 'not-two-parents'
                continue
            base, reason = source.single_base(*parents)
            if reason:
                merge['skip'] = reason
                continue
            merge['base'] = base.decode()
            left, right = [source.changes(base, parent) for parent in parents]
            paths = sorted(left.keys() & right.keys())
            merge['both_changed_paths'] = len(paths)
            for path in paths:
                a, b = left[path], right[path]
                assert a[0] == b[0] and a[2] == b[2]
                modes = (a[0], a[1], b[1])
                if any(mode not in (b'100644', b'100755') for mode in modes):
                    skipped['not-three-regular-files'] += 1
                    continue
                if len(set(modes)) != 1:
                    skipped['mode-mismatch'] += 1
                    continue
                historical_entry = source.entry(commit, path)
                if historical_entry is None or historical_entry[0] not in (b'100644', b'100755'):
                    skipped['no-regular-historical-output'] += 1
                    continue
                if len(records) >= case_limit:
                    skipped['case-cap'] += 1
                    continue
                oids = (a[2], a[3], b[3])
                inputs = tuple(source.blob(oid) for oid in oids)
                historical = source.blob(historical_entry[2])
                if any(data is None for data in (*inputs, historical)):
                    skipped['blob-over-1MiB'] += 1
                    continue
                tokens = list(map(driver.lines, inputs))
                basecounts = Counter(tokens[0])
                sidecounts = list(map(Counter, tokens[1:]))
                matches = [sum(count * counts[token] for token, count in basecounts.items())
                           for counts in sidecounts]
                start = perf_counter()
                candidate = driver.merge_text(*inputs)
                elapsed = perf_counter() - start
                # Diagnostics are deliberately outside the timed execution.
                diagnostic = alignment_diagnostic(inputs)
                root = Path(temporary) / str(len(records))
                root.mkdir()
                outputs = {'candidate': candidate, 'git': git_merge(root, inputs)}
                if with_jj:
                    outputs['jj'] = jj_merge(root, inputs)
                record = {'merge': commit.decode(), 'base': base.decode(),
                          'parents': [p.decode() for p in parents], 'path_hex': path.hex(),
                          'input_git_oids': [oid.decode() for oid in oids],
                          'input_sha256': list(map(digest, inputs)),
                          'input_bytes': list(map(len, inputs)),
                          'input_lines': list(map(len, tokens)), 'match_pairs': matches,
                          'historical_git_oid': historical_entry[2].decode(),
                          'historical_sha256': digest(historical),
                          'outcomes': {tool: verdict(data, historical) for tool, data in outputs.items()},
                          'output_sha256': {tool: digest(data) for tool, data in outputs.items()},
                          'alignment': diagnostic, 'python_seconds': elapsed}
                records.append(record)
                samples.append(inputs)
    if not records:
        raise ValueError('no eligible triples; inspect the recorded history/limits')
    # Profile one predeclared resource stratum: maximum equal-line match pairs,
    # ties in sample order. Inputs/diagnostics are already resident, untraced.
    index = max(range(len(records)), key=lambda i: sum(records[i]['match_pairs']))
    tracemalloc.start()
    profiled = driver.merge_text(*samples[index])
    _, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    assert digest(profiled) == records[index]['output_sha256']['candidate']
    tools = ('candidate', 'git', 'jj') if with_jj else ('candidate', 'git')
    return {'source': source.metadata(), 'python_version': platform.python_version(),
            'machine': platform.machine(),
            'study_sources': fingerprints(__file__, git_study_inputs.__file__,
                                          text_merge_study.__file__),
            'jj_version': run(Path.cwd(), 'jj', '--version').decode().strip() if with_jj else None,
            'driver_sha256': digest(Path(driver.__file__).read_bytes()),
            'selection': {'first_parent_merge_limit': merge_limit, 'case_limit': case_limit,
                          'order': 'rev-list newest first; raw-byte sorted both-changed paths',
                          'renames': 'not inferred', 'max_blob_bytes': 1_048_576},
            'scope': 'One public project/release; selected same-path regular files, not general '
                     'accuracy or semantic correctness. Historical resolution is a comparison '
                     'reference; amendments can explain different clean output. No CLI speedup claim.',
            'summary': {tool: dict(Counter(row['outcomes'][tool] for row in records)) for tool in tools},
            'alignment_routes': dict(Counter(row['alignment']['route'] for row in records)),
            'merge_skips': dict(Counter(row['skip'] for row in merges if 'skip' in row)),
            'path_skips': dict(skipped),
            'python_allocation_probe': {'case_index': index, 'incremental_peak_bytes': peak,
                                        'selection': 'maximum sum of match pairs; inputs resident; '
                                                     'separate deterministic replay, not RSS'},
            'merges': merges, 'cases': records}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', required=True)
    parser.add_argument('--merges', type=int, default=40)
    parser.add_argument('--cases', type=int, default=64)
    parser.add_argument('--jj', action='store_true')
    args = parser.parse_args()
    if not 1 <= args.merges <= 120 or not 1 <= args.cases <= 128:
        parser.error('bounds: 1..120 merges and 1..128 cases')
    print(json.dumps(study(args.repository, args.merges, args.cases, args.jj), indent=2))
