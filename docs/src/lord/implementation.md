Lord Implementation Notes
=========================

Data directory layout
---------------------

Lord stores the cardinal block index as a heed3 LMDB environment per chain:

```
{data_dir}/
  index/                   # mainnet only: heed3 LMDB env root (cardinal index)
  wallets/                 # mainnet only: per-wallet heed3 LMDB env roots
    <name>/
      data.mdb
      lock.mdb
  carbonado/               # Carbonado blob files ({bao_root_hex}.c{NN})
  filepack/                # filepack manifest directories
    {fingerprint}/
      manifest.filepack
  storage/                 # commitment metadata LMDB env (no blob bytes)
    data.mdb
    lock.mdb
    master.key             # 32-byte symmetric key (odd formats; mode 0600)
  {chain}/                 # signet, regtest, testnet3, testnet4
    index/                 # heed3 LMDB env root (cardinal index)
      data.mdb
      lock.mdb
      savepoints/          # optional reorg snapshots (production only)
        {height}/
          data.mdb
          lock.mdb
    wallets/               # per-wallet heed3 LMDB env roots
      <name>/
        data.mdb
        lock.mdb
    carbonado/
    filepack/
    storage/
```

The default index path is `{data_dir}/index` on mainnet and
`{data_dir}/{chain}/index` on other chains (a directory, not a file).

### Clean-break policy

There is **no migration** from legacy redb databases:

- **Index:** if `index.redb` is present in the chain data directory, `Index::open` fails with an actionable error instructing the operator to delete `index.redb` and re-index.
- **Wallet:** if `wallets/<name>.redb` is present, `WalletStore::open` fails with an actionable error instructing the operator to delete the `.redb` file and recreate the wallet with `lord wallet create`.

`lord-db` crate (`crates/lord-db`)
----------------------------------

- **`RkyvCodec<T>`** — heed3 `BytesEncode` / `BytesDecode` for zero-copy `rkyv` reads
- **`LordEnv`** — opens the LMDB environment, stores `schema_version` in a `metadata` named database, and creates versioned named databases on demand
- **`LordEnvOptions`** — configurable `map_size` and `max_dbs` for mainnet-scale deployments
- **`DupTable`** — `MDB_DUPSORT` multimap wrapper for `SCRIPT_PUBKEY_TO_OUTPOINT`

Schema version namespaces
-------------------------

Lord uses **two independent schema version namespaces**:

| Namespace | Location | Purpose |
|-----------|----------|---------|
| **Lord index schema** | heed3 `STATISTIC_TO_COUNT`, key `0` | Cardinal index layout version (currently **35**) |
| **Lord wallet schema** | heed3 `STATISTIC_TO_COUNT`, key `0` in `wallets/<name>/` | Wallet metadata layout version (currently **2**) |
| **Lord storage schema** | heed3 `STATISTIC_TO_COUNT`, key `0` in `storage/` | Commitment metadata layout version (currently **1**) |
| **lord-db `SCHEMA_VERSION`** | heed3 `metadata` database, key `schema_version` | Gates `LordEnv` named-database layouts and rkyv record migrations |

These must not be conflated. Bumping `lord-db::SCHEMA_VERSION` rejects incompatible LMDB environments with an actionable error; it does **not** validate or migrate the cardinal index statistic at key `0`.

Index tables (schema 35)
------------------------

| Database | Key type | Value type | Notes |
|----------|----------|------------|-------|
| `HEIGHT_TO_BLOCK_HEADER` | `u32` BE | `HeaderRecord` (rkyv) | Block headers by height |
| `OUTPOINT_TO_UTXO_ENTRY` | 36-byte outpoint | `ByteVecRecord` (rkyv) | UTXO entries |
| `SCRIPT_PUBKEY_TO_OUTPOINT` | script pubkey bytes | 36-byte outpoint | Duplicate-key multimap |
| `SAT_TO_SATPOINT` | `u64` BE | `SatPointRecord` (rkyv) | Written only when `index_sats` is enabled |
| `STATISTIC_TO_COUNT` | `u64` BE | `u64` BE | Counters including schema version at key `0` |
| `WRITE_TRANSACTION_STARTING_BLOCK_COUNT_TO_TIMESTAMP` | `u32` BE | `u128` BE | Index update transaction windows |

