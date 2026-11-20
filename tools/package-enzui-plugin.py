#!/usr/bin/env python3
import argparse
import json
from pathlib import Path


parser = argparse.ArgumentParser(description='Bundle a source manifest and Python worker into one bounded enzUI plugin package')
parser.add_argument('manifest', type=Path)
parser.add_argument('program', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
manifest = json.loads(args.manifest.read_text(encoding='utf-8'))
if not isinstance(manifest, dict) or 'program' in manifest or manifest.get('format') != 1:
    parser.error('Expected a format 1 manifest without an embedded program')
program = args.program.read_text(encoding='utf-8')
if len(program.encode('utf-8')) > 512 * 1024:
    parser.error('Plugin program exceeds 512 KiB')
manifest['program'] = program
payload = json.dumps(manifest, ensure_ascii=False, separators=(',', ':')).encode('utf-8')
if len(payload) > 1024 * 1024:
    parser.error('Plugin package exceeds 1 MiB')
with args.output.open('xb') as package:
    package.write(payload)
print(f'Wrote {args.output} ({len(payload)} bytes)')
