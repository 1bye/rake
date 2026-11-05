//! Independent wgpu 28 producer; accepts a host target and never opens a window.
#![cfg(all(target_os = "macos", feature = "gpu"))]

use anyhow::{Context as _, Result};
use web_time::Instant;

mod input;
mod native_input;
mod text_input;
pub use native_input::NativeInputController;
pub use text_input::TextInput;

/// CPU submission timing, excluding GPU completion; elapsed time includes pacing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Metrics {
    pub frames: u64,
    pub mean_submit_ms: f64,
    pub last_submit_ms: f64,
    pub max_submit_ms: f64,
}

/// A renderer with its own device, queue, swapchain and pacing supplied by its caller.
pub struct Renderer {
    // The surface must be released before the retained platform target.
    surface: wgpu::Surface<'static>,
    _target: metal::MetalLayer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    started: Instant,
    metrics: Metrics,
    input: input::InputState,
}

impl Renderer {
    /// Copies validated native input into this renderer on its owning thread.
    pub fn update_input(&mut self, controller: &NativeInputController) {
        self.input = controller.snapshot();
    }

    /// Describes input actually copied into the renderer, for spike diagnostics.
    pub fn input_diagnostics(&self) -> String {
        self.input.diagnostics()
    }

    /// Creates an independent device for the supplied target.
    ///
    /// # Errors
    /// Returns adapter, device or unsupported surface configuration failures.
    pub async fn attach(target: metal::MetalLayer) -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = {
            use foreign_types::ForeignType as _;
            // SAFETY: MetalLayer owns a valid CAMetalLayer, retained in Self until
            // after Surface is dropped. AppKit only manipulates its host view.
            unsafe {
                instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(
                    target.as_ptr().cast(),
                ))?
            }
        };
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await?;
        let config = surface
            .get_default_config(&adapter, 1, 1)
            .context("no supported surface configuration")?;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("surface spike edges and moving shape"),
            source: wgpu::ShaderSource::Wgsl(include_str!("pattern.wgsl").into()),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("time and physical dimensions"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            surface,
            _target: target,
            device,
            queue,
            config,
            pipeline,
            uniform,
            bind_group,
            started: Instant::now(),
            metrics: Metrics::default(),
            input: input::InputState::default(),
        })
    }

    /// Reports actual device limits for comparing the independent producer with its host.
    pub fn limits(&self) -> wgpu::Limits {
        self.device.limits()
    }

    /// Configures physical dimensions and draws one frame. Zero extent pauses presentation.
    ///
    /// # Errors
    /// Returns swapchain acquisition failures, allowing the owner to stop or retry.
    pub fn frame(&mut self, width_px: u32, height_px: u32) -> Result<()> {
        if width_px == 0 || height_px == 0 {
            return Ok(());
        }
        let begin = Instant::now();
        if self.metrics.frames == 0
            || self.config.width != width_px
            || self.config.height != height_px
        {
            self.config.width = width_px;
            self.config.height = height_px;
            self.surface.configure(&self.device, &self.config);
        }
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.config);
                self.surface.get_current_texture()?
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let values = [
            self.started.elapsed().as_secs_f32(),
            width_px as f32,
            height_px as f32,
            0.0,
            self.input.position[0],
            self.input.position[1],
            self.input.zoom,
            f32::from(self.input.focused),
            f32::from(self.input.has_pointer),
            f32::from(self.input.pressed),
            (self.input.key_downs % 2) as f32,
            0.,
        ];
        let mut bytes = [0_u8; 48];
        for (slot, value) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(values) {
            slot.copy_from_slice(&value.to_ne_bytes());
        }
        self.queue.write_buffer(&self.uniform, 0, &bytes);
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        frame.present();
        let ms = begin.elapsed().as_secs_f64() * 1000.;
        self.metrics.frames += 1;
        self.metrics.last_submit_ms = ms;
        self.metrics.mean_submit_ms +=
            (ms - self.metrics.mean_submit_ms) / self.metrics.frames as f64;
        self.metrics.max_submit_ms = self.metrics.max_submit_ms.max(ms);
        Ok(())
    }

    /// Returns submission metrics without waiting for GPU completion.
    pub fn metrics(&self) -> Metrics {
        self.metrics
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn animated_pattern_is_valid_for_the_producer_shader_version() {
        let module = naga::front::wgsl::parse_str(include_str!("pattern.wgsl"))
            .expect("the standalone producer shader must parse");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .expect("the producer shader must validate without optional device capabilities");
    }
}
