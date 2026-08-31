#!/usr/bin/env python3
"""Generate docs/srp-vectors.json.

Run from the repo root. Regenerating is not routine: the file is a wire
commitment, so a change to it is a protocol change and belongs in PLAN.md.
"""

import base64
import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import ed25519_ref as ed  # noqa: E402

RELAY_PREFIX = "storm-relay-auth:v1:"
CHALLENGE_PREFIX = "storm-challenge:v1:"


def b64(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).decode().rstrip("=")


def relay_msg(server_id: str, nonce: str) -> bytes:
    return f"{RELAY_PREFIX}{server_id}:{nonce}".encode()


def challenge_msg(server_id: str, nonce: str) -> bytes:
    return f"{CHALLENGE_PREFIX}{server_id}:{nonce}".encode()


# A fixed seed, so the vectors are reproducible and the private key in this
# file is worthless by construction — it signs nothing but test messages.
SEED = bytes(range(32))
PK = ed.publickey(SEED)

SERVER_ID = "srv_01ARZ3NDEKTSV4RRFFQ69G5FAV"
NONCE = "hbpNZ1WGiFB8kQvJ7RXt2A"

doc = {
    "version": 1,
    "generated_by": "tools/gen-srp-vectors (RFC 8032 reference implementation)",
    "about": (
        "Shared test vectors for SRP v1's signed bytes. apps/server, apps/relay "
        "and apps/client each re-derive these functions in a different language "
        "and cannot depend on one another; nothing in any build makes them "
        "agree. Every implementation reads this file, so drift fails a test "
        "instead of surfacing as auth_failed at runtime, which is what a "
        "genuine attack also looks like. Changing this file is a protocol "
        "change."
    ),
    "encoding": (
        "Every binary field is base64url with no padding, matching the wire. "
        "message_utf8 is the same bytes as message_b64, given readably; an "
        "implementation may check either."
    ),
    "domains": {
        "relay_auth": RELAY_PREFIX,
        "client_challenge": CHALLENGE_PREFIX,
        "note": (
            "The two prefixes must never coincide. A signature proving the "
            "right to register at a relay must be structurally impossible to "
            "replay as one proving identity to a client."
        ),
    },
    "keys": {
        "test_public_key_b64": b64(PK),
        "note": (
            "Derived from the seed 00..1f. The matching private key is public "
            "by design — it signs test messages only."
        ),
    },
}

# --- message construction ---------------------------------------------------

msg_cases = [
    ("ascii", "srv_ABC", "nonce-0123456789"),
    ("realistic", SERVER_ID, NONCE),
    ("min_length_nonce", "s", "a" * 16),
    ("max_length_nonce", "s", "a" * 128),
    ("max_length_server_id", "s" * 128, NONCE),
    ("server_id_with_dash_and_underscore", "srv_a-b_c", NONCE),
]
doc["relay_auth_message"] = [
    {
        "name": name,
        "server_id": sid,
        "nonce": nonce,
        "message_utf8": relay_msg(sid, nonce).decode(),
        "message_b64": b64(relay_msg(sid, nonce)),
    }
    for name, sid, nonce in msg_cases
]
doc["client_challenge_message"] = [
    {
        "name": name,
        "server_id": sid,
        "nonce": nonce,
        "message_utf8": challenge_msg(sid, nonce).decode(),
        "message_b64": b64(challenge_msg(sid, nonce)),
    }
    for name, sid, nonce in msg_cases[:2]
]

# --- the collision that both validators exist to prevent --------------------

doc["delimiter_confusion"] = {
    "about": (
        "The message is colon-delimited, so a colon inside EITHER field "
        "re-splits it. Both pairs below produce identical signed bytes: one "
        "signature would cover two different (server_id, nonce) readings. "
        "Validating only the nonce leaves the server_id half open — that was "
        "PLAN.md decision 60. An implementation is correct here only if it "
        "rejects both pairs before signing or verifying."
    ),
    "colliding_pairs": [
        {
            "a": {"server_id": "srv_a", "nonce": "b:cccccccccccccccc"},
            "b": {"server_id": "srv_a:b", "nonce": "cccccccccccccccc"},
            "shared_message_utf8": relay_msg("srv_a", "b:cccccccccccccccc").decode(),
            "both_must_be_rejected": True,
        }
    ],
}
assert relay_msg("srv_a", "b:cccccccccccccccc") == relay_msg(
    "srv_a:b", "cccccccccccccccc"
), "the collision this section documents no longer exists"

