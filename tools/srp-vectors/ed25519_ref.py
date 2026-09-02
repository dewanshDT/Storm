"""RFC 8032 Ed25519 reference implementation, used only to generate test vectors.

Not production code and not on any request path — it exists so
docs/srp-vectors.json can be built without adding a dependency to any crate.
Its correctness is checked by the consumers: a signature it produces has to
verify under ed25519-dalek in apps/relay, or the vector test fails.
"""

import hashlib

b = 256
q = 2**255 - 19
el = 2**252 + 27742317777372353535851937790883648493


def H(m):
    return hashlib.sha512(m).digest()


def inv(x):
    return pow(x, q - 2, q)


d = -121665 * inv(121666) % q
I = pow(2, (q - 1) // 4, q)


def xrecover(y):
    xx = (y * y - 1) * inv(d * y * y + 1)
    x = pow(xx, (q + 3) // 8, q)
    if (x * x - xx) % q != 0:
        x = (x * I) % q
    if x % 2 != 0:
        x = q - x
    return x


By = 4 * inv(5)
Bx = xrecover(By)
B = [Bx % q, By % q]


def edwards(P, Q):
    x1, y1 = P
    x2, y2 = Q
    x3 = (x1 * y2 + x2 * y1) * inv(1 + d * x1 * x2 * y1 * y2)
    y3 = (y1 * y2 + x1 * x2) * inv(1 - d * x1 * x2 * y1 * y2)
    return [x3 % q, y3 % q]


def scalarmult(P, e):
    Q = [0, 1]
    while e > 0:
        if e & 1:
            Q = edwards(Q, P)
        P = edwards(P, P)
        e >>= 1
    return Q


def encodeint(y):
    return int(y).to_bytes(b // 8, "little")


def encodepoint(P):
    x, y = P
    return (y | ((x & 1) << (b - 1))).to_bytes(b // 8, "little")


def bit(h, i):
    return (h[i // 8] >> (i % 8)) & 1


def clamped_scalar(sk):
    h = H(sk)
    return 2 ** (b - 2) + sum(2**i * bit(h, i) for i in range(3, b - 2))


def publickey(sk):
    return encodepoint(scalarmult(B, clamped_scalar(sk)))


def Hint(m):
    h = H(m)
    return sum(2**i * bit(h, i) for i in range(2 * b))


def signature(m, sk, pk):
    h = H(sk)
    a = clamped_scalar(sk)
    r = Hint(bytes(h[b // 8 : b // 4]) + m)
    R = scalarmult(B, r)
    S = (r + Hint(encodepoint(R) + pk + m) * a) % el
    return encodepoint(R) + encodeint(S)
