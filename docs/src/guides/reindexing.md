Reindexing
==========

Sometimes the `lord` database must be reindexed, which means deleting the
database and restarting the indexing process with either `lord index update` or
`lord server`. Reasons to reindex are:

1. A new major release of lord, which changes the database scheme
2. The database got corrupted somehow

Lord stores the cardinal block index as a heed3 LMDB environment directory named
`index`. There is **no migration** from the legacy `index.redb` redb file. If
`index.redb` is present in the chain data directory, `Index::open` fails with an
actionable error instructing the operator to delete `index.redb` and re-index.

Wallet metadata is stored as a heed3 LMDB environment directory at
`{data_dir}/wallets/<name>/` (schema version 2). There is **no migration** from
legacy `wallets/<name>.redb` files. If a `.redb` wallet file is present,
`WalletStore::open` fails with an actionable error instructing the operator to
delete the `.redb` file and recreate the wallet with `lord wallet create` (or
`lord wallet restore`).

By default the data directory is stored in different locations depending on
your operating system. Lord inherits ord's default paths (`ord` subdirectory)
for compatibility; operators may use `--datadir` to point at a `lord`-specific
directory instead.

|Platform | Value                                            | Example                                      |
| ------- | ------------------------------------------------ | -------------------------------------------- |
| Linux   | `$XDG_DATA_HOME`/ord or `$HOME`/.local/share/ord | /home/alice/.local/share/ord                 |
| macOS   | `$HOME`/Library/Application Support/ord          | /Users/Alice/Library/Application Support/ord |
| Windows | `{FOLDERID_RoamingAppData}`\ord                  | C:\Users\Alice\AppData\Roaming\ord           |

Mainnet uses `{data_dir}/index` and `{data_dir}/wallets/<name>/`. Other chains
use `{data_dir}/{chain}/index` and `{data_dir}/{chain}/wallets/<name>/` (for
example `{data_dir}/regtest/index` and `{data_dir}/regtest/wallets/ord/`).

So to delete the index database and reindex on macOS you would run the
following commands in the terminal:

```bash
rm -rf ~/Library/Application\ Support/ord/index
lord index update
```

To recreate a wallet after deleting a legacy `wallets/<name>.redb` file:

```bash
rm -f ~/Library/Application\ Support/ord/wallets/ord.redb
lord wallet create
```

You can also set the location of the data directory yourself with `lord
--datadir <DIR> index update` or give the index a specific path with `lord
--index <DIR> index update`.