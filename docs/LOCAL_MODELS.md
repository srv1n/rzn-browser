# Running RZN against local models

`llm-auto` talks to OpenAI's Chat Completions / Responses API. Any server that speaks the same
protocol works — Ollama, LM Studio, vLLM, llama.cpp's `llama-server`, LiteLLM — by pointing
`OPENAI_BASE_URL` at it.

## Environment variables

| Variable | Meaning |
| --- | --- |
| `OPENAI_BASE_URL` | Base URL of the OpenAI-compatible endpoint. Unset = `https://api.openai.com/v1`. Trailing slashes are trimmed. |
| `OPENAI_API_BASE` | Alias for the above. `OPENAI_BASE_URL` wins when both are set. |
| `OPENAI_API_KEY` | Required only when talking to api.openai.com. Optional (and ignored) for local servers. |
| `OPENAI_MODEL_PLANNING` | Model name as your local server reports it. |
| `OPENAI_MAX_TOKENS` | Per-completion cap. Local models are slow — keep it modest. |

`LLM_PROVIDER` stays `openai`; only the base URL changes.

## Ollama

```bash
ollama serve
ollama pull qwen2.5:14b

export LLM_PROVIDER=openai
export OPENAI_BASE_URL=http://localhost:11434/v1
export OPENAI_MODEL_PLANNING=qwen2.5:14b
export OPENAI_MAX_TOKENS=2048
# no OPENAI_API_KEY needed

cargo run -p rzn_browser -- llm-auto "open the top Hacker News story and extract its title" \
  --url https://news.ycombinator.com --max-steps 10 --json
```

## LM Studio

Start the local server from the LM Studio UI (Developer → Start Server), then:

```bash
export LLM_PROVIDER=openai
export OPENAI_BASE_URL=http://localhost:1234/v1
export OPENAI_MODEL_PLANNING=qwen2.5-14b-instruct   # exact id from /v1/models
export OPENAI_API_KEY=lm-studio                     # optional, ignored

cargo run -p rzn_browser -- llm-auto "search for rust async book and open the first result" \
  --url https://duckduckgo.com --max-steps 12
```

Check the model id your server actually exposes:

```bash
curl -s "$OPENAI_BASE_URL/models" | jq -r '.data[].id'
```

## What model size is actually realistic

Honest expectations. Planning in `llm-auto` means reading a compressed DOM (often 10-30 KB of
context) and emitting a strict JSON action, sometimes as a tool call. That is a long-context,
strict-format task, which is where small models fall over first.

- **Under 7B** — not usable for planning. They break JSON structure, hallucinate element ids, and
  loop on the same action. Fine for nothing here.
- **7B-8B** (llama3.1:8b, qwen2.5:7b) — occasionally completes trivial single-page goals; expect
  frequent malformed-JSON retries. Useful for smoke-testing the plumbing, not for real runs.
- **14B-32B** (qwen2.5:14b/32b, mistral-small) — the practical floor for multi-step planning on a
  workstation. Slower per step, but structurally reliable enough to finish short goals.
- **70B+ / hosted** — what the deterministic prompts were actually tuned against.

Two more caveats:

- Local servers vary in tool-calling support. If tool calls fail, the planner falls back to the
  plain JSON path; that fallback is the well-tested one.
- **Replaying a saved workflow uses no LLM at all.** Once a workflow is cached under
  `RZN_WORKFLOWS_DIR`, replay is pure deterministic execution, so model size is irrelevant there.
  The realistic split is: plan once with the strongest model you can reach, then replay locally
  forever.
