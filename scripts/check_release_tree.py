"""Validate a staged release tree. Does not read or package any sample library."""
from pathlib import Path
import re,sys
root=Path(sys.argv[1] if len(sys.argv)>1 else '.').resolve()
blocked={'.wav','.mp3','.flac','.aif','.aiff','.ogg','.opus','.m4a','.caf','.pyc'}
for p in root.rglob('*'):
 if not p.is_file() or '.git' in p.relative_to(root).parts:continue
 relative=p.relative_to(root)
 assert not any(x in relative.parts for x in ['artifacts','target','.cargo-home','.codex','.serena','.agents','.aws','__pycache__']),relative
 assert p.suffix.lower() not in blocked,relative
 assert not p.name.endswith(('.leviathan.json','.events.json')),relative
 assert p.name!='build_downloads_demo.py',relative
 data=p.read_bytes()
 assert not re.search(rb'/(?:home|Users)/[^/\s\x00]+/',data),relative
 assert not re.search(rb'gh[pousr]_[A-Za-z0-9]{30,}|-----BEGIN (?:RSA |OPENSSH |EC )?PRIVATE KEY-----',data),relative
 assert data[:4] not in (b'RIFF',b'fLaC',b'OggS'),relative
print('Release tree verified: no audio/sample projects, personal paths, private config or credential patterns')
