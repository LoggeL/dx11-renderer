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

## Benchmark

```
cargo run --release --bin bench
```

Misst CPU-Frametime und GPU-Zeit (Timestamp-Queries, 8-Frame-Ring ohne Stalls) über
10k–2M Instanzen, je 40 Warmup- + 300 Messframes. Fenster dabei im Vordergrund lassen,
sonst drückt DWM-Throttling die p99-Werte.

Referenz RTX 3090 Ti, 1280x720:

| instances | gpu avg (ms) | fps |
|----------:|-------------:|----:|
|   100 000 |          1,3 | ~750 |
|   500 000 |          2,9 | ~340 |
| 1 000 000 |          5,4 | ~180 |
| 2 000 000 |         12,5 |  ~80 |

## Font-Renderer

`src/text.rs`: Consolas (System-TTF) wird beim Start via fontdue in einen 512²-R8-Atlas
gebacken (ASCII 32–126, Shelf-Packing). `draw_text()` sammelt Quads CPU-seitig,
`flush()` lädt sie einmal pro Frame per map-discard in einen dynamischen Vertex-Buffer
und zeichnet alpha-geblendet ohne Depth — Pixel-Koordinaten rein, NDC im VS. Der
Playground rendert damit sein HUD (FPS, CPU-/GPU-Frametime, Controls).

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

Stats stehen im HUD (Text-Renderer). `shaders/scene.hlsl` editieren + speichern → Hot-Reload,
Compile-Fehler landen in Konsole + HUD.

## Struktur

- `src/window.rs` — Win32-Fenster, Message-Pump, Keyboard-Polling, Mouselook
- `src/gfx.rs` — Device/Swapchain/Targets, Resize, Present, Buffer-Helper, HLSL-Compile (fxc), GPU-Timer
- `src/scene.rs` — Cube-Mesh, Instanz-Generierung, Pipeline, Draw (geteilt von Playground + Bench)
- `src/text.rs` — Font-Atlas + Text-Batching
- `src/camera.rs` — LH-Fly-Cam (glam, 0..1-Depth)
- `src/bin/playground.rs` — Input, HUD, Loop
- `src/bin/bench.rs` — Messloop, Markdown-Tabelle
- `shaders/scene.hlsl`, `shaders/text.hlsl`
