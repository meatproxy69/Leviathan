from pathlib import Path
import struct, math, hashlib, json, sys
root=Path(sys.argv[1])
a=(root/'first-current.wav').read_bytes(); b=(root/'first-current-repeat.wav').read_bytes(); assert a==b
j=(root/'first-current.events.json').read_bytes(); assert j==(root/'first-current-repeat.events.json').read_bytes()
assert a[:4]==b'RIFF' and a[8:12]==b'WAVE'
pos=12; chunks={}
while pos+8<=len(a):
 kind,size=struct.unpack_from('<4sI',a,pos); chunks[kind]=a[pos+8:pos+8+size]; pos+=8+size+(size%2)
fmt,channels,rate,_,align,bits=struct.unpack_from('<HHIIHH',chunks[b'fmt ']); assert (channels,rate,bits)==(2,48000,32)
if fmt==65534:
 assert len(chunks[b'fmt '])>=40 and struct.unpack_from('<H',chunks[b'fmt '],24)[0]==3
else: assert fmt==3
values=struct.unpack('<'+'f'*(len(chunks[b'data'])//4),chunks[b'data']); frames=len(values)//2
assert frames==504000 and all(math.isfinite(v) and abs(v)<=1 for v in values)
energy=sum(v*v for v in values); assert energy>0
stereo=sum(abs(values[i]-values[i+1]) for i in range(0,len(values),2)); assert stereo>0
manifest=json.loads(j); assert manifest['musical_frames']==480000 and manifest['tail_frames']==24000
report={'frames':frames,'seconds':frames/rate,'channels':channels,'sample_rate':rate,'peak':max(map(abs,values)),'energy':energy,'stereo_difference_sum':stereo,'wav_sha256':hashlib.sha256(a).hexdigest(),'manifest_sha256':hashlib.sha256(j).hexdigest(),'byte_identical_repeat':True}
print(json.dumps(report))
