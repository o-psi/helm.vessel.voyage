#!/usr/bin/env python3
"""Bounded POSIX PTYs of real plain attachment and native process manager.
No model or network is constructed. The inert Rust driver is not test evidence.
"""
import argparse, errno, fcntl, os, pty, re, select, signal, struct, subprocess, tempfile, termios, time
DRIVER='plain_terminal::fixture::native_driver'
CSI=re.compile(rb'\x1b\[[0-?]*[ -/]*[@-~]')
class Terminal:
    def __init__(self,binary,env,mode='normal'):
        self.master,self.slave=pty.openpty();self.initial=termios.tcgetattr(self.slave)
        self.output=bytearray();self.deadline=time.monotonic()+20;self.resize(90,20)
        def setup():
            os.setsid();fcntl.ioctl(self.slave,termios.TIOCSCTTY,0)
        self.process=subprocess.Popen([binary,'--exact',DRIVER,'--nocapture'],stdin=self.slave,stdout=self.slave,stderr=self.slave,env=dict(env,HELM_PLAIN_TERMINAL_DRIVER=mode),preexec_fn=setup)
    def resize(self,columns,rows):fcntl.ioctl(self.slave,termios.TIOCSWINSZ,struct.pack('HHHH',rows,columns,0,0))
    def pump(self):
        assert time.monotonic()<self.deadline,bytes(self.output[-4000:])
        if select.select([self.master],[],[],.03)[0]:
            try:self.output.extend(os.read(self.master,65536))
            except OSError as error:
                if error.errno!=errno.EIO:raise
        assert len(self.output)<4*1024*1024,'fixture output cap'
    def wait(self,value):
        while value not in CSI.sub(b'',self.output):
            self.pump();assert self.process.poll() is None or value in CSI.sub(b'',self.output),bytes(self.output[-4000:])
    def send(self,data):os.write(self.master,data)
    def finish(self):
        self.wait(b'DRIVER_CLEANUP_OK')
        while self.process.poll() is None:self.pump()
        assert self.process.returncode==0,bytes(self.output[-4000:])
        assert termios.tcgetattr(self.slave)==self.initial,'input flags not restored'
        assert b'\x1b[?1049h' in self.output and b'\x1b[?1049l' in self.output,'alternate screen not restored'
        assert b'\x1b]52;' not in self.output,'untrusted OSC forwarded'
    def close(self):
        if self.process.poll() is None:os.killpg(self.process.pid,signal.SIGKILL);self.process.wait(timeout=5)
        os.close(self.master);os.close(self.slave)
def main():
    parser=argparse.ArgumentParser();parser.add_argument('--test-binary',required=True);args=parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='helm-plain-pty-') as tmp:
        env=dict(os.environ,HOME=tmp,XDG_CONFIG_HOME=tmp+'/config',XDG_DATA_HOME=tmp+'/data',TERM='xterm-256color')
        cases=['ctrl-t','ctrl-bracket','partial','coalesced-command','resize','too-small','interrupt','inner-exit']
        for case in cases:
            terminal=Terminal(args.test_binary,env)
            try:
                terminal.wait(b'DRIVER_PROMPT')
                if case=='coalesced-command':
                    terminal.send('/terminal\n'.encode()+b"printf 'PRIVATE_CANARY\\n'\n\x14"+'next🧭\n'.encode())
                else:
                    terminal.send(b'/terminal\n');terminal.wait(b'INNER_READY')
                    if case=='resize':terminal.resize(30,8)
                    if case=='too-small':terminal.resize(2,2);terminal.wait(b'DRIVER_ERROR=')
                    elif case=='interrupt':os.kill(terminal.process.pid,signal.SIGUSR1);terminal.wait(b'DRIVER_ERROR=')
                    elif case=='inner-exit':terminal.send(b'exit\n');terminal.wait(b'DRIVER_INNER_EXIT')
                    else:
                        terminal.send(b"printf 'PRIVATE_CANARY\\n'; printf '\\033]52;c;PRIVATE_CANARY\\007'\n")
                        terminal.wait(b'PRIVATE_CANARY')
                        chord=b'\x1d' if case=='ctrl-bracket' else b'\x14'
                        if case=='partial':
                            terminal.send(chord+b'next');terminal.wait(b'DRIVER_RESUMED');terminal.send('🧭\n'.encode())
                        else:terminal.send(chord+'next🧭\n'.encode())
                if case not in ['too-small','interrupt','inner-exit']:terminal.wait(b'DRIVER_HANDOFF_OK')
                terminal.finish();print('PASS plain terminal '+case,flush=True)
            finally:terminal.close()
        result=subprocess.run([args.test_binary,'--exact',DRIVER,'--nocapture'],input=b'',stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=dict(env,HELM_PLAIN_TERMINAL_DRIVER='non-tty'),timeout=15)
        assert result.returncode==0,(result.stdout,result.stderr)
        assert b'requires local TTY' in result.stdout and b'DRIVER_CLEANUP_OK' in result.stdout
        print('PASS plain terminal non-TTY refusal',flush=True)
if __name__=='__main__':main()
