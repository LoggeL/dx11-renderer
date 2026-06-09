use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use glam::{Mat4, Vec3};
use windows::core::PCSTR;
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R16_UINT, DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32B32_FLOAT,
    DXGI_FORMAT_R32_FLOAT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_A, VK_CONTROL, VK_D, VK_DOWN, VK_E, VK_ESCAPE, VK_F1, VK_LSHIFT, VK_Q, VK_R, VK_S, VK_SPACE,
    VK_UP, VK_V, VK_W,
};

use dx11_renderer::camera::FlyCamera;
use dx11_renderer::gfx::{blob_bytes, compile_shader, Gfx};
use dx11_renderer::window::{Input, MouseLook, Window};

const EMBEDDED_SHADER: &str = include_str!("../../shaders/scene.hlsl");
const SHADER_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/scene.hlsl");

const VERTEX_STRIDE: u32 = 24;
const INSTANCE_STRIDE: u32 = 48;
const MIN_INSTANCES: u32 = 1_000;
const MAX_INSTANCES: u32 = 2_000_000;

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    pos: [f32; 3],
    normal: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Instance {
    pos: [f32; 3],
    axis: [f32; 3],
    speed: f32,
    scale: f32,
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FrameCB {
    view_proj: Mat4,
    cam_pos: Vec3,
    time: f32,
    light_dir: Vec3,
    _pad: f32,
}

fn cube_mesh() -> ([Vertex; 24], [u16; 36]) {
    fn v(p: [f32; 3], n: [f32; 3]) -> Vertex {
        Vertex {
            pos: [p[0] * 0.5, p[1] * 0.5, p[2] * 0.5],
            normal: n,
        }
    }
    let vertices = [
        // +Y
        v([-1.0, 1.0, -1.0], [0.0, 1.0, 0.0]),
        v([1.0, 1.0, -1.0], [0.0, 1.0, 0.0]),
        v([1.0, 1.0, 1.0], [0.0, 1.0, 0.0]),
        v([-1.0, 1.0, 1.0], [0.0, 1.0, 0.0]),
        // -Y
        v([-1.0, -1.0, -1.0], [0.0, -1.0, 0.0]),
        v([1.0, -1.0, -1.0], [0.0, -1.0, 0.0]),
        v([1.0, -1.0, 1.0], [0.0, -1.0, 0.0]),
        v([-1.0, -1.0, 1.0], [0.0, -1.0, 0.0]),
        // -X
        v([-1.0, -1.0, 1.0], [-1.0, 0.0, 0.0]),
        v([-1.0, -1.0, -1.0], [-1.0, 0.0, 0.0]),
        v([-1.0, 1.0, -1.0], [-1.0, 0.0, 0.0]),
        v([-1.0, 1.0, 1.0], [-1.0, 0.0, 0.0]),
        // +X
        v([1.0, -1.0, 1.0], [1.0, 0.0, 0.0]),
        v([1.0, -1.0, -1.0], [1.0, 0.0, 0.0]),
        v([1.0, 1.0, -1.0], [1.0, 0.0, 0.0]),
        v([1.0, 1.0, 1.0], [1.0, 0.0, 0.0]),
        // -Z
        v([-1.0, -1.0, -1.0], [0.0, 0.0, -1.0]),
        v([1.0, -1.0, -1.0], [0.0, 0.0, -1.0]),
        v([1.0, 1.0, -1.0], [0.0, 0.0, -1.0]),
        v([-1.0, 1.0, -1.0], [0.0, 0.0, -1.0]),
        // +Z
        v([-1.0, -1.0, 1.0], [0.0, 0.0, 1.0]),
        v([1.0, -1.0, 1.0], [0.0, 0.0, 1.0]),
        v([1.0, 1.0, 1.0], [0.0, 0.0, 1.0]),
        v([-1.0, 1.0, 1.0], [0.0, 0.0, 1.0]),
    ];
    let indices = [
        3, 1, 0, 2, 1, 3, // +Y
        6, 4, 5, 7, 4, 6, // -Y
        11, 9, 8, 10, 9, 11, // -X
        14, 12, 13, 15, 12, 14, // +X
        19, 17, 16, 18, 17, 19, // -Z
        22, 20, 21, 23, 20, 22, // +Z
    ];
    (vertices, indices)
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    /// uniform [0, 1)
    fn f32(&mut self) -> f32 {
        (self.next() >> 8) as f32 / 16_777_216.0
    }
    /// uniform [-1, 1)
    fn signed(&mut self) -> f32 {
        self.f32() * 2.0 - 1.0
    }
}

/// Two-arm spiral galaxy of cubes, colors blend warm core -> blue rim.
fn gen_instances(count: u32) -> Vec<Instance> {
    let mut rng = Rng(0x4d595df4);
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let t = rng.f32();
        let radius = 6.0 + t.sqrt() * 260.0;
        let arm = if i % 2 == 0 { 0.0 } else { std::f32::consts::PI };
        let spread = 0.18 + (1.0 - t) * 0.5;
        let angle = radius * 0.035 + arm + rng.signed() * spread * std::f32::consts::PI;

        let y = (rng.signed() + rng.signed()) * (4.0 + radius * 0.05);
        let pos = [angle.cos() * radius, y, angle.sin() * radius];

        let axis = Vec3::new(rng.signed(), rng.signed(), rng.signed())
            .normalize_or(Vec3::Y)
            .to_array();

        let warm = Vec3::new(1.0, 0.78, 0.45);
        let cool = Vec3::new(0.36, 0.55, 1.0);
        let mut col = warm.lerp(cool, t) * (0.55 + rng.f32() * 0.6);
        if rng.f32() < 0.02 {
            col = Vec3::new(1.0, 0.25, 0.42) * 1.2; // sprinkle of accents
        }

        out.push(Instance {
            pos,
            axis,
            speed: rng.signed() * 2.2,
            scale: 0.35 + rng.f32() * rng.f32() * 1.5,
            color: [col.x, col.y, col.z, 1.0],
        });
    }
    out
}

