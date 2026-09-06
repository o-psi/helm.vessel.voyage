#!/usr/bin/env python3
"""Actual main Terminal approver/sink under owned stdin and panic/drop restoration."""
import fcntl,os,pty,select,signal,struct,subprocess,sys,tempfile,termios,time
with tempfile.TemporaryDirectory(prefix='helm-plain-frontend-') as tmp:
    for mode in ['ownership','panic']:
        master,slave=pty.openpty();initial=termios.tcgetattr(slave)
        fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',20,80,0,0))
        def setup():os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
        process=subprocess.Popen([sys.argv[1],'--exact','plain_terminal_frontend_tests::frontend_driver','--nocapture'],stdin=slave,stdout=slave,stderr=slave,preexec_fn=setup,env=dict(os.environ,HELM_PLAIN_FRONTEND_DRIVER=mode,HOME=tmp,XDG_CONFIG_HOME=tmp+'/config',XDG_DATA_HOME=tmp+'/data'))
        output=bytearray();deadline=time.monotonic()+10
        try:
            while process.poll() is None:
                assert time.monotonic()<deadline,bytes(output)
                if select.select([master],[],[],.02)[0]:output.extend(os.read(master,65536));assert len(output)<1024*1024
            while select.select([master],[],[],0)[0]:output.extend(os.read(master,65536))
            assert process.returncode==0,bytes(output)
            assert b'FRONTEND_OWNERSHIP_OK' in output,bytes(output)
            assert b'SUPPRESSED_FRONTEND_CANARY' not in output
            assert b'Proceed?' not in output and b'must-not-approve' not in output
            assert termios.tcgetattr(slave)==initial
            assert b'\x1b[?1049h' in output and b'\x1b[?1049l' in output
            print('PASS plain frontend '+mode,flush=True)
        finally:
            if process.poll() is None:os.killpg(process.pid,signal.SIGKILL);process.wait(timeout=5)
            os.close(master);os.close(slave)
