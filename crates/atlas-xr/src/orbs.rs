//! Orbs: glass spheres with a glowing rim, drawn in 3D inside the
//! projection layer (multiview: a single pass for both eyes).

use std::io::Cursor;

use ash::vk;
use atlas_spatial::{icosphere, EyeCamera, HandMeshData, HandVertex, Vec3};

use crate::vk_ctx::{VkContext, PIPELINE_DEPTH};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const VERT: &[u8] = include_bytes!("../shaders/orb.vert.spv");
const FRAG: &[u8] = include_bytes!("../shaders/orb.frag.spv");
const SKY_VERT: &[u8] = include_bytes!("../shaders/sky.vert.spv");
const SKY_FRAG: &[u8] = include_bytes!("../shaders/sky.frag.spv");
const ROLL_VERT: &[u8] = include_bytes!("../shaders/roll.vert.spv");
const ROLL_FRAG: &[u8] = include_bytes!("../shaders/roll.frag.spv");
const HAND_VERT: &[u8] = include_bytes!("../shaders/hand.vert.spv");
const HAND_FRAG: &[u8] = include_bytes!("../shaders/hand.frag.spv");
const SURF_VERT: &[u8] = include_bytes!("../shaders/surface.vert.spv");
const SURF_FRAG: &[u8] = include_bytes!("../shaders/surface.frag.spv");

/// Sphere subdivisions: 3 is 1280 triangles, enough for a small orb.
const SPHERE_SUBDIVISIONS: u32 = 3;

/// An orb to draw.
#[derive(Debug, Clone, Copy)]
pub struct OrbInstance {
    pub center: Vec3,
    pub radius: f32,
    /// Warm color of the core.
    pub core: [f32; 3],
    /// Cool color of the rim.
    pub rim: [f32; 3],
    /// Brightness multiplier (1 = normal; more to highlight it).
    pub intensity: f32,
    /// Pulse amplitude (fraction of the radius) and its speed in rad/s.
    pub pulse: f32,
    pub speed: f32,
    pub phase: f32,
}

/// Flat horizontal surface (the desk) with its wave effect.
#[derive(Debug, Clone, Copy)]
pub struct Surface {
    pub center: Vec3,
    /// Half the side of the square that is drawn (m).
    pub half: f32,
    pub color: [f32; 3],
    /// Charge progress, from 0 to 1 (only while calibrating).
    pub progress: f32,
    /// Seconds since it was mapped; negative while calibrating.
    pub age: f32,
    /// Where the surface faces: (0, 1, 0) for the desk, horizontal for a wall.
    pub normal: Vec3,
    /// Rotation (rad) around the vertical, for horizontal surfaces.
    pub yaw: f32,
}

/// A hand to draw this frame: its already skinned mesh and its color.
pub struct HandDraw {
    /// 0 = left, 1 = right.
    pub hand: usize,
    pub vertices: Vec<HandVertex>,
    pub tint: [f32; 3],
    pub alpha: f32,
}

/// What the GPU keeps of the hand meshes.
struct HandGpu {
    index_buffers: [HostBuffer; 2],
    index_counts: [u32; 2],
    vertex_counts: [usize; 2],
    /// One vertex buffer per frame in flight (both hands in a row).
    vertex_buffers: Vec<HostBuffer>,
}

/// Roll of the scroll: a holographic tube along `axis`.
#[derive(Debug, Clone, Copy)]
pub struct Roll {
    pub center: Vec3,
    pub axis: Vec3,
    pub half_length: f32,
    pub radius: f32,
    pub tint: [f32; 3],
}

/// Frame data (std140): one matrix and one position per eye.
#[repr(C)]
#[derive(Clone, Copy)]
struct FrameData {
    view_proj: [[f32; 16]; 2],
    cam_pos: [[f32; 4]; 2],
    time: [f32; 4],
    /// Inverses of view_proj, to know where each pixel looks (the environment).
    inv_view_proj: [[f32; 16]; 2],
}

