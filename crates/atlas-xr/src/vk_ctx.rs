//! Vulkan context created through OpenXR (XR_KHR_vulkan_enable2).

use ash::vk::{self, Handle};
use openxr as xr;

pub const COLOR_FORMAT: vk::Format = vk::Format::R8G8B8A8_SRGB;
pub const VIEW_COUNT: u32 = 2;
/// Frames in flight.
pub const PIPELINE_DEPTH: u32 = 2;
/// Panel textures that can be uploaded in the same frame.
pub const UPLOAD_SLOTS: usize = 6;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub struct XrSwapchain {
    pub handle: xr::Swapchain<xr::Vulkan>,
    pub resolution: vk::Extent2D,
    framebuffers: Vec<(vk::Framebuffer, vk::ImageView)>,
}

/// 2D texture for a Quad layer: filled from the CPU through a persistent
/// staging buffer.
pub struct QuadTexture {
    pub handle: xr::Swapchain<xr::Vulkan>,
    pub width: u32,
    pub height: u32,
    images: Vec<vk::Image>,
    staging: vk::Buffer,
    staging_mem: vk::DeviceMemory,
    staging_ptr: *mut u8,
    staging_len: usize,
}

pub struct VkContext {
    _entry: ash::Entry,
    pub instance: ash::Instance,
    pub physical_device: vk::PhysicalDevice,
    pub device: ash::Device,
    pub queue_family: u32,
    queue: vk::Queue,
    render_pass: vk::RenderPass,
    pool: vk::CommandPool,
    cmds: Vec<vk::CommandBuffer>,
    fences: Vec<vk::Fence>,
}