# --- validation -------------------------------------------------------------

doc["validate_nonce"] = [
    {"name": "typical", "nonce": NONCE, "valid": True},
    {"name": "min_length", "nonce": "a" * 16, "valid": True},
    {"name": "one_below_min", "nonce": "a" * 15, "valid": False},
    {"name": "max_length", "nonce": "a" * 128, "valid": True},
    {"name": "one_above_max", "nonce": "a" * 129, "valid": False},
    {"name": "empty", "nonce": "", "valid": False},
    {"name": "contains_colon", "nonce": "aaaaaaaa:aaaaaaaa", "valid": False},
    {"name": "contains_quote", "nonce": 'aaaaaaaa"aaaaaaaa', "valid": False},
    {"name": "contains_space", "nonce": "aaaaaaaa aaaaaaaa", "valid": False},
    {"name": "contains_newline", "nonce": "aaaaaaaa\naaaaaaaa", "valid": False},
    {"name": "contains_tab", "nonce": "aaaaaaaa\taaaaaaaa", "valid": False},
    {"name": "non_ascii", "nonce": "aaaaaaaaéaaaaaaaa", "valid": False},
    {"name": "punctuation_is_allowed", "nonce": "a-b_c.d~e!f@g#h$i%", "valid": True},
]

doc["validate_server_id"] = [
    {"name": "typical", "server_id": SERVER_ID, "valid": True},
    {"name": "dash_and_underscore", "server_id": "srv_a-b_c", "valid": True},
    {"name": "single_char", "server_id": "s", "valid": True},
    {"name": "max_length", "server_id": "s" * 128, "valid": True},
    {"name": "one_above_max", "server_id": "s" * 129, "valid": False},
    {"name": "empty", "server_id": "", "valid": False},
    {"name": "contains_colon", "server_id": "srv:a", "valid": False},
    {"name": "contains_slash", "server_id": "srv/a", "valid": False},
    {"name": "contains_dot", "server_id": "srv.a", "valid": False},
    {"name": "contains_percent", "server_id": "srv%2Fa", "valid": False},
    {"name": "contains_space", "server_id": "srv a", "valid": False},
    {"name": "non_ascii", "server_id": "srv_é", "valid": False},
]

# --- signature verification -------------------------------------------------

good_sig = ed.signature(relay_msg(SERVER_ID, NONCE), SEED, PK)
challenge_sig = ed.signature(challenge_msg(SERVER_ID, NONCE), SEED, PK)
other_sig = ed.signature(relay_msg("srv_OTHER", NONCE), SEED, PK)

doc["verify_relay_auth"] = [
    {
        "name": "valid",
        "public_key_b64": b64(PK),
        "server_id": SERVER_ID,
        "nonce": NONCE,
        "signature_b64": b64(good_sig),
        "verifies": True,
    },
    {
        "name": "signature_over_the_client_challenge_domain",
        "why": (
            "Domain separation, asserted rather than assumed. Same key, same "
            "server_id, same nonce — only the prefix differs, and it must not "
            "verify as a relay registration."
        ),
        "public_key_b64": b64(PK),
        "server_id": SERVER_ID,
        "nonce": NONCE,
        "signature_b64": b64(challenge_sig),
        "verifies": False,
    },
    {
        "name": "signature_over_a_different_server_id",
        "public_key_b64": b64(PK),
        "server_id": SERVER_ID,
        "nonce": NONCE,
        "signature_b64": b64(other_sig),
        "verifies": False,
    },
    {
        "name": "signature_over_a_different_nonce",
        "public_key_b64": b64(PK),
        "server_id": SERVER_ID,
        "nonce": "a" * 22,
        "signature_b64": b64(good_sig),
        "verifies": False,
    },
    {
        "name": "signature_truncated",
        "public_key_b64": b64(PK),
        "server_id": SERVER_ID,
        "nonce": NONCE,
        "signature_b64": b64(good_sig[:63]),
        "verifies": False,
    },
    {
        "name": "signature_not_base64url",
        "why": "Standard-alphabet base64 must not be accepted for a base64url field.",
        "public_key_b64": b64(PK),
        "server_id": SERVER_ID,
        "nonce": NONCE,
        "signature_b64": base64.b64encode(good_sig).decode().rstrip("="),
        "verifies": False,
    },
]