/// Per-orb data, sent as push constants (64 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
struct OrbPush {
    center_radius: [f32; 4],
    color_core: [f32; 4],
    color_rim: [f32; 4],
    params: [f32; 4],
}

struct HostBuffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    ptr: *mut u8,
}

pub struct OrbRenderer {
    set_layout: vk::DescriptorSetLayout,
    pool: vk::DescriptorPool,
    sets: Vec<vk::DescriptorSet>,
    frame_buffers: Vec<HostBuffer>,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    surface_pipeline: vk::Pipeline,
    sky_pipeline: vk::Pipeline,
    roll_pipeline: vk::Pipeline,
    hand_pipeline: vk::Pipeline,
    hand_gpu: Option<HandGpu>,
    vertices: HostBuffer,
    indices: HostBuffer,
    index_count: u32,
}

impl OrbRenderer {
    /// # Safety
    /// `vk` must be a valid context whose render pass is compatible with the
    /// pipeline (one color attachment and two-view multiview).
    pub unsafe fn new(vk: &VkContext) -> Result<Self> {
        let device = &vk.device;

        let mesh = icosphere(SPHERE_SUBDIVISIONS);
        let vertices = host_buffer(vk, (mesh.positions.len() * 12) as u64, vk::BufferUsageFlags::VERTEX_BUFFER)?;
        std::ptr::copy_nonoverlapping(mesh.positions.as_ptr() as *const u8, vertices.ptr, mesh.positions.len() * 12);
        let indices = host_buffer(vk, (mesh.indices.len() * 4) as u64, vk::BufferUsageFlags::INDEX_BUFFER)?;
        std::ptr::copy_nonoverlapping(mesh.indices.as_ptr() as *const u8, indices.ptr, mesh.indices.len() * 4);

        // One frame data buffer per frame in flight.
        let frame_size = std::mem::size_of::<FrameData>() as u64;
        let frame_buffers = (0..PIPELINE_DEPTH)
            .map(|_| host_buffer(vk, frame_size, vk::BufferUsageFlags::UNIFORM_BUFFER))
            .collect::<Result<Vec<_>>>()?;

        let bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)];
        let set_layout =
            device.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings), None)?;
        let pool_sizes = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(PIPELINE_DEPTH)];
        let pool = device.create_descriptor_pool(
            &vk::DescriptorPoolCreateInfo::default().max_sets(PIPELINE_DEPTH).pool_sizes(&pool_sizes),
            None,
        )?;
        let layouts = vec![set_layout; PIPELINE_DEPTH as usize];
        let sets = device
            .allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(pool).set_layouts(&layouts))?;
        for (set, buf) in sets.iter().zip(&frame_buffers) {
            let info = [vk::DescriptorBufferInfo::default().buffer(buf.buffer).offset(0).range(frame_size)];
            device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .buffer_info(&info)],
                &[],
            );
        }

        let push_ranges = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
            .offset(0)
            .size(std::mem::size_of::<OrbPush>() as u32)];
        let set_layouts = [set_layout];
        let pipeline_layout = device.create_pipeline_layout(
            &vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts).push_constant_ranges(&push_ranges),
            None,
        )?;

        let pipeline = create_pipeline(vk, pipeline_layout, VERT, FRAG, VertexLayout::Position)?;
        let surface_pipeline = create_pipeline(vk, pipeline_layout, SURF_VERT, SURF_FRAG, VertexLayout::None)?;
        let sky_pipeline = create_pipeline(vk, pipeline_layout, SKY_VERT, SKY_FRAG, VertexLayout::None)?;
        let roll_pipeline = create_pipeline(vk, pipeline_layout, ROLL_VERT, ROLL_FRAG, VertexLayout::None)?;
        let hand_pipeline = create_pipeline(vk, pipeline_layout, HAND_VERT, HAND_FRAG, VertexLayout::PositionNormal)?;

        Ok(Self {
            set_layout,
            pool,
            sets,
            frame_buffers,
            pipeline_layout,
            pipeline,
            surface_pipeline,
            sky_pipeline,
            roll_pipeline,
            hand_pipeline,
            hand_gpu: None,
            vertices,
            indices,
            index_count: mesh.indices.len() as u32,
        })
    }

    /// Stores the meshes of both hands on the GPU (done once, at the start).
    ///
    /// # Safety
    /// `vk` must be the context the renderer was created with.
    pub unsafe fn set_hand_meshes(&mut self, vk: &VkContext, meshes: &[HandMeshData; 2]) -> Result<()> {
        let make_index = |m: &HandMeshData| -> Result<HostBuffer> {
            let buf = host_buffer(vk, (m.indices.len() * 2) as u64, vk::BufferUsageFlags::INDEX_BUFFER)?;
            std::ptr::copy_nonoverlapping(m.indices.as_ptr() as *const u8, buf.ptr, m.indices.len() * 2);
            Ok(buf)
        };
        let total = (meshes[0].vertex_count() + meshes[1].vertex_count()) * std::mem::size_of::<HandVertex>();
        let vertex_buffers = (0..PIPELINE_DEPTH)
            .map(|_| host_buffer(vk, total as u64, vk::BufferUsageFlags::VERTEX_BUFFER))
            .collect::<Result<Vec<_>>>()?;
        self.hand_gpu = Some(HandGpu {
            index_buffers: [make_index(&meshes[0])?, make_index(&meshes[1])?],
            index_counts: [meshes[0].indices.len() as u32, meshes[1].indices.len() as u32],
            vertex_counts: [meshes[0].vertex_count(), meshes[1].vertex_count()],
            vertex_buffers,
        });
        Ok(())
    }

    /// Records the drawing of the orbs. It must be called inside the render pass.
    ///
    /// # Safety
    /// `cmd` must be recording inside `vk`'s render pass, and `frame` must be the
    /// frame in flight whose fence was already waited on.
    pub unsafe fn record(
        &self,
        device: &ash::Device,
        cmd: vk::CommandBuffer,
        frame: usize,
        extent: vk::Extent2D,
        eyes: &[EyeCamera; 2],
        seconds: f32,
        orbs: &[OrbInstance],
        surfaces: &[Surface],
        environment: u32,
        rolls: &[Roll],
        hands: &[HandDraw],
    ) {
        if orbs.is_empty() && surfaces.is_empty() && rolls.is_empty() && hands.is_empty() && environment == 0 {
            return;
        }
        let data = FrameData {
            view_proj: [eyes[0].view_proj.to_cols_array(), eyes[1].view_proj.to_cols_array()],
            cam_pos: [
                [eyes[0].position.x, eyes[0].position.y, eyes[0].position.z, 1.0],
                [eyes[1].position.x, eyes[1].position.y, eyes[1].position.z, 1.0],
            ],
            time: [seconds, 0.0, 0.0, 0.0],
            inv_view_proj: [eyes[0].view_proj.inverse().to_cols_array(), eyes[1].view_proj.inverse().to_cols_array()],
        };
        std::ptr::copy_nonoverlapping(
            &data as *const FrameData as *const u8,
            self.frame_buffers[frame].ptr,
            std::mem::size_of::<FrameData>(),
        );

        device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.surface_pipeline);
        device.cmd_set_viewport(
            cmd,
            0,
            &[vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: extent.width as f32,
                height: extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            }],
        );
        device.cmd_set_scissor(cmd, 0, &[vk::Rect2D { offset: vk::Offset2D::default(), extent }]);
        device.cmd_bind_descriptor_sets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            self.pipeline_layout,
            0,
            &[self.sets[frame]],
            &[],
        );
        // The environment, if there is one, covers the whole background.
        if environment > 0 {
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.sky_pipeline);
            let push = OrbPush { center_radius: [environment as f32, 0.0, 0.0, 0.0], color_core: [0.0; 4], color_rim: [0.0; 4], params: [0.0; 4] };
            let bytes = std::slice::from_raw_parts(&push as *const OrbPush as *const u8, std::mem::size_of::<OrbPush>());
            device.cmd_push_constants(cmd, self.pipeline_layout, vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT, 0, bytes);
            device.cmd_draw(cmd, 3, 1, 0, 0);
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.surface_pipeline);
        }
        // First the surfaces (they go at the back) and then the orbs, without a depth test.
        for surface in surfaces {
            let push = OrbPush {
                center_radius: [surface.center.x, surface.center.y, surface.center.z, surface.half],
                color_core: [surface.color[0], surface.color[1], surface.color[2], 1.0],
                color_rim: [surface.progress, surface.age, 0.0, 0.0],
                params: [surface.normal.x, surface.normal.y, surface.normal.z, surface.yaw],
            };
            let bytes = std::slice::from_raw_parts(&push as *const OrbPush as *const u8, std::mem::size_of::<OrbPush>());
            device.cmd_push_constants(
                cmd,
                self.pipeline_layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                bytes,
            );
            device.cmd_draw(cmd, 6, 1, 0, 0);
        }
        if !rolls.is_empty() {
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.roll_pipeline);
            for roll in rolls {
                let push = OrbPush {
                    center_radius: [roll.center.x, roll.center.y, roll.center.z, roll.half_length],
                    color_core: [roll.axis.x, roll.axis.y, roll.axis.z, roll.radius],
                    color_rim: [roll.tint[0], roll.tint[1], roll.tint[2], 0.0],
                    params: [0.0; 4],
                };
                let bytes = std::slice::from_raw_parts(&push as *const OrbPush as *const u8, std::mem::size_of::<OrbPush>());
                device.cmd_push_constants(cmd, self.pipeline_layout, vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT, 0, bytes);
                device.cmd_draw(cmd, 6, 1, 0, 0);
            }
        }
        device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
        device.cmd_bind_vertex_buffers(cmd, 0, &[self.vertices.buffer], &[0]);
        device.cmd_bind_index_buffer(cmd, self.indices.buffer, 0, vk::IndexType::UINT32);

        for orb in orbs {
            let push = OrbPush {
                center_radius: [orb.center.x, orb.center.y, orb.center.z, orb.radius],
                color_core: [orb.core[0], orb.core[1], orb.core[2], 1.0],
                color_rim: [orb.rim[0], orb.rim[1], orb.rim[2], 1.0],
                params: [orb.intensity, orb.pulse, orb.speed, orb.phase],
            };
            let bytes = std::slice::from_raw_parts(&push as *const OrbPush as *const u8, std::mem::size_of::<OrbPush>());
            device.cmd_push_constants(
                cmd,
                self.pipeline_layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                bytes,
            );
            device.cmd_draw_indexed(cmd, self.index_count, 1, 0, 0, 0);
        }

        // The hands go on top of everything: a glass silhouette.
        if let (Some(gpu), false) = (&self.hand_gpu, hands.is_empty()) {
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.hand_pipeline);
            let stride = std::mem::size_of::<HandVertex>();
            for draw in hands.iter().filter(|d| d.hand < 2 && d.vertices.len() == gpu.vertex_counts[d.hand]) {
                let offset = if draw.hand == 0 { 0 } else { gpu.vertex_counts[0] * stride };
                let buffer = &gpu.vertex_buffers[frame];
                std::ptr::copy_nonoverlapping(draw.vertices.as_ptr() as *const u8, buffer.ptr.add(offset), draw.vertices.len() * stride);
                device.cmd_bind_vertex_buffers(cmd, 0, &[buffer.buffer], &[offset as u64]);
                device.cmd_bind_index_buffer(cmd, gpu.index_buffers[draw.hand].buffer, 0, vk::IndexType::UINT16);
                let push = OrbPush {
                    center_radius: [draw.tint[0], draw.tint[1], draw.tint[2], draw.alpha],
                    color_core: [0.0; 4],
                    color_rim: [0.0; 4],
                    params: [0.0; 4],
                };
                let bytes = std::slice::from_raw_parts(&push as *const OrbPush as *const u8, std::mem::size_of::<OrbPush>());
                device.cmd_push_constants(cmd, self.pipeline_layout, vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT, 0, bytes);
                device.cmd_draw_indexed(cmd, gpu.index_counts[draw.hand], 1, 0, 0, 0);
            }
        }
    }

    /// # Safety
    /// The GPU must no longer be using any of the renderer's resources.
    pub unsafe fn destroy(self, device: &ash::Device) {
        device.destroy_pipeline(self.pipeline, None);
        device.destroy_pipeline(self.surface_pipeline, None);
        device.destroy_pipeline(self.sky_pipeline, None);
        device.destroy_pipeline(self.roll_pipeline, None);
        device.destroy_pipeline(self.hand_pipeline, None);
        if let Some(gpu) = self.hand_gpu {
            for b in gpu.index_buffers.into_iter().chain(gpu.vertex_buffers) {
                device.unmap_memory(b.memory);
                device.destroy_buffer(b.buffer, None);
                device.free_memory(b.memory, None);
            }
        }
        device.destroy_pipeline_layout(self.pipeline_layout, None);
        device.destroy_descriptor_pool(self.pool, None);
        device.destroy_descriptor_set_layout(self.set_layout, None);
        for b in self.frame_buffers.into_iter().chain([self.vertices, self.indices]) {
            device.unmap_memory(b.memory);
            device.destroy_buffer(b.buffer, None);
            device.free_memory(b.memory, None);
        }
    }
}

