Lord Design Document
====================

> **Status:** Design reference — **foundation shipped** (PR0–PR3: ord fork, heed3
> index/wallet, Carbonado storage, filepack, OTS commitments, breccia append log,
> commitment explorer). **Operator stack shipped** (embedded Rust calendar,
> `commit upgrade`, Bitcoin attestation verify, txindex fallback, operator docs).
> **Planned:** PR6 RGB. **C2 skeleton shipped:** embedded LDK Lightning (`lord-lightning`,
> `lord lightning *`, feature `lightning`). Storage market gossip (Iroh P2P) shipped in Phase B.
> See [implementation notes](implementation.md) for schemas and CLI details.
> **Networking draft:** [CHIP LTP-0001](chip-ltp-0001.md) (Iroh mempool, storage market, payments annex).
> This document captures the intended design for **lord**, a fork of [ord](https://github.com/ordinals/ord).

Table of Contents
-----------------

1. [What is Lord?](#what-is-lord)
2. [Relationship to ord](#relationship-to-ord)
3. [Storage: Carbonado v2](#storage-carbonado-v2)
4. [Proof of Possession: Bao Stream Verification](#proof-of-possession-bao-stream-verification)
5. [Metadata: Filepack](#metadata-filepack)
6. [Timestamping and Canonical Ordering](#timestamping-and-canonical-ordering)
7. [Global Index: Breccia](#global-index-breccia)
8. [Lightning: LDK Node](#lightning-ldk-node)
9. [Tokens: RGB (replacing Runes)](#tokens-rgb-replacing-runes)
10. [Storage Market](#storage-market)
11. [Mutual Aid Mode](#mutual-aid-mode)
12. [Content Addressability and DDoS Resistance](#content-addressability-and-ddos-resistance)
13. [Open Questions and TODOs](#open-questions-and-todos)

What is Lord?
-------------

**Lord** is a fork of **ord** that preserves the command-line interface, HTTP API, and core wallet/explorer functionality of ord, while **completely removing** inscription capability and runes.

In their place, lord introduces a content-addressed storage and commitment system built on:

- **Carbonado v2** for durable, encoded storage
- **Bao** stream verification for proofs of possession
- **OpenTimestamps** for temporal commitment
- **Breccia** (Peter Todd) for global tracking of committed data
- **Filepack** (Casey Rodarmor) for metadata archival
- **LDK** for an embedded Lightning node with funding support
- **RGB** (planned) as the token/asset layer replacing runes

Lord creates a **storage market** for both **public** and **private** data, treating the two classes separately. Scarcity is quantified via **replication factor**. Nodes can operate in **mutual aid** mode, offering storage capacity in exchange for others storing their data.

Canonical ordering of committed data is **not** based on Ordinal theory. Instead,
ordering follows the **merkle path** of the OpenTimestamps proof; breadth-first,
left-to-right traversal is used only to **discover** the first attestation leaf.

Relationship to ord
---------------------

### Preserved

| Area | Notes |
|------|-------|
| **CLI** | Same command structure and ergonomics as ord where applicable |
| **HTTP API** | Cardinal explorer API preserved; removed inscription/rune routes return **410 Gone** (see [implementation notes](implementation.md#pr1d-complete-slim-server-api-compatibility-sats-feature)) |
| **Core functionality** | Wallet, block explorer, Bitcoin Core integration, indexing infrastructure |

### Removed

| Area | Notes |
|------|-------|
| **Inscriptions** | Entire inscription pipeline, index entries, explorer pages, wallet commands, and API endpoints are removed |
| **Runes** | Entire runes pipeline, index entries, explorer pages, wallet commands, and API endpoints are removed |
| **Ordinal theory ordering** | Sat-based numismatic ordering is not used for lord's canonical data ordering |

### Replaced / Added

| ord concept | lord replacement |
|-------------|------------------|
| Inscriptions (on-chain content commitment) | Carbonado v2 storage + OpenTimestamps commitment + breccia global index |
| Runes (fungible tokens) | RGB (TODO) |
| Sat ordinal ordering | OTS merkle-path order keys (BFS discovery only) |

### Current persistence (shipped — PR1b/PR1c/PR2/PR3)

Lord persists local state under `{data_dir}/` (mainnet) or `{data_dir}/{chain}/`
(other chains). Key stores:

| Store | Path | Notes |
|-------|------|-------|
| Cardinal index | `{data_dir}/index/` or `{data_dir}/{chain}/index/` | heed3 LMDB; schema **35** |
| Wallet metadata | `{data_dir}/wallets/<name>/` or `{data_dir}/{chain}/wallets/<name>/` | heed3 LMDB; schema **2** |
| Carbonado blobs | `{data_dir}/carbonado/` or `{data_dir}/{chain}/carbonado/` | `{bao_root_hex}.c{NN}` files |
| Filepack | `{data_dir}/filepack/` or `{data_dir}/{chain}/filepack/` | manifest directories |
| Commitment metadata | `{data_dir}/storage/` or `{data_dir}/{chain}/storage/` | heed3 LMDB (`StorageStore`) |
| OTS proofs | `{data_dir}/ots/` or `{data_dir}/{chain}/ots/` | `{bao_root_hex}.ots` |
| Breccia log | `{data_dir}/breccia/` or `{data_dir}/{chain}/breccia/` | `global.breccia` append log |
| Calendar state | `{data_dir}/calendar/` or `{data_dir}/{chain}/calendar/` | `state.json` snapshot |

All heed3 environments use the in-repo `lord-db` crate (heed3 + rkyv). There is
**no migration** from legacy ord `index.redb` or `wallets/<name>.redb` files.
Operators must delete legacy redb files and re-index or recreate wallets. See
[implementation notes](implementation.md) for layout details and clean-break error
messages.

Storage: Carbonado v2
---------------------

Lord uses the **Carbonado v2** storage format as its primary persistence layer.

### Public vs Private Encoding (c-format parity)

In Carbonado, the **c-format** encodes visibility:

| c-format | Visibility |
|----------|------------|
| **Odd**  | **Private** — encrypted / not publicly readable |
| **Even** | **Public** — readable without decryption keys |

This parity rule is a first-class design constraint: any tooling that reads or writes Carbonado blobs must respect odd = private, even = public.

### Inboard vs Outboard Encoding

Lord supports two Carbonado encoding modes:

| Mode | Behavior |
|------|----------|
| **Inboard** | Standard Carbonado encoding; public data is not directly readable on disk without decoding |
| **Outboard** | Alternative encoding where **any public (even c-format) data is readable on-disk** without additional decoding steps |

The operator chooses which encoding to use. This affects local disk layout, replication behavior, and what mutual-aid peers can serve without decryption keys.

Proof of Possession: Bao Stream Verification
--------------------------------------------

Remote storage providers must demonstrate they actually hold the data they claim to store. Lord uses **Bao** (BLAKE3 out-of-core proof-of-integrity) stream verification for this.

### Mechanism

1. Content is chunked and hashed into a Bao tree.
2. A verifier requests **probabilistic samples** of the remote proof.
3. The remote party streams the sampled ranges.
4. The verifier checks the streamed chunks against the expected Bao hash tree **without downloading the entire blob**.

### Why probabilistic sampling

Full re-download of every stored object is impractical at market scale. Probabilistic sampling gives high confidence of possession at low bandwidth cost. Sampling frequency and challenge parameters are market/policy configuration (TBD).

### Relationship to filepack

Every file inside a filepack archive carries its **own Bao hash** (see [Metadata: Filepack](#metadata-filepack)). Proofs of possession can be verified per-file within an archive, not only at the archive level.

Metadata: Filepack
------------------

Lord uses **filepack**, Casey Rodarmor's metadata archival format, to bundle metadata alongside stored content.

### Two-layer composition

1. **Carbonado layer** — encoded blobs, `CommitmentMeta` in LMDB, per-file Bao roots.
2. **Casey filepack layer** — stock CBOR `manifest.filepack` (with `--filepack-compat`)
   whose `package` tree lists source files by raw BLAKE3 hash so upstream `filepack
   verify` works on the original directory.

Lord binds the layers via a **separate sidecar file** alongside the manifest,
`{fingerprint}/lord.carbonado.cbor` (not inside the Casey archive `files` map):

```text
path → { bao_root, format, carbonado_path }
```

### Properties

- Structured, archivable metadata container
- **Each file** carries Carbonado binding metadata in the Lord sidecar; the Casey
  `package` tree uses source-file BLAKE3 hashes for third-party verification
- Enables fine-grained content addressing within a single archive
- Composes with Carbonado storage and OTS timestamping
- Casey `package1…` bech32m fingerprint when compat mode is enabled

### Role in the system

Filepack is the canonical metadata layer for third-party verification. Carbonado holds
the encoded payload; the Lord sidecar records how source paths map to Carbonado
commitments.

Timestamping and Canonical Ordering
---------------------------------

Lord timestamps committed data using **OpenTimestamps** (OTS).

### What gets timestamped

Data commitments are timestamped via **SHA256(bao_root)** as the OTS start digest
(filepack manifests reference the same Bao root). Proofs are submitted to Lord's
embedded OpenTimestamps calendar (or an explicit `calendar_url` override),
producing an OTS proof anchored in the Bitcoin blockchain. Canonical ordering uses
merkle-path `ots_order_key` encoding — see
[implementation notes](implementation.md) and the
[commitments guide](../guides/commitments.md).

### Canonical ordering (not Ordinal theory)

Lord does **not** use Ordinal theory (sat numbering, satpoints, rarity) for ordering committed data.

Instead, canonical order is determined by the **merkle path** (per-fork child
indices) from the proof root to the first attestation leaf. The proof tree is
walked **breadth-first, left-to-right** only to find that leaf; the sort key is
path-only (no BFS index suffix).

```
Breadth-first, left-to-right traversal example:

        [root]
       /      \
    [A]        [B]
   /  \        /  \
 [C]  [D]    [E]  [F]

Traversal order: root → A → B → C → D → E → F
```

Two commitments are ordered by comparing their **merkle-path order keys** derived
from the OTS proof tree.

### `OtsOrderKey` encoding (normative)

1. Walk the proof tree **breadth-first, left-to-right** (same queue discipline
   as the traversal diagram above).
2. On finding the first `Attestation` leaf, record the **child index at each
   `Fork`** on the path from root to that leaf (`0` = leftmost child).
3. `OtsOrderKey` is the concatenation of one byte per fork decision on the
   path, e.g. path `[1, 0]` encodes as `0x01 0x00`.
4. **Attestation at root** (no `Fork` on the path): empty path `[]`.
5. **No attestation leaf:** 8-byte big-endian `u64::MAX` sentinel (sorts last;
   keeps compatibility with PR3 sentinel ordering).
6. Compare commitments by **lexicographic order** on `OtsOrderKey` bytes
   (`COMMITMENT_ORDER` in LMDB).

Path-only encoding is used first; a BFS tie-break suffix is deferred unless
collisions are observed in practice. Cross-calendar aggregation remains TBD.

### Why this ordering

- **Objective:** anchored in Bitcoin block timestamps via OTS
- **Deterministic:** same proof tree → same order, regardless of local node
- **Decoupled from sats:** no dependency on inscription sat allocation or ordinal numbering

Global Index: Breccia
---------------------

Lord uses a **breccia** append log to track committed data. The long-term target
is [Peter Todd's breccia](https://github.com/petertodd/python-breccia) mark-word
database for global replication and coordination.

**PR3 (implemented):** `{data_dir}/breccia/global.breccia` is a lord-specific
append log (`LORBRECC` file-header magic + `u32` header version). Each appended
blob is length-prefixed:

- **v1** — bincode `CommitmentEntry` (bao root, order key, timestamp, carbonado path)
- **v2 (Phase B)** — `LBV2` magic + bincode `CommitmentEntryV2` adding Bitcoin
  attestation metadata (`attestation_height`, `attestation_txid`, `replication_target`)

The file header bumps to version 2 on first v2 append (`ensure_v2_header`) without
rewriting existing v1 blobs. It is **not** the Peter Todd breccia format; migration
or federation with that format is planned for a later phase.

### Role

- Single source of truth for what has been committed across the network
- Indexes content addresses, OTS proofs, ordering keys, and replication state
- Feeds the explorer/API with global commitment history
- Supports the storage market (who stores what, at what replication factor)

Breccia is the global coordination layer; Carbonado is the local/per-node storage layer.

Lightning: LDK Node
-------------------

Lord embeds a **Lightning node** built on **LDK** (Lightning Dev Kit).

### Capabilities

- Full Lightning node functionality within the lord process
- **Funding support:** operators can fund the embedded node (on-chain deposit, channel management — details TBD)
- Payment rails for the storage market (pay for replication, receive for providing storage — details TBD)

### Relationship to Bitcoin wallet

Lord inherits ord's Bitcoin Core wallet integration. The Lightning node shares the same underlying Bitcoin connectivity. Exact wallet/channel architecture is TBD.

Tokens: RGB (replacing Runes)
-----------------------------

Runes are **removed entirely**. Lord plans to use **RGB** as the token and smart-contract layer.

> **Status: TODO** — RGB integration is not yet designed in detail.

### Intended role

RGB contracts would represent:

- Storage market agreements (replication contracts, payment terms)
- Asset issuance where applicable
- Private state transitions tied to committed data

The RGB layer sits above Bitcoin/Lightning; exact mapping to Carbonado commitments and breccia entries is TBD.

Storage Market
------------

Lord creates a **storage market** where participants buy and sell durable storage for committed data.

### Public vs Private Markets

Public and private data are **treated separately**:

| Class | c-format | Market behavior |
|-------|----------|-----------------|
| **Public** | Even | Openly replicable; readable on disk (especially with outboard encoding); pricing driven by replication demand |
| **Private** | Odd | Encrypted; replication requires ciphertext handling; separate pricing and peer eligibility rules |

Mixing public and private storage obligations in a single contract is discouraged; the market layer keeps them distinct.

### Replication Factor

**Replication factor** quantifies **scarcity**:

- How many independent nodes hold a given commitment
- Higher replication → higher cost, higher durability guarantee
- Lower replication → cheaper, higher risk of data loss

Replication factor is the primary scarcity metric in the market (as opposed to sat rarity in ord).

### Market operations (high level)

1. Committer timestamps data via OTS and registers in breccia
2. Committer requests replication at a target factor
3. Storage providers bid/offer capacity (optionally filtered by mutual-aid preferences)
4. Providers prove possession via Bao stream verification
5. Payment settles over Lightning (or on-chain — TBD)

Mutual Aid Mode
---------------

Nodes can operate in **mutual aid** mode: your node offers to help store other people's data **if** you also store other people's data (reciprocal storage).

### Configuration options

| Setting | Description |
|---------|-------------|
| **Mutual aid enabled** | Node participates in reciprocal storage; offers capacity in exchange for others storing your data |
| **Encrypted-only preference** | Node prefers (or only accepts) storing **encrypted (private / odd c-format)** data |
| **Open to unencrypted** | Node is willing to store **unencrypted (public / even c-format)** data as well |

These preferences affect which market offers the node sees, which peers it replicates for, and how mutual-aid reciprocity is calculated.

### Encoding preference

Separately from encrypted/unencrypted preference, the operator chooses:

- **Inboard** Carbonado encoding, or
- **Outboard** Carbonado encoding (public data readable on-disk)

This interacts with mutual aid: a peer using outboard encoding can serve public blobs without decoding, which may affect proof-of-possession latency and market pricing.

Content Addressability and DDoS Resistance
------------------------------------------

Lord's protocol is **highly content-addressable**:

- Carbonado blobs are addressed by content hash
- Every filepack entry carries its own Bao hash
- OTS proofs bind commitments to Bitcoin timestamps
- Breccia indexes by content address

### DDoS resistance via stream verification

Because integrity is verified via **Bao stream verification** rather than full download:

- Verifiers request only the chunks they need to check a proof
- Providers cannot cheaply flood verifiers with junk data (invalid chunks fail the Bao check immediately)
- Probabilistic sampling limits bandwidth while maintaining integrity guarantees

This makes the storage and verification protocol **effectively DDoS-proof** at the content-delivery layer: attackers cannot sustain bogus responses against valid challenges without failing verification.

Open Questions and TODOs
------------------------

| Item | Status |
|------|--------|
| Lightning payment flow for storage contracts | **Shipped (C6)** — live BOLT11 + Bao challenge gate; reuse long-lived node via `SharedRunningNode` |
| Ecash micro-payments (challenge fees, LTP) | **Shipped (C6)** — CDK wallet ledger-backed `verify_micro`; mint allowlists; LTP `PaymentProof` gossip |
| Storage market contracts + LMDB (C1) | **Shipped** — `lord-market`, `lord market *` |
| Storage market pricing and contract enforcement | **Shipped (C6)** — YAML `market_*_sats` + per-contract `CONTRACT_PRICING` side table |
| RGB integration (replacing runes) | **TODO** (PR6) |
| Mutual-aid reciprocity scoring algorithm | **TODO** |
| Cross-calendar OTS ordering aggregation | **Deferred** — single embedded calendar per chain; federation later |
| Bao sampling parameters (frequency, challenge size) | **Open** — tune under load |
| Breccia federation / replication state fields | **Partial** — Phase B adds breccia v2 attestation fields + LTP gossip; full federation TBD |
| LTP Iroh transport + local mempool | **Shipped (Phase B)** — `lord-ltp`, `lord-iroh`, `lord ltp *`, `lord p2p *` |
| Calendar anchor fee caps | **Shipped (Phase B)** — `calendar_max_anchor_fee_sats`, `calendar_min_wallet_balance_sats` |
| OTS ↔ Carbonado ↔ filepack binding | **Resolved** — `SHA256(bao_root)` as OTS start digest; `ots_order_key` from merkle-path encoding; see [implementation notes](implementation.md) and [commitments guide](../guides/commitments.md) |
| API/CLI surface (inscription/rune removal, storage/commit/calendar) | **Resolved** — PR0–PR3 + operator stack shipped |
| Migration path from ord codebase | **Resolved** — heed3 index/wallet, slim server, `lord.yaml` default probe |
| Embedded OpenTimestamps calendar | **Resolved** — `crates/lord-calendar` on all chains; `calendar_enabled` / `lord calendar serve` |