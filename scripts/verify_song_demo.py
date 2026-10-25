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
if len(sys.argv)>2:
 assert len(sys.argv)==3 and sys.argv[2] in ('--effects','--phrases','--arrangement','--groove','--generate')
 commands=[event['command'] for event in manifest['events']]
 assert all(a['frame']<=b['frame'] for a,b in zip(manifest['events'],manifest['events'][1:]))
 if sys.argv[2]=='--effects':
  assert manifest['schema']==2 and manifest['evaluator']=='leviathan-song-v2'
  assert any(command=='Cut' for command in commands)
  assert any(isinstance(command,dict) and 'Speed' in command for command in commands)
  assert any(isinstance(command,dict) and command.get('Off',{}).get('reason')=='Retrigger' for command in commands)
  assert any(isinstance(command,dict) and command.get('On',{}).get('reverse') for command in commands)
  assert any(isinstance(command,dict) and command.get('On',{}).get('offset',0)>0 for command in commands)
  report.update(verified_effects=['retrigger','cut','offset','reverse','glide'],events=len(commands))
 elif sys.argv[2]=='--phrases':
  assert manifest['schema']==3 and manifest['evaluator']=='leviathan-song-v3'
  phrase_events=[event for event in manifest['events'] if event.get('phrase')]
  assert {event['phrase']['instrument'] for event in phrase_events}=={1,3,4}
  assert all(event['column']>=256 and event['phrase']['phrase']==1 for event in phrase_events)
  project=json.loads((root/'first-current.leviathan.json').read_text())['song']
  for owner in ('1','3','4'):
   phrase=project['instruments'][owner]['phrases']['1']
   assert phrase['rows']>0 and phrase['lines_per_beat']==8
   assert all(not cell.get('phrase') and cell.get('instrument') is None for columns in phrase['cells'].values() for cell in columns.values())
  report.update(verified_phrase_instruments=[1,3,4],phrase_events=len(phrase_events),events=len(commands))
 if sys.argv[2] in ('--arrangement','--groove','--generate'):
  assert manifest['schema']==4 and manifest['evaluator']=='leviathan-song-v4'
  info=manifest['arrangement']; assert info['isolated_voices'] and info['aliases']
  assert len(info['order'])==3 and all(o.get('section') for o in info['order'])
  assert info['order'][1]['muted_tracks']==[4]
  assert not any(e['track']==4 and e['position'] and e['position'][0]==1 and isinstance(e['command'],dict) and 'On' in e['command'] for e in manifest['events'])
  assert any(e.get('phrase') for e in manifest['events'])
  report.update(verified_arrangement=['aliases','sections','variation','occurrence_mute','phrases'],events=len(commands))
 if sys.argv[2] in ('--groove','--generate'):
  project=json.loads((root/'first-current.leviathan.json').read_text())['song']
  hats=project['patterns']['1']['tracks']['2']
  assert all(columns['0'].get('delay')==0.25 for row,columns in hats.items() if int(row)%4==2)
  assert hats['0']['0']['velocity']==0.4
  last=str(project['order'][2]['pattern']); assert project['patterns'][last]['tracks']['3']['0']['0']['note']=={'Note':45}
  delayed=next(e for e in manifest['events'] if e['track']==2 and e['position']==[0,2] and isinstance(e['command'],dict) and 'On' in e['command'])
  assert delayed['frame']==13500
  report.update(verified_transforms=['committed_groove','scoped_transpose','explicit_velocity_scale'])
 if sys.argv[2]=='--generate':
  recipes=project['generation_recipes']; recipe=recipes['Seeded return chords']
  assert recipe['seed']==42 and recipe['probability']==0.75 and recipe['hits']==5 and recipe['cycle_steps']==8
  assert recipe['revision']=='8505bfe09111d4123a202af5222466f55021db9f' and recipe['schema']==1
  lead=project['patterns'][last]['tracks']['4']
  assert all(not c.get('phrase') for cols in lead.values() for c in cols.values())
  assert any('1' in cols for cols in lead.values()) and any(c['note']=='Off' for cols in lead.values() for c in cols.values())
  assert any(e['position'] and e['position'][0]==2 and e['track']==4 and not e.get('phrase') and isinstance(e['command'],dict) and 'On' in e['command'] for e in manifest['events'])
  report.update(verified_generation=['seeded_pattrns','editable_chords','explicit_off','saved_recipe'],seed=42)
print(json.dumps(report))
