#!/usr/bin/env python3
"""Exercise real three-computer routing, trusted layout sync and live edits on private X11 displays."""
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import time
from ui_e2e import wait, free_port
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib


def rpc(profile, request="Status"):
    with socket.socket(socket.AF_UNIX) as sock:
        sock.settimeout(2)
        sock.connect(str(profile.with_name("control.sock")))
        data=json.dumps(request).encode()
        sock.sendall(len(data).to_bytes(4,"big")+data)
        header=sock.recv(4)
        if len(header)!=4: raise OSError("agent closed control connection")
        length=int.from_bytes(header,"big"); body=b""
        while len(body)<length:
            part=sock.recv(length-len(body))
            if not part: raise OSError("short control response")
            body+=part
        return json.loads(body)


def write_config(path, name, port, peers, layout=None, owner=None):
    text=f'version = 1\nname = "{name}"\nlisten = "127.0.0.1:{port}"\nclipboard = true\n'
    if owner: text+=f'layout_owner = "{owner}"\n'
    for key, address in peers:
        text+=f'\n[[peers]]\nname = "Computer {key[:6]}"\npublic_key = "{key}"\naddress = "{address}"\n'
    if layout:
        text+=f'\n[layout]\nschema = 1\nrevision = {layout["revision"]}\nowner = "{layout["owner"]}"\n'
        for screen in layout["screens"]:
            r=screen["rect"]
            text+=f'\n[[layout.screens]]\nid = "{screen["id"]}"\nname = "{screen["name"]}"\n[layout.screens.rect]\nx = {r["x"]}\ny = {r["y"]}\nwidth = {r["width"]}\nheight = {r["height"]}\n'
    path.write_text(text)


