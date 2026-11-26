# BusCraft

A desktop tool for planning and commissioning KNX building installations, written in Rust with egui.
It runs on Windows, Linux and macOS and works on its own – no other KNX software is needed.

- Projects with topology, buildings, group addresses and devices in flexible stacked panels
- Import of `.knxproj` projects and `.knxprod` product files
- Bus connection over KNXnet/IP (tunnelling and routing) or a built-in simulation
- Group monitor, bus monitor, writing and reading group values
- Programming individual addresses and downloading applications
- Diagnostics (Monitors panel or **Diagnostics ▾** in the toolbar): device info, devices in
  programming mode (switch it on or off), individual address check and line scan
- Built-in MCP server, so an AI assistant can read and change projects, watch the group monitor
  and send values (see below)

## Build on Linux

1. Install a C linker and Rust (once):

   ```
   sudo apt install build-essential git          # Debian/Ubuntu; Fedora: sudo dnf install gcc git
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. Get the code and build it:

   ```
   git clone git@github.com:kai-berlin/buscraft.git
   cd buscraft
   cargo build --release
   ./target/release/buscraft
   ```

The first build downloads and compiles the libraries and takes a few minutes. File dialogs use
the desktop's portal (`xdg-desktop-portal`), which GNOME and KDE already have.

## Build on Windows

```
cargo build --release
```

With the GNU Rust toolchain, a complete MinGW `dlltool` is needed. Point to it in
`.cargo/config.toml` (not part of the repository), for example:

```toml
[target.x86_64-pc-windows-gnu]
rustflags = ["-C", "dlltool=C:/path/to/mingw64/bin/dlltool.exe"]
```

## Moving to another computer

The code is in this repository; your projects and imported products are stored locally:

| System  | Data folder |
|---------|-------------|
| Windows | `%APPDATA%\BusCraft` |
| Linux   | `~/.local/share/BusCraft` |
| macOS   | `~/Library/Application Support/BusCraft` |

1. On the old computer: **Back up everything…** (on the Projects page, or **💾 Backup** in the project toolbar) saves one `.zip` file.
2. Copy that file to the new computer (USB stick, cloud drive, …).
3. On the new computer: **Restore backup…** on the Projects page, and choose the file.

Product files (`.knxprod`) and projects (`.knxproj`) only need to be imported again if you
didn't make a backup. For the bus, the new computer must reach the KNX IP interface or router on the
network (UDP port 3671).

## AI assistants (MCP server)

While BusCraft runs, it offers an MCP server on this computer only: `http://127.0.0.1:47811/mcp`
(switch it off or change the port in **Settings › AI assistants**). Assistants that connect by URL
use that address. Assistants that start a program (such as Claude Desktop) use the bridge, which
passes everything on to the running app:

```json
{ "mcpServers": { "buscraft": { "command": "C:/path/to/buscraft.exe", "args": ["--mcp-stdio"] } } }
```

Add `"--port", "<port>"` to the args if you changed the port. **Copy settings** in Settings fills in
the right path. Tools: list/open projects, project overview, devices (list, details, add, change,
delete, parameters, rooms), group addresses (list, create, change, delete, name ranges, link and
unlink group objects), product catalog, bus status, group monitor telegrams, and writing or reading
group values. Changes to the open project show up at once and can be undone; changed devices
still have to be downloaded from the app.

## Other

Run `buscraft --screenshot out.png` to render one frame to a picture and exit.
`cargo test` runs the tests; some extra tests use sample files in `../reference` when present.