fn as_bytes<T: Copy>(slice: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(slice.as_ptr() as *const u8, std::mem::size_of_val(slice))
    }
}

struct Pipeline {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    layout: ID3D11InputLayout,
}

fn shader_source() -> (String, Option<SystemTime>) {
    let path = PathBuf::from(SHADER_PATH);
    match std::fs::read_to_string(&path) {
        Ok(src) => {
            let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            (src, mtime)
        }
        Err(_) => (EMBEDDED_SHADER.to_string(), None),
    }
}

fn build_pipeline(gfx: &Gfx) -> Result<Pipeline, String> {
    let (src, _) = shader_source();
    let vs_blob = compile_shader(&src, "scene.hlsl", "vs_main", "vs_5_0")?;
    let ps_blob = compile_shader(&src, "scene.hlsl", "ps_main", "ps_5_0")?;

    let sem = |name: &'static [u8]| PCSTR(name.as_ptr());
    let elem = |name: &'static [u8],
                format,
                slot: u32,
                offset: u32,
                per_instance: bool| D3D11_INPUT_ELEMENT_DESC {
        SemanticName: sem(name),
        SemanticIndex: 0,
        Format: format,
        InputSlot: slot,
        AlignedByteOffset: offset,
        InputSlotClass: if per_instance {
            D3D11_INPUT_PER_INSTANCE_DATA
        } else {
            D3D11_INPUT_PER_VERTEX_DATA
        },
        InstanceDataStepRate: if per_instance { 1 } else { 0 },
    };
    let elements = [
        elem(b"POSITION\0", DXGI_FORMAT_R32G32B32_FLOAT, 0, 0, false),
        elem(b"NORMAL\0", DXGI_FORMAT_R32G32B32_FLOAT, 0, 12, false),
        elem(b"I_POS\0", DXGI_FORMAT_R32G32B32_FLOAT, 1, 0, true),
        elem(b"I_AXIS\0", DXGI_FORMAT_R32G32B32_FLOAT, 1, 12, true),
        elem(b"I_SPEED\0", DXGI_FORMAT_R32_FLOAT, 1, 24, true),
        elem(b"I_SCALE\0", DXGI_FORMAT_R32_FLOAT, 1, 28, true),
        elem(b"I_COLOR\0", DXGI_FORMAT_R32G32B32A32_FLOAT, 1, 32, true),
    ];

    unsafe {
        let mut vs = None;
        gfx.device
            .CreateVertexShader(blob_bytes(&vs_blob), None, Some(&mut vs))
            .map_err(|e| e.to_string())?;
        let mut ps = None;
        gfx.device
            .CreatePixelShader(blob_bytes(&ps_blob), None, Some(&mut ps))
            .map_err(|e| e.to_string())?;
        let mut layout = None;
        gfx.device
            .CreateInputLayout(&elements, blob_bytes(&vs_blob), Some(&mut layout))
            .map_err(|e| e.to_string())?;
        Ok(Pipeline {
            vs: vs.unwrap(),
            ps: ps.unwrap(),
            layout: layout.unwrap(),
        })
    }
}

