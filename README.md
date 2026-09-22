# Patent Tagger

Desktop application for importing, tagging and exporting patents via EPO Open Patent Services. See `docs/SPEC.md` for the full specification.

## Development

```
cargo tauri dev
```

Requires Node.js (frontend, `ui/`) and, on Linux, the Tauri 2 build prerequisites (WebKitGTK 4.1, GTK 3 and related development packages — see `.github/workflows/ci.yml` for the exact list).

## Build

```
cargo tauri build --no-bundle                  # Windows: release .exe
cargo tauri build --bundles deb,appimage       # Linux: .deb and AppImage
```

The AppImage needs FUSE 2 to run (`libfuse2`, or `libfuse2t64` on Ubuntu 24.04); otherwise run it with `--appimage-extract-and-run`.
