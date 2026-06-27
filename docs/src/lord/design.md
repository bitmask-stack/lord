Lord Design Document
====================

> **Status:** Design reference — Phase 0 foundation in progress (PR1a–PR3 complete:
> inscription/rune removal, heed3 index, heed3 wallet, slim server + `sats` feature,
> Carbonado storage, OpenTimestamps ordering, breccia append log).
> See [implementation notes](implementation.md) for what is implemented today.  
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

Canonical ordering of committed data is **not** based on Ordinal theory. Instead, ordering follows the **merkle path of the OpenTimestamps proof** combined with the **breadth-first, left-to-right position** of the timestamp commitment.

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
| Sat ordinal ordering | OTS merkle-path + breadth-first timestamp commitment ordering |

### Current persistence (Phase 0 — PR1b/PR1c)

Until Carbonado and breccia land in later phases, lord persists local state with
**heed3 LMDB** environments via the in-repo `lord-db` crate (heed3 + rkyv):

| Store | Path | Schema version |
|-------|------|----------------|
| Cardinal index | `{data_dir}/index/` (mainnet) or `{data_dir}/{chain}/index/` | `STATISTIC_TO_COUNT` key `0` → **35** |
| Wallet metadata | `{data_dir}/wallets/<name>/` | `STATISTIC_TO_COUNT` key `0` → **2** |

There is **no migration** from legacy ord `index.redb` or `wallets/<name>.redb`
files. Operators must delete legacy redb files and re-index or recreate wallets.
See [implementation notes](implementation.md) for layout details and clean-break
error messages.

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

### Properties

- Structured, archivable metadata container
- **Each file** in a filepack archive has **its own Bao hash**
- Enables fine-grained content addressing within a single archive
- Composes with Carbonado storage and OTS timestamping

### Role in the system

Filepack is the canonical metadata layer. Carbonado holds the encoded payload; filepack describes what is stored, how it relates to commitments, and carries per-file integrity hashes.

Timestamping and Canonical Ordering
---------------------------------

Lord timestamps committed data using **OpenTimestamps** (OTS).

### What gets timestamped

Data commitments (Carbonado content addresses / Bao roots / filepack manifests — exact binding TBD) are submitted to the OpenTimestamps calendar network, producing an OTS proof anchored in the Bitcoin blockchain.

### Canonical ordering (not Ordinal theory)

Lord does **not** use Ordinal theory (sat numbering, satpoints, rarity) for ordering committed data.

Instead, canonical order is determined by:

1. **The merkle path** within the OpenTimestamps proof tree, and
2. **The position of the timestamp commitment** when the commitment tree is read **left to right, breadth-first**.

```
Breadth-first, left-to-right traversal example:

        [root]
       /      \
    [A]        [B]
   /  \        /  \
 [C]  [D]    [E]  [F]

Traversal order: root → A → B → C → D → E → F
```

Two commitments are ordered by comparing their positions in this traversal of the OTS merkle structure. Exact tie-breaking and cross-calendar aggregation rules are TBD.

### Why this ordering

- **Objective:** anchored in Bitcoin block timestamps via OTS
- **Deterministic:** same proof tree → same order, regardless of local node
- **Decoupled from sats:** no dependency on inscription sat allocation or ordinal numbering

Global Index: Breccia
---------------------

Lord uses **breccia**, Peter Todd's database, to track **all committed data globally**.

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
| RGB integration (replacing runes) | TODO |
| Exact OTS ↔ Carbonado ↔ filepack binding format | TBD |
| Cross-calendar OTS ordering aggregation | TBD |
| Bao sampling parameters (frequency, challenge size) | TBD |
| Lightning payment flow for storage contracts | TBD |
| Mutual-aid reciprocity scoring algorithm | TBD |
| Breccia schema and replication state fields | TBD |
| API/CLI surface changes (inscription/rune removal, new commands) | TBD |
| Migration path from ord codebase | TBD |