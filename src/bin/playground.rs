use std::time::{Duration, Instant};

use glam::Vec3;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_A, VK_CONTROL, VK_D, VK_DOWN, VK_E, VK_ESCAPE, VK_F1, VK_LSHIFT, VK_Q, VK_R, VK_S, VK_SPACE,
    VK_UP, VK_V, VK_W,
};

use dx11_renderer::camera::FlyCamera;
use dx11_renderer::gfx::{Gfx, GpuTimer};
use dx11_renderer::scene::{shader_mtime, FrameCB, Scene, MAX_INSTANCES, MIN_INSTANCES};
use dx11_renderer::text::TextRenderer;
use dx11_renderer::window::{Input, MouseLook, Window};

const HUD_COLOR: [f32; 4] = [0.92, 0.94, 1.0, 1.0];
const HUD_DIM: [f32; 4] = [0.55, 0.58, 0.68, 1.0];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let window = Window::new("dx11 playground", 1600, 900)?;
    let (w, h) = window.client_size();
    let mut gfx = Gfx::new(window.hwnd, w.max(1), h.max(1))?;
    println!("adapter: {}", gfx.adapter_name);

    let mut scene = Scene::new(&gfx, 100_000)?;
    let mut text = TextRenderer::new(&gfx, 17.0)?;
    let mut timer = GpuTimer::new(&gfx.device)?;

    let mut camera = FlyCamera::new(Vec3::new(0.0, 90.0, -330.0), 0.0, -0.25);
    let mut input = Input::new();
    let mut mouse = MouseLook::new();
    let mut vsync = false;

    let start = Instant::now();
    let mut last_frame = Instant::now();
    let mut last_shader_mtime = shader_mtime();
    let mut shader_check = Instant::now();

    // stats smoothed over a 0.25 s window
    let (mut acc_cpu, mut acc_gpu, mut acc_frames, mut acc_gpu_frames) = (0.0f64, 0.0f64, 0u32, 0u32);
    let (mut cpu_ms, mut gpu_ms, mut fps) = (0.0f64, 0.0f64, 0.0f64);
    let mut shader_status: Option<(String, Instant)> = None;

    while window.pump() {
        let now = Instant::now();
        let dt = (now - last_frame).as_secs_f32().min(0.1);
        last_frame = now;

        if let Some((w, h)) = window.take_resize() {
            gfx.resize(w, h)?;
        }
        if gfx.width == 0 || gfx.height == 0 {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }

        input.update(window.focused());
        if input.pressed(VK_ESCAPE) {
            break;
        }
        if input.pressed(VK_V) {
            vsync = !vsync;
        }
        if input.pressed(VK_F1) {
            scene.wireframe = !scene.wireframe;
        }
        if input.pressed(VK_UP) && scene.instance_count < MAX_INSTANCES {
            let n = scene.instance_count * 2;
            scene.set_instance_count(&gfx, n)?;
        }
        if input.pressed(VK_DOWN) && scene.instance_count > MIN_INSTANCES {
            let n = scene.instance_count / 2;
            scene.set_instance_count(&gfx, n)?;
        }

        // shader hot reload: R key or file mtime change (checked 4x/s)
        let mut want_reload = input.pressed(VK_R);
        if shader_check.elapsed() > Duration::from_millis(250) {
            shader_check = Instant::now();
            let mtime = shader_mtime();
            if mtime.is_some() && mtime != last_shader_mtime {
                last_shader_mtime = mtime;
                want_reload = true;
            }
        }
        if want_reload {
            let msg = match scene.reload_shaders(&gfx) {
                Ok(()) => "shader reloaded".to_string(),
                Err(e) => {
                    eprintln!("shader reload failed:\n{e}");
                    format!("shader error: {}", e.lines().next().unwrap_or("?"))
                }
            };
            shader_status = Some((msg, Instant::now()));
        }

        // camera
        let (mdx, mdy) = mouse.update(&input);
        camera.rotate(mdx, mdy);
        let mut mv = Vec3::ZERO;
        if input.down(VK_W) {
            mv += camera.forward();
        }
        if input.down(VK_S) {
            mv -= camera.forward();
        }
        if input.down(VK_D) {
            mv += camera.right();
        }
        if input.down(VK_A) {
            mv -= camera.right();
        }
        if input.down(VK_E) || input.down(VK_SPACE) {
            mv += Vec3::Y;
        }
        if input.down(VK_Q) || input.down(VK_CONTROL) {
            mv -= Vec3::Y;
        }
        let speed = if input.down(VK_LSHIFT) { 260.0 } else { 55.0 };
        camera.pos += mv.normalize_or_zero() * speed * dt;

        // draw
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
        if let Some(ms) = timer.end(&gfx.ctx) {
            acc_gpu += ms;
            acc_gpu_frames += 1;
        }

        // stats
        acc_cpu += dt as f64 * 1000.0;
        acc_frames += 1;
        if acc_cpu >= 250.0 {
            cpu_ms = acc_cpu / acc_frames as f64;
            fps = acc_frames as f64 / (acc_cpu / 1000.0);
            if acc_gpu_frames > 0 {
                gpu_ms = acc_gpu / acc_gpu_frames as f64;
            }
            (acc_cpu, acc_gpu, acc_frames, acc_gpu_frames) = (0.0, 0.0, 0, 0);
        }

        // HUD
        let lh = text.line_height;
        text.draw_text_shadowed(
            12.0,
            10.0,
            &format!("{fps:5.0} fps   cpu {cpu_ms:5.2} ms   gpu {gpu_ms:5.2} ms"),
            HUD_COLOR,
        );
        text.draw_text_shadowed(
            12.0,
            10.0 + lh,
            &format!(
                "{} cubes   vsync {}{}",
                scene.instance_count,
                if vsync { "on" } else { "off" },
                if scene.wireframe { "   wireframe" } else { "" },
            ),
            HUD_COLOR,
        );
        text.draw_text_shadowed(
            12.0,
            10.0 + lh * 2.0,
            "rmb look   wasd/qe move   shift fast   up/down count   v vsync   f1 wire   r reload   esc quit",
            HUD_DIM,
        );
        let status_expired = shader_status
            .as_ref()
            .is_some_and(|(_, since)| since.elapsed() >= Duration::from_secs(3));
        if status_expired {
            shader_status = None;
        }
        if let Some((msg, _)) = &shader_status {
            text.draw_text_shadowed(12.0, 10.0 + lh * 3.5, msg, [1.0, 0.8, 0.3, 1.0]);
        }
        text.flush(&gfx)?;

        gfx.present(vsync)?;
    }

    Ok(())
}
