Embedded Rust Calendar and First Commitment
=============================================

This guide walks through the **MVP commitment ceremony** on Bitcoin regtest
using Lord's embedded OpenTimestamps calendar and the commitment CLI.

Commitments-only quickstart (no txindex)
----------------------------------------

Storage and OTS workflows do **not** require `txindex=1` or `lord server`:

```bash
lord storage encode myfile.txt --format c12
lord commit timestamp <bao_root_hex>    # needs calendar + bitcoind for live anchor
lord commit verify <bao_root_hex> --full --digest-only   # cross-store check without RPC
```

| Need | bitcoind | txindex | lord server |
|------|----------|---------|-------------|
| Encode + Bao verify | No | No | No |
| OTS timestamp/upgrade | Yes (calendar anchor wallet) | No | No |
| Cardinal explorer / addresses | Yes | **Yes** | Yes |

On **mainnet**, omit `--regtest` / `--signet` flags. Use `just ceremony` for a
regtest dry-run (encode + `--dry-run` timestamp + `--full` verify). For signet
acceptance, `just ceremony signet` then complete live steps and fill
[`ACCEPTANCE-SIGNET.md`](../../../ACCEPTANCE-SIGNET.md).

Prerequisites
-------------

- `cargo build --release`
- `bitcoind` on **regtest** with RPC cookie (or username/password)
- Funded regtest wallet in bitcoind (mine blocks to the default wallet)

Phase L0 — Embedded calendar
----------------------------

Lord ships a chain-aware Rust calendar in `crates/lord-calendar`. It persists
state under the per-chain data directory:

```
{data_dir}/calendar/              # mainnet
{data_dir}/regtest/calendar/      # regtest
{data_dir}/signet/calendar/       # signet
{data_dir}/testnet3/calendar/     # testnet
{data_dir}/testnet4/calendar/     # testnet4
```

**Option A — standalone calendar**

```bash
lord --regtest calendar serve
```

Binds `http://127.0.0.1:14788/timestamp` by default and runs the anchor worker
against bitcoind.

**Option B — embedded in the explorer**

Enable in `lord.yaml`:

```yaml
calendar_enabled: true
calendar_listen: 127.0.0.1:14788
calendar_uri: http://127.0.0.1:14788
```

Then `lord server` starts the calendar HTTP server and anchor worker in-process.

Phase L1 — Chain-aware defaults
-------------------------------

When the embedded calendar is enabled, or on **regtest**, Lord defaults to the
local calendar when no override is set:

```bash
lord --regtest calendar url
# → http://127.0.0.1:14788/timestamp

lord --regtest calendar doctor
# probes calendar HTTP, /health, bitcoind RPC, wallet UTXOs, and last anchor
```

Override via `lord.yaml` / `--config`:

```yaml
calendar_url: http://127.0.0.1:14788/timestamp
```

MVP ceremony
------------

```bash
# 1. Start calendar (if not embedded in lord server)
lord --regtest calendar serve &

# 2. Encode a file
lord --regtest storage encode hello.txt --format c12

# 3. Timestamp through the embedded calendar
lord --regtest commit timestamp <bao_root_hex>

# 4. Mine blocks so the calendar anchors the merkle batch
bitcoin-cli -regtest -generate 3

# 5. Upgrade the local proof (after anchor confirms)
lord --regtest commit upgrade <bao_root_hex>

# 6. Verify attestation and cross-store consistency
lord --regtest commit verify <bao_root_hex> --full

# 7. Inspect
lord --regtest commit list
lord --regtest server   # /commitment/{bao_root}, /commitments, /content/{bao_root}
```

When `calendar_enabled: true` (or on regtest), `lord commit timestamp` submits
digests to the embedded calendar via HTTP loopback (`POST {calendar_uri}/timestamp`).
The running anchor worker reloads queue state from disk each tick, so timestamps
written by a separate `lord commit` process are visible to `lord server` or
`lord calendar serve`. `TimestampOptions.calendar` is reserved for callers that
pass an explicit shared `Arc<CalendarService>` in-process.

Explicit calendar URL (always works):

```bash
lord --regtest commit timestamp <bao_root_hex> \
  --calendar-url http://127.0.0.1:14788/timestamp
```

MVP acceptance criteria
-----------------------

| Check | Expected |
|-------|----------|
| `storage/` LMDB | `CommitmentMeta` with `ots_proof_path`, `ots_order_key`, `timestamped_at` |
| `ots/{bao_root}.ots` | Detached proof from embedded calendar (not `--dry-run` stub URI) |
| `breccia/global.breccia` | `CommitmentEntry` appended |
| `commit upgrade` | `upgraded: true` after anchor; `upgraded: false` when proof already current |
| `commit verify` | `digest_valid: true`, `attestation: confirmed` with `height` and `confirmations` (after anchor + upgrade) |
| `commit verify --full` | `valid: true`, `cross_store.valid: true` (LMDB ↔ OTS ↔ breccia ↔ carbonado header binding) |
| `ots_order_key` | Merkle-path encoding (fork child indices on path to first attestation) |
| Explorer | `/commitments` lists entry; `/content` serves decoded `hello.txt` bytes |

`--dry-run` remains available for tests without a calendar:

```bash
lord --regtest commit timestamp <bao_root_hex> --dry-run
```

Reorg caveat: `commit verify` and `/commitment/{bao_root}` reflect the node's
current best chain (1-conf semantics). A reorg can invalidate a previously
`confirmed` attestation until re-verified. `confirmations` counts blocks from
the attested height to the chain tip when bitcoind RPC is available.