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

Drei Suiten: Instancing (10k–2M Cubes), CPU-Glyph-Rasterization, GPU-Text-Rendering.
GPU-Zeit über Timestamp-Queries (8-Frame-Ring ohne Stalls), je 40 Warmup-Frames.
Fenster im Vordergrund lassen, sonst drückt DWM-Throttling die p99-Werte.

Referenz RTX 3090 Ti, 1280x720:

| instances | gpu avg (ms) | fps |
|----------:|-------------:|----:|
|   100 000 |          0,7 | ~780 |
|   500 000 |          3,0 | ~300 |
| 1 000 000 |          5,6 | ~170 |
| 2 000 000 |         11,6 |  ~85 |

| glyph raster | 12 px | 17 px | 32 px | 64 px |
|---|---:|---:|---:|---:|
| µs/Glyph (CPU) | 2,3 | 2,4 | 3,3 | 5,4 |

| text gpu | 1k chars | 16k | 64k |
|---|---:|---:|---:|
| ms/Frame | 0,16 | 1,6 | 6,0 |

## Font-Renderer

Komplett selbst gebaut, keine Dependencies:

- `src/font.rs` — TrueType-Parser (cmap Format 4, loca, glyf inkl. Composites, head/hhea/hmtx)
  plus Rasterizer: quadratische Outlines werden adaptiv zu Liniensegmenten geflattet, pro
  Zelle wird signed Coverage akkumuliert und mit einem einzigen linearen Prefix-Sum-Pass
  aufgelöst — analytisches Anti-Aliasing ohne Supersampling, O(Segmente × Scanlines + Pixel).
  ~413k Glyphen/s bei 17 px.
- `src/text.rs` — Consolas wird beim Start in einen 512²-R8-Atlas gebacken (ASCII, Shelf-
  Packing). `draw_text()` sammelt Quads CPU-seitig, `flush()` lädt sie per map-discard in
  einen dynamischen Vertex-Buffer und zeichnet alpha-geblendet ohne Depth — Pixel-
  Koordinaten rein, NDC im VS. Der Playground rendert damit sein HUD.

`cargo test` prüft Parsing + Rasterization gegen das System-Consolas.

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
- `src/font.rs` — TTF-Parser + Scanline-Rasterizer (dependency-frei)
- `src/text.rs` — Font-Atlas + Text-Batching
- `src/camera.rs` — LH-Fly-Cam (glam, 0..1-Depth)
- `src/bin/playground.rs` — Input, HUD, Loop
- `src/bin/bench.rs` — Messloop, Markdown-Tabelle
- `shaders/scene.hlsl`, `shaders/text.hlsl`
