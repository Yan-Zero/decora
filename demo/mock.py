"""Loopback-only native Decisions fixtures; never records headers or credentials."""
import argparse
import json
import os
from pathlib import Path
import socket
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

# Deliberately different synthetic opinions, never claims about real models.
# kind: (predicate probability, three-way choice probabilities, score P(high))
OPINIONS = {
    "openai": (0.91, (0.05, 0.10, 0.85), 0.90),
    "gemini": (0.84, (0.10, 0.20, 0.70), 0.75),
    "typesafe": (0.88, (0.15, 0.10, 0.75), 0.80),
    "openrouter": (0.79, (0.20, 0.15, 0.65), 0.65),
}

PROVIDERS = {
    "openai": ("OPENAI_API_KEY", "gpt-6-luna", "https://api.openai.com/v1/decisions"),
    "gemini": ("GEMINI_API_KEY", "gemini-decision-1", "https://generativelanguage.googleapis.com/v1beta/decisions"),
    "typesafe": ("TYPESAFE_API_KEY", "jev-latest", "https://api.typesafe.ai/v1/systemone"),
    "router": ("OPENROUTER_API_KEY", "typesafe/jev-1.13", "https://openrouter.ai/api/alpha/decisions"),
}
COUNTS = {}
JSON_DATA_SEEN = False


def fixture(body: dict[str, Any], kind: str, phantom: bool = False) -> dict[str, Any]:
    global JSON_DATA_SEEN
    predicate_p, choice_probs, high_p = OPINIONS[kind]
    score_probs = [round(1.0 - high_p, 2), high_p]
    score = sum(i * p for i, p in enumerate(score_probs))
    source = body.get("state", body.get("input"))
    if isinstance(source, str) and '"image_url"' in source:
        decoded = json.loads(source)
        JSON_DATA_SEEN = any(
            part.get("type") == "input_json" and "image_url" in part.get("data", {})
            for msg in decoded.get("data", []) for part in msg.get("content", [])
        )
    answer_list: list[dict[str, Any]] = []
    answer_map: dict[str, dict[str, Any]] = {}
    questions = body["questions"]
    pairs = enumerate(questions) if isinstance(questions, list) else questions.items()
    for key, q in pairs:
        t = q["type"]
        a: dict[str, Any] = {"type": t}
        if kind == "openai":
            a["name"] = q.get("name")
        if t in ("predicate", "noul"):
            a["probability" if t == "predicate" else "noul"] = predicate_p
        elif t == "choice":
            values = [c["value"] for c in q["choices"]] if kind == "openai" else list(q["criteria"])
            probs = list(choice_probs) if len(values) == 3 else [round(1.0 - choice_probs[-1], 2), choice_probs[-1]]
            if phantom:
                probs[-1] -= 0.07
            a.update(choice=values[-1], confidence=choice_probs[-1])
            a["probabilities"] = ([{"value": v, "probability": p} for v, p in zip(values, probs)]
                                  if kind == "openai" else dict(zip(values, probs)))
        elif t == "score":
            a.update(score=score, confidence=max(score_probs))
            if kind == "openai":
                a["probabilities"] = [{"value": i, "label": level["label"], "probability": p}
                                      for i, (level, p) in enumerate(zip(q["levels"], score_probs))]
            else:
                a["legend"] = {str(i): c for i, c in enumerate(q["criteria"])}
                a["probabilities"] = {str(i): p for i, p in enumerate(score_probs)}
        if kind == "openai":
            answer_list.append(a)
        else:
            answer_map[str(key)] = a
    usage: dict[str, Any]
    if kind == "openai":
        usage = {"input_tokens": 100, "input_tokens_details": {"cached_tokens": 10, "cache_write_tokens": 20},
                 "output_tokens": 4, "output_tokens_details": {"reasoning_tokens": 2}, "total_tokens": 104}
    elif kind == "gemini":
        usage = {"total_input_tokens": 120, "total_output_tokens": 0, "total_tokens": 120,
                 "input_tokens_by_modality": [{"modality": "text", "tokens": 120}]}
    else:
        usage = {"input_tokens": 100, "output_tokens": 4}
        if kind == "openrouter":
            usage["cost"] = 0.0001
    result: dict[str, Any] = {"answers": answer_list if kind == "openai" else answer_map, "usage": usage}
    if kind != "gemini":
        result["model"] = body["model"]
    return result


class Handler(BaseHTTPRequestHandler):
    def log_message(self, format: str, *args: Any) -> None:
        pass

    def reply(self, status, body):
        raw = json.dumps(body, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self):
        self.reply(200, {"calls": COUNTS, "input_json_image_url_is_data": JSON_DATA_SEEN})

    def do_POST(self):
        COUNTS[self.path] = COUNTS.get(self.path, 0) + 1
        if self.path == "/unavailable":
            self.reply(503, {"error": "demo primary unavailable"})
            return
        try:
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            kind = self.path.strip("/")
            self.reply(200, fixture(body, "openai" if kind == "phantom" else kind, kind == "phantom"))
        except Exception:
            self.reply(400, {"error": "invalid demo fixture request"})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("runtime")
    args = parser.parse_args()
    runtime = Path(args.runtime)
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    mock_url = f"http://127.0.0.1:{server.server_port}"
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    backends, aliases, labels = {}, {}, {}
    real_enabled = os.getenv("DEMO_REAL") == "1" and os.getenv("DEMO_MOCK_ONLY") != "1"
    forced_mocks = set(os.getenv("DEMO_MOCK_PROVIDERS", "").split(","))
    for alias, (env, model, endpoint) in PROVIDERS.items():
        kind = "openrouter" if alias == "router" else alias
        real = real_enabled and alias not in forced_mocks and bool(os.getenv(env, "").strip())
        backends[alias] = {"kind": kind, "model": model, "endpoint": endpoint if real else f"{mock_url}/{kind}",
                           "api_key_env": env if real else "DEMO_LOOPBACK_KEY", "priority": 10, "weight": 1,
                           "capabilities": {"question_types": ["predicate", "choice", "score"], "images": False, "max_questions": 400}}
        aliases[f"decora-{alias}"] = [alias]
        labels[alias] = "real API" if real else "loopback mock"
    for name, kind, path, priority in [("down", "openai", "unavailable", 0),
                                        ("backup", "typesafe", "typesafe", 10),
                                        ("phantom", "openai", "phantom", 0)]:
        backends[name] = {"kind": kind, "model": "demo-fixture", "endpoint": f"{mock_url}/{path}",
                          "api_key_env": "DEMO_LOOPBACK_KEY", "priority": priority, "weight": 1,
                          "capabilities": {"question_types": ["predicate", "choice", "score"], "images": False}}
    aliases.update({"decora-fallback": ["down", "backup"], "decora-phantom": ["phantom"], "decora-text": ["backup"]})
    config = {"listen": f"127.0.0.1:{port}", "allow_http_loopback": True, "usage_policy": "compatible",
              "max_attempts": 2, "attempt_timeout_ms": 30000, "request_timeout_ms": 65000,
              "backends": backends, "models": aliases}
    (runtime / "config.json").write_text(json.dumps(config), encoding="utf-8")
    (runtime / "state.json").write_text(json.dumps({"gateway": f"http://127.0.0.1:{port}", "mock": mock_url, "labels": labels}), encoding="utf-8")
    server.serve_forever()


if __name__ == "__main__":
    main()
