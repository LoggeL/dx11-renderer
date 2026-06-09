use windows::core::{Interface, Result, BOOL, PCSTR};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::Fxc::{
    D3DCompile, D3DCOMPILE_ENABLE_STRICTNESS, D3DCOMPILE_OPTIMIZATION_LEVEL3,
};
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0,
    D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

/// Device, immediate context and a flip-model swapchain with an sRGB render
/// target view plus D32 depth. Tearing (uncapped present) is used when the
/// OS/driver support it and vsync is off.
pub struct Gfx {
    pub device: ID3D11Device,
    pub ctx: ID3D11DeviceContext,
    pub swapchain: IDXGISwapChain1,
    pub rtv: Option<ID3D11RenderTargetView>,
    pub dsv: Option<ID3D11DepthStencilView>,
    pub width: u32,
    pub height: u32,
    allow_tearing: bool,
}

impl Gfx {
    pub fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        unsafe {
            let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
            let mut flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
            if cfg!(debug_assertions) {
                flags |= D3D11_CREATE_DEVICE_DEBUG;
            }

            let mut device = None;
            let mut ctx = None;
            let mut fl = D3D_FEATURE_LEVEL::default();
            let mut result = D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                flags,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                Some(&mut fl),
                Some(&mut ctx),
            );
            if result.is_err() && flags.contains(D3D11_CREATE_DEVICE_DEBUG) {
                // No SDK layers installed — retry without the debug layer.
                flags &= !D3D11_CREATE_DEVICE_DEBUG;
                result = D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    flags,
                    Some(&levels),
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    Some(&mut fl),
                    Some(&mut ctx),
                );
            }
            result?;
            let device = device.unwrap();
            let ctx = ctx.unwrap();

            let dxgi_device: IDXGIDevice = device.cast()?;
            let adapter = dxgi_device.GetAdapter()?;
            let factory: IDXGIFactory2 = adapter.GetParent()?;

            let allow_tearing = factory
                .cast::<IDXGIFactory5>()
                .map(|f5| {
                    let mut allow = BOOL(0);
                    f5.CheckFeatureSupport(
                        DXGI_FEATURE_PRESENT_ALLOW_TEARING,
                        &mut allow as *mut _ as *mut _,
                        std::mem::size_of::<BOOL>() as u32,
                    )
                    .is_ok()
                        && allow.as_bool()
                })
                .unwrap_or(false);

            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                Scaling: DXGI_SCALING_NONE,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                Flags: if allow_tearing {
                    DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING.0 as u32
                } else {
                    0
                },
                ..Default::default()
            };
            let swapchain = factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None)?;
            let _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);

            let mut gfx = Self {
                device,
                ctx,
                swapchain,
                rtv: None,
                dsv: None,
                width,
                height,
                allow_tearing,
            };
            gfx.create_targets()?;
            Ok(gfx)
        }
    }

    fn create_targets(&mut self) -> Result<()> {
        unsafe {
            let back: ID3D11Texture2D = self.swapchain.GetBuffer(0)?;
            let rtv_desc = D3D11_RENDER_TARGET_VIEW_DESC {
                Format: DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
                ViewDimension: D3D11_RTV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_RENDER_TARGET_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_RTV { MipSlice: 0 },
                },
            };
            let mut rtv = None;
            self.device
                .CreateRenderTargetView(&back, Some(&rtv_desc), Some(&mut rtv))?;
            self.rtv = rtv;

            let depth_desc = D3D11_TEXTURE2D_DESC {
                Width: self.width,
                Height: self.height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_D32_FLOAT,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_DEPTH_STENCIL.0 as u32,
                ..Default::default()
            };
            let mut depth_tex = None;
            self.device
                .CreateTexture2D(&depth_desc, None, Some(&mut depth_tex))?;
            let mut dsv = None;
            self.device
                .CreateDepthStencilView(&depth_tex.unwrap(), None, Some(&mut dsv))?;
            self.dsv = dsv;
        }
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 || (width == self.width && height == self.height) {
            return Ok(());
        }
        self.rtv = None;
        self.dsv = None;
        unsafe {
            self.ctx.OMSetRenderTargets(None, None);
            self.swapchain.ResizeBuffers(
                0,
                width,
                height,
                DXGI_FORMAT_UNKNOWN,
                if self.allow_tearing {
                    DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING
                } else {
                    DXGI_SWAP_CHAIN_FLAG(0)
                },
            )?;
        }
        self.width = width;
        self.height = height;
        self.create_targets()
    }

    pub fn begin(&self, clear: [f32; 4]) {
        let rtv = self.rtv.as_ref().unwrap();
        unsafe {
            self.ctx
                .OMSetRenderTargets(Some(&[Some(rtv.clone())]), self.dsv.as_ref());
            self.ctx.ClearRenderTargetView(rtv, &clear);
            self.ctx.ClearDepthStencilView(
                self.dsv.as_ref().unwrap(),
                D3D11_CLEAR_DEPTH.0 as u32,
                1.0,
                0,
            );
            self.ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: self.width as f32,
                Height: self.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
        }
    }

    pub fn present(&self, vsync: bool) -> Result<()> {
        let (interval, flags) = if vsync {
            (1, DXGI_PRESENT(0))
        } else if self.allow_tearing {
            (0, DXGI_PRESENT_ALLOW_TEARING)
        } else {
            (0, DXGI_PRESENT(0))
        };
        unsafe { self.swapchain.Present(interval, flags).ok() }
    }

    /// Creates a buffer with initial contents.
    pub fn buffer(
        &self,
        data: &[u8],
        bind: D3D11_BIND_FLAG,
        usage: D3D11_USAGE,
    ) -> Result<ID3D11Buffer> {
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: data.len() as u32,
            Usage: usage,
            BindFlags: bind.0 as u32,
            ..Default::default()
        };
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: data.as_ptr() as *const _,
            ..Default::default()
        };
        let mut buf = None;
        unsafe {
            self.device.CreateBuffer(&desc, Some(&init), Some(&mut buf))?;
        }
        Ok(buf.unwrap())
    }

    /// Creates an empty dynamic constant buffer (CPU write, map-discard).
    pub fn dynamic_cbuffer(&self, size: usize) -> Result<ID3D11Buffer> {
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: size as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            ..Default::default()
        };
        let mut buf = None;
        unsafe {
            self.device.CreateBuffer(&desc, None, Some(&mut buf))?;
        }
        Ok(buf.unwrap())
    }

    /// Uploads `data` into a dynamic buffer via map-discard.
    pub fn update<T: Copy>(&self, buf: &ID3D11Buffer, data: &T) -> Result<()> {
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx
                .Map(buf, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            std::ptr::copy_nonoverlapping(
                data as *const T as *const u8,
                mapped.pData as *mut u8,
                std::mem::size_of::<T>(),
            );
            self.ctx.Unmap(buf, 0);
        }
        Ok(())
    }
}

pub fn blob_bytes(blob: &ID3DBlob) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize())
    }
}

/// Compiles HLSL source with fxc. Returns the compile log as the error so
/// hot-reload failures are readable.
pub fn compile_shader(
    src: &str,
    name: &str,
    entry: &str,
    target: &str,
) -> std::result::Result<ID3DBlob, String> {
    let name_c = std::ffi::CString::new(name).unwrap();
    let entry_c = std::ffi::CString::new(entry).unwrap();
    let target_c = std::ffi::CString::new(target).unwrap();
    let mut blob: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    let hr = unsafe {
        D3DCompile(
            src.as_ptr() as *const _,
            src.len(),
            PCSTR(name_c.as_ptr() as *const u8),
            None,
            None,
            PCSTR(entry_c.as_ptr() as *const u8),
            PCSTR(target_c.as_ptr() as *const u8),
            D3DCOMPILE_ENABLE_STRICTNESS | D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut blob,
            Some(&mut errors),
        )
    };
    match hr {
        Ok(()) => Ok(blob.unwrap()),
        Err(e) => {
            let log = errors
                .map(|b| String::from_utf8_lossy(blob_bytes(&b)).into_owned())
                .unwrap_or_else(|| e.to_string());
            Err(log)
        }
    }
}
