# Arch Terminal for Windows

A native Windows Terminal startup renderer based on [areofyl/fetch](https://github.com/areofyl/fetch), adapted for Windows in Rust. It renders the Arch silhouette with sub-character resolution and animation alongside Fastfetch system information

## Files

- `src/main.rs` — original personalized native renderer (Windows APIs, no WSL)
- `config/startup.jsonc` — Fastfetch modules, icons and layout
- `scripts/startup-original.ps1` — original PowerShell entry point, retained as a reference
- `UPSTREAM-LICENSE.txt` — upstream ISC license

## Build

On Windows, install the Rust toolchain, then run:

```powershell
rustc --edition 2024 -O src/main.rs -o arch-terminal.exe
```

Install Fastfetch and make sure `fastfetch` is on PATH. Start with `./arch-terminal.exe --shell=PowerShell`

## Integration

The existing laptop profile loads `startup.ps1` from `Documents/fetch-bin-arch/windows-fetch`, which invokes `fetch-arch-personalized.exe`. This repository preserves that original script as reference; adapt its executable path when installing elsewhere. Keep a backup of the existing profile before changing it

## Credits

Based on the Arch animation algorithm from `areofyl/fetch`, originally ISC-licensed. See `UPSTREAM-LICENSE.txt`; preserve upstream attribution when redistributing

**Important:** the native renderer searches for `startup.jsonc` beside its executable, so keep that file in the executable's directory when deploying
