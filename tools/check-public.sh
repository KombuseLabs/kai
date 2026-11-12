#!/bin/sh
# Refuses what must not be in a public repository: host paths, tracker ids,
# e-mail addresses and private project names in tracked files, and commits
# whose author or committer is not a GitHub noreply address (a person's, or
# GitHub's own, which commits what is merged or edited on the website).
#
# The private names are not listed here, since that would publish them:
# they are read, one extended regular expression per line, from
# specs/private-names.txt, which exists only in a maintainer's checkout.
# Where the file is missing (in CI, in a fresh clone) that one check is
# skipped, and the script says so.
#
# Exit status: 0 nothing found, 1 something found (each line is printed).
set -eu
cd "$(git rev-parse --show-toplevel)"

found=0

# check <what it is> <extended regular expression> [<expression to let through>]
check() {
  hits=$(git grep -nIE -e "$2" -- . || true)
  if [ -n "${3:-}" ] && [ -n "$hits" ]; then
    hits=$(printf '%s\n' "$hits" | grep -vE -e "$3" || true)
  fi
  if [ -n "$hits" ]; then
    printf 'check-public: %s:\n%s\n' "$1" "$hits" >&2
    found=1
  fi
}

check 'an absolute path into a home directory (write ~/… instead)' \
  '/(home|Users)/[A-Za-z0-9._-]+/'
check 'a tracker id' \
  '(^|[^A-Za-z0-9-])A-[0-9]{2,}([^0-9]|$)'
check 'an e-mail address' \
  '[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}' \
  '@users\.noreply\.github\.com|git@github\.com'

if [ -f specs/private-names.txt ]; then
  while IFS= read -r name; do
    [ -n "$name" ] || continue
    check "a private project name ($name)" "$name"
  done < specs/private-names.txt
else
  echo 'check-public: specs/private-names.txt is not here; private names are not checked' >&2
fi

if git rev-parse --verify --quiet HEAD >/dev/null; then
  people=$(git log --format='%ae%n%ce' | sort -u | grep -vE '@users\.noreply\.github\.com$|^noreply@github\.com$' || true)
  if [ -n "$people" ]; then
    printf 'check-public: commits by an address that is not a GitHub noreply address:\n%s\n' "$people" >&2
    found=1
  fi
fi

[ "$found" -eq 0 ] && echo 'check-public: nothing found'
exit "$found"
