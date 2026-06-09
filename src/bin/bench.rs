//! Throughput benchmarks: the instanced scene at increasing instance counts,
//! CPU glyph rasterization, and GPU text rendering. CPU frame time is wall
//! clock, GPU time comes from timestamp queries.
//!
//!   cargo run --release --bin bench

use std::time::Instant;

use glam::Vec3;

use dx11_renderer::camera::FlyCamera;
use dx11_renderer::font;
use dx11_renderer::gfx::{Gfx, GpuTimer};
use dx11_renderer::scene::{FrameCB, Scene};
use dx11_renderer::text::{TextRenderer, MAX_CHARS};
use dx11_renderer::window::Window;

const COUNTS: &[u32] = &[10_000, 50_000, 100_000, 250_000, 500_000, 1_000_000, 2_000_000];
const WARMUP_FRAMES: u32 = 40;
const MEASURE_FRAMES: usize = 300;

const RASTER_SIZES: &[f32] = &[12.0, 17.0, 32.0, 64.0];
const RASTER_REPEATS: usize = 50;
const TEXT_COUNTS: &[usize] = &[1_000, 4_000, 16_000, 64_000];
const TEXT_FRAMES: usize = 200;

struct Stats {
    avg: f64,
    p99: f64,
}

fn stats(samples: &mut [f64]) -> Stats {
    samples.sort_by(|a, b| a.total_cmp(b));
    let avg = samples.iter().sum::<f64>() / samples.len().max(1) as f64;
    let p99 = samples
        .get(((samples.len() as f64 - 1.0) * 0.99) as usize)
        .copied()
        .unwrap_or(f64::NAN);
    Stats { avg, p99 }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let window = Window::new("dx11 bench", 1280, 720)?;
    let (w, h) = window.client_size();
    let mut gfx = Gfx::new(window.hwnd, w.max(1), h.max(1))?;
    let mut scene = Scene::new(&gfx, COUNTS[0])?;
    let mut timer = GpuTimer::new(&gfx.device)?;

    let camera = FlyCamera::new(Vec3::new(0.0, 140.0, -420.0), 0.0, -0.3);

    println!("adapter: {}", gfx.adapter_name);
    println!(
        "{}x{}, vsync off, {} warmup + {} measured frames per count\n",
        gfx.width, gfx.height, WARMUP_FRAMES, MEASURE_FRAMES
    );
    println!("## instanced cubes\n");
    println!("| instances | cpu avg (ms) | cpu p99 (ms) | gpu avg (ms) | gpu p99 (ms) | fps |");
    println!("|----------:|-------------:|-------------:|-------------:|-------------:|----:|");

    let start = Instant::now();
    'outer: for &count in COUNTS {
        scene.set_instance_count(&gfx, count)?;

        let mut cpu = Vec::with_capacity(MEASURE_FRAMES);
        let mut gpu = Vec::with_capacity(MEASURE_FRAMES);
        let mut frame_idx: u32 = 0;
        let mut last = Instant::now();

        while cpu.len() < MEASURE_FRAMES {
            if !window.pump() {
                break 'outer;
            }
            if let Some((w, h)) = window.take_resize() {
                gfx.resize(w, h)?;
            }

            let aspect = gfx.width as f32 / gfx.height as f32;
            let frame = FrameCB {
                view_proj: camera.view_proj(aspect),
                cam_pos: camera.pos,
                time: start.elapsed().as_secs_f32(),
                light_dir: Vec3::new(-0.45, -0.8, 0.35).normalize(),
                _pad: 0.0,
            };

            timer.begin(&gfx.ctx);
            gfx.begin([0.013, 0.015, 0.022, 1.0]);
            scene.draw(&gfx, &frame)?;
            let gpu_ms = timer.end(&gfx.ctx);
            gfx.present(false)?;

            let now = Instant::now();
            let dt = (now - last).as_secs_f64() * 1000.0;
            last = now;

            frame_idx += 1;
            if frame_idx > WARMUP_FRAMES {
                cpu.push(dt);
                if let Some(ms) = gpu_ms {
                    gpu.push(ms);
                }
            }
        }

        let c = stats(&mut cpu);
        let g = stats(&mut gpu);
        println!(
            "| {:>9} | {:>12.3} | {:>12.3} | {:>12.3} | {:>12.3} | {:>4.0} |",
            count,
            c.avg,
            c.p99,
            g.avg,
            g.p99,
            1000.0 / c.avg,
        );
    }

    bench_rasterizer()?;
    bench_text(&window, &mut gfx, &mut timer)?;

    Ok(())
}

