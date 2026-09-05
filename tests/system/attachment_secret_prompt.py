#!/usr/bin/env python3
"""Native hidden invitation input; real PTY/console, synthetic credentials only."""
import json, os, pathlib, subprocess, tempfile, time, uuid
ROOT = pathlib.Path(__file__).resolve().parents[2]
HELM = pathlib.Path(os.environ.get('HELM_BIN', ROOT/('target/release/helm'+('.exe' if os.name=='nt' else '')))).resolve()
MARKER = b'Invitation key (hidden): '
CANARY = b'PromptSecretMustNeverEcho'.ljust(43,b'_')
assert len(CANARY)==43

def unix_case(args, data, expected, signal_number=None):
    import pty, select, termios, signal
    master, slave = pty.openpty()
    before=termios.tcgetattr(slave)
    # Deliberately nondefault mode: restoration must preserve the whole value.
    before[3] &= ~termios.ECHOK
    termios.tcsetattr(slave,termios.TCSANOW,before)
    proc=subprocess.Popen([str(HELM),*args],stdin=slave,stdout=slave,stderr=slave,env=dict(os.environ,RUST_LOG='trace'))
    transcript=bytearray()
    try:
        until=time.monotonic()+5
        while MARKER not in transcript:
            assert proc.poll() is None, 'process exited without hidden prompt'
            assert time.monotonic()<until, 'hidden prompt did not arrive'
            if select.select([master],[],[],.1)[0]:transcript.extend(os.read(master,65536))
        assert not termios.tcgetattr(slave)[3]&termios.ECHO
        if data:os.write(master,data)
        if signal_number:proc.send_signal(signal_number)
        proc.wait(timeout=40 if not data or b'\r' in data or b'\n' in data else 8)
        while select.select([master],[],[],.05)[0]:transcript.extend(os.read(master,65536))
        assert proc.returncode==expected, f'unexpected exit {proc.returncode}; expected {expected}'
        assert CANARY not in transcript and b'PromptSecret' not in transcript, 'secret echoed'
        assert termios.tcgetattr(slave)==before, 'prior terminal mode not restored'
        # Check queued input directly before any shell could consume/echo it.
        after=termios.tcgetattr(slave);probe=list(after);probe[3]&=~termios.ICANON;probe[6]=list(probe[6]);probe[6][termios.VMIN]=0;probe[6][termios.VTIME]=0
        termios.tcsetattr(slave,termios.TCSANOW,probe)
        assert os.read(slave,65536)==b'', 'secret tail remained queued'
        termios.tcsetattr(slave,termios.TCSANOW,after)
        return bytes(transcript)
    finally:
        if proc.poll() is None:proc.kill();proc.wait()
        os.close(master);os.close(slave)

def unix_stalled_output():
    import pty, select, termios, signal
    master,slave=pty.openpty()
    before=termios.tcgetattr(slave);before[3]&=~termios.ECHOK
    termios.tcsetattr(slave,termios.TCSANOW,before)
    # The marker write blocks in the kernel. Cancellation must restore modes
    # without acquiring an output lock or waiting for queued output to drain.
    termios.tcflow(slave,termios.TCOOFF)
    proc=None
    try:
        with tempfile.TemporaryDirectory() as tmp:
            directory=pathlib.Path(tmp)/'identity'
            proc=subprocess.Popen([str(HELM),'attachment','--directory',str(directory),'--origin','http://127.0.0.1:1',
                '--allow-insecure-loopback','enroll','--invitation-id',str(uuid.uuid4())],stdin=slave,stdout=slave,stderr=slave)
            until=time.monotonic()+5
            while termios.tcgetattr(slave)[3]&termios.ECHO:
                assert proc.poll() is None and time.monotonic()<until, 'stalled prompt did not enter hidden mode'
                time.sleep(.01)
            os.write(master,CANARY)
            proc.send_signal(signal.SIGINT);proc.wait(timeout=3)
            assert proc.returncode==130 and not directory.exists()
            assert termios.tcgetattr(slave)==before, 'stalled-output cancellation did not restore prior mode'
            probe=termios.tcgetattr(slave);probe[3]&=~termios.ICANON;probe[6][termios.VMIN]=0;probe[6][termios.VTIME]=0
            termios.tcsetattr(slave,termios.TCSANOW,probe)
            assert os.read(slave,65536)==b'', 'stalled-output cancellation left queued secret'
            termios.tcsetattr(slave,termios.TCSANOW,before)
            termios.tcflow(slave,termios.TCOON)
            output=bytearray()
            while select.select([master],[],[],.05)[0]:output.extend(os.read(master,65536))
            assert CANARY not in output and b'PromptSecret' not in output, 'stalled output echoed secret'
    finally:
        if proc is not None and proc.poll() is None:proc.kill();proc.wait()
        termios.tcflow(slave,termios.TCOON);os.close(master);os.close(slave)

