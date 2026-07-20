# Reaper 💀

A terminal application for monitoring and killing listening processes.

I made this small tool because I can never remember the exact `lsof` command and flags needed to see what's listening on which ports. It's a basic TUI for browsing and killing listening processes — it reads `/proc` directly, so it doesn't need `lsof` at all.

## Controls

- `↑/↓` - Navigate process list
- `/` - Enter search mode
- `s` - Cycle through sort columns
- `1-7` - Sort by a specific column (press again to flip direction)
- `a` - Show/hide restricted processes (other users' listeners)
- `Enter` - Kill selected process (with confirmation)
- `r` - Refresh process list
- `Esc` - Clear search or return to main view
- `q` - Quit application

## Installation

Reaper is Linux-only (it reads `/proc` directly). To see and kill processes owned by other users, run it with `sudo`.

**From Releases** (no Rust needed — static binary, works on any distro):

```bash
curl -fsSL https://github.com/aymenkrifa/reaper/releases/latest/download/reaper-x86_64-unknown-linux-musl.tar.gz | tar -xz
sudo install reaper /usr/local/bin/
```

For ARM machines, replace `x86_64` with `aarch64`. All builds and checksums are on the [Releases page](https://github.com/aymenkrifa/reaper/releases).

**With Cargo** (requires Rust 1.85+):

```bash
cargo install --git https://github.com/aymenkrifa/reaper --locked
```

**From source:**

```bash
git clone https://github.com/aymenkrifa/reaper.git
cd reaper
cargo install --path . --locked
```

## License

MIT License - see [LICENSE](LICENSE) file.

## Acknowledgements

Inspired by [gruyère 🧀](https://github.com/savannahostrowski/gruyere) by Savannah Ostrowski built in Go. This project was built as a learning experience to explore Rust.