def run(binary):
    processes=[]; files=[]
    with tempfile.TemporaryDirectory(prefix="desklink-layout-") as directory:
        root=Path(directory); profiles=[root/f"{i}/config.toml" for i in range(3)]; ports=[free_port() for _ in profiles]
        def spawn(args, env=None, log=None, **kwargs):
            if log:
                f=open(log,"ab",buffering=0);files.append(f);kwargs.update(stdout=f,stderr=f)
            proc=subprocess.Popen(args,env=env,**kwargs);processes.append(proc);return proc
        def display(width,height):
            proc=spawn(["Xvfb","-displayfd","1","-screen","0",f"{width}x{height}x24","-nolisten","tcp","-ac"],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)
            number=proc.stdout.readline().decode().strip();assert number.isdigit()
            env=dict(os.environ,DISPLAY=f":{number}",XDG_SESSION_TYPE="x11",NO_COLOR="1");env.pop("WAYLAND_DISPLAY",None);return env
        def cli(profile,*args): return subprocess.check_output([str(binary),"--config",str(profile),*args],stderr=subprocess.PIPE).decode().strip()
        def mouse(env,*args):return subprocess.check_output(["xdotool",*map(str,args)],env=env,stderr=subprocess.PIPE).decode()
        def log(i):return (root/f"events-{i}.log").read_text(errors="replace")
        def active():return rpc(profiles[0])["active_computer"]
        def move_until(target,delta):
            for _ in range(12):
                if active()==target:return
                mouse(envs[0],"mousemove_relative","--",str(delta),"0");time.sleep(0.08)
            raise AssertionError("pointer did not reach target computer")
        try:
            envs=[display(800,600),display(1000,700),display(900,800)]
            keys=[]
            for i,profile in enumerate(profiles):
                cli(profile,"init","--name",f"Screen {i+1}","--listen",f"127.0.0.1:{ports[i]}");keys.append(cli(profile,"identity"))
            layout={"revision":1,"owner":keys[0],"screens":[{"id":key,"name":f"Screen {i+1}","rect":{"x":i*240,"y":0,"width":240,"height":150}} for i,key in enumerate(keys)]}
            for i,profile in enumerate(profiles):
                peers=[(keys[j],f"127.0.0.1:{ports[j]}") for j in (range(1,3) if i==0 else [0])]
                write_config(profile,f"Screen {i+1}",ports[i],peers,layout if i==0 else None,keys[0] if i else None)
            for i in (1,2):
                spawn(["stdbuf","-oL","xev","-name",f"Desklink layout {i}","-geometry","780x580+0+0"],env=envs[i],log=root/f"events-{i}.log")
                wid=wait(lambda:mouse(envs[i],"search","--name",f"Desklink layout {i}").splitlines()[0],"receiver window")
                mouse(envs[i],"windowfocus","--sync",wid)
            agents=[spawn([str(binary),"--config",str(profile),"run"],env=envs[i],log=root/f"agent-{i}.log") for i,profile in enumerate(profiles)]
            wait(lambda:len([p for p in rpc(profiles[0])["peers"] if p["connected"]])==2,"controller connections")
            for profile in profiles[1:]:
                wait(lambda p=profile:rpc(p)["layout_revision"]==1,"trusted shared layout received")
            print("PASS: controller layout synchronized to both receivers without copying trust lists",flush=True)
            mouse(envs[0],"mousemove","400","300");time.sleep(0.1);mouse(envs[0],"mousemove","799","300")
            wait(lambda:active()==keys[1],"enter middle computer")
            mouse(envs[0],"keydown","Control_L")
            wait(lambda:"keysym 0xffe3, Control_L" in log(1),"held modifier on middle computer")
            releases=log(1).count("KeyRelease event")
            move_until(keys[2],7000)
            wait(lambda:"keysym 0xffe3, Control_L" in log(2),"held modifier restored on third computer")
            wait(lambda:log(1).count("KeyRelease event")>releases,"old receiver's held input released")
            mouse(envs[0],"key","a");mouse(envs[0],"keyup","Control_L")
            wait(lambda:"keysym 0x61, a" in log(2),"keyboard input reaches third computer")
            move_until(keys[0],-7000)
            print("PASS: one keyboard traverses A→B→C→B→A with held input balanced at each switch",flush=True)

            layout["revision"]=2;layout["screens"][1]["rect"].update(x=0,y=-150);layout["screens"][2]["rect"].update(x=240,y=0)
            write_config(profiles[0],"Screen 1",ports[0],[(keys[j],f"127.0.0.1:{ports[j]}") for j in (1,2)],layout)
            response=rpc(profiles[0],"Apply");assert response["layout_revision"]==2 and not response["restart_required"],response
            assert agents[0].poll() is None
            for profile in profiles[1:]:wait(lambda p=profile:rpc(p)["layout_revision"]==2,"live layout update")
            time.sleep(0.3);mouse(envs[0],"mousemove","400","300");time.sleep(0.1);mouse(envs[0],"mousemove","400","0")
            wait(lambda:active()==keys[1],"new top screen route")
            mouse(envs[0],"mousemove_relative","--","0","7000")
            wait(lambda:active()==keys[0],"return from newly placed top screen")
            print("PASS: live layout edits change routes and synchronize without restarting the controller",flush=True)
            agents[2].terminate();agents[2].wait(timeout=4)
            wait(lambda:not next(p for p in rpc(profiles[0])["peers"] if p["public_key"]==keys[2])["connected"],"offline screen detected")
            time.sleep(0.3);mouse(envs[0],"mousemove","400","300");time.sleep(0.1);mouse(envs[0],"mousemove","799","300");time.sleep(0.1)
            assert active()==keys[0]
            mouse(envs[0],"mousemove","200","150");time.sleep(0.1)
            assert "x:200 y:150" in mouse(envs[0],"getmouselocation")
            print("PASS: an offline destination cannot strand the local pointer",flush=True)
            print("ALL SHARED LAYOUT CHECKS PASSED",flush=True)
        except Exception:
            for path in root.glob("agent-*.log"):print(path.name,path.read_text(errors="replace")[-6000:])
            raise
        finally:
            for profile in profiles:
                try:rpc(profile,"Stop")
                except (OSError,ValueError):pass
            for proc in reversed(processes):
                if proc.poll() is None:proc.terminate()
            for proc in reversed(processes):
                try:proc.wait(timeout=3)
                except subprocess.TimeoutExpired:proc.kill();proc.wait(timeout=3)
            for f in files:f.close()


if __name__=="__main__":
    import argparse
    p=argparse.ArgumentParser(description=__doc__);p.add_argument("--binary",type=Path,default=Path(__file__).resolve().parents[1]/"target/debug/desklink");run(p.parse_args().binary.resolve())
