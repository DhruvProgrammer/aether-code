---
title: "AETHER — Configuration Reference"
status: "canonical"
audience: "users / ops"
summary: "config.toml shape for the OpenAI-compatible agent. No hardcoded keys; env-var references only."
---

# Configuration — `aether`

A fresh `aether init` writes the defaults below to `~/.aether/config.toml`. Values are
examples, never forced. **No API key is ever stored in this file** — only the env var name
to read (spec §6, §25).

---

## 1. Top-level shape

```toml
[agent]
controller_model = "controller"
executor_model   = "executor"
max_iterations   = 30
routing_policy   = "balanced"   # quality | balanced | cheap | fast

[memory]
enabled           = true
backend           = "embedded"  # embedded | local_server
graph_store       = "redb"
vector_store      = "usearch"
embedding_provider = "openai_compatible"
memory_top_k      = 8

[permissions]
read  = "allow"
edit  = "allow"
bash  = "ask"
delete = "ask"
git_commit = "ask"
network = "ask"

[context]
max_tokens = 128000

[display]                 # see design.md
theme   = "light"         # light is canonical; dark = opt-in
accent  = "still-blue"
emoji   = false
```

---

## 2. Models — OpenAI-compatible (the only required provider for v1)

```toml
[models.controller]
provider     = "openai_compatible"
base_url     = "https://api.openai.com/v1"
model        = "gpt-4o-mini"
api_key_env  = "OPENAI_API_KEY"

[models.executor]
provider     = "openai_compatible"
base_url     = "https://api.openai.com/v1"
model        = "gpt-4o"
api_key_env  = "OPENAI_API_KEY"

[models.fallback_executor]
provider     = "openai_compatible"
base_url     = "https://openrouter.ai/api/v1"
model        = "openai/gpt-4o"
api_key_env  = "OPENROUTER_API_KEY"

[models.fast]
provider     = "openai_compatible"
base_url     = "https://api.openai.com/v1"
model        = "gpt-4o-mini"
api_key_env  = "OPENAI_API_KEY"
```

Swapping GPT ↔ MiniMax ↔ GLM ↔ Ollama ↔ local vLLM requires **editing config only** — never
agent code (spec §6). Any endpoint speaking `/v1/chat/completions` works.

### Optional: non-standard request fields
```toml
[models.executor.extra_body.chat_template_kwargs]
thinking = true
reasoning_effort = "high"
```
Merged verbatim into the JSON body (jcode-style `extra_body`).

---

## 3. Permissions matrix (spec §14)

| Permission | Values | Note |
|---|---|---|
| `read` `edit` `bash` `delete` `network` | `allow` \| `deny` \| `ask` | dangerous cmds always `ask` |
| dangerous commands | — | `rm -rf`, `sudo`, `git reset --hard`, `git push --force` → forced `ask` |

---

## 4. Env file (secrets)

`~/.aether/<profile>.env`
```
OPENAI_API_KEY=sk-...
```
Key never written to `config.toml`. Use `--api-key-env NAME` to reference an existing var.

---

## See also
- [architecture.md](./architecture.md) — `aether-config`, `aether-models`
- [plan.md](./plan.md) — provider strategy
- [design.md](./design.md) — `[display]` theme tokens
- [roadmap.md](./roadmap.md) — `aether init` behaviour