doc["verify_client_challenge"] = [
    {
        "name": "valid",
        "why": (
            "The client's half of the same commitment. apps/client re-derives "
            "challengeMessage in Dart, where base64Url.decode accepts the "
            "standard alphabet as well — so the strict cases below are the "
            "ones that catch a lenient decoder."
        ),
        "public_key_b64": b64(PK),
        "server_id": SERVER_ID,
        "nonce": NONCE,
        "signature_b64": b64(challenge_sig),
        "verifies": True,
    },
    {
        "name": "signature_over_the_relay_auth_domain",
        "why": "The mirror of the relay-side domain-separation case.",
        "public_key_b64": b64(PK),
        "server_id": SERVER_ID,
        "nonce": NONCE,
        "signature_b64": b64(good_sig),
        "verifies": False,
    },
    {
        "name": "signature_over_a_different_server_id",
        "public_key_b64": b64(PK),
        "server_id": "srv_OTHER",
        "nonce": NONCE,
        "signature_b64": b64(challenge_sig),
        "verifies": False,
    },
    {
        "name": "public_key_in_the_standard_alphabet",
        "why": (
            "base64url only. A decoder that also accepts '+/' gives one key "
            "two spellings, which is what SRP v1 3.1 forbids."
        ),
        "public_key_b64": base64.b64encode(PK).decode().rstrip("="),
        "server_id": SERVER_ID,
        "nonce": NONCE,
        "signature_b64": b64(challenge_sig),
        "verifies": False,
    },
    {
        "name": "signature_in_the_standard_alphabet",
        "public_key_b64": b64(PK),
        "server_id": SERVER_ID,
        "nonce": NONCE,
        "signature_b64": base64.b64encode(challenge_sig).decode().rstrip("="),
        "verifies": False,
    },
    {
        "name": "public_key_padded",
        "public_key_b64": base64.urlsafe_b64encode(PK).decode(),
        "server_id": SERVER_ID,
        "nonce": NONCE,
        "signature_b64": b64(challenge_sig),
        "verifies": False,
    },
    {
        "name": "public_key_wrong_length",
        "public_key_b64": b64(PK[:31]),
        "server_id": SERVER_ID,
        "nonce": NONCE,
        "signature_b64": b64(challenge_sig),
        "verifies": False,
    },
]

doc["public_key_parsing"] = {
    "about": (
        "A key is compared and stored as decoded bytes, never as the encoded "
        "string: padded and unpadded encodings of the same key are different "
        "strings, and a string comparison would read that as a key change and "
        "refuse a legitimate re-registration for ever (SRP v1 4.1 has no way "
        "back)."
    ),
    "cases": [
        {"name": "unpadded", "public_key_b64": b64(PK), "accepted": True},
        {
            "name": "padded",
            "public_key_b64": base64.urlsafe_b64encode(PK).decode(),
            "accepted": False,
            "why": "The wire form is unpadded; accepting both invites two spellings of one key.",
        },
        {
            "name": "standard_alphabet",
            "public_key_b64": base64.b64encode(PK).decode().rstrip("="),
            "accepted": False,
        },
        {"name": "too_short", "public_key_b64": b64(PK[:31]), "accepted": False},
        {"name": "too_long", "public_key_b64": b64(PK + b"\x00"), "accepted": False},
        {"name": "empty", "public_key_b64": "", "accepted": False},
    ],
}

# A signature the *standard* base64 alphabet spells differently from base64url
# is only a useful vector if the two spellings actually differ.
assert base64.b64encode(good_sig).decode().rstrip("=") != b64(good_sig), (
    "pick a different test key: this signature encodes identically in both "
    "base64 alphabets, so the alphabet vector proves nothing"
)
assert base64.b64encode(PK).decode().rstrip("=") != b64(PK), (
    "pick a different test key: this public key encodes identically in both "
    "base64 alphabets"
)
assert base64.b64encode(challenge_sig).decode().rstrip("=") != b64(challenge_sig), (
    "pick a different test key: this challenge signature encodes identically "
    "in both base64 alphabets"
)

out = pathlib.Path("docs/srp-vectors.json")
out.write_text(json.dumps(doc, indent=2, ensure_ascii=False) + "\n")
print(f"wrote {out} ({out.stat().st_size} bytes)")
