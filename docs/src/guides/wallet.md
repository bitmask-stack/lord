Wallet
======

Lord uses **Bitcoin Core** for private keys, transaction signing, and broadcast.
`lord wallet` subcommands talk to a Bitcoin Core descriptor wallet over RPC; Lord
does not implement its own key storage.

This guide covers:

1. Bitcoin Core setup for Lord (RPC, cookie auth, chain flags)
2. Creating and using a Lord wallet
3. Calendar anchor wallet funding (same or separate from your operator wallet)
4. When `txindex` is required vs optional

For OpenTimestamps commitment ceremony, see [Commitments](commitments.md).

Getting Help
------------

If you get stuck, check the [Lord repository](https://github.com/bitmask-stack/lord)
for issues and discussions.

Prerequisites
-------------

- Bitcoin Core **28 or newer**
- `bitcoind` running on the chain you pass to Lord (`--chain`, `--signet`, etc.)
- RPC reachable via cookie file (default) or `--bitcoin-rpc-username` /
  `--bitcoin-rpc-password`

Do **not** use `bitcoin-qt` for server deployments; run `bitcoind` directly.

Configuring Bitcoin Core
------------------------

### RPC and cookie authentication

Lord expects cookie-based RPC by default. In `bitcoin.conf`:

- Do **not** set `rpcuser`, `rpcpassword`, or `rpcauth` unless you also configure
  Lord with matching `--bitcoin-rpc-username` and `--bitcoin-rpc-password`.
- Do **not** set `disablewallet=1` if you use `lord wallet` or the embedded calendar
  anchor worker (both need a funded wallet in bitcoind).
- Ensure `.cookie` exists under your chain data directory (e.g. `~/.bitcoin/.cookie`
  on mainnet, `~/.bitcoin/signet/.cookie` on signet).

If you use a custom `datadir`, pass the cookie path to Lord:

```bash
lord --cookie-file /path/to/bitcoin/.cookie wallet balance
```

### Chain flags

Run `bitcoind` on the same network Lord uses:

| Lord flag | `bitcoind` network | Default RPC port |
|-----------|-------------------|------------------|
| (default) mainnet | mainnet | 8332 |
| `--signet` | signet | 38332 |
| `--testnet` | testnet3 | 18332 |
| `--testnet4` | testnet4 | 48332 |
| `--regtest` | regtest | 18443 |

Lord's `--chain` must match the chain reported by `bitcoin-cli getblockchaininfo`.

### `txindex`: optional vs required

| Workload | `txindex=1` required? |
|----------|----------------------|
| `lord storage`, `lord filepack`, `lord commit` | **No** |
| Embedded calendar anchoring | **No** (pruned bitcoind is fine) |
| Block explorer, address/sat indexing (`lord server`) | **Yes** |

Without `txindex`, Lord runs in reduced explorer mode and reports status on
`/status`. Carbonado and commitment workflows continue normally.

To enable the full explorer:

```
txindex=1
```

in `bitcoin.conf`, or `bitcoind -txindex`. Sync until `bitcoin-cli getindexinfo`
reports `"txindex": { "synced": true, ... }`.

Syncing the Blockchain
----------------------

Run `bitcoind` (with `-txindex` only if you need the explorer) and wait until
`bitcoin-cli getblockcount` matches a public block explorer.

Leave `bitcoind` running while you use Lord. Lord connects over RPC; it does not
replace the full node.

Troubleshooting
---------------

| Symptom | Fix |
|---------|-----|
| `Could not connect to the server` | Start `bitcoind` |
| `Could not locate RPC credentials` | Pass `--cookie-file` or set RPC user/pass |
| `Method not found` on `listwallets` | Remove `disablewallet=1` from `bitcoin.conf` |
| `Bitcoin RPC server is on X but ord is on Y` | Literal CLI string (ord compat); means Lord's `--chain` does not match bitcoind — align them |
| Explorer slow / missing txs | Enable `txindex=1` and wait for sync |

Installing Lord
---------------

Build from source or install a release binary from the
[bitmask-stack/lord](https://github.com/bitmask-stack/lord) repository.

```bash
lord --version
```

Configuration
-------------

Lord loads settings from the command line, `ORD_*` environment variables, then a
config file. See [Settings](settings.md).

Default config probe order in the data directory:

1. `lord.yaml` (preferred)
2. `ord.yaml` (compatibility fallback)

Lord Wallet Commands
--------------------

Lord ships these `lord wallet` subcommands (inscription/rune wallet commands from
upstream ord are removed):

| Command | Purpose |
|---------|---------|
| `create` | Create a new descriptor wallet (default name: `ord`) |
| `restore` | Restore from mnemonic or descriptor |
| `dump` | Export wallet descriptors (contains private keys) |
| `receive` | Generate a receive address |
| `send` | Send bitcoin (sats) |
| `balance` | Wallet balance |
| `outputs` | List unspent outputs |
| `cardinals` | List unspent cardinal outputs |
| `addresses` | List wallet addresses |
| `transactions` | List wallet transactions |
| `sign` | Sign a message |
| `sweep` | Sweep assets from a private key |

With the `sats` feature enabled, `label` and `sats` are also available.

Most wallet commands require `lord server` (or `--server-url`) when the wallet
needs to sync against the Lord index. Commitment and calendar workflows do **not**
require `lord server`.

Creating a Wallet
-----------------

Ensure `bitcoind` is running with wallet enabled:

```bash
bitcoind          # add -txindex only for explorer
lord server       # only needed for index-backed wallet sync
```

Create the default wallet named `ord`:

```bash
lord wallet create
```

Save the printed mnemonic securely. Use a different name with `--name`:

```bash
lord wallet --name operator create
```

Receiving and Sending
---------------------

Receive funds:

```bash
lord wallet receive
```

Check balance and pending activity:

```bash
lord wallet balance
lord wallet transactions
lord wallet outputs
```

Send sats:

```bash
lord wallet send --fee-rate <FEE_RATE> <ADDRESS> <AMOUNT>
```

Restoring and Dumping
---------------------

Export descriptors (sensitive):

```bash
lord wallet dump
```

Restore from mnemonic:

```bash
lord wallet restore --from mnemonic
```

Restore from a descriptor file or stdin:

```bash
cat descriptor.json | lord wallet restore --from descriptor
```

Calendar Anchor Wallet
----------------------

The embedded OpenTimestamps calendar anchors merkle batches with **OP_RETURN**
transactions paid from bitcoind's **default loaded wallet** (not a named
`lord wallet` database). The anchor worker uses:

- `getnewaddress` for change
- `listunspent` for a spendable UTXO
- `signrawtransactionwithwallet` + `sendrawtransaction`

You can use the same bitcoind wallet you fund for normal operations, or a
dedicated wallet loaded in bitcoind for anchoring only. Lord does not yet support
`wallet_name` in calendar anchor config — keep one funded wallet loaded.

### Funding by chain

| Chain | Funding |
|-------|---------|
| **Mainnet** | Send real sats to an address from the loaded wallet; keep enough for recurring anchor fees |
| **Signet / testnet** | Use a faucet, then `bitcoin-cli -<network> getnewaddress` and fund the loaded wallet |
| **Regtest** | `bitcoin-cli -regtest -generate 101` (or mine to your address) |

Check anchor readiness:

```bash
lord calendar doctor
```

`calendar doctor` reports `wallet_balance_sats`, `wallet_error`, pending digests,
and last anchor txid when the embedded calendar and bitcoind RPC are reachable.

### Anchor fees and frequency

Anchor behavior is chain-aware (`anchor_config_for_chain`):

| Chain | Min interval between anchors | Poll interval |
|-------|------------------------------|---------------|
| Regtest, signet, testnet | ~200 ms | ~1 s |
| Mainnet | ~1 hour | ~30 s |

Fees are estimated from bitcoind (`estimatesmartfee` with fallback). Each anchor
batch includes up to 64 pending digests. Mainnet operators should keep a modest
UTXO balance so anchors are not delayed by `no spendable UTXO` errors.

See [Operator runbook](operator.md) for per-chain firewall, data directory, and
`calendar_listen` notes.