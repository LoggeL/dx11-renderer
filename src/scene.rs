//! Instanced cube galaxy: mesh, instance generation and the draw pipeline.
//! Shared between the playground and the benchmark.

use std::path::PathBuf;
use std::time::SystemTime;

use glam::{Mat4, Vec3};
use windows::core::PCSTR;
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R16_UINT, DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32B32_FLOAT,
    DXGI_FORMAT_R32_FLOAT,
};

use crate::gfx::{blob_bytes, compile_shader, Gfx};

const EMBEDDED_SHADER: &str = include_str!("../shaders/scene.hlsl");
const SHADER_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/scene.hlsl");

const VERTEX_STRIDE: u32 = 24;
const INSTANCE_STRIDE: u32 = 48;
const INDEX_COUNT: u32 = 36;
pub const MIN_INSTANCES: u32 = 1_000;
pub const MAX_INSTANCES: u32 = 2_000_000;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Instance {
    pub pos: [f32; 3],
    pub axis: [f32; 3],
    pub speed: f32,
    pub scale: f32,
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FrameCB {
    pub view_proj: Mat4,
    pub cam_pos: Vec3,
    pub time: f32,
    pub light_dir: Vec3,
    pub _pad: f32,
}

pub fn as_bytes<T: Copy>(slice: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(slice.as_ptr() as *const u8, std::mem::size_of_val(slice))
    }
}

pub fn cube_mesh() -> ([Vertex; 24], [u16; 36]) {
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
pub fn gen_instances(count: u32) -> Vec<Instance> {
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

struct Pipeline {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    layout: ID3D11InputLayout,
}

fn shader_source() -> String {
    std::fs::read_to_string(SHADER_PATH).unwrap_or_else(|_| EMBEDDED_SHADER.to_string())
}

pub fn shader_mtime() -> Option<SystemTime> {
    std::fs::metadata(PathBuf::from(SHADER_PATH))
        .and_then(|m| m.modified())
        .ok()
}

fn build_pipeline(gfx: &Gfx) -> Result<Pipeline, String> {
    let src = shader_source();
    let vs_blob = compile_shader(&src, "scene.hlsl", "vs_main", "vs_5_0")?;
    let ps_blob = compile_shader(&src, "scene.hlsl", "ps_main", "ps_5_0")?;

    let elem = |name: &'static [u8],
                format,
                slot: u32,
                offset: u32,
                per_instance: bool| D3D11_INPUT_ELEMENT_DESC {
        SemanticName: PCSTR(name.as_ptr()),
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

pub struct Scene {
    vbuf: ID3D11Buffer,
    ibuf: ID3D11Buffer,
    cbuf: ID3D11Buffer,
    instbuf: ID3D11Buffer,
    pipeline: Pipeline,
    rs_solid: ID3D11RasterizerState,
    rs_wire: ID3D11RasterizerState,
    pub instance_count: u32,
    pub wireframe: bool,
}

impl Scene {
    pub fn new(gfx: &Gfx, instance_count: u32) -> Result<Self, String> {
        let (vertices, indices) = cube_mesh();
        let err = |e: windows::core::Error| e.to_string();
        let vbuf = gfx
            .buffer(
                as_bytes(&vertices),
                D3D11_BIND_VERTEX_BUFFER,
                D3D11_USAGE_IMMUTABLE,
            )
            .map_err(err)?;
        let ibuf = gfx
            .buffer(
                as_bytes(&indices),
                D3D11_BIND_INDEX_BUFFER,
                D3D11_USAGE_IMMUTABLE,
            )
            .map_err(err)?;
        let cbuf = gfx
            .dynamic_cbuffer(std::mem::size_of::<FrameCB>())
            .map_err(err)?;
        let instances = gen_instances(instance_count);
        let instbuf = gfx
            .buffer(
                as_bytes(&instances),
                D3D11_BIND_VERTEX_BUFFER,
                D3D11_USAGE_IMMUTABLE,
            )
            .map_err(err)?;
        Ok(Self {
            vbuf,
            ibuf,
            cbuf,
            instbuf,
            pipeline: build_pipeline(gfx)?,
            rs_solid: raster_state(gfx, false).map_err(err)?,
            rs_wire: raster_state(gfx, true).map_err(err)?,
            instance_count,
            wireframe: false,
        })
    }

    pub fn set_instance_count(&mut self, gfx: &Gfx, count: u32) -> windows::core::Result<()> {
        let count = count.clamp(MIN_INSTANCES, MAX_INSTANCES);
        let instances = gen_instances(count);
        self.instbuf = gfx.buffer(
            as_bytes(&instances),
            D3D11_BIND_VERTEX_BUFFER,
            D3D11_USAGE_IMMUTABLE,
        )?;
        self.instance_count = count;
        Ok(())
    }

    pub fn reload_shaders(&mut self, gfx: &Gfx) -> Result<(), String> {
        self.pipeline = build_pipeline(gfx)?;
        Ok(())
    }

    pub fn draw(&self, gfx: &Gfx, frame: &FrameCB) -> windows::core::Result<()> {
        gfx.update(&self.cbuf, frame)?;
        unsafe {
            let ctx = &gfx.ctx;
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.IASetInputLayout(&self.pipeline.layout);
            let bufs = [Some(self.vbuf.clone()), Some(self.instbuf.clone())];
            let strides = [VERTEX_STRIDE, INSTANCE_STRIDE];
            let offsets = [0u32, 0u32];
            ctx.IASetVertexBuffers(
                0,
                2,
                Some(bufs.as_ptr()),
                Some(strides.as_ptr()),
                Some(offsets.as_ptr()),
            );
            ctx.IASetIndexBuffer(&self.ibuf, DXGI_FORMAT_R16_UINT, 0);
            ctx.VSSetShader(&self.pipeline.vs, None);
            ctx.PSSetShader(&self.pipeline.ps, None);
            ctx.VSSetConstantBuffers(0, Some(&[Some(self.cbuf.clone())]));
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.cbuf.clone())]));
            ctx.RSSetState(if self.wireframe {
                &self.rs_wire
            } else {
                &self.rs_solid
            });
            ctx.DrawIndexedInstanced(INDEX_COUNT, self.instance_count, 0, 0, 0);
        }
        Ok(())
    }
}
