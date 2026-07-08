Lord Transport Protocol (LTP) — CHIP draft
============================================

> **CHIP:** LTP-0001 (draft)  
> **Status:** Design — not a consensus change; documents Lord's planned networking and market layers.

Lord Transport Protocol (LTP) describes how Lord nodes exchange commitments,
storage proofs, and (later) payments beyond the shipped Carbonado + OTS + breccia
stack.

LTP
---

**LTP** is Lord's application protocol for:

1. **Commitment gossip** — breccia heads, OTS upgrade hints, replication offers
2. **Storage market** — contract bids, Bao challenge/response, replication factor
3. **Payments annex** — Lightning (LDK) settlement for storage contracts (design only here)

LTP rides on **Iroh** for P2P transport (PR5). Phase A uses HTTP (explorer,
embedded calendar) and local LMDB; LTP is the bridge to federated replication.

Iroh mempool
------------

Lord maintains an **Iroh mempool** of pending LTP messages:

| Queue | Contents |
|-------|----------|
| Commitment | New breccia entries, OTS upgrade notifications |
| Storage | Replication requests, Bao proof challenges |
| Market | Contract offers, fee quotes |

The mempool is **not** a Bitcoin mempool. It buffers outbound/inbound LTP frames
until peers acknowledge or contracts expire. Backpressure uses replication factor
and mutual-aid quotas.

Breccia post-mine
-----------------

After a calendar **anchor** confirms on Bitcoin:

1. Embedded calendar builds merkle batch → Bitcoin attestation
2. Operators run `lord commit upgrade` to enrich local `.ots` proofs
3. **Post-mine** breccia append records the confirmed `ots_order_key` and block height
4. LTP gossips breccia tail + attestation height to peers

Post-mine entries are append-only; reorgs invalidate attestations until re-verified
(same semantics as `commit verify` today).

Miner fees
----------

Calendar anchors spend from bitcoind's loaded wallet (not Lord LMDB wallet):

| Chain | Fee source | Operator note |
|-------|------------|---------------|
| **Mainnet** | Wallet UTXOs | Monitor `lord calendar doctor`; fund for fee spikes |
| Signet | Faucet / own signet | Lower stakes; still require spendable UTXOs |
| Testnet3/4 | Faucets | Same anchor worker as mainnet |
| Regtest | `generatetoaddress` | Dev only |

Lord does not subsidize anchors. Phase B ships `calendar_max_anchor_fee_sats` and
`calendar_min_wallet_balance_sats` in `lord.yaml`; when either policy blocks an
anchor tick, `lord calendar doctor` reports `last_anchor_skipped_reason`.

Chain profiles
--------------

| Profile | bitcoind | txindex | Calendar | Explorer |
|---------|----------|---------|----------|----------|
| **Mainnet production** | Full sync | Recommended | Embedded or standalone | Full |
| Signet acceptance | Full sync | Optional | Embedded | Reduced OK |
| Commitments-only | Pruned OK | **No** | Required | Not required |
| Regtest dev | Local | No | Embedded default | Optional |

Data directories are chain-scoped — see [operator guide](../guides/operator.md).

Ord compatibility
-----------------

Lord preserves ord CLI/HTTP shape where cardinal features remain. Removed surfaces
return **410 Gone** (inscriptions, runes). LTP does not alter Bitcoin consensus or
ord wire formats.

Payments annex (LDK + ecash design)
-----------------------------------

**Phase C2/C3** embeds **LDK** for Lightning and **CDK** for ecash:

| Rail | Crate | Hot path | Amount routing |
|------|-------|----------|----------------|
| Lightning (BOLT11 / HTLC) | `lord-lightning` | Storage contract settlement | `amount_sats >= ecash_settlement_threshold_sats` |
| Ecash (Cashu) | `lord-ecash` | LTP micro-payments, challenge fees | `amount_sats < threshold` (challenge fees always ecash) |

- [`lord-payments`](../../crates/lord-payments) defines `MicroPaymentProvider`,
  `LightningSettlementProvider`, `LightningInvoicePayer`, and
  `SettlementCoordinator` (routes by `ecash_settlement_threshold_sats`, default 1000 sats).
- Payments bind to committed roots via `PaymentBinding { bao_root, purpose, amount_sats }`.
- **C4 shipped:** live LDK BOLT11 invoice flow via `LightningPaymentProvider`; CDK SQLite
  wallet + binding-tied ecash receipts; melt bridge via `LightningInvoicePayer`.
  Live mint HTTP remains optional (no mandatory testnut in default CI).
- RGB invoices are out of scope until PR6

CLI (feature-gated, default off):

```text
lord ecash status          # feature `ecash`
lord lightning status    # feature `lightning`
lord market invoice <bao_root>   # feature `lightning` (live settlement wiring)
```

Storage market annex
--------------------

**Phase PR5** (Iroh):

