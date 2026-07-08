# Signet acceptance checklist

Fill this template when validating Lord on **signet** before a mainnet operator
rollout. Do not commit secrets (RPC passwords, `master.key`, wallet seeds).

**Quick start:** `just ceremony signet` encodes a test file and prints
signet-specific next steps with your `bao_root`. Complete the live steps below,
then check boxes.

See also: [Operator runbook](src/guides/operator.md), [Commitments](src/guides/commitments.md),
[Release checklist](RELEASE.md).

---

## Environment

| Field | Value |
|-------|-------|
| Date | |
| Lord version / git revision | `lord --version` / `git rev-parse HEAD` |
| Host OS | |
| bitcoind version | `bitcoin-cli -signet getnetworkinfo \| jq .version` |
| `lord.yaml` path | |
| Data directory | default `~/.local/share/ord/signet/` or `--datadir` |

### bitcoind signet (prerequisites)

```ini
# bitcoin.conf (signet)
signet=1
server=1
txindex=1          # optional for commitments-only; required for full explorer
```

```bash
bitcoind -signet -daemon
bitcoin-cli -signet getblockchaininfo   # chain: signet, blocks synced
bitcoin-cli -signet createwallet ""     # if no wallet loaded
# Fund via https://signet.bc-2.jp/ or your signet provider
bitcoin-cli -signet getbalance
```

### Environment variables (optional)

| Variable | Purpose |
|----------|---------|
| _(none)_ | `just ceremony signet` sets chain via recipe argument |
| `LORD_DATADIR` | Override temp dir in `just ceremony` (default: mktemp) |
| `BITCOIN_RPC_CONNECT` | e.g. `127.0.0.1:38332` if non-default signet RPC |
| `BITCOIN_RPC_COOKIE` | Path to `.cookie` if not default datadir |

Lord uses the same RPC discovery as ord (`bitcoin.conf`, cookie file, or
`--bitcoin-rpc-*` flags).

---

## Chain configuration

| Check | Pass? | Notes |
|-------|-------|-------|
| `lord --signet` matches bitcoind signet | ☐ | `lord --signet calendar doctor` → `bitcoind_reachable` |
| Data dir `{data_dir}/signet/` created | ☐ | |
| Calendar `lord --signet calendar doctor` healthy | ☐ | `calendar_reachable`, `embedded_health` |
| Anchor wallet funded | ☐ | `wallet_balance_sats` > dust + anchor fee |

```bash
lord --signet calendar doctor
lord --signet calendar url
```

---

## Commitment ceremony

Run `just ceremony signet` for encode + printed commands, or follow manually:

```bash
# 1. Calendar (standalone or embedded in lord server)
lord --signet calendar serve &
# or: calendar_enabled: true in lord.yaml + lord --signet server

# 2. Encode (ceremony does this in a temp datadir)
bao_root=$(lord --signet storage encode hello.txt --format c12 | jq -r '.bao_root')

# 3. Timestamp (live — not --dry-run)
lord --signet commit timestamp "$bao_root"

# 4. Wait for anchor (signet: ~200 ms min spacing; watch doctor)
lord --signet calendar doctor   # pending_digests → 0, last_anchor_txid set

# 5. Upgrade after anchor confirms
lord --signet commit upgrade "$bao_root"

# 6. Cross-store verify
lord --signet commit verify "$bao_root" --full

# 7. List order
lord --signet commit list
```

| Step | Pass? | Notes |
|------|-------|-------|
| `storage encode` → carbonado on disk | ☐ | `{datadir}/signet/carbonado/{bao_root}.c12` |
| `commit timestamp` → `.ots` file | ☐ | `{datadir}/signet/ots/{bao_root}.ots` |
| Calendar anchor after wait | ☐ | `last_anchor_txid` in doctor |
| `commit upgrade` → `upgraded: true` | ☐ | |
| `commit verify --full` → valid | ☐ | `cross_store.valid: true` |
| `commit list` ordered by `ots_order_key` | ☐ | |

### Expected `commit verify --full` (after upgrade)

| Field | Expected |
|-------|----------|
| `valid` | `true` |
| `digest_valid` | `true` |
| `cross_store.valid` | `true` |
| `attestation.status` | `confirmed` (with `height`, `confirmations`) |

---

## Explorer (optional)

| Check | Pass? | Notes |
|-------|-------|-------|
| `lord --signet server` starts | ☐ | Requires synced bitcoind |
| `/commitment/{bao_root}` shows attestation | ☐ | |
| `/content/{bao_root}` serves public blob | ☐ | Bao decode of c12 blob |

---

## Backup / restore spot-check

| Check | Pass? | Notes |
|-------|-------|-------|
| Archive `storage/`, `carbonado/`, `ots/`, `breccia/` | ☐ | Under `{datadir}/signet/` |
| Restore to fresh dir; `commit verify --full` passes | ☐ | |

```bash
tar -czvf lord-signet-backup.tgz -C ~/.local/share/ord signet/storage signet/carbonado signet/ots signet/breccia
```

---

## Sign-off

| Role | Name | Date |
|------|------|------|
| Operator | | |
| Reviewer | | |

**Result:** ☐ Accepted ☐ Blocked — link issues:

---

## Blockers (common)

| Symptom | Likely fix |
|---------|------------|
| `bitcoind_reachable: false` | Start signet bitcoind; check RPC cookie/port 38332 |
| `wallet_error` / insufficient funds | Faucet + `createwallet` / load wallet |
| `calendar_reachable: false` | `lord --signet calendar serve` or `calendar_enabled: true` |
| `upgrade` stays false | Wait for anchor tx confirmation; re-run doctor |
| `cross_store.valid: false` | Missing file under carbonado/ots/breccia; re-encode or restore backup |