**Removed in PR1b:** all inscription/rune redb tables, `TRANSACTION_ID_TO_TRANSACTION`, and `src/index/legacy/` shims. The explorer fetches raw transactions via bitcoind RPC when needed.

### heed3 transaction rules

- `Database` handles embed the environment pointer active at open time; **do not cache handles across env close/reopen**. `IndexDatabases` is opened per LMDB transaction.
- LMDB forbids a read transaction on a thread that already holds a write transaction. Reorg detection reads through the active `RwTxn` (via `Deref` to `RoTxn`), not a separate `begin_read()`.

rkyv schema intent
------------------

Indexed records are stored as rkyv-archived structs in heed3 named databases. Hot read paths deserialize via `rkyv::api::high::access` returning `&Archived<T>` without copying.

`RkyvCodec` decoded references are tied to the input byte slice lifetime and therefore to the enclosing heed3 read transaction. Do not use archived references after the transaction commits or drops.

PR1a (complete): inscription/rune surface removal
-------------------------------------------------

User-facing inscription and rune functionality has been removed from the `lord` binary. Cardinal wallet and block explorer core remain.

**Removed**

- CLI: `balances`, `decode`, `runes`, `teleburn`; wallet `batch`, `burn`, `inscribe`, `inscriptions`, `mint`, `offer`, `pending`, `resume`, `runics`, `split`
- Server routes: `/inscription*`, `/inscriptions*`, `/rune*`, `/runes*`, `/collections*`, `/galleries*`, `/preview*`, `/children*`, `/parents*`, `/item*`, `/decode*`, `/content*`, `/metadata*`, `/feed.xml`, recursive inscription endpoints
- `crates/ordinals` rune/runestone modules (sat types + `varint` kept)
- HTML templates and Rust template modules for inscription/rune UI
- Integration tests for inscriptions, runes, batch, offers, etc.

**Preserved**

- Explorer: `/`, `/blocks`, `/block/:id`, `/tx/:txid`, `/output/:outpoint`, `/address/:address`, `/status`, `/clock`, `/search`, `/r/*` (blockhash, blockinfo, tx, utxo)
- Sat explorer routes (`/sat/:sat`, `/ordinal/:sat`, `/rare.txt`, `/satpoint/:satpoint`, `/r/sat/*`) require building lord with `--features sats` (PR1d); default builds return **410 Gone**
- Wallet: `create`, `dump`, `receive`, `restore`, `sign`, `transactions`, `addresses`, `send` (BTC), `balance`, `cardinals`, `outputs`, `sweep`, `label`; `wallet sats` requires `--features sats` (PR1d)
- `sats` feature declaration (optional; `index_sats` settings flag behavior preserved)

PR1b (complete): index migration redb → heed3+rkyv
--------------------------------------------------

The cardinal block index now uses heed3 LMDB at `{data_dir}/{chain}/index/` via `crates/lord-db`.

**Changed**

- `src/index/store.rs` — `IndexStore` wrapping `LordEnv`; per-txn `IndexDatabases`; savepoint copy/restore
- `src/index/records.rs` — rkyv record types (`HeaderRecord`, `ByteVecRecord`, `SatPointRecord`, …)
- `src/index/updater.rs` — cardinal UTXO/sats/address indexing only; completion timestamps per update
- `src/index.rs` — redb removed; inscription/rune methods deleted
- `src/settings.rs` — default index path `data_dir.join("index")`
- `lord.yaml`, `ord.yaml`, `bin/replicate`, `bin/swap`, `justfile` — index directory paths
- Explorer/server — raw transaction lookup via bitcoind RPC (no `TRANSACTION_ID_TO_TRANSACTION`)

