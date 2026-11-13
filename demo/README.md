# Decora live proof

Run from Git Bash on Windows with Rust, Python 3, curl and jq on PATH.

```bash
bash demo/capture.sh
```

The default is **all loopback mocks**, even if provider keys exist. For a later,
explicitly authorized real run, enable available keys with one line:

```bash
DEMO_REAL=1 bash demo/capture.sh
```

`DEMO_MOCK_ONLY=1 bash demo/capture.sh` always selects mocks, even with
`DEMO_REAL=1`. Explicit process-level switches override switches in the env file.
Keep `.env.demo` for credentials only; use command-line switches so capture naming
also reflects the requested mode. Real-mode captures use `demo_real.cast` and
`demo_real.txt` beside the mock captures, leaving `demo.cast` / `demo.txt` intact.

If a provider's one-shot probe fails, keep it on a labelled mock with, for example,
`DEMO_MOCK_PROVIDERS=openai` (comma-separated aliases: `openai,gemini,typesafe,router`).
Do not run the full real capture until probes and the real-call budget have been
explicitly authorized. No probes are run automatically by these scripts.

## Credentials

Optional `demo/.env.demo` is sourced by Git Bash with `set -a`. Put only the
provider variables you actually have in it, using shell assignment syntax:

- `OPENAI_API_KEY`
- `GEMINI_API_KEY`
- `TYPESAFE_API_KEY`
- `OPENROUTER_API_KEY`

Never enable shell tracing or add commands that print secrets to that file.
The repository ignores `.env` and `.env.*` (except `.env.example`), including
`demo/.env.demo`; `run.sh` checks this before doing work. No credential template
or real credential file is generated. Backend YAML/JSON stores variable names,
not values. An ephemeral mock credential is generated into process environment
only. The gateway itself is unauthenticated and bound to 127.0.0.1 on a temporary
port: run on a trusted local machine, not a shared/public host.

Missing provider credentials select mocks independently, with `(loopback mock)`
in each affected header. Real providers say `(real API)`. Fallback, phantom and
JSON-as-data beats always remain mocks, so failures are deterministic and free.
A real run normally makes one OpenAI request and two Gemini requests, plus one
each for TypeSafe/OpenRouter only if their keys are present. Gateway retry policy
permits up to two attempts, so errors/timeouts can increase calls and billing.

## Endpoints and models

These are the values documented in `docs/PROTOCOL.md`, not newly verified
account entitlements. No chat-completions API is used.

| Provider | Native endpoint | Model |
| --- | --- | --- |
| OpenAI | `https://api.openai.com/v1/decisions` | `gpt-6-luna` |
| Gemini | `https://generativelanguage.googleapis.com/v1beta/decisions` | `gemini-decision-1` |
| TypeSafe | `https://api.typesafe.ai/v1/systemone` | `jev-latest` |
| OpenRouter | `https://openrouter.ai/api/alpha/decisions` | `typesafe/jev-1.13` |

## What runs

`run.sh` builds `cargo build --locked --release`, starts `mock.py` and the actual
release Decora executable, waits for `/healthz`, and sends eight Decisions HTTP
requests using curl, with jq validation. A single support-case request template
is used for the four provider aliases: only `model` changes. It includes system
and user roles, an `input_json` part, predicate, mixed `true` / `"true"` / `"售后"`
choice, and a two-level score. Responses are real gateway output, not canned
terminal text; only upstream fixture responses are synthetic.

Each provider kind has a deliberately different synthetic distribution; these are
not observations of real models. After the compact JSON/usage lines, `table.jq`
formats the same answers into four labelled rows with columns `provider | damaged p |
team choice (conf) | urgency score`. It makes no extra HTTP calls and does not
recompute or normalize probabilities. The table fits within 100 display columns.

Screen commands are concise command previews; transport additionally disables
proxies, sets timeouts, captures status and checks expected HTTP codes. Filenames
are runtime inputs under the temporary `$DATA` directory (internally `$RUNTIME`),
not persistent files in the current directory. Each provider's actual request is
`request.json`, generated from `support.json`. The full commands live in
`run.sh`; no manual request preparation is needed.

The Python mock speaks the native response shapes from `tests/offline.rs`,
constructs answers from the actual incoming question keys/criteria, and never
records headers. Its audit output proves the primary received one failed call
and the text backend received `image_url` inside `input_json`, not an image part.
The phantom choice probabilities sum to 0.93. The gateway must reject rather
than normalize them.

Temporary config, state and request/response files live under `demo/.runtime-*`
and are deleted on normal exit, failure and handled interruption. Both child
processes stop on exit. Build/server logs are suppressed, not captured or saved.
On a build error, run the build command separately for diagnostics.

## Capture

`capture.sh` uses the current **Git Bash executable**, not Windows' other `bash`
launchers. `capture.py` streams the actual merged stdout/stderr and saves:

- `E:/source/video/projects/decora-kiroween/assets/capture/demo.cast`
- `E:/source/video/projects/decora-kiroween/assets/capture/demo.txt`

The cast is asciicast v2 with monotonic real timing, 100 columns and 32 rows.
UTF-8 is used throughout; cast newlines are CRLF. Compact JSON may wrap naturally
at 100 terminal columns. There are no artificial typing delays; the release-build
wait is preserved. The capture is a timed pipe recording, not a pseudo-terminal,
which is sufficient for these noninteractive curl/jq commands. A rerun replaces
both files. Exit status propagates from `run.sh`.