/// CPU cost of the TTF rasterizer: full printable-ASCII set per size.
fn bench_rasterizer() -> Result<(), String> {
    let font = font::load_system()?;
    println!("\n## glyph rasterization (cpu, printable ascii x{RASTER_REPEATS})\n");
    println!("| px | glyphs/s | us/glyph |");
    println!("|---:|---------:|---------:|");
    for &px in RASTER_SIZES {
        let glyphs = 95 * RASTER_REPEATS;
        let start = Instant::now();
        let mut sink = 0usize;
        for _ in 0..RASTER_REPEATS {
            for c in 32u8..=126 {
                sink += font.rasterize(c as char, px).coverage.len();
            }
        }
        let secs = start.elapsed().as_secs_f64();
        std::hint::black_box(sink);
        println!(
            "| {:>2} | {:>8.0} | {:>8.2} |",
            px,
            glyphs as f64 / secs,
            secs * 1e6 / glyphs as f64,
        );
    }
    Ok(())
}

/// GPU text throughput: n glyphs per frame, flushed in MAX_CHARS batches.
fn bench_text(
    window: &Window,
    gfx: &mut Gfx,
    timer: &mut GpuTimer,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut text = TextRenderer::new(gfx, 17.0)?;
    let line: String = (0..100)
        .map(|i| (32 + (i * 7 + 11) % 95) as u8 as char)
        .collect();
    let lines_visible = (gfx.height as f32 / text.line_height) as usize - 1;

    println!("\n## text rendering (gpu, 100-char lines, {TEXT_FRAMES} frames)\n");
    println!("| chars/frame | cpu avg (ms) | gpu avg (ms) | fps |");
    println!("|------------:|-------------:|-------------:|----:|");

    'outer: for &chars in TEXT_COUNTS {
        let mut cpu = Vec::with_capacity(TEXT_FRAMES);
        let mut gpu = Vec::with_capacity(TEXT_FRAMES);
        let mut frame_idx: u32 = 0;
        let mut last = Instant::now();
        while cpu.len() < TEXT_FRAMES {
            if !window.pump() {
                break 'outer;
            }
            if let Some((w, h)) = window.take_resize() {
                gfx.resize(w, h)?;
            }

            timer.begin(&gfx.ctx);
            gfx.begin([0.013, 0.015, 0.022, 1.0]);
            let mut queued = 0usize;
            let mut drawn = 0usize;
            while drawn < chars {
                let row = (drawn / 100) % lines_visible;
                text.draw_text(
                    8.0,
                    8.0 + row as f32 * text.line_height,
                    &line,
                    [0.9, 0.93, 1.0, 1.0],
                );
                drawn += 100;
                queued += 100;
                if queued + 100 > MAX_CHARS {
                    text.flush(gfx)?;
                    queued = 0;
                }
            }
            text.flush(gfx)?;
            let gpu_ms = timer.end(&gfx.ctx);
            gfx.present(false)?;

            let now = Instant::now();
            let dt = (now - last).as_secs_f64() * 1000.0;
            last = now;
            frame_idx += 1;
            if frame_idx > WARMUP_FRAMES {
                cpu.push(dt);
                if let Some(ms) = gpu_ms {
                    gpu.push(ms);
                }
            }
        }
        let c = stats(&mut cpu);
        let g = stats(&mut gpu);
        println!(
            "| {:>11} | {:>12.3} | {:>12.3} | {:>4.0} |",
            chars,
            c.avg,
            g.avg,
            1000.0 / c.avg,
        );
    }
    Ok(())
}
