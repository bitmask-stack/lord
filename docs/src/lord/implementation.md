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
  ots/                     # detached OpenTimestamps proofs ({bao_root_hex}.ots)
  breccia/                 # append-only global commitment log
    global.breccia
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
    ots/
    breccia/
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
| **Lord storage schema** | heed3 `STATISTIC_TO_COUNT`, key `0` in `storage/` | Commitment metadata layout version (currently **3**) |
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
lord filepack create <dir> [--format 12|c12] [--layout inboard|outboard] [--master-key-hex HEX] [--filepack-compat]
lord-pack create <dir> [--format 12|c12] [--layout inboard|outboard] [--data-dir DIR] [--master-key-hex HEX] [--filepack-compat]
```

`--format` accepts either numeric (`12`) or prefixed (`c12`) Carbonado format
numbers (c0..c15).

### On-disk layout

| Layer | Path | Backend |
|-------|------|---------|
| Blobs | `{data_dir}/carbonado/{bao_root_hex}.c{NN}` | Carbonado v2 files |
| Filepack | `{data_dir}/filepack/{fingerprint}/` | `manifest.filepack` (JSON legacy or Casey CBOR) + `lord.carbonado.cbor` sidecar in compat mode |
| Commitment metadata | `{data_dir}/storage/` | heed3 LMDB env via `StorageStore` |

Mainnet uses `{data_dir}/carbonado` etc. at the data-dir root; other chains use
`{data_dir}/{chain}/carbonado` (same pattern as index/wallet).

### `storage/` LMDB environment (PR2)

`{data_dir}/storage/` is a **separate heed3 LMDB environment** from the cardinal
index and wallet stores. It holds:

| Database | Key | Value | Notes |
|----------|-----|-------|-------|
| `STATISTIC_TO_COUNT` | `u64` BE | `u64` BE | Storage schema version at key `0` (currently **3**) |
| `COMMITMENT_META` | `bao_root` (32 bytes) | `CommitmentMeta` (rkyv) | Metadata pointers only |
| `COMMITMENT_ORDER` | `ots_order_key` (bytes) | `bao_root` (32 bytes) | Sorted explorer/CLI list index |

`lord-db::LordEnv` also stores its own `metadata.schema_version` gate for the
environment wrapper; do not conflate that with the storage statistic at key `0`.

HTTP commitment explorer routes (`/commitment/{bao_root}`, `/commitments`) read
from this same `storage/` env, `COMMITMENT_ORDER`, and `carbonado/` blobs. No
separate `commitments/` database is planned; `COMMITMENT_META` remains the
canonical metadata table.

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
| `ots_proof_path` | `Option<String>` | Relative path under `{data_dir}/ots/` (PR3) |
| `ots_order_key` | `Option<Vec<u8>>` | BFS OTS merkle order key (PR3) |
| `timestamped_at` | `Option<u64>` | Unix timestamp when OTS proof was written (PR3 schema v3) |

`filepack_fp` records **at most one** filepack membership per commitment. If a
file already belongs to one filepack and is included in another, the field is
overwritten with the latest fingerprint (last filepack create wins).

### Filepack manifests (two-layer model)

Lord composes two layers:

1. **Carbonado layer** — blobs under `{data_dir}/carbonado/`, `CommitmentMeta` in LMDB,
   `filepack_fp` updated on create.
2. **Casey filepack layer** — standard CBOR `manifest.filepack` when `--filepack-compat`
   is set (or via `lord-pack create --filepack-compat`).

#### Legacy JSON manifest (default, version 1)

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

#### Casey-compatible CBOR manifest (`--filepack-compat`, version 2)

Written to `{fingerprint}/manifest.filepack` as **stock** Casey CBOR (not JSON).
The archive has the same top-level CBOR fields as upstream filepack (`version`,
`root`, `files`). Casey’s logical `embedded` map is derived at unpack time from
package-tree file entries whose payloads are inlined in `files`; Lord does not add
extra blobs to `files`, so upstream `filepack verify` works on the source directory.

- **`package` tree** — raw BLAKE3 hashes and sizes of **source files** (verified
  by stock `filepack verify` against the original directory).
- **`signatures`** — empty signatures directory (Casey format requirement).
- **`lord.carbonado.cbor`** — separate file in the same `{fingerprint}/` directory
  (not inside the Casey archive). CBOR map:

  `path → { bao_root, format, carbonado_path }`

  binding each source path to its Carbonado commitment. Read by Lord tooling only.

- **`fingerprint`** — Casey `package1…` bech32m fingerprint (hash of the package
  directory entry). Stored in LMDB `filepack_fp` and used as the on-disk directory name.

The CLI still prints a JSON descriptor (`version: 2`, `package1…` fingerprint,
`entries` with per-file Bao roots) for scripting either format.

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
commits `filepack_fp` LMDB updates **before** on-disk publish (rolls back LMDB
on any publish failure).

Legacy JSON mode publishes `manifest.filepack` in one rename after LMDB commit.

Compat mode (`--filepack-compat`) uses a two-step publish after LMDB commit:

1. Rename `lord.carbonado.cbor` from its temp file (Carbonado bindings sidecar).
2. Rename `manifest.filepack` last — the manifest is the **ready** marker; consumers
   may assume that when `manifest.filepack` exists, the sidecar is already present.

On sidecar rename failure: LMDB rollback, sidecar temp removed, manifest temp
removed (manifest not published). On manifest rename failure: LMDB rollback,
published sidecar removed, manifest temp removed.

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

PR3 (complete): commitment, OTS ordering, breccia
------------------------------------------------

OpenTimestamps commitment ordering, breccia append log, and commitment explorer
routes live in `crates/lord-commit` and are wired into the `lord` binary.

### CLI

```text
lord commit timestamp <bao_root> [--dry-run] [--force] [--calendar-url URL]
lord commit verify <bao_root>
lord commit list
```

`lord commit timestamp` requires an existing `CommitmentMeta` from
`lord storage encode`. `--dry-run` builds a local stub OTS proof for tests
without contacting a calendar server. Re-timestamping an already-timestamped
commitment fails unless `--force` is passed; `--force` deletes the previous
`COMMITMENT_ORDER` key in the same LMDB transaction before inserting the new one.

**Atomic timestamp flow:** pending OTS write → LMDB txn (delete stale order key if
re-timestamping, `put_commitment`, `put_commitment_order`) → breccia append →
publish OTS (`atomic_write` to final path). If breccia or publish fails after LMDB
commit, LMDB changes are rolled back and the pending OTS file is removed without
touching any previously published proof. If LMDB already records a timestamp but
breccia is missing the entry, retry appends breccia without `--force`.

`lord commit verify` performs a **binding check** only: the proof must parse as
SHA256 and its `start_digest` must equal `SHA256(bao_root)`. Full Bitcoin
attestation verification is deferred.

`lord commit list` and `/commitments` list **timestamped commitments only**
(entries in `COMMITMENT_ORDER`). Encoded-but-not-timestamped roots appear on
`/commitment/{bao_root}` but not in the sorted list.

### On-disk layout (additions)

| Layer | Path | Backend |
|-------|------|---------|
| OTS proofs | `{data_dir}/ots/{bao_root_hex}.ots` | Detached OTS proof files (not in LMDB) |
| Breccia log | `{data_dir}/breccia/global.breccia` | Append-only `LORBRECC` length-prefixed bincode blobs |

**Breccia format (PR3):** `global.breccia` uses a lord-specific append log with
`LORBRECC` magic and length-prefixed bincode `CommitmentEntry` records. This is
**not** [Peter Todd's breccia](https://github.com/petertodd/python-breccia)
mark-word database; a future phase may migrate to or interoperate with that
format for global replication.

### `storage/` schema version 3

Migrations chain in place on open:

1. **v1 → v2:** each `CommitmentMetaV1` record gains `ots_proof_path` and
   `ots_order_key` set to `None`; statistic bumped to **2**.
2. **v2 → v3:** each `CommitmentMetaV2` record gains `timestamped_at`; for
   already-timestamped rows (`ots_order_key` present) the value is taken from the
   matching breccia entry when available, otherwise `created_at`; statistic bumped
   to **3**.

Schema **0** or versions newer than **3** are rejected.

`COMMITMENT_ORDER` maps `ots_order_key` bytes → `bao_root` for sorted
`lord commit list` and `/commitments` explorer pages.

### OTS order key (BFS left-to-right, PR3 minimal)

PR3 implements the **BFS index** half of the design-doc ordering rule. The order
key is the big-endian `u64` breadth-first index of the first `Attestation` leaf
when the OTS proof tree is traversed left-to-right, breadth-first. Proofs with
no attestation leaf use `u64::MAX` as a sentinel (sorts last). The full
**merkle path** (per-fork left/right branch indices) described in
`design.md` is deferred to a later phase; PR3 keys are comparable and
deterministic for stub and calendar proofs but do not encode branch paths.

1. Model each `Fork` as an ordered list of child subtrees (left-to-right).
2. Breadth-first traverse: root, then level 1 left-to-right, then level 2, …
3. Find the first `Attestation` leaf; its BFS index becomes the order key.
4. Compare commitments by lexicographic order on those bytes.

`COMMITMENT_ORDER` rejects duplicate order keys mapping to different `bao_root`
values (collision detection).

Calendar HTTP uses 10s connect and 30s request timeouts. Breccia read/append
caps each blob at `MAX_BRECCIA_BLOB_LEN` (1 MiB).

Stub proofs built by `--dry-run` use a deterministic fork layout keyed off the
`bao_root` so integration tests get stable ordering without calendar network
access.

### `CommitmentEntry` (breccia blob)

Bincode-serialized struct appended to `global.breccia`:

| Field | Type |
|-------|------|
| `bao_root` | `[u8; 32]` |
| `ots_order_key` | `Vec<u8>` |
| `timestamped_at` | `u64` |
| `carbonado_path` | `String` |

### Explorer routes

| Route | Behavior |
|-------|----------|
| `/commitment/{bao_root}` | HTML commitment detail (metadata, OTS status, links) |
| `/commitments`, `/commitments/{page}` | Paginated list ordered by `ots_order_key` |
| `/content/{bao_root}` | Serve raw carbonado file bytes for public formats (403 for private; 32 MiB max; PR3 minimal — Bao streaming decode deferred) |
| `/r/commitment/{bao_root}`, `/r/commitments`, `/r/commitments/{page}` | JSON API when enabled |

`fallback` / `search`: 64-hex `bao_root` queries that match a stored commitment
redirect to `/commitment/{bao_root}` (block/tx routing unchanged). Inscription
and rune queries remain **410 Gone**.

### Exit criteria verified

```bash
cargo build --release
cargo test -p lord-commit --lib
cargo test -p lord-storage --lib
cargo test -p lord --lib
cargo test --test integration commit
cargo test --tests
cargo fmt --check
cargo clippy -p lord -p lord-db -p lord-storage -p lord-commit -- -D warnings
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

`lord-db`, `lord-storage`, and `lord-commit` are wired as root dependencies.
`carbonado` is a path dependency via `lord-storage` (`../carbonado` sibling crate).
`opentimestamps` is pulled in via `lord-commit` for proof parse/serialize.

### Release tooling

`install.sh` and `justfile` `publish-release` / `publish-tag-and-crate` recipes target **bitmask-stack/lord**, not ordinals/ord. Operational `bin/*` scripts use `/var/lib/lord` paths and the `lord` systemd unit.