/// What each vertex carries: nothing (the shader generates them), only position, or position and normal.
#[derive(Clone, Copy)]
enum VertexLayout {
    None,
    Position,
    PositionNormal,
}

impl VertexLayout {
    fn stride(self) -> u32 {
        match self {
            VertexLayout::None | VertexLayout::Position => 12,
            VertexLayout::PositionNormal => 24,
        }
    }
}

/// Creates a drawing pipeline with premultiplied alpha blending. With `vertex_buffer` it takes
/// positions from a vertex buffer; without it the shader itself generates the vertices.
unsafe fn create_pipeline(
    vk: &VkContext,
    layout: vk::PipelineLayout,
    vert_spv: &[u8],
    frag_spv: &[u8],
    layout_kind: VertexLayout,
) -> Result<vk::Pipeline> {
    let device = &vk.device;
    let vert = shader_module(device, vert_spv)?;
    let frag = shader_module(device, frag_spv)?;
    let entry = std::ffi::CStr::from_bytes_with_nul(b"main\0")?;
    let stages = [
        vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::VERTEX).module(vert).name(entry),
        vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::FRAGMENT).module(frag).name(entry),
    ];
    let binding = [vk::VertexInputBindingDescription::default()
        .binding(0)
        .stride(layout_kind.stride())
        .input_rate(vk::VertexInputRate::VERTEX)];
    let attributes = [
        vk::VertexInputAttributeDescription::default().location(0).binding(0).format(vk::Format::R32G32B32_SFLOAT).offset(0),
        vk::VertexInputAttributeDescription::default().location(1).binding(0).format(vk::Format::R32G32B32_SFLOAT).offset(12),
    ];
    let used = match layout_kind {
        VertexLayout::None => 0,
        VertexLayout::Position => 1,
        VertexLayout::PositionNormal => 2,
    };
    let vertex_input = if used == 0 {
        vk::PipelineVertexInputStateCreateInfo::default()
    } else {
        vk::PipelineVertexInputStateCreateInfo::default().vertex_binding_descriptions(&binding).vertex_attribute_descriptions(&attributes[..used])
    };
    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default().topology(vk::PrimitiveTopology::TRIANGLE_LIST);
    let viewport_state = vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1);
    let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);
    let multisample = vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(vk::SampleCountFlags::TYPE_1);
    // Premultiplied alpha, which is what the projection layer expects.
    let blend_attachment = [vk::PipelineColorBlendAttachmentState::default()
        .blend_enable(true)
        .src_color_blend_factor(vk::BlendFactor::ONE)
        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ONE)
        .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .alpha_blend_op(vk::BlendOp::ADD)
        .color_write_mask(vk::ColorComponentFlags::RGBA)];
    let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachment);
    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let created = device.create_graphics_pipelines(
        vk::PipelineCache::null(),
        &[vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterization)
            .multisample_state(&multisample)
            .color_blend_state(&blend)
            .dynamic_state(&dynamic)
            .layout(layout)
            .render_pass(vk.render_pass())
            .subpass(0)],
        None,
    );
    device.destroy_shader_module(vert, None);
    device.destroy_shader_module(frag, None);
    Ok(created.map_err(|(_, e)| e)?[0])
}

