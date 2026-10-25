#!/usr/bin/env python3
"""Independent float WAV inspection and byte comparison of two demo directories."""
import hashlib
import json
import math
from pathlib import Path
import struct
import sys

first, second = map(Path, sys.argv[1:3])
files = sorted(first.iterdir())
assert len(files) == 16
for path in files:
    assert path.read_bytes() == (second / path.name).read_bytes(), path.name
path = first / "aqua-five-parts.wav"
data = path.read_bytes()
assert data[:4] == b"RIFF" and data[8:12] == b"WAVE"
offset = 12
pcm = None
while offset + 8 <= len(data):
    tag, size = struct.unpack_from("<4sI", data, offset)
    chunk = data[offset + 8 : offset + 8 + size]
    if tag == b"fmt ":
        fmt = struct.unpack_from("<HHIIHH", chunk)
        # hound emits WAVE_FORMAT_EXTENSIBLE with the IEEE_FLOAT subformat GUID.
        assert fmt[0] == 3 or (
            fmt[0] == 65534
            and chunk[24:40] == bytes.fromhex("0300000000001000800000aa00389b71")
        )
        assert fmt[1] == 2 and fmt[2] == 48000 and fmt[5] == 32
    if tag == b"data":
        pcm = chunk
    offset += 8 + size + size % 2
assert pcm is not None
samples = struct.unpack("<" + "f" * (len(pcm) // 4), pcm)
assert len(samples) // 2 == 408000
assert all(math.isfinite(v) and abs(v) < 1 for v in samples)
assert any(abs(samples[i] - samples[i + 1]) > 0.01 for i in range(0, len(samples), 2))
result = {
    "frames": len(samples) // 2,
    "channels": 2,
    "sample_rate": 48000,
    "peak": max(map(abs, samples)),
    "energy": sum(x * x for x in samples),
    "byte_identical_files": len(files),
    "sha256": hashlib.sha256(data).hexdigest(),
}
assert result["energy"] > 0
print(json.dumps(result, indent=2))
