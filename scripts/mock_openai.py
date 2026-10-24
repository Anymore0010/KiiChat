"""Throwaway OpenAI/Anthropic-compatible mock used to verify KiiChat end to end.

GET  /v1/models             -> three models
POST /v1/chat/completions   -> SSE chunks in Chat Completions shape
POST /v1/responses          -> SSE events in Responses shape
POST /v1/messages           -> SSE events in Messages shape (needs x-api-key)

Each endpoint asserts the shape it must receive, so a request built for the
wrong API fails loudly instead of silently streaming.
"""

import json
import os
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = 18080
MODELS = ["mock-mini", "mock-large", "mock-anthropic"]
# `MOCK_SLOW=1` stretches the completion to ~10 s, long enough to drive the
# stop button through UI Automation.
SLOW = os.environ.get("MOCK_SLOW") == "1"


def sse(handler, events):
    for event in events:
        handler.wfile.write(("data: " + json.dumps(event) + "\n\n").encode())
        handler.wfile.flush()
        time.sleep(0.12)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        print("%s - %s" % (self.address_string(), fmt % args), flush=True)

    def _json(self, payload, status=200):
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _sse_headers(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()

    def do_GET(self):
        if self.path.rstrip("/").endswith("/models"):
            self._json({
                "object": "list",
                "data": [
                    {
                        "id": "mock-mini",
                        "context_length": 32768,
                        "max_completion_tokens": 4096,
                    },
                    {
                        "id": "mock-large",
                        "context_length": 128000,
                        "max_tokens": 8192,
                    },
                    {"id": "mock-anthropic", "max_output_tokens": 8192},
                ],
            })
        else:
            self.send_error(404)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        payload = json.loads(self.rfile.read(length) or b"{}")
        path = self.path.rstrip("/")
        auth = self.headers.get("Authorization", "")
        key = self.headers.get("x-api-key", "")
        instructions = payload.get("instructions") or payload.get("system") or ""
        if isinstance(instructions, list):
            instructions = " ".join(
                part.get("text", "") for part in instructions if isinstance(part, dict)
            )
        print(
            "POST %s model=%s stream=%s auth=%s x-api-key=%s "
            "max_tokens=%s max_output_tokens=%s temperature=%s system=%r "
            "reasoning_effort=%s reasoning=%s thinking=%s"
            % (
                path,
                payload.get("model"),
                payload.get("stream"),
                bool(auth),
                bool(key),
                payload.get("max_tokens"),
                payload.get("max_output_tokens"),
                payload.get("temperature"),
                instructions,
                payload.get("reasoning_effort"),
                payload.get("reasoning"),
                payload.get("thinking"),
            ),
            flush=True,
        )

        if path.endswith("/chat/completions"):
            if not auth.startswith("Bearer ") or "messages" not in payload:
                return self._json(
                    {"error": {"message": "chat/completions 需要 Bearer 与 messages"}}, 400
                )
            if not isinstance(payload.get("max_tokens"), int):
                return self._json(
                    {"error": {"message": "chat/completions 需要 max_tokens"}}, 400
                )
            if "temperature" in payload and not isinstance(payload["temperature"], (int, float)):
                return self._json({"error": {"message": "temperature 必须是数字"}}, 400)
            effort = payload.get("reasoning_effort")
            if effort is not None and effort not in ("low", "medium", "high"):
                return self._json({"error": {"message": "reasoning_effort 不合法"}}, 400)
            self._sse_headers()
            if SLOW:
                for index in range(30):
                    sse(self, [{"choices": [{"delta": {"content": f"片段{index} "}}]}])
                    time.sleep(0.2)
                self.wfile.write(b"data: [DONE]\n\n")
                self.wfile.flush()
                self.close_connection = True
                return
            # reasoning first, then the answer, so the thinking strip is exercised.
            sse(self, [{"choices": [{"delta": {"reasoning_content": "先拆题…"}}]}])
            for piece in ["来自 ", "**chat** ", "completions。"]:
                sse(self, [{"choices": [{"delta": {"content": piece}}]}])
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
            self.close_connection = True
            return

        if path.endswith("/responses"):
            if not auth.startswith("Bearer ") or "input" not in payload:
                return self._json(
                    {"error": {"message": "responses 需要 Bearer 与 input"}}, 400
                )
            if not isinstance(payload.get("max_output_tokens"), int):
                return self._json(
                    {"error": {"message": "responses 需要 max_output_tokens"}}, 400
                )
            if "max_tokens" in payload:
                return self._json(
                    {"error": {"message": "responses 不应该收到 max_tokens"}}, 400
                )
            reasoning = payload.get("reasoning")
            if reasoning is not None and (
                not isinstance(reasoning, dict)
                or reasoning.get("effort") not in ("low", "medium", "high")
            ):
                return self._json({"error": {"message": "reasoning.effort 不合法"}}, 400)
            self._sse_headers()
            sse(self, [{"type": "response.created"}])
            sse(self, [{"type": "response.output_text.delta", "delta": "来自 "}])
            sse(self, [{"type": "response.output_text.delta", "delta": "**responses** "}])
            sse(self, [{"type": "response.output_text.delta", "delta": "接口。"}])
            sse(self, [{"type": "response.completed"}])
            self.close_connection = True
            return

        if path.endswith("/messages"):
            if not key:
                return self._json({"error": {"message": "messages 需要 x-api-key"}}, 401)
            if self.headers.get("anthropic-version") != "2023-06-01":
                return self._json({"error": {"message": "缺少 anthropic-version"}}, 400)
            if not payload.get("max_tokens"):
                return self._json({"error": {"message": "messages 需要 max_tokens"}}, 400)
            thinking = payload.get("thinking")
            if thinking is not None:
                if not isinstance(thinking, dict) or not isinstance(
                    thinking.get("budget_tokens"), int
                ):
                    return self._json({"error": {"message": "thinking 不合法"}}, 400)
                if payload["max_tokens"] <= thinking["budget_tokens"]:
                    return self._json(
                        {"error": {"message": "max_tokens 必须大于 thinking 预算"}}, 400
                    )
            self._sse_headers()
            sse(self, [{"type": "message_start"}])
            sse(self, [{"type": "content_block_start", "index": 0}])
            sse(self, [{"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "来自 "}}])
            sse(self, [{"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "**messages** "}}])
            sse(self, [{"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "（Claude）接口。"}}])
            sse(self, [{"type": "content_block_stop", "index": 0}])
            sse(self, [{"type": "message_stop"}])
            self.close_connection = True
            return

        self.send_error(404)


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()