<h1 align=center><code>lord</code></h1>

<p align=center>
  <strong>Bitcoin wallet, block explorer, and content-addressed storage — forked from <a href="https://github.com/ordinals/ord">ord</a>, without inscriptions or runes.</strong>
</p>

<p align=center>
  <a href="https://github.com/bitmask-stack/lord/actions"><img src="https://img.shields.io/github/actions/workflow/status/bitmask-stack/lord/ci.yaml?branch=lord" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-CC0--1.0-blue" alt="License"></a>
</p>

---

## Monorepo layout

Lord is developed alongside sibling crates under `surmount/`:

```
surmount/
├── lord/          # this repository (CLI, server, lord-storage, lord-commit, …)
├── carbonado/     # Carbonado v2 encode/decode (path dep from lord-storage)
└── bao-tree/      # Bao stream verification (path dep from lord-storage)
```

Clone all three before building. `just deps-check` **fails fast** (exit 1) when either sibling is missing — it does not warn and continue.

`lord-storage` depends on `bao-tree` via a path dependency. Carbonado also pulls `bao-tree` from git; [`.cargo/config.toml`](.cargo/config.toml) patches that git URL to the same sibling checkout so the workspace resolves one `bao-tree` revision:

```toml
[patch."https://github.com/SurmountSystems/bao-tree.git"]
bao-tree = { path = "../bao-tree" }
```

---

## What Lord is

**Lord** keeps what operators rely on from ord — CLI shape, HTTP explorer API, Bitcoin Core wallet integration, cardinal indexing — and **removes inscription and rune functionality entirely**.

