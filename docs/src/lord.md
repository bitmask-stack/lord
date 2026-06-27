Lord
====

**Lord** is a fork of ord that preserves the CLI, HTTP API, and core wallet/explorer functionality, while removing inscriptions and runes entirely.

In their place, lord introduces content-addressed storage, OpenTimestamps-based canonical ordering, a storage market with mutual-aid replication, an embedded LDK Lightning node, and (planned) RGB tokens.

> **Status:** Planning phase — see the [design document](lord/design.md) for the full specification.

Design Document
---------------

The complete lord design — including Carbonado v2 storage, Bao proofs of possession, filepack metadata, breccia global indexing, OTS ordering, the storage market, mutual aid mode, and all configuration options — is captured in:

**[Lord Design Document](lord/design.md)**