Operator Runbook
================

Per-chain notes for running Lord with the embedded OpenTimestamps calendar:
configuration, firewall, anchor wallet funding, and health checks.

For the regtest commitment ceremony, see [Commitments](commitments.md). For
signet acceptance before mainnet, use `just ceremony signet` and fill
[`ACCEPTANCE-SIGNET.md`](../../../ACCEPTANCE-SIGNET.md). For release tagging,
see [`RELEASE.md`](../../../RELEASE.md). For Bitcoin Core wallet setup, see
[Wallet](wallet.md).

Overview
--------

Lord persists chain-scoped state under `{data_dir}`:

| Chain | Lord data directory suffix |
|-------|---------------------------|
| Mainnet | `{data_dir}/` |
| Regtest | `{data_dir}/regtest/` |
| Signet | `{data_dir}/signet/` |
| Testnet3 | `{data_dir}/testnet3/` |
| Testnet4 | `{data_dir}/testnet4/` |

Calendar state lives at `{chain_data_dir}/calendar/` (`state.json` snapshot of
queue + anchor store; legacy `pending.json` / `store.json` may exist on upgrade;
`uri` holds the active calendar base URI). See
[implementation notes](../lord/implementation.md).

Embedded calendar settings in `lord.yaml`:

```yaml
calendar_enabled: true
calendar_listen: 127.0.0.1:14788
calendar_uri: http://127.0.0.1:14788
# optional override for commit timestamp/upgrade:
# calendar_url: http://127.0.0.1:14788/timestamp
```

**Option A — embedded:** `calendar_enabled: true` and `lord server` starts the
HTTP calendar and anchor worker in-process.

**Option B — standalone:** `lord calendar serve` (or `lord --chain <CHAIN> calendar serve`).

`calendar_listen` and `calendar_uri`
------------------------------------

| Setting | Default | Purpose |
|---------|---------|---------|
| `calendar_listen` | `127.0.0.1:14788` | Bind address for `POST /timestamp` and `GET /health` |
| `calendar_uri` | `http://127.0.0.1:14788` | Base URI written to `{calendar}/uri` for local clients |

### Firewall

Default `calendar_listen` is **loopback only** (`127.0.0.1:14788`). This is
intentional:

- `lord commit timestamp` and `lord calendar doctor` talk to the calendar over HTTP on localhost.
- Do **not** expose `calendar_listen` to the public internet unless you operate a
  deliberate public calendar service and understand the abuse surface.

If you bind a non-loopback address, restrict ingress with host firewall rules
(e.g. allow only trusted IPs on TCP 14788).

Bitcoin Core requirements
-------------------------

| Requirement | Calendar / commitments | Explorer |
|-------------|------------------------|----------|
| Full node sync | Yes | Yes |
| `txindex=1` | **No** | Yes |
| Pruned node | **OK** for calendar | Reduced / incompatible with full explorer |
| Wallet enabled | **Yes** (anchor spends) | Optional for explorer-only |

RPC cookie (or user/pass) must match Lord's `--chain`. Lord probes the chain via
`getblockchaininfo` and errors on mismatch.

Anchor wallet
-------------