PR2 (complete): Lord storage core
---------------------------------

Carbonado blob storage, filepack manifests, and commitment metadata live in
`crates/lord-storage` and are wired into the `lord` binary.

### CLI

```text
lord storage encode <path> [--format 12|c12] [--layout inboard|outboard] [--master-key-hex HEX]
lord storage verify <bao_root> [--sample-rate N] [--master-key-hex HEX]
lord filepack create <dir> [--format 12|c12] [--layout inboard|outboard] [--master-key-hex HEX]
```

`--format` accepts either numeric (`12`) or prefixed (`c12`) Carbonado format
numbers (c0..c15).

### On-disk layout

| Layer | Path | Backend |
|-------|------|---------|
| Blobs | `{data_dir}/carbonado/{bao_root_hex}.c{NN}` | Carbonado v2 files |
| Filepack | `{data_dir}/filepack/{fingerprint}/manifest.filepack` | JSON manifest |
| Commitment metadata | `{data_dir}/storage/` | heed3 LMDB env via `StorageStore` |

Mainnet uses `{data_dir}/carbonado` etc. at the data-dir root; other chains use
`{data_dir}/{chain}/carbonado` (same pattern as index/wallet).

### `storage/` LMDB environment (PR2)

`{data_dir}/storage/` is a **separate heed3 LMDB environment** from the cardinal
index and wallet stores. It holds:

| Database | Key | Value | Notes |
|----------|-----|-------|-------|
| `STATISTIC_TO_COUNT` | `u64` BE | `u64` BE | Storage schema version at key `0` (currently **1**) |
| `COMMITMENT_META` | `bao_root` (32 bytes) | `CommitmentMeta` (rkyv) | Metadata pointers only |

`lord-db::LordEnv` also stores its own `metadata.schema_version` gate for the
environment wrapper; do not conflate that with the storage statistic at key `0`.

**PR3 reconciliation:** HTTP commitment explorer routes (`/commitment/{bao_root}`)
will read from this same `storage/` env and `carbonado/` blobs. No separate
`commitments/` database is planned; `COMMITMENT_META` remains the canonical
metadata table.

**No blob bytes in LMDB** — only `CommitmentMeta` rkyv records in
`COMMITMENT_META` keyed by `bao_root` bytes. Carbonado files are written
atomically (temp + rename after LMDB commit).

### `CommitmentMeta` (rkyv)

| Field | Type | Notes |
|-------|------|-------|
| `bao_root` | `[u8; 32]` | Primary key |
| `carbonado_path` | `String` | Relative path under `carbonado/` |
| `format` | `u8` | Carbonado c-format number (c0..c15) |
| `visibility` | `Public` / `Private` | Odd format = private, even = public |
| `layout` | `Inboard` / `Outboard` | Outboard writes `{bao_root}.plain` for public formats |
| `filepack_fp` | `Option<String>` | Set by `lord filepack create` (single-valued; see below) |
| `created_at` | `u64` | Unix timestamp |

`filepack_fp` records **at most one** filepack membership per commitment. If a
file already belongs to one filepack and is included in another, the field is
overwritten with the latest fingerprint (last filepack create wins).

### Filepack manifest (version 1, JSON)

Written to `manifest.filepack`:

```json
{
  "version": 1,
  "fingerprint": "<blake3-hex>",
  "root": "<source-dir-basename>",
  "entries": [
    { "path": "relative/path", "size": 1234, "bao_root": "<hex>", "format": 12 }
  ]
}
```

`fingerprint` is a BLAKE3 hex digest over sorted entry `(path, size, bao_root, format)`
tuples. Each entry carries its own per-file Bao root from Carbonado encode.

### Master key (PR2 minimal)

- **Odd (encrypted) formats:** derive key from `{data_dir}/storage/master.key`
  (created on first encode if missing: 32 random bytes, mode `0600`), or pass
  `--master-key-hex` for testing.
- **Even (public) formats:** zero master key (Carbonado unencrypted path).
- Full HSM integration is out of scope for PR2.

