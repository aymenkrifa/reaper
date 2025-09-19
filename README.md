# Reaper 💀

A terminal application for monitoring and killing listening processes.

I made this small tool because I can never remember the exact `lsof` command and flags needed to see what's listening on which ports. It's just a simple wrapper around `lsof -i -P -n -sTCP:LISTEN` with a basic TUI for browsing and killing processes.

## Controls ⚡

- `↑/↓` or `j/k` - Navigate process list
- `/` - Enter search mode
- `s` - Sort by different criteria
- `Enter` - Kill selected process (with confirmation)
- `r` - Refresh process list
- `1-6` - Sort by different criteria
- `Esc` - Clear search or return to main view
- `q` - Quit application

## Installation 📦

**From Releases:**
Download the latest binary from the [Releases page](https://github.com/aymenkrifa/reaper/releases).

**From Source:**

```bash
git clone https://github.com/aymenkrifa/reaper.git
cd reaper
cargo run
```

## License 📄

MIT License - see [LICENSE](LICENSE) file.

## Acknowledgements 🙏

Inspired by [gruyere 🧀](https://github.com/savannahostrowski/gruyere) by Savannah Ostrowski built in Go. This project was built as a learning experience to explore Rust.
