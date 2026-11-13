#!/usr/bin/env bash
# All providers are mocked unless DEMO_REAL=1 explicitly enables available keys.
set +x
set -euo pipefail
# Preserve explicit process-level safety switches over values in the env file.
MOCK_OVERRIDE="${DEMO_MOCK_ONLY-}"
REAL_OVERRIDE="${DEMO_REAL-}"
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
if [[ -f demo/.env.demo ]]; then
  set -a
  . demo/.env.demo
  set +a
fi
set +x
[[ -z "$MOCK_OVERRIDE" ]] || export DEMO_MOCK_ONLY="$MOCK_OVERRIDE"
[[ -z "$REAL_OVERRIDE" ]] || export DEMO_REAL="$REAL_OVERRIDE"
export COLUMNS=100
export PYTHONIOENCODING=utf-8
for tool in cargo curl jq python; do
  command -v "$tool" >/dev/null || { printf 'Missing required tool: %s\n' "$tool"; exit 1; }
done
git check-ignore -q demo/.env.demo || { printf 'Refusing to run: demo/.env.demo is not ignored.\n'; exit 1; }
RUNTIME="$(mktemp -d "$ROOT/demo/.runtime-XXXXXX")"
MOCK_PID=''; SERVER_PID=''
cleanup() {
  [[ -z "$SERVER_PID" ]] || kill "$SERVER_PID" 2>/dev/null || true
  [[ -z "$MOCK_PID" ]] || kill "$MOCK_PID" 2>/dev/null || true
  [[ -z "$SERVER_PID" ]] || wait "$SERVER_PID" 2>/dev/null || true
  [[ -z "$MOCK_PID" ]] || wait "$MOCK_PID" 2>/dev/null || true
  rm -rf -- "$RUNTIME"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
printf '# Release build; local gateway; no keys displayed\n'
if ! cargo build --locked --release >/dev/null 2>&1; then
  printf 'Release build failed; run cargo build --locked --release separately.\n'; exit 1
fi
printf 'release build: OK\n'
# Ephemeral mock credential exists only in process environments, never on disk.
export DEMO_LOOPBACK_KEY="$(python -I -c 'import secrets; print(secrets.token_urlsafe(32))')"
python -I "$ROOT/demo/mock.py" "$(cygpath -w "$RUNTIME")" >/dev/null 2>&1 &
MOCK_PID=$!
for ((i=0; i<100; i++)); do
  [[ ! -f "$RUNTIME/state.json" ]] || break
  kill -0 "$MOCK_PID" 2>/dev/null || { printf 'Mock startup failed.\n'; exit 1; }
  sleep 0.1
done
[[ -f "$RUNTIME/state.json" ]] || { printf 'Mock startup timed out.\n'; exit 1; }
BASE="$(jq -r .gateway "$RUNTIME/state.json")"
MOCK="$(jq -r .mock "$RUNTIME/state.json")"
BIN="$ROOT/target/release/decora.exe"
[[ -f "$BIN" ]] || BIN="$ROOT/target/release/decora"
"$BIN" "$(cygpath -w "$RUNTIME/config.json")" >/dev/null 2>&1 &
SERVER_PID=$!
healthy=0
for ((i=0; i<100; i++)); do
  if curl --noproxy '*' -fsS "$BASE/healthz" >"$RUNTIME/health.json" 2>/dev/null; then healthy=1; break; fi
  kill -0 "$SERVER_PID" 2>/dev/null || { printf 'Decora startup failed.\n'; exit 1; }
  sleep 0.1
done
[[ "$healthy" == 1 ]] || { printf 'Health check timed out.\n'; exit 1; }
printf '$ curl -s "$BASE/healthz" | jq -c .\n'
jq -ce 'select(.status == "ok")' "$RUNTIME/health.json"

# One request shared by every public provider alias; only model changes.
jq -n '{model:"decora-openai",input:[{role:"system",content:"你是售后分单助手。"},{role:"user",content:[{type:"input_text",text:"包裹破损，请今天换货。"},{type:"input_json",data:{order:"A17",damaged:true}}]}],questions:[{type:"predicate",name:"damaged",instructions:"包裹是否破损？"},{type:"choice",name:"team",instructions:"选择分单标签。",choices:[{value:true},{value:"true"},{value:"售后"}]},{type:"score",name:"urgency",instructions:"处理紧迫性。",levels:[{label:"低"},{label:"高"}]}]}' >"$RUNTIME/support.json"
post() {
  local path="$1" input="$2" expected="${3:-200}" status
  status="$(curl --noproxy '*' -sS --max-time 70 -o "$RUNTIME/response.json" -w '%{http_code}' \
    "$BASE$path" -H 'Content-Type: application/json' --data-binary "@$input")"
  if [[ "$status" != "$expected" ]]; then
    printf 'Unexpected HTTP %s (expected %s); response omitted for credential safety.\n' "$status" "$expected"
    return 1
  fi
}
for provider in openai gemini typesafe router; do
  label="$(jq -r --arg p "$provider" '.labels[$p]' "$RUNTIME/state.json")"
  printf '\n# Same OpenAI request -> decora-%s (%s)\n' "$provider" "$label"
  jq --arg model "decora-$provider" '.model=$model' "$RUNTIME/support.json" >"$RUNTIME/request.json"
  printf '$ curl -s "$BASE/v1/decisions" -H "Content-Type: application/json" -d @request.json | jq -c .answers\n'
  post /v1/decisions "$RUNTIME/request.json"
  jq -ce '.answers' "$RUNTIME/response.json"
  jq -c 'if .usage.decora then .usage.decora else "native usage (no usage.decora)" end' "$RUNTIME/response.json"
  jq -c --arg provider "decora-$provider ($label)" '{provider:$provider,answers:.answers}' \
    "$RUNTIME/response.json" >>"$RUNTIME/opinions.jsonl"
done
printf '\n# Four providers, one shape: synthetic opinions when labelled mock\n'
printf '$ jq -rs -f demo/table.jq "$DATA/opinions.jsonl"\n'
jq -rs -f "$ROOT/demo/table.jq" "$RUNTIME/opinions.jsonl"

label="$(jq -r .labels.gemini "$RUNTIME/state.json")"
printf '\n# Gemini -> Gemini: native keys preserved (%s)\n' "$label"
jq -n '{model:"decora-gemini",input:"包裹破损，请今天换货。",questions:{damage:{type:"noul",instructions:"包裹是否破损？"},route:{type:"choice",instructions:"分单",criteria:{support:"售后",sales:"销售"}},urgency:{type:"score",instructions:"紧迫性",criteria:["低","高"]}}}' >"$RUNTIME/gemini.json"
printf '$ curl -s "$BASE/v1beta/decisions" -H "Content-Type: application/json" -d @gemini.json | jq -c .\n'
post /v1beta/decisions "$RUNTIME/gemini.json"
jq -ce 'select(.answers.damage.type == "noul" and .answers.route.probabilities.support != null and .answers.urgency.legend["1"] == "高" and .usage.decora == null) | {model,answers,usage}' "$RUNTIME/response.json"

printf '\n# Fallback: primary 503 -> TypeSafe backup (loopback mock)\n'
jq '.model="decora-fallback" | .questions=[.questions[0]]' "$RUNTIME/support.json" >"$RUNTIME/fallback.json"
printf '$ curl -s "$BASE/v1/decisions" -H "Content-Type: application/json" -d @fallback.json | jq -c .answers\n'
post /v1/decisions "$RUNTIME/fallback.json"
jq -ce 'select(.usage.decora.provider == "typesafe") | .answers' "$RUNTIME/response.json"
curl --noproxy '*' -fsS "$MOCK/audit" | jq -ce 'select(.calls["/unavailable"] == 1) | {primary_503_calls:.calls["/unavailable"]}'

printf '\n# Phantom: distribution sum 0.93 is rejected (loopback mock)\n'
jq '.model="decora-phantom" | .questions=[.questions[1]]' "$RUNTIME/support.json" >"$RUNTIME/phantom.json"
printf '$ curl -s -w "HTTP %%{http_code}" "$BASE/v1/decisions" -d @phantom.json\n'
post /v1/decisions "$RUNTIME/phantom.json" 502
printf 'HTTP 502\n'
jq -ce 'select(.error.code == "upstream_protocol_error")' "$RUNTIME/response.json"

printf '\n# input_json.image_url is data -> text-only TypeSafe (loopback mock)\n'
jq '.model="decora-text" | .input=[{role:"user",content:[{type:"input_json",data:{image_url:"https://example.invalid/order.png",damaged:true}}]}] | .questions=[.questions[0]]' "$RUNTIME/support.json" >"$RUNTIME/json-data.json"
printf '$ curl -s "$BASE/v1/decisions" -H "Content-Type: application/json" -d @json-data.json | jq -c .answers\n'
post /v1/decisions "$RUNTIME/json-data.json"
jq -ce '.answers' "$RUNTIME/response.json"
curl --noproxy '*' -fsS "$MOCK/audit" | jq -ce 'select(.input_json_image_url_is_data == true) | {input_json_image_url_is_data}'
printf '\n# Done: all assertions passed; gateway and mocks stop automatically\n'
