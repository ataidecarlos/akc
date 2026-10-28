CREATE TABLE IF NOT EXISTS items (
  vault TEXT NOT NULL,
  id TEXT NOT NULL,        -- PRF(secret, name); never the name itself
  seq INTEGER NOT NULL,    -- per-vault cursor; assigned by the server
  updated_at INTEGER NOT NULL,
  deleted INTEGER NOT NULL DEFAULT 0,
  hash TEXT NOT NULL,      -- lets the server skip identical writes
  blob TEXT,               -- AES-256-GCM ciphertext, or NULL for a tombstone
  PRIMARY KEY (vault, id)
);

CREATE INDEX IF NOT EXISTS items_by_seq ON items (vault, seq);

-- A retired identity. `--rotate` deletes the ciphertext the previous master key
-- opened and leaves this marker, so a machine still holding that key is told to
-- rotate instead of repopulating a stale copy.
CREATE TABLE IF NOT EXISTS vaults (
  vault TEXT PRIMARY KEY,
  superseded INTEGER NOT NULL DEFAULT 0
);
