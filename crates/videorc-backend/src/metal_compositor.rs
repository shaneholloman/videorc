//! Metal/GPU compositor core (plan Phase 3).
//!
//! The shipping compositor composes frames with a CPU YUV420P loop. OBS composes on the
//! GPU. This module is the GPU foundation: it creates a Metal device and composites
//! textured source quads into an offscreen render target on the GPU, proving the path
//! works on this hardware (Apple M4 / Metal 4) before it is wired into the live
//! preview/recording hot path (the remaining integration, which needs on-device visual
//! validation and a zero-copy IOSurface export to the encoder).
//!
//! macOS-only. Everything renders to an offscreen `MTLTexture` and reads the pixels back,
//! so it is testable headlessly (no window) wherever a Metal device is available.

#![cfg(target_os = "macos")]
#![allow(dead_code)]

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSString;
use objc2_metal::{
    MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue,
    MTLCreateSystemDefaultDevice, MTLDevice, MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat,
    MTLPrimitiveType, MTLRegion, MTLRenderCommandEncoder, MTLRenderPassDescriptor,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLResourceOptions, MTLSamplerDescriptor,
    MTLSamplerMinMagFilter, MTLSamplerState, MTLSize, MTLStoreAction, MTLTexture,
    MTLTextureDescriptor, MTLTextureUsage,
};

type MetalDevice = ProtocolObject<dyn MTLDevice>;
type MetalTexture = ProtocolObject<dyn MTLTexture>;

const SHADER_SOURCE: &str = r#"
#include <metal_stdlib>
using namespace metal;
struct VOut { float4 pos [[position]]; float2 uv; };
vertex VOut v_main(uint vid [[vertex_id]], const device float4* verts [[buffer(0)]]) {
    VOut out;
    float4 v = verts[vid];
    out.pos = float4(v.x, v.y, 0.0, 1.0);
    out.uv = float2(v.z, v.w);
    return out;
}
fragment float4 f_main(VOut in [[stage_in]],
                       texture2d<float> tex [[texture(0)]],
                       sampler samp [[sampler(0)]]) {
    return tex.sample(samp, in.uv);
}
"#;

/// One source layer to composite: BGRA8 pixels at `width`×`height`, drawn into the
/// destination rectangle `dest` = (x, y, w, h) in normalized [0,1] coordinates with the
/// origin at the top-left (the convention the scene model uses).
pub struct GpuSource<'a> {
    pub bgra: &'a [u8],
    pub width: usize,
    pub height: usize,
    pub dest: [f32; 4],
}

/// True when a Metal device is available on this machine.
pub fn metal_available() -> bool {
    MTLCreateSystemDefaultDevice().is_some()
}

/// Render a solid clear colour into an offscreen BGRA8 texture and read the pixels back.
/// `rgba` components are 0.0..=1.0. Returns `None` when no Metal device is available.
pub fn metal_clear_probe(width: usize, height: usize, rgba: [f64; 4]) -> Option<Vec<u8>> {
    let device = MTLCreateSystemDefaultDevice()?;
    let queue = device.newCommandQueue()?;
    let texture = make_texture(
        &device,
        width,
        height,
        MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead,
    )?;

    let command_buffer = queue.commandBuffer()?;
    let encoder = {
        let pass = clear_pass(&texture, rgba);
        command_buffer.renderCommandEncoderWithDescriptor(&pass)?
    };
    encoder.endEncoding();
    command_buffer.commit();
    command_buffer.waitUntilCompleted();

    Some(read_texture_bgra(&texture, width, height))
}

/// Composite `sources` over a cleared `background` into a `out_width`×`out_height` BGRA8
/// render target on the GPU, and read the result back. Returns `None` when no Metal
/// device is available. This is the GPU analogue of the CPU compositor's per-source blit.
pub fn composite_sources(
    out_width: usize,
    out_height: usize,
    background: [f64; 4],
    sources: &[GpuSource<'_>],
) -> Option<Vec<u8>> {
    let device = MTLCreateSystemDefaultDevice()?;
    let queue = device.newCommandQueue()?;
    let pipeline = build_pipeline(&device)?;
    let sampler = build_sampler(&device)?;
    let target = make_texture(
        &device,
        out_width,
        out_height,
        MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead,
    )?;

    let command_buffer = queue.commandBuffer()?;
    let encoder = {
        let pass = clear_pass(&target, background);
        command_buffer.renderCommandEncoderWithDescriptor(&pass)?
    };
    encoder.setRenderPipelineState(&pipeline);
    unsafe { encoder.setFragmentSamplerState_atIndex(Some(&sampler), 0) };

    for source in sources {
        let texture = upload_texture(&device, source)?;
        let vertices = quad_vertices(source.dest);
        let buffer = unsafe {
            device.newBufferWithBytes_length_options(
                NonNull::new(vertices.as_ptr() as *mut c_void)?,
                std::mem::size_of_val(&vertices),
                MTLResourceOptions::StorageModeShared,
            )?
        };
        unsafe {
            encoder.setVertexBuffer_offset_atIndex(Some(&buffer), 0, 0);
            encoder.setFragmentTexture_atIndex(Some(&texture), 0);
            encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 6);
        }
    }

    encoder.endEncoding();
    command_buffer.commit();
    command_buffer.waitUntilCompleted();

    Some(read_texture_bgra(&target, out_width, out_height))
}

// --- helpers ---

fn make_texture(
    device: &MetalDevice,
    width: usize,
    height: usize,
    usage: MTLTextureUsage,
) -> Option<Retained<MetalTexture>> {
    let descriptor = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            MTLPixelFormat::BGRA8Unorm,
            width,
            height,
            false,
        )
    };
    descriptor.setUsage(usage);
    device.newTextureWithDescriptor(&descriptor)
}