unsafe fn shader_module(device: &ash::Device, bytes: &[u8]) -> Result<vk::ShaderModule> {
    // SPIR-V is 32-bit words: `read_spv` takes care of the alignment.
    let words = ash::util::read_spv(&mut Cursor::new(bytes))?;
    Ok(device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None)?)
}

unsafe fn host_buffer(vk: &VkContext, size: u64, usage: vk::BufferUsageFlags) -> Result<HostBuffer> {
    let device = &vk.device;
    let buffer = device.create_buffer(
        &vk::BufferCreateInfo::default().size(size).usage(usage).sharing_mode(vk::SharingMode::EXCLUSIVE),
        None,
    )?;
    let req = device.get_buffer_memory_requirements(buffer);
    let mem_type = vk.memory_type(
        req.memory_type_bits,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;
    let memory = device
        .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(mem_type), None)?;
    device.bind_buffer_memory(buffer, memory, 0)?;
    let ptr = device.map_memory(memory, 0, size, vk::MemoryMapFlags::empty())? as *mut u8;
    Ok(HostBuffer { buffer, memory, ptr })
}

/// Names of the environments; 0 is the passthrough.
pub const ENVIRONMENTS: [&str; 4] = ["your room (passthrough)", "canyon at day", "sunset", "starry night"];