def windows_case(args, data, expected, signal_number=None):
    # Allocate a real native console even on a redirected CI runner. Child uses
    # explicit console handles; no terminal emulation or mocked console modes.
    import ctypes as c, msvcrt
    from ctypes import wintypes as w
    k=c.WinDLL('kernel32',use_last_error=True)
    class Coord(c.Structure): _fields_=[('X',c.c_short),('Y',c.c_short)]
    class Rect(c.Structure): _fields_=[('Left',c.c_short),('Top',c.c_short),('Right',c.c_short),('Bottom',c.c_short)]
    class Info(c.Structure): _fields_=[('size',Coord),('cursor',Coord),('attributes',w.WORD),('window',Rect),('maximum',Coord)]
    class Key(c.Structure): _fields_=[('down',w.BOOL),('repeat',w.WORD),('virtual',w.WORD),('scan',w.WORD),('char',w.WCHAR),('control',w.DWORD)]
    class Event(c.Union): _fields_=[('key',Key),('padding',c.c_byte*16)]
    class Record(c.Structure): _fields_=[('kind',w.WORD),('event',Event)]
    assert c.sizeof(Record)==20
    k.CreateFileW.argtypes=[w.LPCWSTR,w.DWORD,w.DWORD,c.c_void_p,w.DWORD,w.DWORD,w.HANDLE];k.CreateFileW.restype=w.HANDLE
    for name in ['GetConsoleMode','SetConsoleMode','FlushConsoleInputBuffer','GetNumberOfConsoleInputEvents','GetConsoleScreenBufferInfo','WriteConsoleInputW','ReadConsoleOutputCharacterW','SetConsoleCursorPosition','FillConsoleOutputCharacterW']:
        getattr(k,name).restype=w.BOOL
    k.GetConsoleMode.argtypes=[w.HANDLE,c.POINTER(w.DWORD)]
    k.SetConsoleMode.argtypes=[w.HANDLE,w.DWORD]
    k.FlushConsoleInputBuffer.argtypes=[w.HANDLE]
    k.GetNumberOfConsoleInputEvents.argtypes=[w.HANDLE,c.POINTER(w.DWORD)]
    k.GetConsoleScreenBufferInfo.argtypes=[w.HANDLE,c.POINTER(Info)]
    k.WriteConsoleInputW.argtypes=[w.HANDLE,c.POINTER(Record),w.DWORD,c.POINTER(w.DWORD)]
    k.ReadConsoleOutputCharacterW.argtypes=[w.HANDLE,w.LPWSTR,w.DWORD,Coord,c.POINTER(w.DWORD)]
    k.SetConsoleCursorPosition.argtypes=[w.HANDLE,Coord]
    k.FillConsoleOutputCharacterW.argtypes=[w.HANDLE,w.WCHAR,w.DWORD,Coord,c.POINTER(w.DWORD)]
    # Free only this fixture process's inherited console; never signal a parent.
    k.FreeConsole();assert k.AllocConsole(), 'native console allocation failed'
    files=[];proc=None
    try:
        for name in ['CONIN$','CONOUT$']:
            handle=k.CreateFileW(name,0xC0000000,3,None,3,0,None)
            assert handle != c.c_void_p(-1).value, 'console handle unavailable'
            files.append(os.fdopen(msvcrt.open_osfhandle(handle,os.O_RDWR),'r+b',buffering=0))
        incoming=msvcrt.get_osfhandle(files[0].fileno());outgoing=msvcrt.get_osfhandle(files[1].fileno())
        saved=w.DWORD();assert k.GetConsoleMode(incoming,c.byref(saved))
        saved.value ^= 0x20  # nondefault INSERT_MODE must be preserved exactly
        assert k.SetConsoleMode(incoming,saved.value)
        assert k.FlushConsoleInputBuffer(incoming)
        info=Info();assert k.GetConsoleScreenBufferInfo(outgoing,c.byref(info))
        size=info.size.X*info.size.Y;count=w.DWORD()
        assert k.FillConsoleOutputCharacterW(outgoing,' ',size,Coord(0,0),c.byref(count))
        assert k.SetConsoleCursorPosition(outgoing,Coord(0,0))
        def transcript():
            buffer=c.create_unicode_buffer(size)
            assert k.ReadConsoleOutputCharacterW(outgoing,buffer,size,Coord(0,0),c.byref(count))
            return buffer[:count.value].encode('utf-8')
        proc=subprocess.Popen([str(HELM),*args],stdin=files[0],stdout=files[1],stderr=files[1],env=dict(os.environ,RUST_LOG='trace'),creationflags=subprocess.CREATE_NEW_PROCESS_GROUP)
        until=time.monotonic()+10
        while MARKER not in transcript():
            assert proc.poll() is None, 'process exited without hidden prompt'
            assert time.monotonic()<until, 'hidden prompt did not arrive'
            time.sleep(.02)
        mode=w.DWORD();assert k.GetConsoleMode(incoming,c.byref(mode));assert mode.value&4==0, 'console echo enabled'
        records=(Record*len(data))()
        for record,byte in zip(records,data):
            record.kind=1;record.event.key=Key(1,1,0,0,chr(byte),0)
        assert k.WriteConsoleInputW(incoming,records,len(records),c.byref(count)) and count.value==len(records)
        if signal_number=='break':
            assert k.GenerateConsoleCtrlEvent(1,proc.pid), 'console cancellation event failed'
        proc.wait(timeout=40 if not data or b'\r' in data or b'\n' in data else 8)
        output=transcript()
        assert proc.returncode==expected, f'unexpected exit {proc.returncode}; expected {expected}'
        assert CANARY not in output and b'PromptSecret' not in output, 'secret echoed'
        assert k.GetConsoleMode(incoming,c.byref(mode)) and mode.value==saved.value, 'prior console mode not restored'
        assert k.GetNumberOfConsoleInputEvents(incoming,c.byref(count)) and count.value==0, 'secret tail remained queued'
        return output
    finally:
        if proc is not None and proc.poll() is None:proc.kill();proc.wait()
        for file in files:file.close()
        k.FreeConsole()

