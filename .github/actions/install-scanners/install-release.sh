#!/usr/bin/env bash
set -euo pipefail

# INVARIANT: install only after SHA256 matches. A mismatch is deleted and not retried.
# Retries stop at the attempt cap or the deadline.

usage() {
  echo "usage: install-release.sh NAME URL SHA256 DEST CACHE_FILE" >&2
  exit 2
}

require_whole_number() {
  local label="$1" value="$2"
  if [[ ! "$value" =~ ^[0-9]+$ ]]; then
    echo "::error::${label} must be a whole number" >&2
    exit 2
  fi
}

download_verified() {
  local url="$1" dest="$2" want="$3" name="$4"
  local attempt=1
  local delay="${INSTALL_RELEASE_RETRY_DELAY:-2}"
  local max_attempts="${INSTALL_RELEASE_MAX_ATTEMPTS:-8}"
  local deadline_seconds="${INSTALL_RELEASE_DEADLINE_SECONDS:-120}"
  local max_delay=15
  local deadline

  require_whole_number INSTALL_RELEASE_RETRY_DELAY "$delay"
  require_whole_number INSTALL_RELEASE_MAX_ATTEMPTS "$max_attempts"
  require_whole_number INSTALL_RELEASE_DEADLINE_SECONDS "$deadline_seconds"
  if [ "$max_attempts" -lt 1 ]; then
    echo "::error::INSTALL_RELEASE_MAX_ATTEMPTS must be at least 1" >&2
    exit 2
  fi

  deadline=$((SECONDS + deadline_seconds))
  mkdir -p "$(dirname "$dest")"

  while true; do
    rm -f "$dest"
    if curl -fsSL --retry 0 --connect-timeout 15 --max-time 180 -o "$dest" "$url"; then
      if printf '%s  %s\n' "$want" "$dest" | sha256sum -c -; then
        return 0
      fi
      echo "::error::checksum mismatch for ${name}; refusing to install" >&2
      rm -f "$dest"
      return 1
    fi
    rm -f "$dest"
    if [ "$attempt" -ge "$max_attempts" ] || [ "$SECONDS" -ge "$deadline" ]; then
      echo "::error::failed to download ${name} after ${attempt} attempt(s)" >&2
      return 22
    fi
    echo "download of ${name} failed (attempt ${attempt}); retrying in ${delay}s"
    sleep "$delay"
    attempt=$((attempt + 1))
    if [ "$delay" -ne 0 ] && [ "$delay" -lt "$max_delay" ]; then
      delay=$((delay * 2))
      if [ "$delay" -gt "$max_delay" ]; then
        delay=$max_delay
      fi
    fi
  done
}

install_release() {
  local name="$1" url="$2" want="$3" dest="$4" cache_file="$5"
  local work

  if [[ ! "$name" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]; then
    echo "::error::refusing scanner name ${name}" >&2
    exit 2
  fi
  if [[ ! "$want" =~ ^[0-9a-f]{64}$ ]]; then
    echo "::error::refusing a digest that is not 64 hex characters" >&2
    exit 2
  fi

  mkdir -p "$dest" "$(dirname "$cache_file")"
  if [ -f "$cache_file" ] && printf '%s  %s\n' "$want" "$cache_file" | sha256sum -c -; then
    echo "using cached ${name} tarball"
  else
    if [ -e "$cache_file" ]; then
      echo "cached ${name} tarball failed checksum; downloading"
      rm -f "$cache_file"
    fi
    download_verified "$url" "$cache_file" "$want" "$name"
  fi

  work="$(mktemp -d)"
  tar -xzf "$cache_file" -C "$work" "$name"
  install -m 0755 "$work/$name" "$dest/$name"
  rm -rf "$work"
  "$dest/$name" --version
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  [ "$#" -eq 5 ] || usage
  install_release "$@"
fi
