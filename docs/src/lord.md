Lord
====

**Lord** is a fork of ord that preserves the CLI, HTTP API, and core wallet/explorer functionality, while removing inscriptions and runes entirely.

In their place, lord introduces content-addressed storage, OpenTimestamps-based canonical ordering, a storage market with mutual-aid replication, an embedded LDK Lightning node, and (planned) RGB tokens.

> **Status:** PR0–PR3 shipped (explorer/wallet fork, Carbonado storage, filepack, OTS commitments, breccia log). Operator stack shipped (embedded Rust calendar, `commit upgrade`, Bitcoin attestation verify, txindex fallback, operator docs). PR4+ (Lightning, Iroh, RGB) remain planned — see the [design document](lord/design.md) and [README.md](https://github.com/bitmask-stack/lord/blob/main/README.md) for the canonical status table.

Design Document
---------------

The complete lord design — including Carbonado v2 storage, Bao proofs of possession, filepack metadata, breccia global indexing, OTS ordering, the storage market, mutual aid mode, and all configuration options — is captured in:

**[Lord Design Document](lord/design.md)**

Transport / networking draft:

**[CHIP LTP-0001 (draft)](lord/chip-ltp-0001.md)** — Lord Transport Protocol (Iroh, storage market, payments annex).