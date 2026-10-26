# akc sync server

One Cloudflare Worker and one D1 database. It stores encrypted blobs and nothing
else: no accounts, no signup, no plaintext, no analytics. Free plan is enough for
personal use (100k requests/day, 5 GB of D1 storage, 100k row writes/day).

## Deploy

```sh
npm install -g wrangler
wrangler login

wrangler d1 create akc-sync
# copy the printed database_id into wrangler.toml

wrangler d1 execute akc-sync --remote --file=./schema.sql
wrangler deploy
```

`wrangler deploy` prints a URL such as
`https://akc-sync.your-subdomain.workers.dev`. Use that URL below.

## Connect a machine

On the first machine:

```sh
akc --sync-init --sync-url https://akc-sync.your-subdomain.workers.dev
```

That writes `~/.config/akc/sync-key` and prints a token that looks like
`akcsync://akc-sync.your-subdomain.workers.dev#<64 hex chars>`. It is printed
once. Back it up somewhere safe: it is the only way to read the synced copy on a
new machine.

On every other machine:

```sh
akc --sync-join 'akcsync://akc-sync.your-subdomain.workers.dev#<64 hex chars>'
akc --sync
```

Then run `akc --sync` whenever you want to exchange changes. It pulls, merges,
and pushes in two requests.

## What the server can see

Item ids (a PRF of the secret name), sizes, timestamps, and how often you sync.
It cannot read names or values, and it cannot derive the token from a stored
hash: it only stores `SHA-256(token)` as the vault name.

Anyone with the token can read the ciphertext and delete the vault. Treat it
like the vault password.

## Local development

```sh
wrangler d1 execute akc-sync --local --file=./schema.sql
wrangler dev            # http://localhost:8787
akc --sync-init --sync-url http://localhost:8787
```
