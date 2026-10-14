#!/usr/bin/env python3
"""Reproducible VCS contract counterexamples; not an AVC implementation/benchmark.

Requires Python 3.11+ and Git. Writes only in a fresh temporary directory.
Run: python probes.py [--output results.json]
A successful run means the stated counterexamples were reproduced, not that a
candidate VCS has passed a durability, security, or performance qualification.
"""
from __future__ import annotations

import argparse
import itertools
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import threading
from pathlib import Path
from typing import Any


def command(root: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    # Exclude caller-specific Git state, hooks and signing settings.
    for key in tuple(env):
        if key.startswith('GIT_'):
            del env[key]
    env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull,
               GIT_TERMINAL_PROMPT='0')
    return subprocess.run(args, cwd=root, env=env, text=True,
                          capture_output=True, check=check, timeout=20)


def git(root: Path, *args: str) -> str:
    return command(root, 'git', *args).stdout.strip()


def repo(root: Path) -> None:
    root.mkdir()
    git(root, 'init', '-q', '-b', 'main')
    git(root, 'config', 'user.name', 'VCS contract probe')
    git(root, 'config', 'user.email', 'probe@example.invalid')
    git(root, 'config', 'commit.gpgsign', 'false')
    git(root, 'config', 'core.hooksPath', os.devnull)


def commit(root: Path, message: str) -> str:
    git(root, 'add', '--all')
    git(root, 'commit', '-q', '-m', message)
    return git(root, 'rev-parse', 'HEAD')


def identical_deletions(root: Path) -> dict[str, Any]:
    """A snapshot revert and a causal un-delete have different contracts."""
    repo(root)
    source = root / 'source.txt'
    source.write_text('keep\nobsolete\n')
    base = commit(root, 'base')
    git(root, 'checkout', '-q', '-b', 'a')
    source.write_text('keep\n')
    a = commit(root, 'A independently deletes obsolete')
    git(root, 'checkout', '-q', '-b', 'b', base)
    source.write_text('keep\n')
    b = commit(root, 'B independently deletes obsolete')
    assert a != b
    assert git(root, 'rev-parse', a + '^{tree}') == git(root, 'rev-parse', b + '^{tree}')
    git(root, 'checkout', '-q', 'a')
    git(root, 'merge', '--no-ff', '-m', 'merge independently authored deletions', 'b')
    merged = source.read_text()
    git(root, 'revert', '--no-edit', a)
    reverted = source.read_text()
    assert merged == 'keep\n'
    assert reverted == 'keep\nobsolete\n'
    return {'reproduced': True, 'after_merge': merged, 'after_revert_A': reverted,
            'meaning': 'Git applies the inverse content change. It does not promise to retract only A\'s deletion label while retaining B\'s identical deletion effect.',
            'not_established': 'No Git defect or superiority of a change-graph VCS is inferred.'}


def semantic_interference(root: Path) -> dict[str, Any]:
    """Two individually passing branches merge cleanly but fail together."""
    repo(root)
    (root / 'api.py').write_text('def value():\n    return 1\n')
    (root / 'consumer.py').write_text('def render(value):\n    return str(value())\n')
    base = commit(root, 'base')
    test = 'from api import value; from consumer import render; assert isinstance(render(value), str)'
    git(root, 'checkout', '-q', '-b', 'a')
    (root / 'api.py').write_text('def value():\n    return "1"\n')
    commit(root, 'A changes API representation')
    # -B avoids creating bytecode; fresh temp dirs avoid stale bytecode caches.
    command(root, sys.executable, '-B', '-c', test)
    git(root, 'checkout', '-q', '-b', 'b', base)
    (root / 'consumer.py').write_text('def render(value):\n    return str(value() + 1)\n')
    commit(root, 'B increments in the caller')
    command(root, sys.executable, '-B', '-c', test)
    merge = command(root, 'git', 'merge', '--no-ff', '-m', 'combine', 'a')
    combined = command(root, sys.executable, '-B', '-c', test, check=False)
    assert merge.returncode == 0 and combined.returncode != 0
    assert 'TypeError' in combined.stderr
    return {'reproduced': True, 'individual_tests_pass': True, 'text_merge_succeeds': True,
            'combined_test_pass': False, 'failure': combined.stderr.strip().splitlines()[-1],
            'meaning': 'Disjoint writes and successful merge do not validate the composed program.'}