- Public (even c-format) and private (odd) markets are separate
- **Replication factor** is the scarcity metric
- Providers prove possession via Bao stream verification (sampled challenges)
- Mutual-aid mode: reciprocal storage offers with encrypted-only preference

Non-goals
---------

| Item | Status |
|------|--------|
| **RGB** tokens | Deferred — design stub only; replaces runes in PR6 |
| Cross-calendar OTS federation | Deferred — single embedded calendar per chain first |
| On-chain storage contracts | Non-goal — commitments are OTS + breccia, not inscriptions |

Phase A bridge
--------------

Shipped today (Phase A):

- Carbonado encode/verify, filepack, LMDB metadata
- OTS timestamp/upgrade/verify (`--full` cross-store check)
- Embedded calendar, breccia append log, `/commitment/*` explorer

LTP Phase B adds Iroh transport atop the same breccia tail and Bao roots.

Phase B normative wire model
----------------------------

### `LtpFrame` envelope

All LTP gossip frames use a versioned envelope:

| Field | Type | Semantics |
|-------|------|-----------|
| `version` | `u8` | Frame schema version (currently **1**) |
| `chain_id` | `u32` | Chain profile id (mainnet **0**, signet **1**, testnet **2**, regtest **3**) |
| `message_type` | enum | See message types below |
| `payload` | bytes | Type-specific JSON (Phase B) or CBOR (future) |

### Message types (Phase B)

| Type | Status | Purpose |
|------|--------|---------|
| `BrecciaTail` | **Normative** | Post-mine breccia head + OTS binding |
| `OtsUpgradeHint` | **Normative** | Notify peers that a digest has an enriched calendar proof |
| `ReplicationOffer` | **Stub** | Storage market offer (Track C) |
| `BaoChallenge` | **Normative (C5)** | Sampled Bao possession challenge (`bao_root`, `sample_offset`, `sample_rate`) |
| `PaymentProof` | **Normative (C5)** | Ecash binding receipt (`ecash:binding:{bao_root}:{purpose}`) |

### `PaymentProof` payload (C5)

| Field | Type | Semantics |
|-------|------|-----------|
| `bao_root` | `[u8; 32]` | Carbonado commitment root |
| `purpose` | string | `storage_contract`, `challenge_fee`, or `ltp_micro_payment` |
| `ecash_reference` | string | Must equal `ecash:binding:{hex(bao_root)}:{purpose}` |
| `amount_sats` | `u64` | Bound payment amount (> 0) |

Inbound proofs validate binding before append to `ltp/inbound_payment_proofs.jsonl`.
Settlement uses persisted [`ChallengeProof`] (from `lord market challenge`) or an
inline sample before HTLC/ecash settle completes.

### `BaoChallenge` payload (C5)

| Field | Type | Semantics |
|-------|------|-----------|
| `bao_root` | `[u8; 32]` | Commitment under challenge |
| `sample_offset` | `u64` | Byte offset for sampled slice |
| `sample_rate` | `u32` | Number of slices to sample (must be > 0) |

### `LtpMempoolEntry`

Local mempool records (persisted at `{chain_data_dir}/ltp/mempool.json`):

| Field | Type | Semantics |
|-------|------|-----------|
| `bao_root` | `[u8; 32]` | Carbonado commitment root |
| `start_digest` | `[u8; 32]` | `SHA256(bao_root)` OTS start digest |
| `enqueued_at` | `u64` | Unix seconds |
| `priority` | `u32` | Higher sorts earlier when `calendar_ltp_priority` is enabled |

### `TreeRoot`

Calendar batch anchor metadata carried in `BrecciaTail` payloads:

| Field | Type | Semantics |
|-------|------|-----------|
| `merkle_root` | `[u8; 32]` | OpenTimestamps merkle batch root |
| `anchor_txid` | string | Bitcoin txid of the calendar anchor |

### Mempool rules

1. **Dedup** — within a queue, `enqueue` rejects duplicate `start_digest` values.
2. **TTL** — `prune_expired` drops entries older than the chain profile TTL
   (mainnet 24h, signet 1h, regtest 5m).
3. **Max depth** — per-queue cap (mainnet 10_000; regtest 1_000). Enqueue fails
   when full.

Inbound gossip tails are staged at `{chain_data_dir}/ltp/inbound_tails.jsonl`
without auto-merge; operators run `lord ltp import` to append breccia v2 entries
idempotently.

Migration
---------

| From | To | Action |
|------|-----|--------|
| ord `index.redb` | Lord heed3 `index/` | Delete redb; re-index |
| ord wallets | Lord `wallets/<name>/` | Recreate wallets |
| Inscriptions | — | Not supported; use Carbonado + OTS |
| Runes | RGB (future) | Deferred |

No automatic migration from ord persistence. Operators back up chain-scoped
directories before upgrades — see [operator backup checklist](../guides/operator.md#backup-checklist-per-chain).