fn clear_pass(texture: &MetalTexture, rgba: [f64; 4]) -> Retained<MTLRenderPassDescriptor> {
    let pass = MTLRenderPassDescriptor::new();
    let attachment = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
    attachment.setTexture(Some(texture));
    attachment.setLoadAction(MTLLoadAction::Clear);
    attachment.setClearColor(MTLClearColor {
        red: rgba[0],
        green: rgba[1],
        blue: rgba[2],
        alpha: rgba[3],
    });
    attachment.setStoreAction(MTLStoreAction::Store);
    pass
}

fn build_pipeline(device: &MetalDevice) -> Option<Retained<ProtocolObject<dyn MTLRenderPipelineState>>> {
    let source = NSString::from_str(SHADER_SOURCE);
    let library = device
        .newLibraryWithSource_options_error(&source, None)
        .ok()?;
    let vertex = library.newFunctionWithName(&NSString::from_str("v_main"))?;
    let fragment = library.newFunctionWithName(&NSString::from_str("f_main"))?;

    let descriptor = MTLRenderPipelineDescriptor::new();
    descriptor.setVertexFunction(Some(&vertex));
    descriptor.setFragmentFunction(Some(&fragment));
    let attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
    attachment.setPixelFormat(MTLPixelFormat::BGRA8Unorm);

    device.newRenderPipelineStateWithDescriptor_error(&descriptor).ok()
}

fn build_sampler(device: &MetalDevice) -> Option<Retained<ProtocolObject<dyn MTLSamplerState>>> {
    let descriptor = MTLSamplerDescriptor::new();
    descriptor.setMinFilter(MTLSamplerMinMagFilter::Nearest);
    descriptor.setMagFilter(MTLSamplerMinMagFilter::Nearest);
    device.newSamplerStateWithDescriptor(&descriptor)
}

fn upload_texture(device: &MetalDevice, source: &GpuSource<'_>) -> Option<Retained<MetalTexture>> {
    let texture = make_texture(device, source.width, source.height, MTLTextureUsage::ShaderRead)?;
    let region = MTLRegion {
        origin: MTLOrigin { x: 0, y: 0, z: 0 },
        size: MTLSize {
            width: source.width,
            height: source.height,
            depth: 1,
        },
    };
    unsafe {
        texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
            region,
            0,
            NonNull::new(source.bgra.as_ptr() as *mut c_void)?,
            source.width * 4,
        );
    }
    Some(texture)
}

/// Two triangles (6 vertices) covering `dest` = (x, y, w, h) in top-left-origin [0,1]
/// space, each vertex packed as float4(ndc_x, ndc_y, u, v).
fn quad_vertices(dest: [f32; 4]) -> [f32; 24] {
    let [x, y, w, h] = dest;
    let x0 = 2.0 * x - 1.0;
    let x1 = 2.0 * (x + w) - 1.0;
    let y0 = 1.0 - 2.0 * y;
    let y1 = 1.0 - 2.0 * (y + h);
    [
        x0, y0, 0.0, 0.0, // top-left
        x0, y1, 0.0, 1.0, // bottom-left
        x1, y0, 1.0, 0.0, // top-right
        x1, y0, 1.0, 0.0, // top-right
        x0, y1, 0.0, 1.0, // bottom-left
        x1, y1, 1.0, 1.0, // bottom-right
    ]
}

fn read_texture_bgra(texture: &MetalTexture, width: usize, height: usize) -> Vec<u8> {
    let bytes_per_row = width * 4;
    let mut out = vec![0u8; bytes_per_row * height];
    let region = MTLRegion {
        origin: MTLOrigin { x: 0, y: 0, z: 0 },
        size: MTLSize {
            width,
            height,
            depth: 1,
        },
    };
    if let Some(ptr) = NonNull::new(out.as_mut_ptr() as *mut c_void) {
        unsafe {
            texture.getBytes_bytesPerRow_fromRegion_mipmapLevel(ptr, bytes_per_row, region, 0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(buf: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * width + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    #[test]
    fn metal_clear_renders_the_requested_colour_or_skips_without_a_gpu() {
        // Clear to opaque red → BGRA [0, 0, 255, 255].
        let Some(pixels) = metal_clear_probe(4, 4, [1.0, 0.0, 0.0, 1.0]) else {
            eprintln!("skipping: no Metal device available in this environment");
            return;
        };
        assert_eq!(pixels.len(), 4 * 4 * 4);
        for chunk in pixels.chunks_exact(4) {
            assert_eq!(chunk, [0, 0, 255, 255]);
        }
    }

    #[test]
    fn gpu_composites_a_source_quad_over_the_background_or_skips_without_a_gpu() {
        if !metal_available() {
            eprintln!("skipping: no Metal device available in this environment");
            return;
        }
        // 8×8 blue background; a 2×2 solid-green source drawn into the right half
        // (dest x=0.5,y=0, w=0.5,h=1.0 → covers columns 4..8).
        let out = 8usize;
        let green = vec![0u8, 255, 0, 255].repeat(2 * 2); // BGRA green, 2×2
        let sources = [GpuSource {
            bgra: &green,
            width: 2,
            height: 2,
            dest: [0.5, 0.0, 0.5, 1.0],
        }];
        let pixels = composite_sources(out, out, [0.0, 0.0, 1.0, 1.0], &sources).unwrap();
        assert_eq!(pixels.len(), out * out * 4);

        // Left half stays background blue; right half is the green source.
        assert_eq!(pixel(&pixels, out, 1, 4), [255, 0, 0, 255], "left should be blue");
        assert_eq!(pixel(&pixels, out, 6, 4), [0, 255, 0, 255], "right should be green");
    }
}
