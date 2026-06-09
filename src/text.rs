//! Immediate-mode text renderer: a system TTF (Consolas) is baked into an R8
//! coverage atlas at startup, draw_text() queues quads into a CPU vec and
//! flush() uploads them once per frame into a dynamic vertex buffer.

use fontdue::{Font, FontSettings};
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R8_UNORM,
    DXGI_SAMPLE_DESC,
};
use windows::core::PCSTR;

use crate::gfx::{blob_bytes, compile_shader, Gfx};

const TEXT_SHADER: &str = include_str!("../shaders/text.hlsl");
const ATLAS_SIZE: usize = 512;
const FIRST_CHAR: u8 = 32;
const LAST_CHAR: u8 = 126;
const MAX_CHARS: usize = 4096;
const VERTEX_STRIDE: u32 = 32;

const FONT_CANDIDATES: &[&str] = &[
    "C:\\Windows\\Fonts\\consola.ttf",
    "C:\\Windows\\Fonts\\cour.ttf",
    "C:\\Windows\\Fonts\\segoeui.ttf",
];

#[repr(C)]
#[derive(Clone, Copy)]
struct TextVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ScreenCB {
    viewport: [f32; 2],
    _pad: [f32; 2],
}

#[derive(Clone, Copy, Default)]
struct Glyph {
    uv0: [f32; 2],
    uv1: [f32; 2],
    size: [f32; 2],
    // offset from pen position: x from glyph xmin, y from baseline to glyph top
    offset: [f32; 2],
    advance: f32,
}

pub struct TextRenderer {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    vbuf: ID3D11Buffer,
    cbuf: ID3D11Buffer,
    blend: ID3D11BlendState,
    depth_off: ID3D11DepthStencilState,
    sampler: ID3D11SamplerState,
    srv: ID3D11ShaderResourceView,
    glyphs: Vec<Glyph>,
    ascent: f32,
    pub line_height: f32,
    verts: Vec<TextVertex>,
}

impl TextRenderer {
    pub fn new(gfx: &Gfx, px: f32) -> Result<Self, String> {
        let font_data = FONT_CANDIDATES
            .iter()
            .find_map(|p| std::fs::read(p).ok())
            .ok_or("no system font found")?;
        let font = Font::from_bytes(font_data, FontSettings::default())?;
        let line = font
            .horizontal_line_metrics(px)
            .ok_or("font has no horizontal metrics")?;

        // bake ASCII into the atlas, simple shelf packing
        let mut atlas = vec![0u8; ATLAS_SIZE * ATLAS_SIZE];
        let mut glyphs = vec![Glyph::default(); (LAST_CHAR - FIRST_CHAR + 1) as usize];
        let (mut x, mut y, mut row_h) = (1usize, 1usize, 0usize);
        for ch in FIRST_CHAR..=LAST_CHAR {
            let (m, bitmap) = font.rasterize(ch as char, px);
            if x + m.width + 1 > ATLAS_SIZE {
                x = 1;
                y += row_h + 1;
                row_h = 0;
            }
            if y + m.height + 1 > ATLAS_SIZE {
                return Err("font atlas overflow".into());
            }
            for (row, src) in bitmap.chunks_exact(m.width.max(1)).enumerate() {
                if m.width == 0 {
                    break;
                }
                let dst = (y + row) * ATLAS_SIZE + x;
                atlas[dst..dst + m.width].copy_from_slice(src);
            }
            let inv = 1.0 / ATLAS_SIZE as f32;
            glyphs[(ch - FIRST_CHAR) as usize] = Glyph {
                uv0: [x as f32 * inv, y as f32 * inv],
                uv1: [(x + m.width) as f32 * inv, (y + m.height) as f32 * inv],
                size: [m.width as f32, m.height as f32],
                offset: [m.xmin as f32, -(m.height as f32 + m.ymin as f32)],
                advance: m.advance_width,
            };
            x += m.width + 1;
            row_h = row_h.max(m.height);
        }

        let err = |e: windows::core::Error| e.to_string();

        // atlas texture + srv
        let tex_desc = D3D11_TEXTURE2D_DESC {
            Width: ATLAS_SIZE as u32,
            Height: ATLAS_SIZE as u32,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: atlas.as_ptr() as *const _,
            SysMemPitch: ATLAS_SIZE as u32,
            ..Default::default()
        };
        let (srv, sampler, blend, depth_off) = unsafe {
            let mut tex = None;
            gfx.device
                .CreateTexture2D(&tex_desc, Some(&init), Some(&mut tex))
                .map_err(err)?;
            let mut srv = None;
            gfx.device
                .CreateShaderResourceView(&tex.unwrap(), None, Some(&mut srv))
                .map_err(err)?;

            let sampler_desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_POINT,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                ..Default::default()
            };
            let mut sampler = None;
            gfx.device
                .CreateSamplerState(&sampler_desc, Some(&mut sampler))
                .map_err(err)?;

            let mut blend_desc = D3D11_BLEND_DESC::default();
            blend_desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
                BlendEnable: true.into(),
                SrcBlend: D3D11_BLEND_SRC_ALPHA,
                DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOp: D3D11_BLEND_OP_ADD,
                SrcBlendAlpha: D3D11_BLEND_ONE,
                DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOpAlpha: D3D11_BLEND_OP_ADD,
                RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
            };
            let mut blend = None;
            gfx.device
                .CreateBlendState(&blend_desc, Some(&mut blend))
                .map_err(err)?;