def enrollment_flow(native_case):
    # Reuse only the existing fixture's local proxy and owner API helpers; the
    # human input path below always crosses a real native terminal and Helm CLI.
    import attachment_enrollment as fixture
    import http.server, socket, threading, urllib.request, urllib.error
    with tempfile.TemporaryDirectory(prefix='helm-secret-enrollment-') as tmp:
        root=pathlib.Path(tmp)
        proxy=http.server.ThreadingHTTPServer(('127.0.0.1',0),fixture.Proxy)
        origin='http://127.0.0.1:'+str(proxy.server_port)
        reserve=socket.socket();reserve.bind(('127.0.0.1',0));port=reserve.getsockname()[1];reserve.close()
        fixture.Proxy.backend='http://127.0.0.1:'+str(port)
        server=subprocess.Popen([str(fixture.VESSEL),'--bind','127.0.0.1:'+str(port),'--public-origin',origin,
            '--allow-insecure-loopback','--attachment-directory',str(root/'authority'),'--database',str(root/'vessel.db')],
            env=dict(os.environ,VESSEL_OPERATOR_TOKEN=fixture.TOKEN,RUST_LOG='error'),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        thread=threading.Thread(target=proxy.serve_forever,daemon=True);thread.start()
        try:
            until=time.monotonic()+10
            while True:
                assert server.poll() is None, 'Vessel exited before readiness'
                try:urllib.request.urlopen(fixture.Proxy.backend+'/ready',timeout=.2).close();break
                except (OSError,urllib.error.URLError):
                    assert time.monotonic()<until,'Vessel readiness timed out'
                    time.sleep(.03)
            def invitation():return fixture.post(origin,'/v2/enrollment/invitations',{'ttl_ms':60000})
            def args(directory,*command):return ['attachment','--directory',str(directory),'--origin',origin,'--allow-insecure-loopback',*command]
            def enter(arguments,key,expected):
                output=native_case(arguments,key.encode()+b'\r',expected)
                normalized=output.translate(None,b' \r\n\x00')
                assert key.encode() not in normalized and fixture.TOKEN.encode() not in normalized, 'credential escaped output'
                return output
            active=root/'active';inv=invitation()
            output=enter(args(active,'enroll','--invitation-id',inv['id']),inv['key'],0)
            assert b'"active"' in output and json.loads((active/'client.json').read_bytes())['status']=='active'
            pending=root/'pending';inv=invitation();fixture.Proxy.drop_next=True
            enter(args(pending,'enroll','--invitation-id',inv['id']),inv['key'],1)
            original=(pending/'client.json').read_bytes();state=json.loads(original)
            assert state['status']=='enrolling'
            native_case(args(pending,'resume'),b'\x1b'+CANARY,130)
            assert (pending/'client.json').read_bytes()==original, 'cancelled prompt changed pending transaction'
            output=enter(args(pending,'resume'),inv['key'],0)
            assert b'"active"' in output
            assert json.loads((pending/'client.json').read_bytes())['machine_id']==state['machine_id']
            denied=root/'denied';inv=invitation()
            enter(args(denied,'enroll','--invitation-id',inv['id']),CANARY.decode(),1)
            original=(denied/'client.json').read_bytes()
            enter(args(denied,'resume'),CANARY.decode(),1)
            assert (denied/'client.json').read_bytes()==original, 'denial changed pending transaction'
        finally:
            server.terminate();server.wait(timeout=5);proxy.shutdown();proxy.server_close();thread.join(timeout=5)

def main():
    import signal
    native_case=windows_case if os.name=='nt' else unix_case
    cases=[(b'\x1b'+CANARY,130,None), (b'\x03'+CANARY,130,None),
           (b'\x04'+CANARY,130,None), (b'!'+CANARY,1,None),
           (CANARY+b'A'+CANARY,1,None), (b'short\r'+CANARY,1,None),
           (CANARY,130,signal.SIGINT), (CANARY,130,signal.SIGTERM),
           (CANARY,130,getattr(signal,'SIGHUP',signal.SIGTERM)), (CANARY+b'\r\n',1,None),
           (CANARY+b'\n',1,None), (b'x\x7f'+CANARY+b'\r',1,None)]
    cases.extend([(b'\xc3\xa9'+CANARY,1,None),(b'A'*512,1,None),(b'',130,None)])
    if os.name=='nt':cases.append((CANARY,130,'break'))
    for index,(data,expected,sig) in enumerate(cases):
        if os.name=='nt' and sig is not None and sig!='break':continue
        print(f'prompt case {index}', flush=True)
        with tempfile.TemporaryDirectory() as tmp:
            directory=pathlib.Path(tmp)/'identity'
            args=['attachment','--directory',str(directory),'--origin','http://127.0.0.1:1','--allow-insecure-loopback','enroll','--invitation-id',str(uuid.uuid4())]
            native_case(args,data,expected,sig)
            if index<9 or index>=12:assert not directory.exists(), 'invalid/cancelled input created identity'
    with tempfile.TemporaryDirectory() as tmp:
        args=['attachment','--directory',str(pathlib.Path(tmp)/'identity'),'--origin','http://127.0.0.1:1','--allow-insecure-loopback','enroll','--invitation-id',str(uuid.uuid4())]
        proc=subprocess.Popen([str(HELM),*args],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        try:
            proc.wait(timeout=3)  # stdin stays open: no unattended read is allowed
            out,err=proc.communicate(timeout=3)
            assert proc.returncode==1 and CANARY not in out+err
        finally:
            if proc.poll() is None:proc.kill();proc.wait()
        assert not (pathlib.Path(tmp)/'identity').exists()
    if os.name!='nt':unix_stalled_output()
    enrollment_flow(native_case)
    print('hidden invitation prompt: native restoration, bounds, cancellation and no echo passed')
if __name__=='__main__':main()
