"""Controlled text edits on pinned real source, not a real-history UX benchmark.

Run with Python, Git and optionally Jujutsu; all writes are in a temporary tree.
Print JSON. The pinned source is data, never imported or executed. Git/Jujutsu
are comparison baselines, not AVC backends. No timing/speedup claim is made.
"""
from collections import Counter
from hashlib import sha256
import json
import os
import platform
from pathlib import Path
import shutil
import subprocess
import tempfile

import text_merge_reference as driver

SOURCE_REVISION = '19710524aa87cdd076bd8755257fa17b71bb7a69'
SOURCES = ('docs/experiments/codec_verify.py', 'docs/experiments/authority_model.py',
           'src/cli.rs', 'src/commands/save.rs')
ROOT = Path(__file__).resolve().parents[2]


def environment():
    env = {k: v for k, v in os.environ.items() if not k.startswith(('GIT_', 'JJ_'))}
    env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull,
               GIT_TERMINAL_PROMPT='0', JJ_CONFIG=os.devnull)
    return env


def run(cwd, *args):
    return subprocess.run(args, cwd=cwd, env=environment(), capture_output=True,
                          check=True, timeout=30).stdout


def cases():
    for name in SOURCES:
        base = run(ROOT, 'git', 'show', f'{SOURCE_REVISION}:{name}')
        tokens = driver.lines(base)
        counts = Counter(tokens)
        unique = [i for i, token in enumerate(tokens) if token.strip() and counts[token] == 1]
        repeated = [i for i in range(1, len(tokens))
                    if tokens[i] in (b'\n', b'\r\n') and tokens[i] == tokens[i - 1]]
        for kind in ('replace', 'delete-blank', 'insert-blank'):
            if kind != 'replace' and not repeated:
                continue
            i = unique[len(unique) // 4] if kind == 'replace' else repeated[0]
            j = next(k for k in reversed(unique) if k > i + 10)
            def annotate(index, label):
                token = tokens[index]
                content = token.rstrip(b'\r\n')
                comment = b' # avc-' if name.endswith('.py') else b' // avc-'
                return [content + comment + label + token[len(content):]]
            change = ((i, i + 1, annotate(i, b'left')) if kind == 'replace' else
                      (i, i + (kind == 'delete-blank'),
                       [] if kind == 'delete-blank' else [tokens[i]]))
            other = (j, j + 1, annotate(j, b'right'))
            def apply(edits):
                result = tokens.copy()
                for start, end, value in sorted(edits, reverse=True):
                    result[start:end] = value
                return b''.join(result)
            # Expected bytes come from the two declared disjoint edits, not a merger.
            yield name, kind, (base, apply([change]), apply([other])), apply([change, other])


def verdict(result, expected):
    return 'conflict' if result is None else 'intended' if result == expected else 'other'


def git_merge(root, inputs):
    for name, data in zip(('base', 'left', 'right'), inputs):
        (root / name).write_bytes(data)
    result = subprocess.run(['git', 'merge-file', '-p', str(root / 'left'),
                             str(root / 'base'), str(root / 'right')], cwd=root,
                            env=environment(), capture_output=True, timeout=30)
    if not 0 <= result.returncode <= 127:
        raise RuntimeError(result.stderr.decode(errors='replace'))
    return result.stdout if result.returncode == 0 else None


def jj_merge(root, inputs):
    def jj(*args, cwd=None):
        return run(cwd or root / 'repo', 'jj', '--no-pager', '--color=never',
                   '--config=user.name="AVC merge study"',
                   '--config=user.email="study@example.invalid"',
                   '--config=signing.behavior="drop"', *args)
    jj('git', 'init', 'repo', cwd=root)
    source = root / 'repo' / 'source.txt'
    source.write_bytes(inputs[0])
    jj('describe', '-m', 'base')
    def identity():
        return jj('log', '-r', '@', '--no-graph', '-T', 'commit_id').decode().strip()
    base = identity()
    parents = []
    for label, data in zip(('left', 'right'), inputs[1:]):
        jj('new', base, '-m', label)
        source.write_bytes(data)
        jj('describe', '-m', label)
        parents.append(identity())
    jj('new', *parents, '-m', 'merge')
    conflicted = jj('log', '-r', '@', '--no-graph', '-T', 'conflict').strip()
    if conflicted not in (b'true', b'false'):
        raise RuntimeError(f'Unexpected Jujutsu conflict result: {conflicted!r}')
    return None if conflicted == b'true' else jj('file', 'show', 'source.txt', '-r', '@')


def study():
    has_jj = shutil.which('jj') is not None
    records = []
    with tempfile.TemporaryDirectory(prefix='avc-text-study-') as temporary:
        for index, (name, kind, inputs, expected) in enumerate(cases()):
            root = Path(temporary) / str(index)
            root.mkdir()
            base, left, right = inputs
            aligned = [driver.unique_alignment(driver.lines(base), driver.lines(side))
                       for side in (left, right)]
            candidate = driver.merge_text(*inputs)
            # The preceding candidate's policy refused every ambiguous alignment.
            old = None if any(a is None for a in aligned) else candidate
            records.append({'source': name, 'edit': kind,
                            'input_sha256': [sha256(data).hexdigest() for data in inputs],
                            'expected_sha256': sha256(expected).hexdigest(),
                            'unique_only': verdict(old, expected),
                            'confluent': verdict(candidate, expected),
                            'git': verdict(git_merge(root, inputs), expected),
                            'jj': verdict(jj_merge(root, inputs), expected) if has_jj else None})
    return {'source_revision': SOURCE_REVISION,
            'driver_sha256': sha256(Path(driver.__file__).read_bytes()).hexdigest(),
            'python_version': platform.python_version(),
            'git_version': run(ROOT, 'git', '--version').decode().strip(),
            'jj_version': run(ROOT, 'jj', '--version').decode().strip() if has_jj else None,
            'scope': 'Controlled disjoint edits on pinned source; not sampled merge histories, '
                     'semantic correctness, a usability trial, or a latency benchmark.',
            'summary': {tool: dict(Counter(row[tool] for row in records if row[tool] is not None))
                        for tool in ('unique_only', 'confluent', 'git', 'jj')},
            'cases': records}


if __name__ == '__main__':
    print(json.dumps(study(), indent=2))
