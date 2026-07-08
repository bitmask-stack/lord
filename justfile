set positional-arguments

watch +args='test':
  cargo watch --clear --exec '{{args}}'

deps-check:
  #!/usr/bin/env bash
  set -euo pipefail
  missing=0
  for dep in ../carbonado ../bao-tree; do
    if [ ! -d "$dep" ]; then
      echo "missing path dependency: $dep" >&2
      missing=1
    fi
  done
  if [ "$missing" -ne 0 ]; then
    echo "Clone carbonado and bao-tree as siblings of lord under surmount/ — see README Monorepo layout." >&2
    exit 1
  fi

smoke:
  cargo test --test integration smoke:: -- --nocapture

# Iroh loopback gossip (serial + wall timeout — avoids stacked endpoint zombies).
p2p-smoke:
  #!/usr/bin/env bash
  set -euo pipefail
  timeout 120 cargo test -p lord-iroh --test loopback_gossip -- --nocapture --test-threads=1

# Full test suite: integration smoke → p2p smoke → workspace `cargo test --all`.
# One command to run everything: `just test`
test-all:
  #!/usr/bin/env bash
  set -euo pipefail
  echo "== 1/3 smoke (integration CLI) =="
  just smoke
  echo "== 2/3 p2p-smoke (Iroh loopback) =="
  just p2p-smoke
  echo "== 3/3 cargo test --all =="
  cargo test --all

test: test-all

ci-local: deps-check clippy forbid
  cargo fmt --all -- --check
  just test-all

ci: clippy forbid
  cargo fmt -- --check
  just test-all
  cargo test --all -- --ignored

forbid:
  ./bin/forbid

fmt:
  cargo fmt --all

clippy:
  cargo clippy --all --all-targets -- --deny warnings

install-git-hooks:
  #!/usr/bin/env bash
  set -euo pipefail
  for hook in hooks/*; do
      name=$(basename "$hook")
      if [ ! -e ".git/hooks/$name" ]; then
          ln -s "$PWD/$hook" ".git/hooks/$name"
      fi
  done

deploy branch remote chain domain:
  ssh root@{{domain}} '\
    export DEBIAN_FRONTEND=noninteractive \
    && mkdir -p deploy \
    && apt-get update --yes \
    && apt-get upgrade --yes \
    && apt-get install --yes git rsync'
  rsync -avz deploy/checkout root@{{domain}}:deploy/checkout
  ssh root@{{domain}} 'cd deploy && ./checkout {{branch}} {{remote}} {{chain}} {{domain}}'

deploy-mainnet-alpha branch='master' remote='bitmask-stack/lord': \
  (deploy branch remote 'main' 'alpha.ordinals.net')

deploy-mainnet-bravo branch='master' remote='bitmask-stack/lord': \
  (deploy branch remote 'main' 'bravo.ordinals.net')

deploy-mainnet-charlie branch='master' remote='bitmask-stack/lord': \
  (deploy branch remote 'main' 'charlie.ordinals.net')

deploy-signet branch='master' remote='bitmask-stack/lord': \
  (deploy branch remote 'signet' 'signet.ordinals.net')

deploy-all: \
  deploy-signet \
  deploy-mainnet-alpha \
  deploy-mainnet-bravo \
  deploy-mainnet-charlie

delete-indices: \
  (delete-index "signet.ordinals.net") \

delete-index domain:
  ssh root@{{domain}} 'systemctl stop lord && rm -rf /var/lib/lord/*/index'

servers := 'alpha bravo charlie signet'

