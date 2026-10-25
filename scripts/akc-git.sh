#!/usr/bin/env bash
# Run an akc command against a vault that lives in git, serializing writers.
#
# The vault is a single encrypted blob, so git cannot merge two versions of it:
# if two people write at once, the second push is rejected and one side's secret
# is stranded. This pulls before the write and pushes after it, rebasing and
# retrying when the push is rejected, so the second writer re-reads the first
# writer's change instead of colliding with it.
#
#   scripts/akc-git.sh secrets.akc api_key --set sk-123
#   scripts/akc-git.sh secrets.akc api_key
#
# The vault must already be tracked by a branch that has an upstream.
#
# Set AKC to test a specific build, e.g. AKC=target/release/akc scripts/akc-git.sh ...
set -euo pipefail

AKC="${AKC:-akc}"

if [[ $# -lt 2 ]]; then
    echo "usage: $(basename "$0") <vault> <akc-args...>" >&2
    exit 2
fi

vault="$1"
shift

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "error: not inside a git work tree" >&2
    exit 1
fi

branch=$(git branch --show-current)
if [[ -z "$branch" ]]; then
    echo "error: detached HEAD; check out a branch before writing to $vault" >&2
    exit 1
fi

# Without an upstream the push below can never succeed, and the retry loop would
# report it as someone else winning a race. Say what is actually wrong.
if [[ -z "$(git config --get "branch.$branch.remote" || true)" ]]; then
    echo "error: branch '$branch' has no upstream; push it once before using this" >&2
    exit 1
fi

# Start from the newest remote state, so this write is not based on a stale read.
# A branch that has never been pushed has no upstream ref yet: there is nothing
# to pull, and the push below creates it.
if git rev-parse --verify --quiet '@{upstream}' >/dev/null 2>&1; then
    if ! git pull --rebase --quiet; then
        echo "error: could not pull; resolve that before touching $vault" >&2
        exit 1
    fi
fi

"$AKC" --keychain "$vault" "$@"

# A read-only command leaves the vault untouched, so there is nothing to publish.
# `git status` rather than `git diff`: diff ignores untracked files, so a vault
# being added to the repo for the first time would look unchanged.
if [[ -z "$(git status --porcelain -- "$vault")" ]]; then
    exit 0
fi

# Only the secret's NAME goes in the message. A value passed to --set would be a
# secret in a commit message, which is plaintext and permanent -- so it is never
# echoed, only the name the caller asked for.
git add -- "$vault"
git commit --quiet -m "akc: ${1:-update} $vault" -- "$vault"

# A rejected push is the lock: it means someone else committed first.
for attempt in 1 2 3; do
    if git push --quiet; then
        exit 0
    fi
    echo "push rejected; rebasing onto the remote (attempt $attempt/3)" >&2
    if ! git pull --rebase --quiet; then
        # Undo the rebase this loop started. A half-finished rebase is a
        # confusing place to land after asking for a one-line `set`, and
        # aborting loses nothing: the replayed change is still a commit.
        #
        # Do not suggest `checkout --ours/--theirs` as the recovery. Inside a
        # rebase those are swapped relative to a merge, so the obvious reading
        # picks the wrong side and silently discards a secret.
        git rebase --abort >/dev/null 2>&1 || true
        cat >&2 <<EOF

error: the remote also changed $vault, and git cannot merge an encrypted blob.
Your write is still in the last commit. Keep it reachable before taking the
remote's version:

  git branch akc-write-backup      # point a name at your commit
  git reset --hard '@{upstream}'   # take the remote's version
  ...then re-run the command you just ran, and commit.

The backup branch holds your commit until you delete it.
EOF
        exit 1
    fi
done

echo "error: push kept being rejected; giving up" >&2
exit 1
