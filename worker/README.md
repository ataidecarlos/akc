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

Point the config at the Worker once per machine:

```text
# ~/.config/akc/config
sync_url = https://akc-sync.your-subdomain.workers.dev
```

Then run `akc --sync` whenever you want to exchange changes. It pulls, merges,
and pushes in two requests.

There is no sync password and no token. The sync identity (the bearer token, the
entry key, and the item ids) is derived from the master password, so every
machine with the same password reaches the same vault, and a new machine builds
its local vault from the download.

## What the server can see

Item ids (a PRF of the secret name), sizes, timestamps, and how often you sync.
It cannot read names or values. It stores only `SHA-256(token)` as the vault
name, where the token is itself a subkey of the Argon2id-stretched master
password.

Anyone with the master password can read the ciphertext and delete the vault.
`--rotate` uploads the vault under the new identity, then deletes the copy the
previous password opened (`DELETE /v1/sync`) and leaves a marker in `vaults`.

The marker matters: a machine still holding the retired password would otherwise
recreate that vault on its next sync. Instead it gets `410 Gone`, so it stops and
tells the user to rotate there too.

Upgrading from the previous Worker: rerun `schema.sql` (it uses
`CREATE TABLE IF NOT EXISTS`) and `wrangler deploy` before rotating.

## Local development

```sh
wrangler d1 execute akc-sync --local --file=./schema.sql
wrangler dev            # http://localhost:8787
akc --sync --sync-url http://localhost:8787
```