### Verification (PR2)

`lord storage verify` performs:

1. **Path validation** — `carbonado_path` from LMDB must be a basename (no `..`
   or path separators).
2. **Header MAC authentication** using the format-appropriate master key (required
   for odd/private formats; public formats use the zero key). Verify **never
   creates** `master.key`; use `--master-key-hex` for testing overrides.
3. **Format cross-check** — header `format` bit must match stored `meta.format`.
4. **Keyed Bao verification + sampled partial proofs** — carbonado stores full
   inline pre-order Bao responses; per-slice keyed decode over partial chunk
   ranges is incompatible with that layout (ParentHashMismatch). PR2 runs one
   keyed `bao-tree` decode over the full response, then cross-checks each
   sampled 1 KB slice against carbonado's `verify_slice` partial proof API
   (sample loop cost scales with `--sample-rate`; keyed decode is O(content_len)
   for PR2).

Encode publishes carbonado blobs **before** LMDB metadata commit. Filepack
commits `filepack_fp` LMDB updates **before** manifest rename (rolls back LMDB
on rename failure).

### c-format parity

Carbonado format bitmask (`carbonado::constants::Format`): odd = encrypted/private,
even = public. Default encode format is **c12** / **12** (Bao + Zfec, public).

### Exit criteria verified

```bash
cargo build --release
cargo test -p lord-storage --lib
cargo test -p lord --lib
cargo test --test integration storage
cargo test -p lord-db --lib
cargo fmt --check
cargo clippy -p lord -p lord-db -p lord-storage -- -D warnings
just forbid
just ci
```

PR1c (complete): wallet migration redb → heed3+rkyv
---------------------------------------------------

Wallet metadata now uses heed3 LMDB at `{data_dir}/wallets/<name>/` via
`src/wallet/store.rs` (`WalletStore` wrapping `LordEnv`).

**Changed**

- `src/wallet/store.rs` — `WalletStore::open`, schema version **2**, legacy `.redb` rejection
- `src/wallet.rs` — `database: Database` replaced with `store: WalletStore`; redb removed
- `src/wallet/wallet_constructor.rs` — opens `WalletStore` instead of redb
- `Cargo.toml` — `redb` dependency removed
- `.gitignore` — `/*.redb` entry removed

**Clean-break policy**

If `wallets/<name>.redb` exists, `WalletStore::open` fails with an actionable
error. Delete the legacy file and run `lord wallet create` (or restore) to
create the heed3 wallet directory.

**Exit criteria verified**

```bash
cargo build --release
cargo test -p lord --lib
cargo test --tests
cargo test -p lord-db --lib
cargo fmt --check
cargo clippy -p lord -p lord-db -- -D warnings
rg 'redb' --glob '!CHANGELOG.md' --glob '!Cargo.lock' --glob '!docs/po/**'
# allowed hits only: legacy rejection messages, test names, migration docs
```

PR1d (complete): slim server, API compatibility, `sats` feature
----------------------------------------------------------------

### HTTP API compatibility (ord removed routes)

Lord deliberately removed inscription and rune functionality in PR1a. Ord
clients hitting former inscription/rune URLs must receive an explicit **410
Gone** (not a silent fallback 404):

| Pattern | Status | Notes |
|---------|--------|-------|
| `/inscription*`, `/inscriptions*` (GET and POST), `/preview*`, `/children*`, `/parents*`, `/item*`, `/decode*`, `/content*`, `/metadata*`, `/feed.xml` | **410 Gone** | Inscriptions permanently removed |
| `/rune*`, `/runes*`, `/collections*`, `/galleries*`, `/gallery*`, `/offer`, `/offers*` | **410 Gone** | Runes/collections/offers permanently removed |
| `/r/inscription*`, `/r/children*`, `/r/parents*`, `/r/sat/*/at/*` | **410 Gone** | Recursive inscription/sat-at-index endpoints removed |
| `fallback` / `search` inscription-id, rune-id, spaced-rune queries | **410 Gone** | Was previously indistinguishable from missing resources |
| `fallback` / `search` inscription-number queries | **410 Gone** (default build) or redirect to `/sat/:n` (`--features sats`) | Numeric queries are sat numbers when sat explorer is enabled |
| Unknown paths, missing blocks/txs/outputs | **404 Not Found** | Cardinal explorer unchanged |
| `Accept: application/json` on removed routes | **410 Gone** + plain-text body | Same error layer as other JSON API errors |

