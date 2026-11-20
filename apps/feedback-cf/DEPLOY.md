# Deploying the feedback intake Worker

The operator's runbook for `apps/feedback-cf` (decision 162). It puts
Storm's "Report a problem" intake on your own Cloudflare account, at
`storm-feedback.dewansh.space` — the domain `apps/server`'s
`DEFAULT_FEEDBACK_INTAKE_URL` ships pointed at. Nothing here is automated:
no Storm build deploys anything to Cloudflare.

What it holds: a report's text, kind, optional email, screenshot and
diagnostics bundle, in R2 under `storm-feedback-reports`, until
`deploy/feedback/feedback-sync.py` picks each one up and deletes it (or 30
days pass unsynced — `RETENTION_MS` in `src/store.ts`).

## 1. Prerequisites

- A Cloudflare account with the `dewansh.space` zone on it.
- Node 22 and this repository. From `apps/feedback-cf`: `npm ci`.
- Auth, either:
  - `npx wrangler login` (opens a browser), or
  - a scoped `CLOUDFLARE_API_TOKEN` with: Workers Scripts:Edit, Workers R2
    Storage:Edit, Workers Routes:Edit, Zone DNS:Edit on `dewansh.space`,
    and Account Settings:Read.

## 2. Deploy

```sh
cd apps/feedback-cf
npm ci
npx wrangler r2 bucket create storm-feedback-reports
npx wrangler secret put SYNC_TOKEN
# paste a freshly generated random token, e.g.:
#   openssl rand -base64 32
npx wrangler deploy
```

The bucket and the `RATE_LIMITER` Durable Object (SQLite-backed, Workers
Free plan is enough) are both declared in `wrangler.jsonc`; the first
`deploy` creates the Durable Object class and its migration. `SYNC_TOKEN`
is the only secret the Worker needs — check `src/env.ts` if that changes;
`FEEDBACK` (the R2 binding) and `RATE_LIMITER` are bindings, not secrets,
and need no separate value.

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

## Updating and removing

Update: pull, `npm ci`, `npx wrangler deploy`.

Remove: `npx wrangler delete`, which also removes the custom domain route.
