//! Measures what a GPU path would cost for the two stages that fit compute shaders
//! best: bilinear resize (1920x1080 to 960x540) and Bayer 4x4 dithering to PICO-8 with
//! redmean (640x480), the same sizes as the CPU benchmarks in docs/performance.md.
//!
//! Run with `cargo run --release` in this folder. It is not part of the workspace and
//! xd-art does not use it; docs/performance.md has the numbers and the decision.
use std::time::{Duration, Instant};

const RESIZE: &str = r#"
@group(0) @binding(0) var<storage, read> src: array<u32>;
@group(0) @binding(1) var<storage, read_write> dst: array<u32>;
const SW: u32 = 1920u; const SH: u32 = 1080u; const DW: u32 = 960u; const DH: u32 = 540u;
fn px(x: u32, y: u32) -> vec4<f32> { return unpack4x8unorm(src[y * SW + x]); }
@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= DW || id.y >= DH) { return; }
    let fx = (f32(id.x) + 0.5) * f32(SW) / f32(DW) - 0.5;
    let fy = (f32(id.y) + 0.5) * f32(SH) / f32(DH) - 0.5;
    let x0 = u32(clamp(floor(fx), 0.0, f32(SW - 1u)));
    let y0 = u32(clamp(floor(fy), 0.0, f32(SH - 1u)));
    let x1 = min(x0 + 1u, SW - 1u); let y1 = min(y0 + 1u, SH - 1u);
    let tx = clamp(fx - floor(fx), 0.0, 1.0); let ty = clamp(fy - floor(fy), 0.0, 1.0);
    let c = mix(mix(px(x0, y0), px(x1, y0), tx), mix(px(x0, y1), px(x1, y1), tx), ty);
    dst[id.y * DW + id.x] = pack4x8unorm(c);
}
"#;

const DITHER: &str = r#"
@group(0) @binding(0) var<storage, read> src: array<u32>;
@group(0) @binding(1) var<storage, read_write> dst: array<u32>;
const W: u32 = 640u; const H: u32 = 480u;
const PAL = array<vec3<f32>, 16>(
    vec3(0.,0.,0.), vec3(29.,43.,83.), vec3(126.,37.,83.), vec3(0.,135.,81.),
    vec3(171.,82.,54.), vec3(95.,87.,79.), vec3(194.,195.,199.), vec3(255.,241.,232.),
    vec3(255.,0.,77.), vec3(255.,163.,0.), vec3(255.,236.,39.), vec3(0.,228.,54.),
    vec3(41.,173.,255.), vec3(131.,118.,156.), vec3(255.,119.,168.), vec3(255.,204.,170.));
const BAYER = array<f32, 16>(0.,8.,2.,10.,12.,4.,14.,6.,3.,11.,1.,9.,15.,7.,13.,5.);
@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= W || id.y >= H) { return; }
    let s = unpack4x8unorm(src[id.y * W + id.x]) * 255.0;
    let t = (BAYER[(id.y % 4u) * 4u + id.x % 4u] / 16.0 - 0.5) * 0.2 * 255.0;
    let c = clamp(s.rgb + vec3(t), vec3(0.0), vec3(255.0));
    var best = 0u; var bd = 1e30;
    for (var i = 0u; i < 16u; i++) {
        let p = PAL[i]; let r = (c.r + p.r) * 0.5; let d = c - p;
        let e = (2.0 + r / 256.0) * d.r * d.r + 4.0 * d.g * d.g + (2.0 + (255.0 - r) / 256.0) * d.b * d.b;
        if (e < bd) { bd = e; best = i; }
    }
    dst[id.y * W + id.x] = pack4x8unorm(vec4(PAL[best] / 255.0, 1.0));
}
"#;

fn median(mut v: Vec<Duration>) -> f64 {
    v.sort();
    v[v.len() / 2].as_secs_f64() * 1000.0
}

