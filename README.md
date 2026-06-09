# dx11-renderer

Schlanker D3D11-Renderer in Rust (`windows`-Crate, raw Win32 — kein winit) plus Playground.

```
cargo run --release
```

## Performance-Design

- Flip-Model-Swapchain (`FLIP_DISCARD`), Tearing-Support → uncapped FPS bei vsync off
- Instancing: ein Cube-Mesh, Instanzdaten (Position, Rotationsachse, Speed, Scale, Farbe) in
  einem **immutable** Vertex-Buffer — die komplette Animation läuft im Vertex-Shader aus einer
  einzigen Time-Konstante. Null Buffer-Uploads pro Frame außer 96 Byte Frame-CB (map-discard).
- sRGB-RTV auf UNORM-Backbuffer, D32-Depth
- Release-Profil mit thin-LTO

Referenz: 100k Cubes ≈ 0,9 ms auf der Entwicklungsmaschine; bis 2M Instanzen skalierbar (Up-Taste).

## Playground

| Taste | Aktion |
|---|---|
| RMB-Drag | Mouselook |
| WASD / QE bzw. Space/Ctrl | Fliegen / runter-hoch |
| Shift | schnell |
| Up / Down | Instanzen verdoppeln / halbieren (1k–2M) |
| V | VSync |
| F1 | Wireframe |
| R | Shader-Reload (lädt bei Dateiänderung auch automatisch) |
| Esc | Beenden |

Frametime/FPS stehen im Fenstertitel. `shaders/scene.hlsl` editieren + speichern → Hot-Reload,
Compile-Fehler landen in der Konsole.

## Struktur

- `src/window.rs` — Win32-Fenster, Message-Pump, Keyboard-Polling, Mouselook
- `src/gfx.rs` — Device/Swapchain/Targets, Resize, Present, Buffer-Helper, HLSL-Compile (fxc)
- `src/camera.rs` — LH-Fly-Cam (glam, 0..1-Depth)
- `src/bin/playground.rs` — Szene, Pipeline, Loop
- `shaders/scene.hlsl` — VS/PS, Rodrigues-Rotation pro Instanz