The calendar anchor worker spends from bitcoind's **default loaded wallet** via
RPC (not `lord wallet`'s LMDB store). Requirements:

1. `disablewallet` must not be set.
2. At least one wallet must be loaded with spendable UTXOs above dust + anchor fee.
3. Named calendar wallets are not supported yet (`wallet_name` in anchor config).

### Funding checklist

| Chain | How to fund |
|-------|-------------|
| **Mainnet** | Deposit sats to the loaded wallet; monitor `lord calendar doctor` → `wallet_balance_sats` |
| **Signet** | [Signet faucet](https://signet.bc-2.jp/) or your signet provider; mine if you run your own signet |
| **Testnet3** | Public testnet faucets; `bitcoin-cli -testnet` |
| **Testnet4** | Testnet4 faucets / community resources |
| **Regtest** | `bitcoin-cli -regtest -generate 101` or mine to your address |

Keep mainnet balance high enough for hourly anchors and fee spikes. A single
anchor uses one UTXO and returns change; very small UTXOs are skipped.

Anchor frequency and fees
-------------------------

Chain-aware defaults from `anchor_config_for_chain`:

| Chain | Min time between anchor txs | Worker poll | Pending timeout |
|-------|----------------------------|-------------|-----------------|
| Regtest, signet, testnet3/4 | 200 ms | 1 s | 60 s |
| Mainnet | 1 hour | 30 s | 1 hour |

Each anchor transaction:

- Batches up to **64** pending digests into one merkle root
- Commits merkle root in **OP_RETURN**
- Pays fee from `estimatesmartfee` (with per-network fallback)

On mainnet, expect at most roughly one anchor per hour when the queue is non-empty.

Health checks
-------------

```bash
lord calendar url      # resolved POST /timestamp URL
lord calendar doctor   # HTTP, /health, bitcoind, wallet, last anchor
```

`calendar doctor` JSON fields:

| Field | Meaning |
|-------|---------|
| `calendar_url` | Resolved `POST /timestamp` URL used for the probe |
| `calendar_reachable` | HEAD/GET to timestamp endpoint succeeded |
| `calendar_error` | HTTP/connect error when calendar is unreachable |
| `bitcoind_reachable` | RPC `getblockchaininfo` succeeded |
| `bitcoind_error` | RPC connect or query error string |
| `embedded_health` | `GET http://{calendar_listen}/health` body |
| `wallet_balance_sats` | Spendable balance in loaded wallet |
| `wallet_error` | e.g. no wallet, insufficient funds |
| `pending_digests` | Digests waiting for next anchor |
| `last_anchor_txid` | Last broadcast anchor tx |

Per-chain quick start
---------------------

### Mainnet

```bash
# lord.yaml under /var/lib/lord or ~/.local/share/ord
calendar_enabled: true
calendar_listen: 127.0.0.1:14788
chain: mainnet
```

```bash
lord server
lord calendar doctor
lord commit timestamp <bao_root_hex>
```

Fund the bitcoind wallet with real sats. Keep `calendar_listen` on loopback unless
you operate a public calendar.

### Signet

```bash
just ceremony signet         # encode + printed live steps; optional LORD_DATADIR=...
lord --signet calendar serve
# or calendar_enabled in lord.yaml + lord --signet server
```

```bash
lord --signet calendar doctor
```

Use signet faucets; anchor interval is fast (sub-second minimum spacing). Record
pass/fail in [`ACCEPTANCE-SIGNET.md`](../../../ACCEPTANCE-SIGNET.md).

### Testnet3

```bash
lord --testnet server
lord --testnet calendar doctor
```

Data directory: `.../testnet3/`. Default RPC port `18332`.

### Testnet4

```bash
lord --testnet4 server
lord --testnet4 calendar doctor
```

Data directory: `.../testnet4/`. Default RPC port `48332`.

### Regtest

```bash
bitcoind -regtest
lord --regtest calendar serve &
lord --regtest calendar doctor
bitcoin-cli -regtest -generate 3
```

On regtest, two behaviors differ:

- **Commit/timestamp URL:** `lord commit timestamp` defaults to the loopback
  calendar (`http://127.0.0.1:14788/timestamp`) even without `calendar_enabled`.
- **Calendar process:** `lord --regtest server` does **not** start the calendar
  unless `calendar_enabled: true`; run `lord calendar serve` separately or enable
  embedding in config.

See [Commitments](commitments.md) for the full ceremony.

Backup checklist per chain
--------------------------

Back up the **chain-scoped data directory** before upgrades or host migration.
Mainnet uses `{data_dir}/`; other chains use `{data_dir}/{chain}/`.

| Path | Required for restore | Notes |
|------|---------------------|-------|
| `storage/` | **Yes** | LMDB commitment metadata + `master.key` (odd formats) |
| `carbonado/` | **Yes** | Encoded blobs (bytes are not in LMDB) |
| `ots/` | **Yes** | Detached OTS proofs |
| `breccia/global.breccia` | **Yes** | Global commitment log |
| `calendar/` | Recommended | Anchor queue state; can rebuild from OTS + bitcoind |
| `index/` | Optional | Cardinal index — can re-sync from bitcoind |
| `wallets/<name>/` | If using Lord wallet | Per-wallet LMDB |
| `filepack/` | If using bundles | Manifests + sidecars |

### Per-chain notes

| Chain | Directory | Backup priority |
|-------|-----------|-----------------|
| **Mainnet** | `{data_dir}/` | Treat `master.key` and private (odd) carbonado as **secrets** |
| Signet | `{data_dir}/signet/` | Use for acceptance testing — template in `docs/ACCEPTANCE-SIGNET.md` |
| Testnet3 | `{data_dir}/testnet3/` | Same layout as signet |
| Testnet4 | `{data_dir}/testnet4/` | Same layout as signet |
| Regtest | `{data_dir}/regtest/` | Dev only; safe to delete and re-encode |

After restore, run `lord commit verify <bao_root> --full` for each timestamped
commitment. For signet acceptance, fill `docs/ACCEPTANCE-SIGNET.md`.

Configuration file probe
------------------------

When `--config` is not set, Lord probes **one directory** (`--config-dir` if set,
else `--datadir`, else default) for config files in order: `lord.yaml`, then
`ord.yaml`. If both `--config-dir` and `--datadir` are set, `--config-dir` wins
for probing only.

Explicit `--config` always wins over any probed file in the same directory:

```bash
lord --config /etc/lord/lord.yaml server
```

See [Settings](settings.md).

Storage market payments (Lightning + ecash)
-------------------------------------------

Settlement for storage contracts, challenge fees, and LTP micro-payments is
configured under the same chain-scoped `{chain_data_dir}` as the market LMDB
(`{chain_data_dir}/market/`).

### Lightning channel funding

Run a long-lived embedded node (recommended for production BOLT11 settlement):

```bash
lord --chain <CHAIN> lightning serve
lord --chain <CHAIN> lightning status
```

| Chain | Notes |
|-------|-------|
| **Regtest** | `bitcoind -regtest`; fund LDK on-chain wallet; open channels between peers for pay/settle smoke |
| **Signet** | Same as regtest with signet RPC; use for acceptance before mainnet |
| **Mainnet** | Fund on-chain wallet; open inbound/outbound channels for invoice liquidity |

Persistence: `{chain_data_dir}/lightning/`. Settlement reuses a running node when
you inject [`SharedRunningNode`] into
[`coordinator_for_chain_with_lightning`] (same process). Separate `lightning serve`
+ `market invoice` processes still use node-per-call until IPC lands — avoid
concurrent `serve` + embedded node-per-call against the same storage dir (LMDB lock).

Regtest/signet smoke creates BOLT11 invoices without open channels; **paying**
invoices requires channel liquidity.

### Ecash mint allowlists

Restrict which Cashu mint URLs operators may use:

```yaml
ecash_enabled: true
ecash_mint_urls:
  - "https://mint.example"
ecash_mint_allowlist:
  - "https://mint.example"
```

Every `ecash_mint_urls` entry must appear in `ecash_mint_allowlist` when the
allowlist is set. Environment override: `ORD_ECASH_MINT_ALLOWLIST` (comma-separated).

Validation runs at `lord.yaml` load and in `EcashConfig::validate_for_use()`.

### Threshold and pricing tuning

| Key | Default | Purpose |
|-----|---------|---------|
| `ecash_settlement_threshold_sats` | `1000` | Amounts **below** this use ecash; at/above use Lightning |
| `market_contract_amount_sats` | `10000` | Storage-contract invoice amount |
| `market_challenge_fee_sats` | `10` | Provider challenge fee (always ecash rail) |

Environment: `ORD_ECASH_SETTLEMENT_THRESHOLD_SATS`,
`ORD_MARKET_CONTRACT_AMOUNT_SATS`, `ORD_MARKET_CHALLENGE_FEE_SATS`.

Zero values for any of the above are rejected at settings load. Per-contract
overrides may be stored in market LMDB `CONTRACT_PRICING` (side table).

### Ecash wallet ledger

Persistent ecash state: `{chain_data_dir}/ecash/` (`wallet.sqlite`, `wallet.seed`,
`micro_payments.jsonl`). When the ledger is open, `verify_micro` checks
`micro_payments.jsonl` for a binding-tied payment — not only receipt string match.

```bash
lord --chain <CHAIN> ecash status
```

### Settlement health checks

```bash
lord --chain <CHAIN> market invoice <bao_root>   # requires contract + ecash-lightning build
lord --chain <CHAIN> market challenge <bao_root>
lord --chain <CHAIN> market settle <bao_root>
just settlement-smoke
just operator-payments-smoke
```

[`SharedRunningNode`]: ../lord/implementation.md
[`coordinator_for_chain_with_lightning`]: ../lord/implementation.md