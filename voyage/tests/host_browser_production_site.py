"""Bounded provider-free site classes for an explicitly approved HTTPS prefix.

Bind only loopback behind the existing, separately authorized public TLS proxy.
No account/provider/control endpoints, viewer assets, credentials or file paths.
Public test data is synthetic. Hosting/proxy configuration is not done here.
"""
import argparse
import hashlib
import html
from http.cookies import SimpleCookie
import http.server
import json
from pathlib import Path
import re
import threading
import urllib.parse


class Site(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def setup(self):
        super().setup()
        self.connection.settimeout(3)

    def log_message(self, *_):
        pass

    def reply(self, route, data, kind='text/html; charset=utf-8', headers=()):
        assert len(data) <= 1024*1024
        with self.server.counter_lock:
            if self.server.bytes+len(data)>64*1024*1024 or sum(self.server.counts.values())>=4096:
                self.send_error(503);self.close_connection=True;return
            self.server.counts[route]=self.server.counts.get(route,0)+1
            self.server.bytes+=len(data)
        self.send_response(200)
        self.send_header('Content-Type',kind)
        self.send_header('Content-Length',str(len(data)))
        self.send_header('Cache-Control','no-store')
        self.send_header('X-Content-Type-Options','nosniff')
        for key,value in headers:
            self.send_header(key,value)
        self.end_headers();self.wfile.write(data)

    def do_POST(self):
        self.send_error(405);self.close_connection=True

    def do_GET(self):
        self.requests=getattr(self,'requests',0)+1
        if self.requests>64:
            self.send_error(429);self.close_connection=True;return
        prefix=self.server.prefix
        raw=urllib.parse.urlsplit(self.path)
        if raw.query or not raw.path.startswith(prefix+'/'):
            self.send_error(404);return
        route=raw.path[len(prefix):]
        base=html.escape(prefix,quote=True)
        child=html.escape(self.server.child_origin+prefix,quote=True)
        if route=='/ui':
            data=('''<!doctype html><title>Qualification counter</title><style>body{font:16px sans-serif;margin:16px;min-height:1800px}</style>
<h1>Qualification fixture</h1><button onclick="count.textContent=String(++window.n)">Increment task counter</button>
<output id="count">0</output><script>window.n=0</script><p>Only this task browser owns the counter.</p>''').encode()
        elif route=='/private':
            data=b'<!doctype html><title>Synthetic private fixture</title><h1>Private fixture</h1><label>Synthetic private input <input autocomplete="off"></label>'
        elif route=='/static':
            data=b'<!doctype html><title>Static fixture</title><h1>Static qualification fixture</h1><p id="static-ready">Readable static content</p>'
        elif route=='/site-classes':
            data=('''<!doctype html><title>Site classes</title><link rel="stylesheet" href="'''+base+'''/site.css">
<h1>Qualification site classes</h1><button onclick="result.textContent='Changed in task browser'">Change page</button>
<p id="result">Waiting for task action</p><div id="shadow"></div><script>shadow.attachShadow({mode:'open'}).innerHTML='<style>span{color:rgb(12,34,56)}</style><span>Open shadow content</span>';</script>
<img id="authenticated-asset" src="'''+base+'''/authenticated.svg"><iframe title="Cross-origin fixture" src="'''+child+'''/frame"></iframe>''').encode()
            self.reply(route,data,headers=(('Set-Cookie','qualification_asset=allowed; Secure; SameSite=Lax; Path='+prefix+'/'),));return
        elif route=='/frame':
            nested=html.escape(self.server.origin+prefix+'/nested',quote=True)
            data=('''<!doctype html><h1>Cross-origin child content</h1><button onclick="this.textContent='Child action observed'">Child action</button><iframe title="Nested fixture" src="'''+nested+'''"></iframe>''').encode()
        elif route=='/nested':
            data=b'<!doctype html><h1>Nested child content</h1><button onclick="this.textContent=\'Nested action observed\'">Nested action</button>'
        elif route=='/site.css':
            self.reply(route,b'body{background:rgb(17,51,85)} #result{color:rgb(90,80,70)} iframe{width:500px;height:240px}', 'text/css');return
        elif route=='/authenticated.svg':
            cookies=SimpleCookie(self.headers.get('Cookie',''))
            if 'qualification_asset' not in cookies or cookies['qualification_asset'].value!='allowed':
                self.send_error(403);return
            self.reply(route,b'<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="green"/></svg>','image/svg+xml');return
        elif route=='/media':
            data=('''<!doctype html><title>Media fixture</title><h1>Media qualification fixture</h1>
<style>body{margin:8px;font:16px sans-serif}.surfaces{display:flex;gap:8px}.frame-row{display:flex;flex-wrap:wrap;gap:4px}iframe{border:0}</style>
<button id="play-media">Play synthetic video</button><output id="video-state" data-state="paused" data-frames="0" data-time-ms="0" data-starts="0">Synthetic video paused</output>
<output id="canvas-state" data-frame="0" data-color="orange">Canvas orange frame 0</output>
<div class="surfaces"><canvas id="canvas" width="240" height="80"></canvas><video id="video" width="240" height="80" muted loop playsinline preload="auto" src="'''+base+'''/synthetic.webm"></video></div>
<button id="change-unsupported">Change unsupported frame</button><output id="unsupported-state" data-changes="0">Unsupported frame orange</output><div class="frame-row">
'''+''.join('<iframe id="'+('unsupported-frame' if i==8 else 'bounded-frame-'+str(i))+'" width="100" height="55" title="Bounded frame '+str(i)+'" src="'+base+'/opaque"></iframe>' for i in range(9))+'''</div>
<script>
const canvas=document.getElementById('canvas'),video=document.getElementById('video'),progress=document.getElementById('video-state'),canvasState=document.getElementById('canvas-state');
let paint=0;const draw=()=>{const color=paint%2?'blue':'orange';canvas.getContext('2d').fillStyle=color;canvas.getContext('2d').fillRect(0,0,240,80);canvasState.dataset.frame=String(paint++);canvasState.dataset.color=color;canvasState.textContent='Canvas '+color+' frame '+(paint-1);};
draw();const timer=setInterval(()=>{draw();if(paint>=120)clearInterval(timer);},700);
let attempted=false,frames=0,last=0;
document.getElementById('play-media').onclick=async event=>{
 if(attempted)return;attempted=true;event.currentTarget.disabled=true;progress.dataset.starts='1';
 if(typeof video.requestVideoFrameCallback!=='function'){progress.dataset.state='unsupported';progress.textContent='Decoded video callbacks unavailable';return;}
 video.playbackRate=.75;
 const observed=(_now,metadata)=>{frames++;if(_now-last>=250||frames===1){last=_now;progress.dataset.frames=String(frames);progress.dataset.timeMs=String(Math.round(metadata.mediaTime*1000));progress.textContent='Decoded video frames '+frames;}
  if(frames<600&&!video.paused)video.requestVideoFrameCallback(observed);else{video.pause();progress.dataset.state='ended';}};
 try{await video.play();progress.dataset.state='playing';video.requestVideoFrameCallback(observed);}catch{progress.dataset.state='refused';progress.textContent='Synthetic playback refused';}
};
video.addEventListener('error',()=>{progress.dataset.state='error';progress.textContent='Synthetic media decoding unavailable';});
document.getElementById('change-unsupported').onclick=event=>{event.currentTarget.disabled=true;document.getElementById('unsupported-frame').src="'''+base+'''/opaque-alt";const marker=document.getElementById('unsupported-state');marker.dataset.changes='1';marker.textContent='Unsupported frame blue';};
</script>''').encode()
        elif route in ('/opaque','/opaque-alt'):
            color='orange' if route=='/opaque' else 'blue'
            data=('<!doctype html><style>html,body{margin:0;width:100%;height:100%;background:'+color+'}</style><title>Bounded fallback '+color+'</title><p style="position:absolute;left:2px;top:2px;margin:0;font:10px sans-serif">'+color+'</p>').encode()
        elif route=='/synthetic.webm':
            self.reply(route,self.server.video,'video/webm');return
        else:
            self.send_error(404);return
        self.reply(route,data)


class Server(http.server.ThreadingHTTPServer):
    daemon_threads=False
    block_on_close=True
    request_queue_size=16

    def __init__(self,*args):
        self.threads=threading.BoundedSemaphore(16)
        super().__init__(*args)

    def process_request(self,request,address):
        if not self.threads.acquire(blocking=False):
            self.shutdown_request(request);return
        try: super().process_request(request,address)
        except BaseException:
            self.threads.release();raise

    def process_request_thread(self,request,address):
        try: super().process_request_thread(request,address)
        finally: self.threads.release()


def origin(value):
    parsed=urllib.parse.urlsplit(value)
    assert parsed.scheme=='https' and parsed.hostname and not parsed.username and not parsed.password
    assert parsed.path in ('','/') and not parsed.query and not parsed.fragment
    return value.rstrip('/')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--origin',required=True)
    parser.add_argument('--child-origin',required=True)
    parser.add_argument('--prefix',required=True)
    parser.add_argument('--port',type=int,required=True)
    parser.add_argument('--seconds',type=int,default=600)
    parser.add_argument('--synthetic-video',type=Path,required=True)
    parser.add_argument('--video-sha256',required=True)
    args=parser.parse_args()
    assert __debug__ and re.fullmatch(r'/release-qualification-333/[A-Za-z0-9_-]{8,48}',args.prefix)
    assert 1024<=args.port<=65535 and 30<=args.seconds<=900
    top,child=origin(args.origin),origin(args.child_origin);assert top!=child
    # Operator-supplied synthetic public media only, no network fetch/dependency
    # generation. The file is not reopened after this bounded read.
    with args.synthetic_video.open('rb') as source:
        video=source.read(1024*1024+1)
    assert 0<len(video)<=1024*1024 and video.startswith(b'\x1a\x45\xdf\xa3')
    assert re.fullmatch('[a-f0-9]{64}',args.video_sha256) and hashlib.sha256(video).hexdigest()==args.video_sha256
    server=Server(('127.0.0.1',args.port),Site)
    server.origin,server.child_origin,server.prefix=top,child,args.prefix
    server.video,server.counts,server.bytes=video,{},0
    server.counter_lock=threading.Lock()
    timer=threading.Timer(args.seconds,server.shutdown);timer.start()
    try:
        server.serve_forever(poll_interval=.1)
    finally:
        timer.cancel();timer.join();server.server_close()
    print(json.dumps({'schema':1,'status':'fixture_stopped','route_counts':server.counts,
                      'response_bytes':server.bytes,'provider_requests':0}))


if __name__=='__main__':
    main()