Explicit route handlers are registered in `src/subcommand/server/removed.rs` so
these paths never hit the cardinal fallback redirect logic.

### `sats` Cargo feature (`crates/ordinals` + root `lord`)

| Build | Behavior |
|-------|----------|
| Default (`cargo build --release`) | Cardinal wallet/explorer only; sat theory types and CLI (`epochs`, `find`, `list`, `parse`, `traits`, `subsidy`, `supply`, `wallet sats`) are compile-time absent; `/sat/*` returns **410 Gone** with message to rebuild using `--features sats` |
| `--features sats` + `--index-sats` | Full sat indexing and explorer routes as in ord |

**`crates/ordinals`:** `varint` always available (index encoding). Sat-theory
modules (`sat`, `sat_point`, `charm`, `rarity`, `degree`, `decimal_sat`,
`epoch`; `height` sat-specific methods) are behind `#[cfg(feature = "sats")]`.
`height::Height` remains available for block-height indexing without the
feature.

**Root `Cargo.toml`:** `sats = ["ordinals/sats"]`.

### CI `bin/forbid`

Extended with targeted `rg` patterns (see `bin/forbid`):

- `dbg!|fixme|todo|xxx` in the repo (excluding `bin/forbid`, `docs/po/*`,
  `docs/src/bounty/frequency.tsv`, `docs/src/lord/design.md`)
- `redb::|use redb` in `src/` and `crates/` `*.rs` files
- `^redb\s*=` in workspace `Cargo.toml` files

`tests/no_redb.rs` also runs `bin/forbid` as a smoke test.

**Exit criteria verified**

```bash
cargo build --release
cargo build --release --features sats
cargo test -p lord --lib
cargo test --tests
cargo test --tests --features sats
cargo test -p lord-db --lib
cargo test -p ordinals --lib
cargo test -p ordinals --lib --features sats
cargo fmt --check
cargo clippy -p lord -p lord-db -p ordinals -- -D warnings
cargo clippy -p lord -p lord-db -p ordinals --features sats -- -D warnings
just forbid
```

Compatibility preserved from ord
--------------------------------

### Environment variables (`ORD_` prefix)

The CLI still reads configuration from environment variables prefixed with `ORD_` (e.g. `ORD_CHAIN`, `ORD_DATA_DIR`). This is **deliberately preserved** for backward compatibility with existing ord deployments, systemd units, and operator scripts. A `LORD_` prefix may be added later; `ORD_` remains supported.

### `sats` feature flag

The root `sats` Cargo feature gates ordinal-theory types (`ordinals/sats`) and
sat-specific CLI/server code. Runtime sat indexing still requires the
`index_sats` settings flag (`--index-sats`).

### Configuration filenames

- **`lord.yaml`** — preferred example config for new deployments (`/var/lib/lord` paths; see repo root `lord.yaml`).
- **`ord.yaml`** — retained for ord compatibility; `Settings` still probes `ord.yaml` in the data directory by default (`src/settings.rs`). Operators may symlink `lord.yaml` → `ord.yaml` or set `--config` explicitly until defaults are switched in a later phase.

### Root crate dependencies

`lord-db` and `lord-storage` are wired as root dependencies. `carbonado` is a path
dependency via `lord-storage` (`../carbonado` sibling crate).

### Release tooling

`install.sh` and `justfile` `publish-release` / `publish-tag-and-crate` recipes target **bitmask-stack/lord**, not ordinals/ord. Operational `bin/*` scripts use `/var/lib/lord` paths and the `lord` systemd unit.