fn make_instance_buffer(gfx: &Gfx, count: u32) -> windows::core::Result<ID3D11Buffer> {
    let instances = gen_instances(count);
    gfx.buffer(
        as_bytes(&instances),
        D3D11_BIND_VERTEX_BUFFER,
        D3D11_USAGE_IMMUTABLE,
    )
}

fn raster_state(gfx: &Gfx, wireframe: bool) -> windows::core::Result<ID3D11RasterizerState> {
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: if wireframe {
            D3D11_FILL_WIREFRAME
        } else {
            D3D11_FILL_SOLID
        },
        CullMode: if wireframe {
            D3D11_CULL_NONE
        } else {
            D3D11_CULL_BACK
        },
        DepthClipEnable: true.into(),
        ..Default::default()
    };
    let mut state = None;
    unsafe {
        gfx.device.CreateRasterizerState(&desc, Some(&mut state))?;
    }
    Ok(state.unwrap())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("dx11 playground");
    println!("  RMB drag        mouse look");
    println!("  WASD / QE       move / down-up (Space/Ctrl also up/down)");
    println!("  Shift           fast");
    println!("  Up / Down       double / halve instance count");
    println!("  V               vsync on/off");
    println!("  F1              wireframe");
    println!("  R               reload shader (auto-reloads on file save too)");
    println!("  Esc             quit");

    let window = Window::new("dx11 playground", 1600, 900)?;
    let (w, h) = window.client_size();
    let mut gfx = Gfx::new(window.hwnd, w.max(1), h.max(1))?;

    let (vertices, indices) = cube_mesh();
    let vbuf = gfx.buffer(
        as_bytes(&vertices),
        D3D11_BIND_VERTEX_BUFFER,
        D3D11_USAGE_IMMUTABLE,
    )?;
    let ibuf = gfx.buffer(
        as_bytes(&indices),
        D3D11_BIND_INDEX_BUFFER,
        D3D11_USAGE_IMMUTABLE,
    )?;
    let cbuf = gfx.dynamic_cbuffer(std::mem::size_of::<FrameCB>())?;

    let mut instance_count: u32 = 100_000;
    let mut instbuf = make_instance_buffer(&gfx, instance_count)?;

    let mut pipeline = build_pipeline(&gfx).map_err(|e| format!("shader compile:\n{e}"))?;
    let rs_solid = raster_state(&gfx, false)?;
    let rs_wire = raster_state(&gfx, true)?;

    let mut camera = FlyCamera::new(Vec3::new(0.0, 90.0, -330.0), 0.0, -0.25);
    let mut input = Input::new();
    let mut mouse = MouseLook::new();

    let mut vsync = false;
    let mut wireframe = false;

    let start = Instant::now();
    let mut last_frame = Instant::now();
    let mut shader_mtime = shader_source().1;
    let mut shader_check = Instant::now();

    let mut acc_time = 0.0f64;
    let mut acc_frames = 0u32;
    let mut title_dirty = true;

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
            title_dirty = true;
        }
        if input.pressed(VK_F1) {
            wireframe = !wireframe;
            title_dirty = true;
        }
        if input.pressed(VK_UP) && instance_count < MAX_INSTANCES {
            instance_count = (instance_count * 2).min(MAX_INSTANCES);
            instbuf = make_instance_buffer(&gfx, instance_count)?;
            title_dirty = true;
        }
        if input.pressed(VK_DOWN) && instance_count > MIN_INSTANCES {
            instance_count = (instance_count / 2).max(MIN_INSTANCES);
            instbuf = make_instance_buffer(&gfx, instance_count)?;
            title_dirty = true;
        }

        // shader hot reload: R key or file mtime change (checked 4x/s)
        let mut want_reload = input.pressed(VK_R);
        if shader_check.elapsed() > Duration::from_millis(250) {
            shader_check = Instant::now();
            let mtime = shader_source().1;
            if mtime.is_some() && mtime != shader_mtime {
                shader_mtime = mtime;
                want_reload = true;
            }
        }
        if want_reload {
            match build_pipeline(&gfx) {
                Ok(p) => {
                    pipeline = p;
                    println!("shader reloaded");
                }
                Err(e) => eprintln!("shader reload failed:\n{e}"),
            }
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

        // frame constants
        let aspect = gfx.width as f32 / gfx.height as f32;
        let frame = FrameCB {
            view_proj: camera.view_proj(aspect),
            cam_pos: camera.pos,
            time: start.elapsed().as_secs_f32(),
            light_dir: Vec3::new(-0.45, -0.8, 0.35).normalize(),
            _pad: 0.0,
        };
        gfx.update(&cbuf, &frame)?;

        // draw
        gfx.begin([0.013, 0.015, 0.022, 1.0]);
        unsafe {
            let ctx = &gfx.ctx;
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.IASetInputLayout(&pipeline.layout);
            let bufs = [Some(vbuf.clone()), Some(instbuf.clone())];
            let strides = [VERTEX_STRIDE, INSTANCE_STRIDE];
            let offsets = [0u32, 0u32];
            ctx.IASetVertexBuffers(
                0,
                2,
                Some(bufs.as_ptr()),
                Some(strides.as_ptr()),
                Some(offsets.as_ptr()),
            );
            ctx.IASetIndexBuffer(&ibuf, DXGI_FORMAT_R16_UINT, 0);
            ctx.VSSetShader(&pipeline.vs, None);
            ctx.PSSetShader(&pipeline.ps, None);
            ctx.VSSetConstantBuffers(0, Some(&[Some(cbuf.clone())]));
            ctx.PSSetConstantBuffers(0, Some(&[Some(cbuf.clone())]));
            ctx.RSSetState(if wireframe { &rs_wire } else { &rs_solid });
            ctx.DrawIndexedInstanced(indices.len() as u32, instance_count, 0, 0, 0);
        }
        gfx.present(vsync)?;

        // stats in the title, 4x/s
        acc_time += dt as f64;
        acc_frames += 1;
        if acc_time >= 0.25 || title_dirty {
            let ms = acc_time * 1000.0 / acc_frames.max(1) as f64;
            let fps = acc_frames as f64 / acc_time.max(1e-6);
            window.set_title(&format!(
                "dx11 playground · {} cubes · {:.2} ms ({:.0} fps) · vsync {}{}",
                instance_count,
                ms,
                fps,
                if vsync { "on" } else { "off" },
                if wireframe { " · wireframe" } else { "" },
            ));
            acc_time = 0.0;
            acc_frames = 0;
            title_dirty = false;
        }
    }

    Ok(())
}