impl VkContext {
    /// # Safety
    /// `xr_instance` and `system` must be valid and with `XR_KHR_vulkan_enable2` enabled.
    pub unsafe fn new(xr_instance: &xr::Instance, system: xr::SystemId) -> Result<Self> {
        let target = vk::make_api_version(0, 1, 1, 0); // 1.1 garantiza multiview
        let reqs = xr_instance.graphics_requirements::<xr::Vulkan>(system)?;
        let want = xr::Version::new(1, 1, 0);
        if want < reqs.min_api_version_supported
            || want.major() > reqs.max_api_version_supported.major()
        {
            return Err("the runtime does not support Vulkan 1.1".into());
        }

        let entry = ash::Entry::load()?;
        let get_proc = std::mem::transmute(entry.static_fn().get_instance_proc_addr);

        let app_info = vk::ApplicationInfo::default().api_version(target);
        let raw_instance = xr_instance
            .create_vulkan_instance(
                system,
                get_proc,
                &vk::InstanceCreateInfo::default().application_info(&app_info) as *const _ as *const _,
            )?
            .map_err(|r| format!("vkCreateInstance: {:?}", vk::Result::from_raw(r)))?;
        let instance =
            ash::Instance::load(entry.static_fn(), vk::Instance::from_raw(raw_instance as _));

        let physical_device = vk::PhysicalDevice::from_raw(
            xr_instance.vulkan_graphics_device(system, instance.handle().as_raw() as _)? as _,
        );
        let queue_family = instance
            .get_physical_device_queue_family_properties(physical_device)
            .iter()
            .position(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .ok_or("no graphics queue")? as u32;

        let mut multiview = vk::PhysicalDeviceMultiviewFeatures { multiview: vk::TRUE, ..Default::default() };
        let queue_info = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&[1.0])];
        let raw_device = xr_instance
            .create_vulkan_device(
                system,
                get_proc,
                physical_device.as_raw() as _,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queue_info)
                    .push_next(&mut multiview) as *const _ as *const _,
            )?
            .map_err(|r| format!("vkCreateDevice: {:?}", vk::Result::from_raw(r)))?;
        let device = ash::Device::load(instance.fp_v1_0(), vk::Device::from_raw(raw_device as _));
        let queue = device.get_device_queue(queue_family, 0);

        let view_mask = !(!0 << VIEW_COUNT);
        let attachments = [vk::AttachmentDescription {
            format: COLOR_FORMAT,
            samples: vk::SampleCountFlags::TYPE_1,
            load_op: vk::AttachmentLoadOp::CLEAR,
            store_op: vk::AttachmentStoreOp::STORE,
            initial_layout: vk::ImageLayout::UNDEFINED,
            final_layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            ..Default::default()
        }];
        let color_ref = [vk::AttachmentReference {
            attachment: 0,
            layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        }];
        let subpasses = [vk::SubpassDescription::default()
            .color_attachments(&color_ref)
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)];
        let deps = [vk::SubpassDependency {
            src_subpass: vk::SUBPASS_EXTERNAL,
            dst_subpass: 0,
            src_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            dst_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            dst_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ..Default::default()
        }];
        let masks = [view_mask];
        let mut mv = vk::RenderPassMultiviewCreateInfo::default()
            .view_masks(&masks)
            .correlation_masks(&masks);
        let render_pass = device.create_render_pass(
            &vk::RenderPassCreateInfo::default()
                .attachments(&attachments)
                .subpasses(&subpasses)
                .dependencies(&deps)
                .push_next(&mut mv),
            None,
        )?;

        let pool = device.create_command_pool(
            &vk::CommandPoolCreateInfo::default()
                .queue_family_index(queue_family)
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
            None,
        )?;
        let cmds = device.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                // One group for the color clear and another for each panel uploaded.
                .command_buffer_count(PIPELINE_DEPTH * (1 + UPLOAD_SLOTS as u32)),
        )?;
        let fences = (0..PIPELINE_DEPTH)
            .map(|_| {
                device.create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(Self {
            _entry: entry,
            instance,
            physical_device,
            device,
            queue_family,
            queue,
            render_pass,
            pool,
            cmds,
            fences,
        })
    }

    /// Multiview render pass the drawing pipelines are created with.
    pub fn render_pass(&self) -> vk::RenderPass {
        self.render_pass
    }

    /// # Safety
    /// `handle` must have been created with format `COLOR_FORMAT` and `VIEW_COUNT` layers.
    pub unsafe fn make_swapchain(
        &self,
        handle: xr::Swapchain<xr::Vulkan>,
        resolution: vk::Extent2D,
    ) -> Result<XrSwapchain> {
        let mut framebuffers = Vec::new();
        for raw in handle.enumerate_images()? {
            let view = self.device.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(vk::Image::from_raw(raw))
                    .view_type(vk::ImageViewType::TYPE_2D_ARRAY)
                    .format(COLOR_FORMAT)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: VIEW_COUNT,
                    }),
                None,
            )?;
            let attachments = [view];
            let fb = self.device.create_framebuffer(
                &vk::FramebufferCreateInfo::default()
                    .render_pass(self.render_pass)
                    .width(resolution.width)
                    .height(resolution.height)
                    .attachments(&attachments)
                    .layers(1), // multiview addresses the layers
                None,
            )?;
            framebuffers.push((fb, view));
        }
        Ok(XrSwapchain { handle, resolution, framebuffers })
    }

    /// Records a render pass that only clears the color.
    ///
    /// # Safety
    /// Call once per frame, before [`Self::submit`].
    pub unsafe fn record_clear(
        &self,
        frame: usize,
        sc: &XrSwapchain,
        image: usize,
        color: [f32; 4],
        draw: impl FnOnce(&ash::Device, vk::CommandBuffer),
    ) -> Result<()> {
        self.device.wait_for_fences(&[self.fences[frame]], true, u64::MAX)?;
        self.device.reset_fences(&[self.fences[frame]])?;
        let cmd = self.cmds[frame];
        self.device.begin_command_buffer(
            cmd,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )?;
        let clear = [vk::ClearValue { color: vk::ClearColorValue { float32: color } }];
        self.device.cmd_begin_render_pass(
            cmd,
            &vk::RenderPassBeginInfo::default()
                .render_pass(self.render_pass)
                .framebuffer(sc.framebuffers[image].0)
                .render_area(vk::Rect2D { offset: vk::Offset2D::default(), extent: sc.resolution })
                .clear_values(&clear),
            vk::SubpassContents::INLINE,
        );
        // The 3D content (orbs) is drawn inside the same render pass.
        draw(&self.device, cmd);
        self.device.cmd_end_render_pass(cmd);
        self.device.end_command_buffer(cmd)?;
        Ok(())
    }

    /// # Safety
    /// [`Self::record_clear`] must have been called with the same `frame`, and
    /// [`Self::record_upload`] for each slot in `uploads`.
    pub unsafe fn submit(&self, frame: usize, uploads: &[usize]) -> Result<()> {
        let depth = PIPELINE_DEPTH as usize;
        let mut cmds = vec![self.cmds[frame]];
        cmds.extend(uploads.iter().map(|slot| self.cmds[(1 + slot) * depth + frame]));
        self.device.queue_submit(
            self.queue,
            &[vk::SubmitInfo::default().command_buffers(&cmds)],
            self.fences[frame],
        )?;
        Ok(())
    }

    pub(crate) fn memory_type(&self, bits: u32, flags: vk::MemoryPropertyFlags) -> Result<u32> {
        let props = unsafe { self.instance.get_physical_device_memory_properties(self.physical_device) };
        (0..props.memory_type_count)
            .find(|&i| bits & (1 << i) != 0 && props.memory_types[i as usize].property_flags.contains(flags))
            .ok_or_else(|| "no compatible memory type".into())
    }

    /// Creates the texture of a Quad layer and its staging buffer (RGBA8).
    ///
    /// # Safety
    /// `handle` must have format `COLOR_FORMAT`, a single layer and `width`x`height`.
    pub unsafe fn make_quad_texture(
        &self,
        handle: xr::Swapchain<xr::Vulkan>,
        width: u32,
        height: u32,
    ) -> Result<QuadTexture> {
        let images = handle.enumerate_images()?.into_iter().map(vk::Image::from_raw).collect();
        let len = (width * height * 4) as usize;
        let staging = self.device.create_buffer(
            &vk::BufferCreateInfo::default()
                .size(len as u64)
                .usage(vk::BufferUsageFlags::TRANSFER_SRC)
                .sharing_mode(vk::SharingMode::EXCLUSIVE),
            None,
        )?;
        let req = self.device.get_buffer_memory_requirements(staging);
        let mem_type = self.memory_type(
            req.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        let staging_mem = self.device.allocate_memory(
            &vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(mem_type),
            None,
        )?;
        self.device.bind_buffer_memory(staging, staging_mem, 0)?;
        let staging_ptr =
            self.device.map_memory(staging_mem, 0, len as u64, vk::MemoryMapFlags::empty())? as *mut u8;
        Ok(QuadTexture { handle, width, height, images, staging, staging_mem, staging_ptr, staging_len: len })
    }

    /// Copies RGBA8 pixels (non-premultiplied alpha) to the staging buffer. It waits for the
    /// GPU to finish the frames in flight, because they may be reading it.
    ///
    /// # Safety
    /// `rgba` debe medir exactamente `width * height * 4` bytes.
    pub unsafe fn write_pixels(&self, quad: &QuadTexture, rgba: &[u8]) -> Result<()> {
        assert_eq!(rgba.len(), quad.staging_len, "unexpected texture size");
        self.device.wait_for_fences(&self.fences, true, u64::MAX)?;
        std::ptr::copy_nonoverlapping(rgba.as_ptr(), quad.staging_ptr, rgba.len());
        Ok(())
    }

    /// Records the copy staging -> image `image` of the texture.
    ///
    /// # Safety
    /// Call after [`Self::record_clear`] of the same `frame` (which waits for its fence).
    pub unsafe fn record_upload(
        &self,
        frame: usize,
        slot: usize,
        quad: &QuadTexture,
        image: usize,
    ) -> Result<()> {
        assert!(slot < UPLOAD_SLOTS, "upload slot out of range");
        let cmd = self.cmds[(1 + slot) * PIPELINE_DEPTH as usize + frame];
        let img = quad.images[image];
        let range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };
        self.device.begin_command_buffer(
            cmd,
            &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )?;
        let to_dst = vk::ImageMemoryBarrier::default()
            .image(img)
            .subresource_range(range)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED);
        self.device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_dst],
        );
        self.device.cmd_copy_buffer_to_image(
            cmd,
            quad.staging,
            img,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &[vk::BufferImageCopy {
                image_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                },
                image_extent: vk::Extent3D { width: quad.width, height: quad.height, depth: 1 },
                ..Default::default()
            }],
        );
        // OpenXR expects COLOR_ATTACHMENT_OPTIMAL when releasing the image.
        let to_color = vk::ImageMemoryBarrier::default()
            .image(img)
            .subresource_range(range)
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED);
        self.device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_color],
        );
        self.device.end_command_buffer(cmd)?;
        Ok(())
    }

    /// # Safety
    /// No OpenXR handle must still be alive.
    pub unsafe fn destroy(&self, swapchain: Option<XrSwapchain>, quads: Vec<QuadTexture>) {
        let _ = self.device.wait_for_fences(&self.fences, true, u64::MAX);
        for q in quads {
            self.device.unmap_memory(q.staging_mem);
            self.device.destroy_buffer(q.staging, None);
            self.device.free_memory(q.staging_mem, None);
        }
        for f in &self.fences {
            self.device.destroy_fence(*f, None);
        }
        if let Some(sc) = swapchain {
            for (fb, view) in sc.framebuffers {
                self.device.destroy_framebuffer(fb, None);
                self.device.destroy_image_view(view, None);
            }
        }
        self.device.destroy_command_pool(self.pool, None);
        self.device.destroy_render_pass(self.render_pass, None);
        self.device.destroy_device(None);
        self.instance.destroy_instance(None);
    }
}
