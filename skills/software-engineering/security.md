# 15. Security

Security is part of implementation, not a final afterthought.

Check relevant risks including:

- authentication,
- authorization,
- secret handling,
- injection,
- path traversal,
- unsafe file handling,
- SSRF,
- XSS,
- CSRF,
- command execution,
- insecure deserialization,
- dependency risk,
- sensitive logging,
- rate limiting,
- abuse cases.

Never hard-code secrets.

Never expose credentials in logs, output, commits, or documentation.

Do not weaken security controls simply to make tests easier unless the change is explicitly isolated, safe, and appropriate.

---
