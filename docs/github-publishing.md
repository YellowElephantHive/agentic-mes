# Publishing Agentic MES on GitHub

## Project name

- Product name: **Agentic MES**
- GitHub repository: **`agentic-mes`**
- Suggested description: **A source-grounded, auditable Agent-as-a-Service MES prototype in Rust.**

`Agentic MES` keeps the purpose clear: this is an agent layer for Manufacturing
Execution Systems, not a replacement for an existing MES. The lowercase,
hyphenated repository name follows common GitHub and Rust project naming.

## Repository policy

The repository stores source code, reproducibility instructions, dataset
provenance and evaluation results. It does not store downloaded dataset bytes,
credentials or local build/runtime state.

### Commit these files

| Path or file | Why it belongs in Git |
|---|---|
| `crates/` | Rust application code, dataset adapters and tests |
| `migrations/` | Reproducible database schema |
| `scripts/` | Dataset acquisition and MVP reproduction commands |
| `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` | Reproducible Rust toolchain and dependency graph |
| `Dockerfile`, `docker-compose.yml` | Reproducible local services |
| `.github/workflows/` | Continuous integration checks |
| `.env.example` | Environment-variable names and safe example values only |
| `.gitignore`, `.dockerignore` | Secret, dataset and build-output exclusions |
| `README.md`, `docs/` | Architecture, usage and milestone evidence |
| `reports/track-a/REPORT.md`, `reports/track-b/REPORT.md` | Separate evaluation results for both mandatory tracks |

`Cargo.lock` is intentionally committed because this repository builds an
application and should resolve the same dependency versions on another machine.

### Do not commit these files

| Path or data | Reason |
|---|---|
| `.env` | May contain an API key or other machine-specific configuration |
| `data/evaluation/` | Downloaded and extracted third-party dataset bytes |
| `target/` | Rebuildable Rust artifacts |
| Database volumes or dumps | Large local runtime state and possible sensitive data |
| Logs, coverage output and editor state | Rebuildable or machine-specific output |
| API keys, access tokens, certificates or private keys | Credentials must never enter Git history |
| Private manufacturing records | May contain confidential or regulated information |

The current `.gitignore` already excludes `.env`, `data/evaluation/`, `target/`
and `.DS_Store`.

If a secret is committed accidentally, deleting it in a later commit is not
enough. Revoke or rotate the secret immediately, then remove it from Git
history before publishing.

## Dataset distribution strategy

Do not commit the Track A or Track B dataset archives, even when they are small
enough for GitHub. Keep the authoritative source URL and pinned SHA-256 digest
in the acquisition script instead:

```bash
./scripts/fetch-evaluation-datasets.sh
```

The script downloads the published UCI archives for both tracks, verifies their
checksums, and extracts them under the ignored `data/evaluation/` directory. A
checksum mismatch or unavailable source must fail the run; the project must
never generate replacement manufacturing values or silently substitute another
dataset.

This design preserves the distinction between:

- source metadata and provenance, which are committed;
- original downloaded records, which remain unchanged locally;
- normalized adapter output, which remains traceable to its source record; and
- evaluation reports, which are committed separately for Track A and Track B.

If a future dataset becomes too large or the upstream host becomes unreliable,
publish a versioned manifest containing its official source, version, license,
checksum and expected files. Use licensed object storage or a GitHub Release as
a mirror only when redistribution is permitted. Prefer this over putting
dataset bytes in normal Git history. Git LFS is a fallback for files that must
be versioned directly, but every collaborator then needs Git LFS and the
repository becomes subject to LFS storage and bandwidth quotas.

GitHub warns about regular Git files larger than 50 MiB and blocks files larger
than 100 MiB. See [About large files on GitHub](https://docs.github.com/en/repositories/working-with-files/managing-large-files/about-large-files-on-github).

## Clean-clone workflow

### Prerequisites

- Git
- a Rust toolchain compatible with `rust-toolchain.toml`
- Docker with Docker Compose
- `curl`
- `unzip`
- either `sha256sum` or `shasum`

### Run local code checks

```bash
git clone https://github.com/YellowElephantHive/agentic-mes.git
cd agentic-mes
cp .env.example .env

cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

These checks use the mock provider and do not require the maintainer's LLM API
key.

### Download only the evaluation datasets

```bash
./scripts/fetch-evaluation-datasets.sh
```

No manual browser download is required.

### Reproduce the complete MVP acceptance flow

```bash
./scripts/reproduce-mvp.sh
```

This command starts PostgreSQL, loads the deterministic simulator data, runs the
source-grounded walkthrough, downloads and verifies both published datasets,
imports them through their dataset-specific adapters, and evaluates Track A and
Track B independently.

The MVP may be declared complete only when the final output contains both:

```text
Track A: PASS
Track B: PASS
```

A pass from one track cannot compensate for a failure, missing dataset or
unevaluated state in the other track.

## Optional real-provider setup

The reproducible MVP checks do not need a real-provider key. To test M6 with a
real provider, each user supplies their own local environment values in `.env`.
For example, the generic HTTP adapter recognizes these names:

```dotenv
AGENT_PROVIDER=http
LLM_DATA_POLICY=external_approved
LLM_HTTP_ENDPOINT=https://provider.example/v1/chat
LLM_HTTP_MODEL=provider-model-name
LLM_HTTP_API_KEY=replace-with-your-own-key
```

For a local Ollama service, no cloud API key is necessary:

```dotenv
AGENT_PROVIDER=ollama
LLM_DATA_POLICY=local_only
OLLAMA_ENDPOINT=http://host.docker.internal:11434/api/chat
OLLAMA_MODEL=your-installed-model
```

Never put a real value into `.env.example`, documentation, test fixtures or CI
configuration committed to the repository. Store CI secrets in the repository
host's encrypted secret settings and expose them only to the job that needs
them.

## Pre-push checklist

Stage the repository, then inspect exactly what Git will publish:

```bash
git add .
git status --short
git diff --cached --stat
```

Confirm that protected paths are ignored:

```bash
git check-ignore -v .env data/evaluation target
```

Confirm that none of them is already tracked:

```bash
git ls-files | rg '^(data/evaluation/|target/|\.env$)'
```

The last command should print nothing. Finally, search the staged diff for
unexpected credential material and review all files before committing and
pushing.
