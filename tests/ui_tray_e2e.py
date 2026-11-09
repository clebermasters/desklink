#!/usr/bin/env python3
"""Native tray smoke test for a running Hyprland session with a tray host.

Opens a disposable editor window, addresses it by PID, and never starts input
capture or changes the user's profile. Run from the repository root after a
release build. Requires hyprctl and busctl; no global mouse/key injection.
"""
import os,json,subprocess,time,tempfile,socket
from pathlib import Path
E=os.environ.copy()
E.setdefault('XDG_RUNTIME_DIR',f'/run/user/{os.getuid()}')
E.setdefault('DBUS_SESSION_BUS_ADDRESS','unix:path='+E['XDG_RUNTIME_DIR']+'/bus')
E['XDG_SESSION_TYPE']='wayland'
for line in subprocess.check_output(['systemctl','--user','show-environment'],env=E,text=True).splitlines():
 key,_,value=line.partition('=')
 if key in {'HYPRLAND_INSTANCE_SIGNATURE','DISPLAY','WAYLAND_DISPLAY'}:E[key]=value
def run(*args):return subprocess.check_output(args,env=E,text=True,stderr=subprocess.STDOUT).strip()
def wait(fn,label):
 end=time.monotonic()+12
 while time.monotonic()<end:
  try:
   result=fn()
   if result:return result
  except (OSError,ValueError,KeyError,subprocess.CalledProcessError):pass
  time.sleep(.1)
 raise AssertionError(label)
def items():return set(json.loads(run('busctl','--user','--json=short','get-property','org.kde.StatusNotifierWatcher','/StatusNotifierWatcher','org.kde.StatusNotifierWatcher','RegisteredStatusNotifierItems'))['data'])
def clients():return json.loads(run('hyprctl','clients','-j'))
binary=str(Path('target/release/desklink').resolve())
with tempfile.TemporaryDirectory(prefix='desklink-tray-test-') as tmp:
 root=Path(tmp);profile=root/'config.toml';state=root/'state.json'
 with socket.socket() as sock:
  sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
 run(binary,'--config',str(profile),'init','--name','Desklink tray test','--listen',f'127.0.0.1:{port}')
 profile.write_text(profile.read_text().replace('clipboard = true','clipboard = false'))
 before=items()
 log=open('/tmp/desklink-tray-test.log','w')
 proc=subprocess.Popen([binary,'--config',str(profile),'ui','--test-state',str(state)],env=E,stdout=log,stderr=log)
 try:
  wait(lambda:state.exists(),'window state')
  wait(lambda:json.loads(state.read_text())['tray_available'],'tray icon registered')
  service=wait(lambda:next(iter(items()-before),None),'new tray service')
  dest,path=service.split('/',1);path='/'+path
  client=wait(lambda:next((c for c in clients() if c['pid']==proc.pid),None),'test window')
  print('Tray registered:',service,'backend',json.loads(state.read_text())['window_backend'],flush=True)
  run('hyprctl','eval', 'hl.dispatch(hl.dsp.window.close({ window = '+json.dumps('address:'+client['address'])+' }))')
  wait(lambda:not any(c['pid']==proc.pid for c in clients()),'window hidden after close')
  assert proc.poll() is None
  print('Close hides window and keeps app/tray alive',flush=True)
  run('busctl','--user','call',dest,path,'org.kde.StatusNotifierItem','Activate','ii','0','0')
  wait(lambda:any(c['pid']==proc.pid for c in clients()),'tray activation reopens window')
  print('Tray activation reopens the same window',flush=True)
  layout=run('busctl','--user','--json=short','call',dest,'/MenuBar','com.canonical.dbusmenu','GetLayout','iias','0','3','0')
  Path('/tmp/desklink-tray-menu.json').write_text(layout)
  labels={item['data'][1].get('label',{}).get('data'):item['data'][0] for item in json.loads(layout)['data'][1][2]}
  assert {'Open Desklink','Start sharing','Quit Desklink'} <= labels.keys()
  client=next(c for c in clients() if c['pid']==proc.pid)
  run('hyprctl','eval', 'hl.dispatch(hl.dsp.window.close({ window = '+json.dumps('address:'+client['address'])+' }))')
  wait(lambda:not any(c['pid']==proc.pid for c in clients()),'hidden for quit test')
  run('busctl','--user','call',dest,'/MenuBar','com.canonical.dbusmenu','Event','isvu',str(labels['Quit Desklink']),'clicked','i','0','0')
  wait(lambda:proc.poll() is not None,'quit from hidden tray')
  assert proc.returncode==0,Path('/tmp/desklink-tray-test.log').read_text()
  wait(lambda:service not in items(),'tray icon removed on quit')
  print('Quit from hidden tray exits cleanly and removes the icon',flush=True)
 finally:
  if proc.poll() is None:proc.terminate();proc.wait(timeout=5)