struct Job {
    pipe: wgpu::ComputePipeline,
    bind: wgpu::BindGroup,
    src: wgpu::Buffer,
    dst: wgpu::Buffer,
    read: wgpu::Buffer,
    groups: (u32, u32),
    out_bytes: u64,
}

fn job(dev: &wgpu::Device, code: &str, src_px: usize, dst_px: usize, groups: (u32, u32)) -> Job {
    let module = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(code.into()),
    });
    let pipe = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let src = dev.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (src_px * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let out_bytes = (dst_px * 4) as u64;
    let dst = dev.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: out_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = dev.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: out_bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind = dev.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipe.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: src.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: dst.as_entire_binding(),
            },
        ],
    });
    Job {
        pipe,
        bind,
        src,
        dst,
        read,
        groups,
        out_bytes,
    }
}

/// One full round trip: upload, dispatch, copy back, map, read. With `full` false the
/// source stays on the GPU and nothing is read back (dispatch and wait only).
fn run(dev: &wgpu::Device, q: &wgpu::Queue, j: &Job, input: &[u8], full: bool) -> Vec<u8> {
    if full {
        q.write_buffer(&j.src, 0, input);
    }
    let mut enc = dev.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_compute_pass(&Default::default());
        pass.set_pipeline(&j.pipe);
        pass.set_bind_group(0, &j.bind, &[]);
        pass.dispatch_workgroups(j.groups.0, j.groups.1, 1);
    }
    if full {
        enc.copy_buffer_to_buffer(&j.dst, 0, &j.read, 0, j.out_bytes);
    }
    q.submit([enc.finish()]);
    if !full {
        dev.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        return Vec::new();
    }
    let slice = j.read.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.unwrap());
    dev.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let out = slice.get_mapped_range().unwrap().to_vec();
    j.read.unmap();
    out
}

fn main() {
    let t0 = Instant::now();
    let inst = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(inst.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .unwrap();
    let t1 = Instant::now();
    let (dev, q) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let t2 = Instant::now();
    let info = adapter.get_info();
    println!("adapter: {} ({:?})", info.name, info.backend);
    println!(
        "instance + adapter: {:.1} ms",
        (t1 - t0).as_secs_f64() * 1000.0
    );
    println!("device: {:.1} ms", (t2 - t1).as_secs_f64() * 1000.0);

    let t3 = Instant::now();
    let resize = job(
        &dev,
        RESIZE,
        1920 * 1080,
        960 * 540,
        (960u32.div_ceil(16), 540u32.div_ceil(16)),
    );
    let dither = job(
        &dev,
        DITHER,
        640 * 480,
        640 * 480,
        (640u32.div_ceil(16), 480u32.div_ceil(16)),
    );
    println!(
        "shaders + buffers: {:.1} ms",
        t3.elapsed().as_secs_f64() * 1000.0
    );

    // Deterministic noise-ish input, like the bench's synthetic images.
    let mk = |n: usize| -> Vec<u8> {
        let mut s = 0x1234_5678u32;
        (0..n * 4)
            .map(|i| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                if i % 4 == 3 { 255 } else { (s >> 24) as u8 }
            })
            .collect()
    };
    let big = mk(1920 * 1080);
    let small = mk(640 * 480);

    for (name, j, input) in [
        ("resize 1920x1080 -> 960x540", &resize, &big),
        ("bayer4 pico-8 640x480", &dither, &small),
    ] {
        let first = Instant::now();
        run(&dev, &q, j, input, true);
        let first = first.elapsed();
        let mut full = Vec::new();
        let mut kernel = Vec::new();
        for _ in 0..50 {
            let t = Instant::now();
            run(&dev, &q, j, input, true);
            full.push(t.elapsed());
            let t = Instant::now();
            run(&dev, &q, j, input, false);
            kernel.push(t.elapsed());
        }
        println!(
            "{name}: first {:.2} ms, round trip {:.2} ms, dispatch only {:.2} ms (median of 50)",
            first.as_secs_f64() * 1000.0,
            median(full),
            median(kernel)
        );
    }
}
