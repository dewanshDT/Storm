# srp-vectors

Generates `docs/srp-vectors.json` — the shared test vectors for SRP v1's
signed bytes.

## Why the file exists

`relay_auth_message`, `validate_nonce`, `validate_server_id` and the base64url
encoding rules are re-derived in three places that **cannot depend on one
another**:

| | Where | Language |
|---|---|---|
| origin server | `apps/server/src/auth/identity.rs` | Rust |
| relay | `apps/relay/src/auth.rs` | Rust |
| client | `apps/client/lib/api/ed25519_verify.dart` | Dart |

`apps/server` is a binary crate, so there is no library for the relay to depend
on — and there must not be one. The relay is never mandatory (R6), and a shared
crate is how "optional" quietly becomes "linked in".

So the agreement between them is a wire commitment with **no compiler
enforcement**. Drift does not fail a build or produce a type error; it surfaces
at runtime as `auth_failed`, which is also what a genuine attack, an expired
nonce and a refused binding all look like. That is the worst possible failure
signature for a mismatch.

Each implementation reads this file in its own test suite, so drift fails a
test instead.

## Running it

```sh
cd <repo root>
python3 tools/srp-vectors/generate.py
```

No dependencies — `ed25519_ref.py` is the RFC 8032 reference implementation,
about eighty lines of integer arithmetic. It exists so the vectors can be built
without adding a crypto dependency to a tool, and it is **not production code
and not on any request path**. Its correctness is checked by the consumers: a
signature it produces has to verify under `ed25519-dalek` in `apps/relay`, or
the vector test fails.

The signing key is derived from the seed `00..1f` and is public by design. It
signs test messages and nothing else.

## Regenerating is a protocol change

The file is a commitment, not a fixture. If a regeneration changes an existing
vector, that is a wire-format change: it belongs in `PLAN.md`'s decision log
and in `docs/srp-v1.md`, not in a quiet commit. Adding new vectors alongside
the existing ones is ordinary work.
