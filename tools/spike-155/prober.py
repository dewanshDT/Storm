import os, select, sys, termios, time, tty
W = os.path.dirname(os.path.abspath(__file__))
mode = open(os.path.join(W, "prober-mode")).read().strip()
fd = 0
old = termios.tcgetattr(fd); tty.setraw(fd)
out = sys.stdout.buffer
def w(b): out.write(b); out.flush()
w(b"\x1b]0;\xe2\x9c\xb3 Prober\x07prober " + mode.encode() + b"\r\n")
time.sleep(0.3)
w(b"\x1b]7501;?\x07\x1b[c")
got = b""; end = time.time() + 1.5
while time.time() < end:
    r, _, _ = select.select([fd], [], [], 0.1)
    if r: got += os.read(fd, 4096)
w(b"reply: " + repr(got).encode() + b"\r\n")
if mode == "garbage":
    w(b"\x1b]7501;state=working:app=prober\x07")
    time.sleep(3)
    for _ in range(3):
        w(b"\x1b]7501;state=thinking:app=prober\x07"); time.sleep(1.5)
for i in range(8):
    w(b"\x1b]0;\xe2\x97\x90 Prober\x07" if i % 2 == 0 else b"\x1b]0;\xe2\x9c\xb3 Prober\x07")
    time.sleep(1.5)
termios.tcsetattr(fd, termios.TCSADRAIN, old)
time.sleep(600)