/// Environment orb: crushing it against the desk changes the environment. Its color anticipates
/// the next one.
pub fn env_orb(center: Vec3, current: u32) -> OrbInstance {
    let (core, rim) = match (current + 1) % 4 {
        1 => ([0.55, 0.80, 1.0], [0.9, 0.95, 1.0]),
        2 => ([1.0, 0.45, 0.25], [1.0, 0.75, 0.4]),
        3 => ([0.35, 0.35, 0.95], [0.75, 0.8, 1.0]),
        _ => ([0.5, 0.95, 0.7], [0.7, 1.0, 0.85]),
    };
    OrbInstance { center, radius: 0.05, core, rim, intensity: 1.3, pulse: 0.06, speed: 1.8, phase: 1.0 }
}

/// Colors of the pointed or grabbed orb: it lights up in warm orange.
pub fn highlight(orb: &mut OrbInstance, grabbed: bool) {
    orb.core = [1.0, 0.55, 0.22];
    orb.rim = [1.0, 0.82, 0.55];
    orb.intensity *= if grabbed { 2.0 } else { 1.6 };
    // Grabbed, it does not grow: it fits the hollow of the hand (see `orb_fit`).
    orb.radius *= if grabbed { 1.0 } else { 1.12 };
}

/// Small glowing sphere at a fingertip: cyan when free, amber when pinching.
pub fn fingertip_marker(position: Vec3, pinching: bool) -> OrbInstance {
    let (core, rim) = if pinching { ([1.0, 0.7, 0.3], [1.0, 0.85, 0.5]) } else { ([0.5, 0.95, 1.0], [0.6, 0.9, 1.0]) };
    OrbInstance {
        center: position,
        radius: if pinching { 0.014 } else { 0.010 },
        core,
        rim,
        intensity: 1.6,
        pulse: 0.0,
        speed: 0.0,
        phase: 0.0,
    }
}
