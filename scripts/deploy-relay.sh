#!/usr/bin/env bash
# Run on the existing Naro relay host after uploading a verified release directory.
set -Eeuo pipefail
version="$1"
expected_previous="$2"
[[ "$version" =~ ^[a-zA-Z0-9._-]+$ ]]
[[ "$expected_previous" =~ ^narolink-relay:[a-zA-Z0-9._-]+$ ]]
image="narolink-relay:$version"
release="/opt/narolink/releases/$version"
backup="/opt/narolink/.backups/compose-$version"
preflight="naro-preflight-$version"
domain="naro.bato.chat"
cd /opt/narolink
exec 9>.deploy.lock
flock -n 9
test "$(docker inspect -f '{{.Config.Image}}' narolink-relay-1)" = "$expected_previous"
test "$(docker compose config --images)" = "$expected_previous"
draft="$(mktemp /opt/narolink/.compose-native.XXXXXX)"
switched=0
finish() {
  result=$?
  trap - EXIT
  docker rm -f "$preflight" >/dev/null 2>&1 || true
  if test "$result" -ne 0 && test "$switched" = 1; then
    cp -p "$backup/compose.yml" "$draft"
    mv "$draft" compose.yml
    docker compose up -d --no-deps --force-recreate relay
    printf 'Rolled back Naro relay to %s\n' "$expected_previous"
  fi
  if test -f "$draft"; then unlink "$draft"; fi
  exit "$result"
}
trap finish EXIT
(cd "$release" && sha256sum -c binary.sha256)
chmod 0755 "$release/narolink-relay"
docker build --network=none -t "$image" "$release"
test "$(docker image inspect -f '{{.Os}}/{{.Architecture}}' "$image")" = linux/amd64
docker run -d --rm --name "$preflight" --read-only --cap-drop ALL --security-opt no-new-privileges \
  --env-file /etc/narolink/relay.env -e NAROLINK_RELAY_ADDR=0.0.0.0:7319 \
  -p 127.0.0.1::7319 "$image" >/dev/null
port="$(docker port "$preflight" 7319/tcp | sed 's/^127\.0\.0\.1://')"
test -n "$port"
verify_assets() {
  local base="$1" secure="$2" hash file actual
  while read -r hash file; do
    if test "$file" = index.html; then file=''; fi
    if test "$secure" = yes; then
      actual="$(curl -fsS --max-time 8 --resolve "$domain:443:127.0.0.1" "$base/$file?release=$version" | sha256sum | cut -d' ' -f1)" || return 1
    else
      actual="$(curl -fsS --max-time 8 -H "Host: $domain" "$base/$file" | sha256sum | cut -d' ' -f1)" || return 1
    fi
    test "$actual" = "$hash" || return 1
  done < "$release/web.sha256"
}
ready=0
for attempt in {1..15}; do
  if verify_assets "http://127.0.0.1:$port" no; then ready=1; break; fi
  sleep 1
done
test "$ready" = 1
install -d -m 0700 "$backup"
cp -p compose.yml "$backup/compose.yml"
python3 -I - "$version" "$expected_previous" compose.yml "$draft" <<'PY'
from pathlib import Path
import sys
version, previous, source, target = sys.argv[1:]
text = Path(source).read_text()
old = f'    image: {previous}\n'
assert text.count(old) == 1
text = text.replace(old, f'    image: narolink-relay:{version}\n')
lines = text.splitlines(keepends=True)
contexts = [i for i, line in enumerate(lines) if line.startswith('      context: ./releases/')]
assert len(contexts) == 1
lines[contexts[0]] = f'      context: ./releases/{version}\n'
Path(target).write_text(''.join(lines))
PY
test "$(docker compose -f "$draft" config --images)" = "$image"
chmod --reference=compose.yml "$draft"
switched=1
mv "$draft" compose.yml
docker compose up -d --no-deps --force-recreate relay
ready=0
for attempt in {1..20}; do
  if verify_assets "https://$domain" yes; then ready=1; break; fi
  sleep 1
done
test "$ready" = 1
test "$(docker inspect -f '{{.Config.Image}}' narolink-relay-1)" = "$image"
docker compose ps relay
printf 'All production Naro assets match the verified release.\n'