In their place, Lord is building a **content-addressed commitment stack**: durable encoded blobs ([Carbonado v2](https://github.com/bitmask-stack/carbonado)), proofs of possession ([Bao](https://github.com/oconnor663/bao)), directory manifests ([filepack](https://github.com/casey/filepack)), OpenTimestamps ordering, and a global append log (breccia). Later phases add a **storage market**, **mutual-aid replication**, an embedded **LDK Lightning** node, and **RGB** tokens (replacing runes).

| ord concept | Lord replacement |
|-------------|------------------|
| On-chain inscriptions | Carbonado blobs + OTS commitments + breccia index |
| Runes | RGB (planned) |
| Sat ordinal ordering | OTS merkle-path + breadth-first commitment order |

**[Design document](docs/src/lord/design.md)** — full specification and roadmap.  
**[Implementation notes](docs/src/lord/implementation.md)** — what is shipped today, schemas, and CLI details.  
**[CHIP LTP-0001 (draft)](docs/src/lord/chip-ltp-0001.md)** — Lord Transport Protocol (Iroh, storage market, payments annex).

---

## Key decisions (read this first)

These are intentional — not oversights:

1. **No ordinal theory for data ordering.** Canonical commitment order comes from the **OpenTimestamps proof merkle path** and **breadth-first, left-to-right** position in the OTS tree — not sat numbering or rarity.

2. **Clean break from redb.** Index, wallet, and storage metadata use **heed3 LMDB + rkyv**. Legacy `index.redb` and `wallets/<name>.redb` are **not migrated**; delete them and re-index or recreate wallets.

3. **Blobs on disk, pointers in LMDB.** Carbonado files live under `carbonado/`; `storage/` holds **metadata only** (bao root, paths, OTS keys). No blob bytes in the database.

4. **Master keys are random files, not passphrases.** Encrypted (odd) Carbonado formats use `{data_dir}/storage/master.key` (32 random bytes, mode `0600`). **Argon2id is not built into Lord or Carbonado** — derive a 32-byte key outside the library if you only have a passphrase.

5. **Ord HTTP compatibility for removed features.** Inscription and rune routes return **410 Gone** (explicit removal), not silent 404. Cardinal explorer routes are unchanged.

6. **Two-layer filepack.** Default manifests are Lord JSON v1 with per-file Bao roots. `--filepack-compat` adds a **stock Casey CBOR** `manifest.filepack` (raw BLAKE3 of source files, verifiable with `filepack verify`) plus a separate **`lord.carbonado.cbor`** sidecar mapping paths to Carbonado addresses.

7. **Lord breccia ≠ Peter Todd breccia.** The global log uses Lord's `LORBRECC` append format at `breccia/global.breccia`, not the upstream breccia library.

8. **Carbonado v2 only.** Symmetric AES-256-CTR + HMAC-SHA512 EtM. **No decode path for legacy v1 ECIES** archives — re-encode externally if needed.

---

## Status

| Phase | Scope | Status |
|-------|-------|--------|
| PR0 | `lord-db`, ord→lord rename | Done |
| PR1 | Remove inscriptions/runes; index+wallet heed3; slim server; `sats` feature | Done |
| PR2 | Carbonado encode/verify, filepack, `storage/` metadata | Done |
| PR3 | OTS timestamp/verify/list, breccia log, commitment explorer | Done |
| Operator stack | Embedded Rust calendar, `commit upgrade`, Bitcoin attestation verify, txindex fallback, operator docs | Shipped |
| PR4 | LDK Lightning | Planned |
| PR5 | Iroh P2P / storage market | Planned |
| PR6 | RGB stub | Planned |

**Shipped today:** wallet, cardinal explorer (with txindex fallback), Carbonado storage CLI, filepack create/verify, OTS commitment workflow (`timestamp` / `upgrade` / `verify` with Bitcoin attestation + confirmations), embedded chain-aware OpenTimestamps calendar (`lord calendar *`, `calendar_enabled` in `lord server`), `/commitment/*` and `/content/*` routes (Bao decode).

**Not shipped:** storage market, Lightning, RGB, multi-calendar federation, breccia replication.

---

## Chains (mainnet first)

| Chain | Lord flag | Data directory | Notes |
|-------|-----------|----------------|-------|
| **Mainnet** | _(default)_ | `{data_dir}/` | Production; embedded calendar optional |
| Signet | `--signet` | `{data_dir}/signet/` | Public test network |
| Testnet3 | `--testnet` | `{data_dir}/testnet3/` | Legacy public testnet |
| Testnet4 | `--testnet4` | `{data_dir}/testnet4/` | Current public testnet |
| Regtest | `--regtest` | `{data_dir}/regtest/` | Local dev; dry-run OTS without calendar |

---

## Quick start

### Build

```sh
# Monorepo checkout (recommended)
git clone https://github.com/bitmask-stack/lord.git surmount/lord
git clone https://github.com/bitmask-stack/carbonado.git surmount/carbonado
git clone https://github.com/bitmask-stack/bao-tree.git surmount/bao-tree
cd surmount/lord
just deps-check
cargo build --release
# binaries: ./target/release/lord  ./target/release/lord-pack
```

Requires Rust ≥ 1.89 (`rust-version` in `Cargo.toml`). On Debian/Ubuntu: `sudo apt-get install pkg-config libssl-dev build-essential`.

Optional sat explorer: `cargo build --release --features sats` and run with `--index-sats`.

### Run

Lord needs a synced `bitcoind` for wallet and explorer modes. Same RPC/cookie conventions as ord — see `lord --help`.

**`txindex` is not required for storage or OTS workflows** (`lord-pack`, `storage`, `commit`). For the block explorer and address/sat indexing, bitcoind's transaction index matters:

| Profile | bitcoind | `txindex=1` | lord server |
|---------|----------|-------------|-------------|
| Storage / commitments only | Optional | No | No |
| Wallet + blocks (reduced) | Yes | No | Yes — blocks and unspent outputs work; `/tx` returns 503 with guidance |
| Full explorer / `--index-addresses` / `--index-sats` | Yes | **Yes** | Yes |

Lord probes `getindexinfo` at index startup, logs a warning when txindex is missing, and continues in reduced mode. `/status` reports `txindex` and `txindex_available`. Enabling `--index-addresses` or `--index-sats` without a synced txindex fails fast with an actionable error.

```sh
lord server                                    # HTTP explorer
lord wallet create                             # wallet (Bitcoin Core-backed)
lord --regtest server                          # regtest chain
```

### Storage & commitments

```sh
# Encode a file to Carbonado (default public format c12)
lord storage encode myfile.txt --format c12

# Verify possession (header MAC + sampled Bao slices)
lord storage verify <bao_root_hex> --sample-rate 8

# Directory → per-file Carbonado + manifest
lord filepack create ./bundle --format c12
lord filepack verify <fingerprint>

# Casey-compatible CBOR manifest + carbonado sidecar
lord filepack create ./bundle --format c12 --filepack-compat

# Standalone helper (no full lord wallet/index required)
lord-pack create ./bundle --format c12 --chain regtest --data-dir ~/.local/share/ord/regtest --filepack-compat

# OpenTimestamps (embedded calendar: set calendar_enabled: true in lord.yaml)
lord calendar url
lord calendar doctor
lord commit timestamp <bao_root_hex> [--dry-run]
lord commit upgrade <bao_root_hex>    # after calendar anchors
lord commit verify <bao_root_hex> [--digest-only] [--full]
lord commit list
```

`--format` accepts `12` or `c12` (c0..c15). Odd formats encrypt with `master.key`; even formats are public.

Explorer: `/commitment/{bao_root}`, `/commitments`, `/content/{bao_root}` (+ JSON under `/r/` when enabled).

---

## Data directory

Mainnet uses `{data_dir}/` at the root; other chains nest under `{data_dir}/{chain}/`.

| Path | Purpose |
|------|---------|
| `index/` | Cardinal block index (heed3, schema **35**) |
| `wallets/<name>/` | Per-wallet LMDB (schema **2**) |
| `carbonado/` | Carbonado blobs `{bao_root_hex}.c{NN}` |
| `filepack/{fingerprint}/` | `manifest.filepack` + optional `lord.carbonado.cbor` |
| `storage/` | Commitment metadata LMDB (schema **4**), `master.key` |
| `ots/` | Detached `.ots` proofs |
| `breccia/global.breccia` | Global commitment append log |
| `{chain}/calendar/` | Embedded OTS calendar state (`state.json`, active `uri`) |

Example config: [`lord.yaml`](lord.yaml) (preferred; `ord.yaml` still supported). Operator guides: [commitments](docs/src/guides/commitments.md), [operator](docs/src/guides/operator.md). Signet acceptance: [`docs/ACCEPTANCE-SIGNET.md`](docs/ACCEPTANCE-SIGNET.md). Release: [`docs/RELEASE.md`](docs/RELEASE.md).

---

## Scope today

**Included:** wallet commands, block/tx/output/address explorer, search, status, Carbonado storage, filepack, OTS commitments (embedded calendar), commitment explorer pages with attestation status.

**Removed:** inscriptions, runes, ordinal collection features, inscription/rune HTTP routes (410 Gone).

**Optional:** `--features sats` + `--index-sats` for `/sat/:sat` and sat-aware indexing.

---

## Fork relationship to ord

Lord is maintained by [bitmask-stack](https://github.com/bitmask-stack/lord) as a **directional fork**, not a drop-in ord release:

- CLI and HTTP surface are preserved where they still apply.
- Inscription/rune codepaths are deleted, not feature-flagged.
- Persistence moved from redb to heed3+rkyv (no automatic migration).
- New subcommands: `storage`, `filepack`, `commit`; new crates `lord-storage`, `lord-commit`, `lord-db`, `lord-pack`.

Upstream ord docs, install scripts, and community channels refer to **ordinals.com / ord** — use those for ordinal-specific tooling. For Lord issues and design, use this repository.

---

## Security

`lord server` hosts untrusted HTML/JavaScript like ord. See [security notes](docs/src/security.md). Segregate wallets: Lord loads Bitcoin Core wallets for its commands; do not point it at high-value cardinal wallets you also manage manually with `bitcoin-cli`.

---

## Contributing

Tests are required for new logic. With [just](https://github.com/casey/just) installed:

```sh
just deps-check   # verify carbonado + bao-tree siblings
just smoke        # fast integration smoke (calendar + commit dry-run)
just ci-local     # deps-check → smoke → clippy → forbid → fmt → test-all
just ci           # fmt, clippy, full test suite (includes smoke)
just ceremony                    # regtest encode + dry-run timestamp + --full verify
just ceremony signet             # signet encode + live-step instructions (see ACCEPTANCE-SIGNET.md)
```

Integration tests use the in-repo [`mockcore`](crates/mockcore) Bitcoin Core mock. See the [justfile](justfile) for more recipes.

---

## License

CC0-1.0 — see [LICENSE](LICENSE). Experimental software; no warranty.