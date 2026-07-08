# Release checklist (operator — GPG signed tags)

Agent prepares drafts only. **You** run version bumps, commits, tags, and
`cargo publish` with your GPG key.

---

## Monorepo vs standalone clone

Lord's workspace depends on sibling crates. Two supported layouts:

### Monorepo (recommended for development)

```
surmount/
├── lord/
├── carbonado/
└── bao-tree/
```

- `just deps-check` verifies siblings exist.
- [`.cargo/config.toml`](../.cargo/config.toml) patches carbonado's git `bao-tree`
  dependency to `../bao-tree` so Cargo resolves one revision.

**Commit `.cargo/config.toml`** with lord so every monorepo checkout builds without
extra setup.

### Standalone `lord` clone (CI, contributors, release tag)

A lone `git clone` of lord **without** siblings will fail until you either:

1. **Clone siblings** next to lord (same as monorepo), or
2. **Remove the patch** and rely on carbonado's published/git `bao-tree` (only
   after carbonado publishes a release that pins a compatible `bao-tree` revision).

Until carbonado publishes, document in release notes:

> Build from the bitmask-stack monorepo layout (`lord` + `carbonado` + `bao-tree`
> under `surmount/`) or apply the `.cargo/config.toml` patch with sibling paths.

`cargo publish` for the `lord` crate on crates.io does **not** include
`lord-storage`'s path deps — publishing the workspace crates requires resolving
carbonado/bao-tree to crates.io or git versions first (deferred; see Tier 3 /
upstream carbonado release).

---

## Pre-release verification

```bash
just deps-check
just ci-local          # smoke → clippy → forbid → fmt → test-all
just ceremony          # regtest dry-run
# Optional human gate:
just ceremony signet         # encode + signet instructions; fill ACCEPTANCE-SIGNET.md
```

---

## Version bump

1. Choose next semver (suggested **0.28.0** for operator stack + `--full` verify).
2. Bump `version` in root [`Cargo.toml`](../Cargo.toml) (workspace `lord` package).
3. Bump crate versions if publishing sub-crates later (`lord-calendar`, etc.).

```bash
# After editing Cargo.toml:
cargo check
just ci-local
```

Existing just recipes (optional):

```bash
just prepare-release        # opens editor, creates release branch — uses git; you run it
just update-changelog       # appends git log lines — review before release
```

---

## CHANGELOG draft — 0.28.0

Paste into [`CHANGELOG.md`](../CHANGELOG.md) **above** the current top entry when
cutting the release. Adjust links to `bitmask-stack/lord` tags.

```markdown
[0.28.0](https://github.com/bitmask-stack/lord/releases/tag/0.28.0) - YYYY-MM-DD
--------------------------------------------------------------------------

### Added
- Embedded chain-aware OpenTimestamps calendar (`lord calendar serve|doctor|url`, `calendar_enabled` in `lord server`)
- `commit upgrade` for Bitcoin-attested OTS proofs after calendar anchor
- `commit verify --full` cross-store binding (LMDB, OTS, breccia, carbonado header)
- `lord-calendar` crate: anchor worker, merkle batching, per-chain poll intervals
- Txindex fallback for `/tx` when bitcoind lacks `txindex=1`
- Integration smoke tests (`just smoke`); `just deps-check`, `ci-local`, `ceremony`
- Operator runbook, commitments quickstart, signet acceptance template
- CHIP LTP-0001 draft (Lord Timestamp Protocol — iroh, storage market annex)

### Changed
- CI runs smoke tests first on `lord` branch
- README documents monorepo layout and mainnet-first chain table
- Explorer `/commitment/*`, `/content/*`; inscription/rune routes remain 410 Gone

### Fixed
- Integration test hangs from background `calendar serve` (tracked `BackgroundProcess`)

### Misc
- `.cargo/config.toml` patches `bao-tree` for monorepo builds
- Deferred: carbonado crate changes, RGB, LTP B1+ implementation
```

---

## Tag and publish (human only)

```bash
version=0.28.0   # match Cargo.toml

git checkout lord
git pull origin lord
# Ensure CHANGELOG + version bump committed and signed
git tag -s "$version" -m "Release $version"
git push origin "$version"
```

GitHub release notes: copy the **0.28.0** section from CHANGELOG. Mention monorepo
clone requirement and link to [ACCEPTANCE-SIGNET.md](ACCEPTANCE-SIGNET.md) for
signet operators.

`cargo publish` from a clean clone is **blocked** until `lord-storage` path deps
resolve on crates.io — use `just publish-tag-and-crate` only after dependency
strategy is decided (coordinate with carbonado/bao-tree releases).

---

## Post-release

- [ ] Signet acceptance checklist signed (`docs/ACCEPTANCE-SIGNET.md`)
- [ ] Mainnet operator smoke on staging host (`lord calendar doctor`)
- [ ] CHIP LTP-0001 review before Tier 3 (iroh mempool B1)