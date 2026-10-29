"""Subprocess boundary for codec fixtures; Rust is an explicit opt-in build.

AVC_RUST_CODEC points at the independently compiled codec_crosscheck binary.
This module never builds, downloads dependencies, or executes project hooks.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess

HERE = Path(__file__).resolve().parent


def commands(mode):
    if mode != '--ids-stdin' and shutil.which('node'):
        yield 'Node', ['node', str(HERE / 'codec_crosscheck.mjs'), mode]
    if binary := os.environ.get('AVC_RUST_CODEC'):
        yield 'Rust', [binary, mode]


def invoke(command, value):
    result = subprocess.run(command, input=json.dumps(value), text=True,
                            capture_output=True, check=True, timeout=35)
    return json.loads(result.stdout)


def corpus(objects, roots):
    return {
        'objects': [{'id_hex': ref.encode().hex(), 'payload_hex': payload.hex()}
                    for ref, payload in objects.items()],
        'roots': [ref.encode().hex() for ref in roots],
    }