initialize-server-keys:
  #!/usr/bin/env bash
  set -euxo pipefail
  rm -rf tmp/ssh
  mkdir -p tmp/ssh
  ssh-keygen -C ordinals -f tmp/ssh/id_ed25519 -t ed25519 -N ''
  for server in {{ servers }}; do
    ssh-copy-id -i tmp/ssh/id_ed25519.pub root@$server.ordinals.net
    scp tmp/ssh/* root@$server.ordinals.net:.ssh
  done
  rm -rf tmp/ssh

install-personal-key key='~/.ssh/id_ed25519.pub':
  #!/usr/bin/env bash
  set -euxo pipefail
  for server in {{ servers }}; do
    ssh-copy-id -i {{ key }} root@$server.ordinals.net
  done

server-keys:
  #!/usr/bin/env bash
  set -euxo pipefail
  for server in {{ servers }}; do
    ssh root@$server.ordinals.net cat .ssh/authorized_keys
  done

log unit='lord' domain='alpha.ordinals.net':
  ssh root@{{domain}} 'journalctl -fu {{unit}}'

fuzz:
  #!/usr/bin/env bash
  set -euxo pipefail
  cd fuzz
  while true; do
    cargo +nightly fuzz run runestone-decipher -- -max_total_time=60
    cargo +nightly fuzz run varint-decode -- -max_total_time=60
    cargo +nightly fuzz run varint-encode -- -max_total_time=60
    cargo +nightly fuzz run transaction-builder -- -max_total_time=60
  done

open:
  open http://localhost

doc:
  cargo doc --workspace --exclude audit-content-security-policy --exclude audit-cache --open

prepare-release revision='master':
  #!/usr/bin/env bash
  set -euxo pipefail
  git checkout {{ revision }}
  git pull origin {{ revision }}
  echo >> CHANGELOG.md
  git log --pretty='format:- %s' >> CHANGELOG.md
  $EDITOR CHANGELOG.md
  $EDITOR Cargo.toml
  version=`sed -En 's/version[[:space:]]*=[[:space:]]*"([^"]+)"/\1/p' Cargo.toml | head -1`
  cargo check
  git checkout -b release-$version
  git add -u
  git commit -m "Release $version"
  gh pr create --web

# Publishes the lord crate from bitmask-stack/lord (not upstream ordinals/ord).
publish-release revision='master':
  #!/usr/bin/env bash
  set -euxo pipefail
  rm -rf tmp/release
  git clone --depth 1 https://github.com/bitmask-stack/lord.git tmp/release
  cd tmp/release
  git checkout {{ revision }}
  cargo publish
  cd ../..
  rm -rf tmp/release

# Tags and publishes lord releases from bitmask-stack/lord.
publish-tag-and-crate revision='master':
  #!/usr/bin/env bash
  set -euxo pipefail
  rm -rf tmp/release
  git clone --depth 1 git@github.com:bitmask-stack/lord.git tmp/release
  cd tmp/release
  git checkout {{revision}}
  version=`sed -En 's/version[[:space:]]*=[[:space:]]*"([^"]+)"/\1/p' Cargo.toml | head -1`
  git tag -a $version -m "Release $version"
  git push git@github.com:bitmask-stack/lord.git $version
  cargo publish
  cd ../..
  rm -rf tmp/release

outdated:
  cargo outdated --root-deps-only --workspace

unused:
  cargo +nightly udeps --workspace

update-modern-normalize:
  curl \
    https://raw.githubusercontent.com/sindresorhus/modern-normalize/main/modern-normalize.css \
    > static/modern-normalize.css

download-log unit='lord' host='alpha.ordinals.net':
  ssh root@{{host}} 'mkdir -p tmp && journalctl -u {{unit}} > tmp/{{unit}}.log'
  mkdir -p tmp/{{unit}}
  rsync --progress --compress root@{{host}}:tmp/{{unit}}.log tmp/{{unit}}.log

graph log:
  ./bin/graph $1

flamegraph dir=`git branch --show-current`:
  ./bin/flamegraph $1

serve-docs: build-docs
  python3 -m http.server --directory docs/build/html --bind 127.0.0.1 8080

open-docs:
  open http://127.0.0.1:8080

install-mdbook:
  cargo install mdbook@0.4.52
  cargo install mdbook-i18n-helpers@0.3.6
  cargo install mdbook-linkcheck@0.7.7

docs: build-docs

# Commitment ceremony. Invoke: `just ceremony` (regtest), `just ceremony signet`, `just ceremony mainnet`.
# Regtest: encode + dry-run timestamp + --full verify (no bitcoind).
# Signet/mainnet: encode + doctor probe + printed live steps. Fill docs/ACCEPTANCE-SIGNET.md.
# Optional: LORD_DATADIR=/path/to/signet just ceremony signet
ceremony CHAIN='regtest':
  #!/usr/bin/env bash
  set -euo pipefail
  chain="{{CHAIN}}"
  case "$chain" in
    regtest) lord_flags="--regtest" ;;
    signet) lord_flags="--signet" ;;
    mainnet) lord_flags="" ;;
    testnet3) lord_flags="--testnet" ;;
    testnet4) lord_flags="--testnet4" ;;
    *) echo "unsupported CHAIN=$chain (use regtest, signet, mainnet, testnet3, testnet4)" >&2; exit 1 ;;
  esac
  if [ -n "${LORD_DATADIR:-}" ]; then
    work="$LORD_DATADIR"
    mkdir -p "$work"
  else
    work="$(mktemp -d)"
    trap 'rm -rf "$work"' EXIT
  fi
  hello="$work/hello.txt"
  echo "hello signet acceptance $(date -Iseconds)" > "$hello"
  lord=(cargo run --quiet --bin lord --)
  echo "== Lord commitment ceremony (chain=$chain, datadir=$work) =="
  if ! command -v jq >/dev/null; then
    echo "jq not found; install jq for bao_root extraction" >&2
    exit 1
  fi
  encode_out="$("${lord[@]}" $lord_flags --datadir "$work" storage encode "$hello" --format c12)"
  echo "$encode_out"
  bao_root="$(echo "$encode_out" | jq -r '.bao_root')"
  if [ -z "$bao_root" ] || [ "$bao_root" = null ]; then
    echo "failed to parse bao_root from storage encode output" >&2
    exit 1
  fi
  echo "bao_root=$bao_root"
  if [ "$chain" = regtest ]; then
    "${lord[@]}" $lord_flags --datadir "$work" commit timestamp "$bao_root" --dry-run
    "${lord[@]}" $lord_flags --datadir "$work" commit verify "$bao_root" --full --digest-only
    echo "Dry-run ceremony complete. For live anchor: calendar serve, timestamp without --dry-run, mine, upgrade."
    exit 0
  fi
  echo ""
  echo "== Live ceremony (requires synced bitcoind + funded anchor wallet) =="
  if [ "$chain" = signet ]; then
    echo "bitcoind: signet=1 in bitcoin.conf; RPC port 38332; fund via signet faucet"
    echo "Acceptance checklist: docs/ACCEPTANCE-SIGNET.md"
  fi
  if "${lord[@]}" $lord_flags calendar doctor 2>/dev/null | jq -e . >/dev/null 2>&1; then
    echo "calendar doctor (global datadir, not ceremony temp dir):"
    "${lord[@]}" $lord_flags calendar doctor | jq '{bitcoind_reachable, calendar_reachable, wallet_balance_sats, pending_digests, last_anchor_txid, wallet_error, calendar_error}'
  else
    echo "calendar doctor skipped or failed — start bitcoind and calendar before live steps"
  fi
  echo ""
  echo "export LORD_DATADIR=$work   # optional: reuse this datadir for live steps"
  echo "cargo run --bin lord -- $lord_flags --datadir $work calendar serve &"
  echo "cargo run --bin lord -- $lord_flags --datadir $work commit timestamp $bao_root"
  echo "# wait for calendar anchor (signet: seconds; mainnet: up to ~1h)"
  echo "cargo run --bin lord -- $lord_flags --datadir $work calendar doctor"
  echo "cargo run --bin lord -- $lord_flags --datadir $work commit upgrade $bao_root"
  echo "cargo run --bin lord -- $lord_flags --datadir $work commit verify $bao_root --full"
  echo "cargo run --bin lord -- $lord_flags --datadir $work commit list"
  if [ "$chain" = signet ]; then
    echo ""
    echo "Record results in docs/ACCEPTANCE-SIGNET.md before mainnet rollout."
  fi

build-docs:
  #!/usr/bin/env bash
  mdbook build docs -d build
  for language in ar de es fil fr hi it ja ko pt ru zh nl; do
    MDBOOK_BOOK__LANGUAGE=$language mdbook build docs -d build/$language
    mv docs/build/$language/html docs/build/html/$language
  done

update-changelog:
  echo >> CHANGELOG.md
  git log --pretty='format:- %s' >> CHANGELOG.md

convert-logo-to-favicon:
  convert -background none -resize 256x256 logo.svg static/favicon.png

update-mdbook-theme:
  curl \
    https://raw.githubusercontent.com/rust-lang/mdBook/v0.4.35/src/theme/index.hbs \
    > docs/theme/index.hbs

audit-cache:
  cargo run --package audit-cache

audit-content-security-policy:
  cargo run --package audit-content-security-policy

coverage:
  cargo llvm-cov

benchmark-server:
  cargo bench --bench server

update-contributors:
  cargo run --release --package update-contributors

replicate:
  rsync --archive bin/replicate root@charlie.ordinals.net:replicate
  ssh root@charlie.ordinals.net ./replicate

swap host:
  rsync --archive bin/swap root@{{ host }}.ordinals.net:swap
  ssh root@{{ host }}.ordinals.net ./swap

changed-files tag:
  git diff --name-only {{tag}}

env:
  cargo run env

env-open:
  open http://127.0.0.1:9001
