<h1 align=center><code>lord</code></h1>

<p align=center>
  <strong>Bitcoin wallet, block explorer, and content-addressed storage — forked from <a href="https://github.com/ordinals/ord">ord</a>, without inscriptions or runes.</strong>
</p>

<p align=center>
  <a href="https://github.com/bitmask-stack/lord/actions"><img src="https://img.shields.io/github/actions/workflow/status/bitmask-stack/lord/ci.yaml?branch=lord" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-CC0--1.0-blue" alt="License"></a>
</p>

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
| PR4 | LDK Lightning | Planned |
| PR5 | Iroh P2P / storage market | Planned |
| PR6 | RGB stub | Planned |

**Shipped today:** wallet, cardinal explorer, Carbonado storage CLI, filepack create, OTS commitment workflow, `/commitment/*` and `/content/*` routes.

**Not shipped:** storage market, Lightning, RGB, full OTS Bitcoin attestation verify (PR3 checks digest binding), Bao streaming decode on `/content` (serves raw carbonado bytes).

---

## Quick start

### Build

```sh
git clone https://github.com/bitmask-stack/lord.git
cd lord
cargo build --release
# binaries: ./target/release/lord  ./target/release/lord-pack
```

Requires Rust ≥ 1.89 (`rust-version` in `Cargo.toml`). On Debian/Ubuntu: `sudo apt-get install pkg-config libssl-dev build-essential`.

Optional sat explorer: `cargo build --release --features sats` and run with `--index-sats`.

### Run

Lord needs a synced `bitcoind` with `-txindex`. Same RPC/cookie conventions as ord — see `lord --help`.

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

# Casey-compatible CBOR manifest + carbonado sidecar
lord filepack create ./bundle --format c12 --filepack-compat

# Standalone helper (no full lord wallet/index required)
lord-pack create ./bundle --format c12 --chain regtest --data-dir ~/.local/share/ord/regtest --filepack-compat

# OpenTimestamps
lord commit timestamp <bao_root_hex>
lord commit verify <bao_root_hex>
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
| `storage/` | Commitment metadata LMDB (schema **3**), `master.key` |
| `ots/` | Detached `.ots` proofs |
| `breccia/global.breccia` | Global commitment append log |

Example config: [`lord.yaml`](lord.yaml).

---

## Scope today

**Included:** wallet commands, block/tx/output/address explorer, search, status, Carbonado storage, filepack, OTS commitments, commitment explorer pages.

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
just ci    # fmt, clippy, full test suite
```

Integration tests use the in-repo [`mockcore`](crates/mockcore) Bitcoin Core mock. See the [justfile](justfile) for more recipes.

---

## License

CC0-1.0 — see [LICENSE](LICENSE). Experimental software; no warranty.