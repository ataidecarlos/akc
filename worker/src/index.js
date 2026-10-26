// Opaque blob store for akc sync.
//
// The server is deliberately dumb: it authenticates with a bearer token, derives
// a vault name from it, and stores whatever encrypted entries the client sends.
// It never sees secret names or values, and it has no accounts, no signup, and
// no user table. A lost token means a lost vault (the client holds the only
// decryption key); that is the whole security model.

const TOKEN = /^[0-9a-f]{64}$/;
const ITEM_ID = /^[0-9a-f]{32}$/;
const MAX_BODY = 512 * 1024;
const MAX_BLOB = 128 * 1024;
const MAX_PUTS = 500;
const MAX_ITEMS = 5000;
const CHUNK = 50;

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === '/health') {
      return json({ ok: true });
    }
    if (url.pathname !== '/v1/sync' || request.method !== 'POST') {
      return json({ error: 'not found' }, 404);
    }

    const token = bearer(request);
    if (!TOKEN.test(token)) {
      return json({ error: 'unauthorized' }, 401);
    }
    const vault = (await sha256hex(token)).slice(0, 32);

    const text = await request.text();
    if (text.length > MAX_BODY) {
      return json({ error: 'body too large' }, 413);
    }
    let body;
    try {
      body = JSON.parse(text);
    } catch {
      return json({ error: 'invalid json' }, 400);
    }

    const since = Number(body.since) || 0;
    if (body.puts != null && (!Array.isArray(body.puts) || body.puts.length > MAX_PUTS)) {
      return json({ error: 'too many entries' }, 413);
    }
    const puts = body.puts || [];
    for (const put of puts) {
      if (!ITEM_ID.test(put.id) || typeof put.hash !== 'string' || put.hash.length > 128) {
        return json({ error: 'invalid entry' }, 400);
      }
      if (put.blob != null && (typeof put.blob !== 'string' || put.blob.length > MAX_BLOB)) {
        return json({ error: 'invalid entry' }, 413);
      }
    }

    const result = await apply(env, vault, since, puts);
    if (result.full) {
      return json({ error: 'vault is full' }, 507);
    }
    return json({ seq: result.seq, changes: result.changes });
  },
};

async function apply(env, vault, since, puts) {
  const existing = await env.DB.prepare('SELECT id, hash FROM items WHERE vault = ?')
    .bind(vault)
    .all();
  const known = new Map(existing.results.map((row) => [row.id, row.hash]));
  const total = existing.results.length;

  const pending = puts.filter((put) => known.get(put.id) !== put.hash);
  const inserts = pending.filter((put) => !known.has(put.id)).length;
  if (inserts > MAX_ITEMS - total) {
    return { full: true };
  }

  for (let i = 0; i < pending.length; i += CHUNK) {
    const statements = pending.slice(i, i + CHUNK).map((put) =>
      env.DB.prepare(
        `INSERT INTO items (vault, id, seq, updated_at, deleted, hash, blob)
         VALUES (?1, ?2, (SELECT COALESCE(MAX(seq), 0) + 1 FROM items WHERE vault = ?1), ?3, ?4, ?5, ?6)
         ON CONFLICT (vault, id) DO UPDATE SET
           seq = excluded.seq,
           updated_at = excluded.updated_at,
           deleted = excluded.deleted,
           hash = excluded.hash,
           blob = excluded.blob`,
      ).bind(vault, put.id, put.updated_at ?? 0, put.deleted ? 1 : 0, put.hash, put.blob ?? null),
    );
    if (statements.length > 0) {
      await env.DB.batch(statements);
    }
  }

  const changed = await env.DB.prepare(
    'SELECT id, seq, updated_at, deleted, blob FROM items WHERE vault = ? AND seq > ? ORDER BY seq',
  )
    .bind(vault, since)
    .all();

  let seq = since;
  for (const row of changed.results) {
    if (row.seq > seq) seq = row.seq;
  }
  return {
    seq,
    changes: changed.results.map((row) => ({
      id: row.id,
      seq: row.seq,
      updated_at: row.updated_at,
      deleted: row.deleted === 1,
      blob: row.blob,
    })),
  };
}

function bearer(request) {
  const header = request.headers.get('authorization') || '';
  const match = header.match(/^Bearer (.+)$/i);
  return match ? match[1] : '';
}

async function sha256hex(text) {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

function json(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}