def restore_race(root: Path) -> dict[str, Any]:
    """Force, rather than probabilistically wait for, a check/replace race."""
    root.mkdir()
    target = root / 'file.txt'
    staged = root / 'staged.txt'
    target.write_text('captured old bytes\n')
    staged.write_text('restored historic bytes\n')
    checked, edited = threading.Event(), threading.Event()
    failures: list[str] = []
    def editor() -> None:
        if not checked.wait(5):
            failures.append('check event timed out')
            return
        target.write_text('new unobserved editor bytes\n')
        edited.set()
    worker = threading.Thread(target=editor, daemon=True)
    worker.start()
    assert target.read_text() == 'captured old bytes\n'
    checked.set()
    assert edited.wait(5)
    # This is atomic name replacement, not an atomic compare-content-and-replace.
    os.replace(staged, target)
    worker.join(5)
    assert not worker.is_alive() and not failures
    overwritten = target.read_text()
    assert overwritten == 'restored historic bytes\n'
    target.write_text('new unobserved editor bytes\n')
    recovery = root / 'new-recovery-directory'
    recovery.mkdir()
    (recovery / 'file.txt').write_text('restored historic bytes\n')
    assert target.read_text() == 'new unobserved editor bytes\n'
    return {'reproduced': True, 'check_then_replace_loses_intervening_write': True,
            'restore_into_new_directory_preserves_original': True,
            'meaning': 'A pre-write content check alone cannot promise no-clobber against an uncoordinated concurrent writer.',
            'not_established': 'No production checkout protocol or cross-platform power-loss guarantee was tested.'}


def gc_interleavings() -> dict[str, Any]:
    """Tiny state model, not a file-store GC implementation."""
    def run(order: tuple[str, ...], pinned: bool) -> bool:
        objects: set[str] = set()
        roots: set[str] = set()
        pins: set[str] = set()
        doomed: set[str] = set()
        for event in order:
            if event == 'prepare':
                objects.add('x')
                if pinned:
                    pins.add('x')
            elif event == 'mark':
                doomed = objects - roots - pins
            elif event == 'publish':
                roots.add('x')
                pins.discard('x')
            else:
                objects -= doomed
        return roots <= objects
    valid = [p for p in itertools.permutations(('prepare','publish','mark','sweep'))
             if p.index('prepare') < p.index('publish') and p.index('mark') < p.index('sweep')]
    unsafe = [list(p) for p in valid if not run(p, False)]
    fixed = [list(p) for p in valid if not run(p, True)]
    assert len(valid) == 6 and unsafe and not fixed
    return {'reproduced': True, 'valid_interleavings_checked': len(valid),
            'unsafe_without_staging_pin': unsafe, 'unsafe_with_model_staging_pin': fixed,
            'meaning': 'Root CAS alone does not keep concurrently prepared objects alive.',
            'not_established': 'Actual pin persistence, segment reclamation, reader epochs and crash recovery remain untested.'}


def scoped_generations() -> dict[str, Any]:
    """Physical serialization is compatible with independent logical preconditions."""
    expected_global = 0
    global_generation = 0
    global_accepts = []
    for _ in ('a', 'b'):
        ok = global_generation == expected_global
        global_accepts.append(ok)
        if ok:
            global_generation += 1
    per_workspace = {'a': 0, 'b': 0}
    scoped_accepts = []
    for name in ('a', 'b'):
        ok = per_workspace[name] == 0
        scoped_accepts.append(ok)
        if ok:
            per_workspace[name] += 1
    assert global_accepts == [True, False] and scoped_accepts == [True, True]
    return {'reproduced': True, 'one_global_precondition': global_accepts,
            'scoped_workspace_preconditions': scoped_accepts,
            'meaning': 'Serialize the short physical commit when necessary; do not manufacture a logical conflict for an unrelated workspace.'}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    if not shutil.which('git'):
        parser.error('Git is required')
    with tempfile.TemporaryDirectory(prefix='vcs-contract-') as temporary:
        root = Path(temporary)
        result = {
            'scope': 'Deterministic contract counterexamples; not comparative performance or production qualification',
            'python': platform.python_version(),
            'git': command(root, 'git', '--version').stdout.strip(),
            'platform': platform.system(),
            'checks': {
                'identical_deletions': identical_deletions(root / 'deletions'),
                'semantic_interference': semantic_interference(root / 'semantics'),
                'restore_check_replace_race': restore_race(root / 'restore'),
                'prepared_object_gc_race': gc_interleavings(),
                'resource_scoped_cas': scoped_generations(),
            },
        }
    text = json.dumps(result, indent=2) + '\n'
    if args.output:
        args.output.write_text(text)
    print(text, end='')

if __name__ == '__main__':
    main()
