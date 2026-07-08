# Commit message drafts (GPG — operator signs)

Agent does **not** run `git commit`, `git push`, or `git tag`. Use one of the
options below when you are ready to sign.

---

## Option A — single commit (recommended)

**Subject:**

```
Lord operator stack: calendar, verify --full, justfile workflows, CHIP LTP-0001
```

**Body:**

```
Embedded chain-aware OpenTimestamps calendar (lord-calendar crate), commit
upgrade/verify with Bitcoin attestation and cross-store --full binding, txindex
fallback, and operator documentation.

Tier 1 tooling:
- just deps-check, smoke, ci-local, ceremony; smoke-first CI on lord branch
- commit verify --full: LMDB ↔ OTS ↔ breccia ↔ carbonado header binding
- Draft chip-ltp-0001; ACCEPTANCE-SIGNET template; README monorepo layout

Includes .cargo/config.toml bao-tree patch for monorepo builds. See
docs/RELEASE.md for standalone-clone strategy and release checklist.

Co-authored-by: <your GPG identity>
```

**Stage everything (including untracked):**

```bash
git add -A
# Review: git diff --cached --stat
git commit -S -F docs/COMMIT_MESSAGE.md   # or paste subject/body manually
```

**Files in this commit (82 tracked + 3 untracked):**

| Area | Paths |
|------|-------|
| CI / tooling | `.github/workflows/ci.yaml`, `justfile`, `.cargo/config.toml` |
| Calendar crate | `crates/lord-calendar/**` |
| Commit / storage | `crates/lord-commit/**`, `crates/lord-storage/**` |
| CLI | `src/calendar.rs`, `src/subcommand/calendar/**`, `src/subcommand/commit/**`, `src/index/txindex.rs`, … |
| Tests | `tests/smoke.rs`, `tests/commit.rs`, `tests/command_builder.rs`, `tests/txindex.rs`, … |
| Docs | `docs/ACCEPTANCE-SIGNET.md`, `docs/COMMIT_MESSAGE.md`, `docs/RELEASE.md`, `docs/src/lord/chip-ltp-0001.md`, `docs/src/guides/{commitments,operator}.md`, … |
| Config | `lord.yaml`, `ord.yaml`, `Cargo.toml`, `Cargo.lock`, `README.md` |

Drop `.log` before commit if it is accidental debug output:

```bash
git reset HEAD .log 2>/dev/null; rm -f .log
```

---

## Option B — two commits (history slice)

### Commit 1 — feature stack

**Subject:** `Add embedded OTS calendar and commitment verify --full`

Stage: `crates/lord-calendar/**`, `crates/lord-commit/**`, `crates/lord-storage/**`,
`src/**` (calendar, commit, index/txindex), `tests/**` (except smoke-only if you
prefer commit 2), `lord.yaml`, `ord.yaml`, `Cargo.toml`, `Cargo.lock`,
`crates/mockcore/**`.

### Commit 2 — operator docs + CI + justfile

**Subject:** `Operator docs, justfile workflows, CHIP LTP-0001 draft`

Stage: `justfile`, `.github/workflows/ci.yaml`, `.cargo/config.toml`, `README.md`,
`docs/**`, remaining test/doc-only tweaks.

---

## Option C — three commits (review-friendly)

1. `lord-calendar: embedded anchor worker and HTTP calendar`
2. `commit: upgrade, attestation verify, cross-store --full`
3. `docs/ci: justfile smoke, operator runbook, CHIP LTP-0001`

Use `git add -p` per slice; same file list as Option A.

---

## After commit (human only)

- Signet acceptance: `just ceremony signet` then fill `docs/ACCEPTANCE-SIGNET.md`
- Release: follow `docs/RELEASE.md` (version bump, CHANGELOG, `git tag -s`)