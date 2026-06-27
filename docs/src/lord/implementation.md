Lord Implementation Notes
=========================

> **Phase 0 (PR0)** — foundation only. Index and wallet still use `redb` until Phase 1.

Data directory layout (preview)
-------------------------------

Lord inherits ord's per-chain data directory structure. Phase 1 adds a heed3 LMDB environment alongside the existing redb index:

```
{data_dir}/
  {chain}/                 # mainnet, signet, regtest, testnet3, testnet4
    index.redb             # current ord index (removed in Phase 1)
    lord-db/               # heed3 LMDB env root (Phase 1+)
      data.mdb
      lock.mdb
```

The `lord-db` crate (`crates/lord-db`) provides:

- **`RkyvCodec<T>`** — heed3 `BytesEncode` / `BytesDecode` for zero-copy `rkyv` reads
- **`LordEnv`** — opens the LMDB environment, stores `schema_version` in a `metadata` named database, and creates versioned named databases on demand
- **`LordEnvOptions`** — configurable `map_size` and `max_dbs` for mainnet-scale deployments

Schema version namespaces
-------------------------

Lord uses **two independent schema version namespaces** during the redb → heed3 migration:

| Namespace | Location | Purpose |
|-----------|----------|---------|
| **redb index schema** | `index.redb` internal tables | Current ord/lord block/inscription index layout (unchanged in Phase 0) |
| **lord-db `SCHEMA_VERSION`** | heed3 `metadata` database, key `schema_version` | Gates heed3 named-database layouts and rkyv record migrations |

These must not be conflated. Bumping `lord-db::SCHEMA_VERSION` rejects incompatible LMDB environments with an actionable error; it does **not** migrate or validate the redb index. Phase 1 will wire heed3 and define redb→heed3 data migration separately.

`LordEnv::open` writes `SCHEMA_VERSION` on first open and fails on reopen when the stored value differs from the compiled constant. Malformed metadata (wrong byte length) is rejected rather than silently ignored.

rkyv schema intent
------------------

Indexed records will be stored as rkyv-archived structs in heed3 named databases (e.g. `records.v1`). Hot read paths deserialize via `rkyv::api::high::access` returning `&Archived<T>` without copying.

`RkyvCodec` decoded references are tied to the input byte slice lifetime and therefore to the enclosing heed3 read transaction. Do not use archived references after the transaction commits or drops.

PR1a (complete): inscription/rune surface removal
-------------------------------------------------

User-facing inscription and rune functionality has been removed from the `lord` binary. Cardinal wallet and block explorer core remain. The **redb index is still in use**; internal legacy types under `src/index/legacy/` and unused redb tables are retained until PR1b.

**Removed**

- CLI: `balances`, `decode`, `runes`, `teleburn`; wallet `batch`, `burn`, `inscribe`, `inscriptions`, `mint`, `offer`, `pending`, `resume`, `runics`, `split`
- Server routes: `/inscription*`, `/inscriptions*`, `/rune*`, `/runes*`, `/collections*`, `/galleries*`, `/preview*`, `/children*`, `/parents*`, `/item*`, `/decode*`, `/content*`, `/metadata*`, `/feed.xml`, recursive inscription endpoints
- `crates/ordinals` rune/runestone modules (sat types + `varint` kept)
- HTML templates and Rust template modules for inscription/rune UI
- Integration tests for inscriptions, runes, batch, offers, etc.

**Preserved**

- Explorer: `/`, `/blocks`, `/block/:id`, `/tx/:txid`, `/output/:outpoint`, `/address/:address`, `/status`, `/clock`, `/sat/:sat`, `/search`, `/r/*` (blockhash, blockinfo, sat, tx, utxo)
- Wallet: `create`, `dump`, `receive`, `restore`, `sats`, `sign`, `transactions`, `addresses`, `send` (BTC/sats), `balance`, `cardinals`, `outputs`, `sweep`, `label`
- `lord-db` crate unchanged; `sats` feature declaration unwired

**Index behavior**

- `InscriptionUpdater` / `RuneUpdater` removed from the block updater; new indexes force `index_inscriptions=false`, `index_runes=false`
- Public index methods for legacy redb tables remain stubbed or internal for compile/schema compatibility

**Exit criteria verified**

```bash
cargo build --release
cargo test --lib      # 171 passed
cargo test --tests    # 109 passed
cargo fmt --check
cargo clippy -p lord -- -D warnings
```

PR1b scope (not yet implemented)
--------------------------------

- Replace `index.redb` with heed3+rkyv databases
- Remove unused redb tables and `src/index/legacy/` compatibility shims
- Wire Carbonado v2 for on-disk blob storage
- Migrate existing index data

Compatibility preserved from ord
--------------------------------

### Environment variables (`ORD_` prefix)

The CLI still reads configuration from environment variables prefixed with `ORD_` (e.g. `ORD_CHAIN`, `ORD_DATA_DIR`). This is **deliberately preserved** for backward compatibility with existing ord deployments, systemd units, and operator scripts. A `LORD_` prefix may be added later; `ORD_` remains supported.

### `sats` feature flag

The root `sats` Cargo feature is **reserved** for future sat-indexing behavior. It is empty in Phase 0 and has no `cfg` gates yet.

### Configuration filenames

- **`lord.yaml`** — preferred example config for new deployments (`/var/lib/lord` paths; see repo root `lord.yaml`).
- **`ord.yaml`** — retained for ord compatibility; `Settings` still probes `ord.yaml` in the data directory by default (`src/settings.rs`). Operators may symlink `lord.yaml` → `ord.yaml` or set `--config` explicitly until defaults are switched in a later phase.

### Root crate dependencies

`lord-db` is a workspace member; `carbonado` is an external sibling crate in `../carbonado` until Phase 1. Neither is linked from the root `lord` binary until Phase 1 wiring.

### Release tooling

`install.sh` and `justfile` `publish-release` / `publish-tag-and-crate` recipes target **bitmask-stack/lord**, not ordinals/ord. Operational `bin/*` scripts use `/var/lib/lord` paths and the `lord` systemd unit.