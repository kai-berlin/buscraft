# BusCraft

A desktop tool for planning and commissioning KNX building installations, written in Rust with egui.

- Projects with topology, buildings, group addresses and devices in flexible stacked panels
- Import of `.knxproj` projects and `.knxprod` product files
- Bus connection over KNXnet/IP (tunnelling and routing) or a built-in simulation
- Group monitor, bus monitor, writing and reading group values
- Programming individual addresses and downloading applications

## Build

```
cargo build --release
```

On Windows with the GNU Rust toolchain, a complete MinGW `dlltool` is needed. Point to it in
`.cargo/config.toml` (not part of the repository), for example:

```toml
[target.x86_64-pc-windows-gnu]
rustflags = ["-C", "dlltool=C:/path/to/mingw64/bin/dlltool.exe"]
```

Run `buscraft --screenshot out.png` to render one frame to a picture and exit.
