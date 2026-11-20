# Deploying the feedback intake Worker

The operator's runbook for `apps/feedback-cf` (decision 162). It puts
Storm's "Report a problem" intake on your own Cloudflare account, at
`storm-feedback.dewansh.space` — the domain `apps/server`'s
`DEFAULT_FEEDBACK_INTAKE_URL` ships pointed at. Nothing here is automated:
no Storm build deploys anything to Cloudflare.

What it holds: a report's text, kind, optional email, screenshot and
diagnostics bundle, in Workers KV under the `REPORTS` binding, until
`deploy/feedback/feedback-sync.py` picks each one up and deletes it (or 30
days pass unsynced — `RETENTION_MS` in `src/store.ts`, enforced by KV's own
`expirationTtl` on every write — there is no cron job).

KV rather than R2: R2 needs a payment method on the account even to stay
inside its own free tier; Workers KV does not, so this runs on Cloudflare's
free plan with no card on file.

## 1. Prerequisites

- A Cloudflare account with the `dewansh.space` zone on it. **No payment
  method needed** — everything here (Workers, KV, the Durable Object) fits
  the free plan.
- Node 22 and this repository. From `apps/feedback-cf`: `npm ci`.
- Auth, either:
  - `npx wrangler login` (opens a browser), or
  - a scoped `CLOUDFLARE_API_TOKEN` with: Workers Scripts:Edit, Workers KV
    Storage:Edit, Workers Routes:Edit, Zone DNS:Edit on `dewansh.space`,
    and Account Settings:Read.

## 2. Deploy

The `storm-feedback-reports` KV namespace already exists on this account —
`wrangler.jsonc`'s `kv_namespaces[0].id` is its real id. Setting this up on
a different account starts with minting one of your own:

```sh
cd apps/feedback-cf
npm ci
npx wrangler kv namespace create storm-feedback-reports
# prints an "id" — put it into wrangler.jsonc's kv_namespaces[0].id
npx wrangler secret put SYNC_TOKEN
# paste a freshly generated random token, e.g.:
#   openssl rand -base64 32
npx wrangler deploy
```

On this account, skip the `kv namespace create` line and go straight to
`secret put` then `deploy`. The KV namespace binding (`REPORTS`) needs its
real `id` in `wrangler.jsonc` before the first deploy — it is not
auto-provisioned by this repo's wrangler version. The `RATE_LIMITER`
Durable Object (SQLite-backed, Workers Free plan is enough) is declared in
`wrangler.jsonc` too; the first `deploy` creates its class and migration.
`SYNC_TOKEN` is the only secret the Worker needs — check `src/env.ts` if
that changes; `REPORTS` (the KV binding) and `RATE_LIMITER` are bindings,
not secrets, and need no separate value once `id` is set.

`wrangler.jsonc` declares the custom domain route
(`storm-feedback.dewansh.space`) directly, so `deploy` requests the
certificate and the DNS record itself — nothing to do in the dashboard.

## 3. Verify

The Worker has no dedicated health route; a GET answers 404 and a POST
missing fields answers 400 — both prove it is live and routing:

```sh
curl -s -o /dev/null -w '%{http_code}\n' https://storm-feedback.dewansh.space/
# 404

curl -s -o /dev/null -w '%{http_code}\n' \
  -X POST https://storm-feedback.dewansh.space/v1/reports -d '{}'
# 400

curl -s -o /dev/null -w '%{http_code}\n' \
  -H "Authorization: Bearer $SYNC_TOKEN" \
  https://storm-feedback.dewansh.space/v1/reports
# 200 — proves SYNC_TOKEN was set correctly
```

## 4. Where `SYNC_TOKEN` goes next

The same value goes into `/etc/storm-feedback-sync.env` on the prod box as
`SYNC_TOKEN=`, alongside `INTAKE_URL=https://storm-feedback.dewansh.space`
— see `deploy/feedback/README.md` for the sync service that files reports
into the vault. A secret cannot be read back from Cloudflare; keep your
own copy until it's in that file.

## Limits and KV's own quirks

- A report's JSON value must stay under KV's 25 MiB per-value cap — the
  Worker's own 5 MB body cap (`MAX_BODY_BYTES`) is well inside that.
- A key (`report:<id>`, `shot:<id>`, `idempotency:<install_id>/<hash>`)
  must stay under 512 bytes — every key here is well under 100.
- **KV is eventually consistent**: a report that just landed can take up
  to about 60 seconds to show up in `list()` (what `GET /v1/reports` uses)
  on another edge location. The sync service polls every 5 minutes, so
  this is never visible in practice — mentioned here only so a report
  missing from the *very next* curl isn't mistaken for a bug.
- Workers KV's free tier gives 100,000 reads and 1,000 writes a day, far
  above what this intake sees.

## Updating and removing

Update: pull, `npm ci`, `npx wrangler deploy`.

Remove: `npx wrangler delete`, which also removes the custom domain route
(the KV namespace itself survives a Worker delete — remove it separately
with `npx wrangler kv namespace delete --namespace-id <id>` if you want the
data gone too).