            let ds_desc = D3D11_DEPTH_STENCIL_DESC {
                DepthEnable: false.into(),
                DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ZERO,
                DepthFunc: D3D11_COMPARISON_ALWAYS,
                ..Default::default()
            };
            let mut depth_off = None;
            gfx.device
                .CreateDepthStencilState(&ds_desc, Some(&mut depth_off))
                .map_err(err)?;

            (
                srv.unwrap(),
                sampler.unwrap(),
                blend.unwrap(),
                depth_off.unwrap(),
            )
        };

        // shaders + layout
        let vs_blob = compile_shader(TEXT_SHADER, "text.hlsl", "vs_main", "vs_5_0")?;
        let ps_blob = compile_shader(TEXT_SHADER, "text.hlsl", "ps_main", "ps_5_0")?;
        let elem = |name: &'static [u8], format, offset: u32| D3D11_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(name.as_ptr()),
            SemanticIndex: 0,
            Format: format,
            InputSlot: 0,
            AlignedByteOffset: offset,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        };
        let elements = [
            elem(b"POSITION\0", DXGI_FORMAT_R32G32_FLOAT, 0),
            elem(b"TEXCOORD\0", DXGI_FORMAT_R32G32_FLOAT, 8),
            elem(b"COLOR\0", DXGI_FORMAT_R32G32B32A32_FLOAT, 16),
        ];
        let (vs, ps, layout) = unsafe {
            let mut vs = None;
            gfx.device
                .CreateVertexShader(blob_bytes(&vs_blob), None, Some(&mut vs))
                .map_err(err)?;
            let mut ps = None;
            gfx.device
                .CreatePixelShader(blob_bytes(&ps_blob), None, Some(&mut ps))
                .map_err(err)?;
            let mut layout = None;
            gfx.device
                .CreateInputLayout(&elements, blob_bytes(&vs_blob), Some(&mut layout))
                .map_err(err)?;
            (vs.unwrap(), ps.unwrap(), layout.unwrap())
        };

        // dynamic vertex buffer
        let vb_desc = D3D11_BUFFER_DESC {
            ByteWidth: (MAX_CHARS * 6 * std::mem::size_of::<TextVertex>()) as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            ..Default::default()
        };
        let vbuf = unsafe {
            let mut b = None;
            gfx.device
                .CreateBuffer(&vb_desc, None, Some(&mut b))
                .map_err(err)?;
            b.unwrap()
        };
        let cbuf = gfx
            .dynamic_cbuffer(std::mem::size_of::<ScreenCB>())
            .map_err(err)?;

        Ok(Self {
            vs,
            ps,
            layout,
            vbuf,
            cbuf,
            blend,
            depth_off,
            sampler,
            srv,
            glyphs,
            ascent: line.ascent,
            line_height: line.new_line_size.ceil(),
            verts: Vec::new(),
        })
    }

    /// Queues `text` with its top-left corner at pixel (x, y).
    pub fn draw_text(&mut self, x: f32, y: f32, text: &str, color: [f32; 4]) {
        let baseline = (y + self.ascent).round();
        let mut pen = x.round();
        for ch in text.chars() {
            let b = if (FIRST_CHAR..=LAST_CHAR).contains(&(ch as u32 as u8))
                && ch.is_ascii()
            {
                ch as u8
            } else {
                b'?'
            };
            let g = self.glyphs[(b - FIRST_CHAR) as usize];
            if g.size[0] > 0.0 && self.verts.len() < MAX_CHARS * 6 {
                let x0 = pen + g.offset[0];
                let y0 = baseline + g.offset[1];
                let (x1, y1) = (x0 + g.size[0], y0 + g.size[1]);
                let v = |px: f32, py: f32, u: f32, vv: f32| TextVertex {
                    pos: [px, py],
                    uv: [u, vv],
                    color,
                };
                self.verts.extend_from_slice(&[
                    v(x0, y0, g.uv0[0], g.uv0[1]),
                    v(x1, y0, g.uv1[0], g.uv0[1]),
                    v(x1, y1, g.uv1[0], g.uv1[1]),
                    v(x0, y0, g.uv0[0], g.uv0[1]),
                    v(x1, y1, g.uv1[0], g.uv1[1]),
                    v(x0, y1, g.uv0[0], g.uv1[1]),
                ]);
            }
            pen += g.advance.round();
        }
    }

    /// Same as draw_text but with a 1px drop shadow for readability.
    pub fn draw_text_shadowed(&mut self, x: f32, y: f32, text: &str, color: [f32; 4]) {
        self.draw_text(x + 1.0, y + 1.0, text, [0.0, 0.0, 0.0, 0.8 * color[3]]);
        self.draw_text(x, y, text, color);
    }

    /// Uploads queued glyphs and draws them. Resets blend/depth state after.
    pub fn flush(&mut self, gfx: &Gfx) -> windows::core::Result<()> {
        if self.verts.is_empty() {
            return Ok(());
        }
        gfx.update(
            &self.cbuf,
            &ScreenCB {
                viewport: [gfx.width as f32, gfx.height as f32],
                _pad: [0.0; 2],
            },
        )?;
        unsafe {
            let ctx = &gfx.ctx;
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(&self.vbuf, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            std::ptr::copy_nonoverlapping(
                self.verts.as_ptr() as *const u8,
                mapped.pData as *mut u8,
                self.verts.len() * std::mem::size_of::<TextVertex>(),
            );
            ctx.Unmap(&self.vbuf, 0);

            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.IASetInputLayout(&self.layout);
            let bufs = [Some(self.vbuf.clone())];
            let strides = [VERTEX_STRIDE];
            let offsets = [0u32];
            ctx.IASetVertexBuffers(
                0,
                1,
                Some(bufs.as_ptr()),
                Some(strides.as_ptr()),
                Some(offsets.as_ptr()),
            );
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
            ctx.VSSetConstantBuffers(0, Some(&[Some(self.cbuf.clone())]));
            ctx.PSSetShaderResources(0, Some(&[Some(self.srv.clone())]));
            ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            ctx.OMSetBlendState(&self.blend, None, 0xffff_ffff);
            ctx.OMSetDepthStencilState(&self.depth_off, 0);
            ctx.Draw(self.verts.len() as u32, 0);

            // back to opaque defaults for the 3D pass
            ctx.OMSetBlendState(None, None, 0xffff_ffff);
            ctx.OMSetDepthStencilState(None, 0);
        }
        self.verts.clear();
        Ok(())
    }
}
