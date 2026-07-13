<div align="center">

# CNTX

### An open-source AI coding assistant for your terminal.

Fast. Lightweight. Token-efficient.

</div>

---

## What is CNTX?

CNTX is an open-source CLI that brings AI into your terminal.

Built for developers, it provides a fast, lightweight interface for working with large language models while keeping context as token-efficient as possible.

The CLI source lives in [`main/`](main/README.md). Install it with:

```bash
cargo install cntx
```

---

## Features

- Token-efficient context handling
- Terminal-first workflow
- Open source (MIT)
- Support for OpenAI
- Support for Anthropic
- Support for Ollama Local
- Support for Ollama Cloud
- Sandboxed file writes
- Model routing by prompt size
- Interactive markdown chat
- Session persistence and skills
- Tool-use loop with read, write, edit, bash, glob, and grep

---

## Example

```bash
cntx

> Explain this repository.
> Refactor src/main.ts.
> Help me debug this error.
> Generate documentation for this project.
```

---

## Supported Providers

| Provider | Status |
|----------|--------|
| OpenAI | Supported |
| Anthropic | Supported |
| Ollama Local | Supported |
| Ollama Cloud | Supported |

---

## Contributing

Contributions are welcome. If you'd like to improve CNTX, feel free to open an issue or submit a pull request.

---

## License

MIT

---

<div align="center">

**Fast AI. Minimal overhead. Right from your terminal.**

</div>
