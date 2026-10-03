//! Vulkan renderer: draws the HUD meshes over a swapchain image right before
//! the game presents it, in its own command buffer submitted on the present
//! queue. The submission waits for the game's semaphores and signals one of
//! ours, which the present then waits for.

use std::collections::HashMap;
use std::ffi::c_void;

use ash::prelude::VkResult;
use ash::vk::{self, Handle};
use epaint::{ColorImage, ImageData, Primitive, TextureId};
use gameviber_common::overlay::OverlayState;

use crate::hud::{Frame as HudFrame, Hud};

const SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/overlay.spv"));
/// Past this wait for a previous overlay frame, skip drawing rather than stall the game.
const FENCE_TIMEOUT_NS: u64 = 100_000_000;
/// epaint vertex: pos (2 x f32), uv (2 x f32), color (4 x u8).
const VERTEX_SIZE: u64 = 20;

pub type SetDeviceLoaderData = unsafe extern "system" fn(vk::Device, *mut c_void) -> vk::Result;

/// What the renderer needs from the device the layer wraps.
pub struct Gpu {
    pub device: ash::Device,
    pub memory: vk::PhysicalDeviceMemoryProperties,
    pub set_loader_data: SetDeviceLoaderData,
}

impl Gpu {
    fn memory_type(&self, bits: u32, flags: vk::MemoryPropertyFlags) -> Option<u32> {
        (0..self.memory.memory_type_count).find(|&i| {
            bits & (1 << i) != 0 && self.memory.memory_types[i as usize].property_flags.contains(flags)
        })
    }

    /// Host-visible buffer, persistently mapped.
    unsafe fn buffer(&self, size: u64, usage: vk::BufferUsageFlags) -> VkResult<Buffer> {
        let d = &self.device;
        let buffer = d.create_buffer(&vk::BufferCreateInfo::default().size(size).usage(usage), None)?;
        let req = d.get_buffer_memory_requirements(buffer);
        let flags = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
        let Some(kind) = self.memory_type(req.memory_type_bits, flags) else {
            d.destroy_buffer(buffer, None);
            return Err(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY);
        };
        let info = vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(kind);
        let memory = match d.allocate_memory(&info, None) {
            Ok(m) => m,
            Err(e) => {
                d.destroy_buffer(buffer, None);
                return Err(e);
            }
        };
        d.bind_buffer_memory(buffer, memory, 0)?;
        let ptr = d.map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())? as *mut u8;
        Ok(Buffer { buffer, memory, size, ptr })
    }
}

struct Buffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: u64,
    ptr: *mut u8,
}

impl Buffer {
    unsafe fn destroy(&self, d: &ash::Device) {
        d.destroy_buffer(self.buffer, None);
        d.free_memory(self.memory, None);
    }
}

/// Keeps `slot` at least `size` bytes large (growing by powers of two).
unsafe fn ensure_buffer(gpu: &Gpu, slot: &mut Option<Buffer>, size: u64, usage: vk::BufferUsageFlags) -> VkResult<*mut u8> {
    if slot.as_ref().is_none_or(|b| b.size < size) {
        if let Some(old) = slot.take() {
            old.destroy(&gpu.device);
        }
        *slot = Some(gpu.buffer(size.next_power_of_two().max(4096), usage)?);
    }
    Ok(slot.as_ref().unwrap().ptr)
}

struct Texture {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    size: [usize; 2],
    /// Not uploaded yet: its layout is still undefined.
    fresh: bool,
}

struct Frame {
    view: vk::ImageView,
    framebuffer: vk::Framebuffer,
    /// Command buffer and the queue family of its pool.
    cmd: Option<(vk::CommandBuffer, u32)>,
    fence: vk::Fence,
    semaphore: vk::Semaphore,
    vertices: Option<Buffer>,
    indices: Option<Buffer>,
    staging: Option<Buffer>,
}

struct Swapchain {
    format: vk::Format,
    extent: vk::Extent2D,
    frames: Vec<Frame>,
}

pub struct Renderer {
    shader: vk::ShaderModule,
    sampler: vk::Sampler,
    set_layout: vk::DescriptorSetLayout,
    pipeline_layout: vk::PipelineLayout,
    descriptor_pool: vk::DescriptorPool,
    descriptor_set: vk::DescriptorSet,
    /// Render pass and pipeline per swapchain format.
    pipelines: HashMap<vk::Format, (vk::RenderPass, vk::Pipeline)>,
    pools: HashMap<u32, vk::CommandPool>,
    texture: Option<Texture>,
    /// CPU copy of the font atlas; epaint sends partial updates.
    atlas: Option<ColorImage>,
    atlas_dirty: bool,
    swapchains: HashMap<vk::SwapchainKHR, Swapchain>,
    hud: Hud,
}

// The raw pointers (mapped memory) are only used under the device's mutex.
unsafe impl Send for Renderer {}

impl Renderer {
    pub unsafe fn new(gpu: &Gpu) -> VkResult<Self> {
        let d = &gpu.device;
        let words: Vec<u32> = SHADER.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let shader = d.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None)?;
        let sampler = d.create_sampler(
            &vk::SamplerCreateInfo::default()
                .mag_filter(vk::Filter::LINEAR)
                .min_filter(vk::Filter::LINEAR)
                .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
            None,
        )?;
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];
        let set_layout = d.create_descriptor_set_layout(&vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings), None)?;
        let push = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
            .size(16)];
        let layouts = [set_layout];
        let pipeline_layout =
            d.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts).push_constant_ranges(&push), None)?;
        let sizes = [
            vk::DescriptorPoolSize { ty: vk::DescriptorType::SAMPLED_IMAGE, descriptor_count: 1 },
            vk::DescriptorPoolSize { ty: vk::DescriptorType::SAMPLER, descriptor_count: 1 },
        ];
        let descriptor_pool = d.create_descriptor_pool(&vk::DescriptorPoolCreateInfo::default().max_sets(1).pool_sizes(&sizes), None)?;
        let descriptor_set =
            d.allocate_descriptor_sets(&vk::DescriptorSetAllocateInfo::default().descriptor_pool(descriptor_pool).set_layouts(&layouts))?[0];
        let sampler_info = [vk::DescriptorImageInfo::default().sampler(sampler)];
        let write = vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(1)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .image_info(&sampler_info);
        d.update_descriptor_sets(&[write], &[]);
        Ok(Self {
            shader,
            sampler,
            set_layout,
            pipeline_layout,
            descriptor_pool,
            descriptor_set,
            pipelines: HashMap::new(),
            pools: HashMap::new(),
            texture: None,
            atlas: None,
            atlas_dirty: false,
            swapchains: HashMap::new(),
            hud: Hud::new(),
        })
    }

    pub unsafe fn add_swapchain(
        &mut self,
        gpu: &Gpu,
        swapchain: vk::SwapchainKHR,
        images: &[vk::Image],
        format: vk::Format,
        extent: vk::Extent2D,
    ) -> VkResult<()> {
        let d = &gpu.device;
        let render_pass = self.pipeline(gpu, format)?.0;
        let mut frames = Vec::new();
        for &image in images {
            let view = d.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format)
                    .subresource_range(color_range()),
                None,
            )?;
            let attachments = [view];
            let framebuffer = d.create_framebuffer(
                &vk::FramebufferCreateInfo::default()
                    .render_pass(render_pass)
                    .attachments(&attachments)
                    .width(extent.width)
                    .height(extent.height)
                    .layers(1),
                None,
            )?;
            let fence = d.create_fence(&vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED), None)?;
            let semaphore = d.create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?;
            frames.push(Frame {
                view,
                framebuffer,
                cmd: None,
                fence,
                semaphore,
                vertices: None,
                indices: None,
                staging: None,
            });
        }
        self.swapchains.insert(swapchain, Swapchain { format, extent, frames });
        Ok(())
    }

    pub unsafe fn remove_swapchain(&mut self, gpu: &Gpu, swapchain: vk::SwapchainKHR) {
        let Some(sc) = self.swapchains.remove(&swapchain) else { return };
        let d = &gpu.device;
        for frame in sc.frames {
            let _ = d.wait_for_fences(&[frame.fence], true, FENCE_TIMEOUT_NS);
            self.destroy_frame(gpu, frame);
        }
    }

    unsafe fn destroy_frame(&self, gpu: &Gpu, frame: Frame) {
        let d = &gpu.device;
        if let Some((cmd, family)) = frame.cmd {
            d.free_command_buffers(self.pools[&family], &[cmd]);
        }
        for buffer in [frame.vertices, frame.indices, frame.staging].into_iter().flatten() {
            buffer.destroy(d);
        }
        d.destroy_fence(frame.fence, None);
        d.destroy_semaphore(frame.semaphore, None);
        d.destroy_framebuffer(frame.framebuffer, None);
        d.destroy_image_view(frame.view, None);
    }

    pub unsafe fn destroy(mut self, gpu: &Gpu) {
        let d = &gpu.device;
        let swapchains: Vec<_> = self.swapchains.keys().copied().collect();
        for sc in swapchains {
            self.remove_swapchain(gpu, sc);
        }
        if let Some(t) = self.texture.take() {
            destroy_texture(d, t);
        }
        for (_, pool) in self.pools.drain() {
            d.destroy_command_pool(pool, None);
        }
        for (_, (render_pass, pipeline)) in self.pipelines.drain() {
            d.destroy_pipeline(pipeline, None);
            d.destroy_render_pass(render_pass, None);
        }
        d.destroy_descriptor_pool(self.descriptor_pool, None);
        d.destroy_pipeline_layout(self.pipeline_layout, None);
        d.destroy_descriptor_set_layout(self.set_layout, None);
        d.destroy_sampler(self.sampler, None);
        d.destroy_shader_module(self.shader, None);
    }

    /// Records and submits the overlay for `image` of `swapchain`. Returns the
    /// semaphore the present must wait for instead of `wait`, or `None` when
    /// nothing was drawn (the present then keeps the game's semaphores).
    pub unsafe fn draw(
        &mut self,
        gpu: &Gpu,
        queue: vk::Queue,
        family: u32,
        swapchain: vk::SwapchainKHR,
        image: u32,
        wait: &[vk::Semaphore],
        state: &OverlayState,
    ) -> VkResult<Option<vk::Semaphore>> {
        let d = &gpu.device;
        let Some(sc) = self.swapchains.get(&swapchain) else { return Ok(None) };
        let (extent, format) = (sc.extent, sc.format);
        let Some(frame) = sc.frames.get(image as usize) else { return Ok(None) };
        if d.wait_for_fences(&[frame.fence], true, FENCE_TIMEOUT_NS).is_err() {
            return Ok(None);
        }
        let hud = self.hud.build(state, extent.width, extent.height);
        if let Some(delta) = &hud.texture {
            self.apply_texture_delta(delta);
        }
        if hud.primitives.is_empty() {
            return Ok(None);
        }
        let upload = self.atlas_dirty;
        if upload {
            // Other frames may still sample the texture being replaced.
            self.wait_all(gpu);
            self.prepare_texture(gpu)?;
        }
        let pool = self.pool(gpu, family)?;
        let (render_pass, pipeline) = self.pipeline(gpu, format)?;

        let sc = self.swapchains.get_mut(&swapchain).unwrap();
        let frame = &mut sc.frames[image as usize];
        let cmd = match frame.cmd {
            Some((cmd, f)) if f == family => cmd,
            other => {
                if let Some((old, f)) = other {
                    d.free_command_buffers(self.pools[&f], &[old]);
                }
                let info = vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1);
                let cmd = d.allocate_command_buffers(&info)?[0];
                // Command buffers are dispatchable: the loader must know them.
                (gpu.set_loader_data)(d.handle(), cmd.as_raw() as *mut c_void).result()?;
                frame.cmd = Some((cmd, family));
                cmd
            }
        };

        let (vertex_count, index_count) = hud.primitives.iter().fold((0, 0), |(v, i), p| match &p.primitive {
            Primitive::Mesh(m) => (v + m.vertices.len(), i + m.indices.len()),
            Primitive::Callback(_) => (v, i),
        });
        let vptr = ensure_buffer(gpu, &mut frame.vertices, vertex_count as u64 * VERTEX_SIZE, vk::BufferUsageFlags::VERTEX_BUFFER)?;
        let iptr = ensure_buffer(gpu, &mut frame.indices, index_count as u64 * 4, vk::BufferUsageFlags::INDEX_BUFFER)?;
        let (mut voff, mut ioff) = (0usize, 0usize);
        for p in &hud.primitives {
            if let Primitive::Mesh(m) = &p.primitive {
                std::ptr::copy_nonoverlapping(m.vertices.as_ptr() as *const u8, vptr.add(voff * VERTEX_SIZE as usize), m.vertices.len() * VERTEX_SIZE as usize);
                std::ptr::copy_nonoverlapping(m.indices.as_ptr() as *const u8, iptr.add(ioff * 4), m.indices.len() * 4);
                voff += m.vertices.len();
                ioff += m.indices.len();
            }
        }

        d.reset_fences(&[frame.fence])?;
        d.reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())?;
        d.begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT))?;
        if upload {
            self.record_upload(gpu, cmd, swapchain, image)?;
        }
        let frame = &self.swapchains[&swapchain].frames[image as usize];
        let area = vk::Rect2D { offset: vk::Offset2D::default(), extent };
        d.cmd_begin_render_pass(
            cmd,
            &vk::RenderPassBeginInfo::default().render_pass(render_pass).framebuffer(frame.framebuffer).render_area(area),
            vk::SubpassContents::INLINE,
        );
        d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline);
        d.cmd_bind_descriptor_sets(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline_layout, 0, &[self.descriptor_set], &[]);
        d.cmd_bind_vertex_buffers(cmd, 0, &[frame.vertices.as_ref().unwrap().buffer], &[0]);
        d.cmd_bind_index_buffer(cmd, frame.indices.as_ref().unwrap().buffer, 0, vk::IndexType::UINT32);
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        d.cmd_set_viewport(cmd, 0, &[viewport]);
        let params = push_constants(&hud, extent, is_srgb(format));
        d.cmd_push_constants(cmd, self.pipeline_layout, vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT, 0, &params);
        let (mut voff, mut ioff) = (0i32, 0u32);
        for p in &hud.primitives {
            let Primitive::Mesh(m) = &p.primitive else { continue };
            if m.texture_id == TextureId::Managed(0) {
                d.cmd_set_scissor(cmd, 0, &[scissor(p.clip_rect, hud.pixels_per_point, extent)]);
                d.cmd_draw_indexed(cmd, m.indices.len() as u32, 1, ioff, voff, 0);
            }
            voff += m.vertices.len() as i32;
            ioff += m.indices.len() as u32;
        }
        d.cmd_end_render_pass(cmd);
        d.end_command_buffer(cmd)?;

        let stages = vec![vk::PipelineStageFlags::ALL_COMMANDS; wait.len()];
        let cmds = [cmd];
        let signal = [frame.semaphore];
        let submit = vk::SubmitInfo::default()
            .wait_semaphores(wait)
            .wait_dst_stage_mask(&stages)
            .command_buffers(&cmds)
            .signal_semaphores(&signal);
        d.queue_submit(queue, &[submit], frame.fence)?;
        Ok(Some(frame.semaphore))
    }

    fn apply_texture_delta(&mut self, delta: &epaint::ImageDelta) {
        let ImageData::Color(image) = &delta.image;
        match (delta.pos, &mut self.atlas) {
            (Some([x, y]), Some(atlas)) => {
                let w = image.size[0];
                for row in 0..image.size[1] {
                    let dst = (y + row) * atlas.size[0] + x;
                    atlas.pixels[dst..dst + w].copy_from_slice(&image.pixels[row * w..(row + 1) * w]);
                }
            }
            _ => self.atlas = Some((**image).clone()),
        }
        self.atlas_dirty = true;
    }

    unsafe fn wait_all(&self, gpu: &Gpu) {
        let fences: Vec<_> = self.swapchains.values().flat_map(|s| s.frames.iter().map(|f| f.fence)).collect();
        if !fences.is_empty() {
            let _ = gpu.device.wait_for_fences(&fences, true, FENCE_TIMEOUT_NS * 10);
        }
    }

    /// (Re)creates the texture when the atlas size changed.
    unsafe fn prepare_texture(&mut self, gpu: &Gpu) -> VkResult<()> {
        let d = &gpu.device;
        let size = self.atlas.as_ref().map(|a| a.size).unwrap_or([1, 1]);
        if self.texture.as_ref().is_some_and(|t| t.size == size) {
            return Ok(());
        }
        if let Some(old) = self.texture.take() {
            destroy_texture(d, old);
        }
        let image = d.create_image(
            &vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(vk::Format::R8G8B8A8_UNORM)
                .extent(vk::Extent3D { width: size[0] as u32, height: size[1] as u32, depth: 1 })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
                .initial_layout(vk::ImageLayout::UNDEFINED),
            None,
        )?;
        let req = d.get_image_memory_requirements(image);
        let kind = gpu
            .memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
            .or_else(|| gpu.memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::empty()))
            .ok_or(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY)?;
        let memory = d.allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(kind), None)?;
        d.bind_image_memory(image, memory, 0)?;
        let view = d.create_image_view(
            &vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(vk::Format::R8G8B8A8_UNORM)
                .subresource_range(color_range()),
            None,
        )?;
        let info = [vk::DescriptorImageInfo::default().image_view(view).image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let write = vk::WriteDescriptorSet::default()
            .dst_set(self.descriptor_set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .image_info(&info);
        d.update_descriptor_sets(&[write], &[]);
        self.texture = Some(Texture { image, memory, view, size, fresh: true });
        Ok(())
    }

    /// Copies the whole atlas to the texture through the frame's staging buffer.
    unsafe fn record_upload(&mut self, gpu: &Gpu, cmd: vk::CommandBuffer, swapchain: vk::SwapchainKHR, image: u32) -> VkResult<()> {
        let d = &gpu.device;
        let (Some(atlas), Some(texture)) = (&self.atlas, &mut self.texture) else { return Ok(()) };
        let frame = &mut self.swapchains.get_mut(&swapchain).unwrap().frames[image as usize];
        let bytes = (atlas.pixels.len() * 4) as u64;
        let ptr = ensure_buffer(gpu, &mut frame.staging, bytes, vk::BufferUsageFlags::TRANSFER_SRC)?;
        std::ptr::copy_nonoverlapping(atlas.pixels.as_ptr() as *const u8, ptr, bytes as usize);
        let old_layout = if texture.fresh { vk::ImageLayout::UNDEFINED } else { vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL };
        let to_transfer = vk::ImageMemoryBarrier::default()
            .old_layout(old_layout)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_access_mask(vk::AccessFlags::SHADER_READ)
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(texture.image)
            .subresource_range(color_range());
        d.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::FRAGMENT_SHADER | vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_transfer],
        );
        let region = vk::BufferImageCopy::default()
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_extent(vk::Extent3D { width: atlas.size[0] as u32, height: atlas.size[1] as u32, depth: 1 });
        d.cmd_copy_buffer_to_image(cmd, frame.staging.as_ref().unwrap().buffer, texture.image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[region]);
        let to_shader = to_transfer
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ);
        d.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_shader],
        );
        texture.fresh = false;
        self.atlas_dirty = false;
        Ok(())
    }

    unsafe fn pool(&mut self, gpu: &Gpu, family: u32) -> VkResult<vk::CommandPool> {
        if let Some(pool) = self.pools.get(&family) {
            return Ok(*pool);
        }
        let info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(family)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let pool = gpu.device.create_command_pool(&info, None)?;
        self.pools.insert(family, pool);
        Ok(pool)
    }

    unsafe fn pipeline(&mut self, gpu: &Gpu, format: vk::Format) -> VkResult<(vk::RenderPass, vk::Pipeline)> {
        if let Some(p) = self.pipelines.get(&format) {
            return Ok(*p);
        }
        let d = &gpu.device;
        let attachments = [vk::AttachmentDescription::default()
            .format(format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::LOAD)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)];
        let color = [vk::AttachmentReference { attachment: 0, layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL }];
        let subpasses = [vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color)];
        let dependencies = [vk::SubpassDependency {
            src_subpass: vk::SUBPASS_EXTERNAL,
            dst_subpass: 0,
            src_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            dst_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            src_access_mask: vk::AccessFlags::empty(),
            dst_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            dependency_flags: vk::DependencyFlags::empty(),
        }];
        let render_pass = d.create_render_pass(
            &vk::RenderPassCreateInfo::default().attachments(&attachments).subpasses(&subpasses).dependencies(&dependencies),
            None,
        )?;

        let stages = [
            vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::VERTEX).module(self.shader).name(c"vs_main"),
            vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::FRAGMENT).module(self.shader).name(c"fs_main"),
        ];
        let bindings = [vk::VertexInputBindingDescription {
            binding: 0,
            stride: VERTEX_SIZE as u32,
            input_rate: vk::VertexInputRate::VERTEX,
        }];
        let attributes = [
            vk::VertexInputAttributeDescription { location: 0, binding: 0, format: vk::Format::R32G32_SFLOAT, offset: 0 },
            vk::VertexInputAttributeDescription { location: 1, binding: 0, format: vk::Format::R32G32_SFLOAT, offset: 8 },
            vk::VertexInputAttributeDescription { location: 2, binding: 0, format: vk::Format::R8G8B8A8_UNORM, offset: 16 },
        ];
        let vertex_input =
            vk::PipelineVertexInputStateCreateInfo::default().vertex_binding_descriptions(&bindings).vertex_attribute_descriptions(&attributes);
        let assembly = vk::PipelineInputAssemblyStateCreateInfo::default().topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewport = vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(vk::SampleCountFlags::TYPE_1);
        // Premultiplied alpha, as epaint produces.
        let blend = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_DST_ALPHA)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let blend_state = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend);
        let dynamic = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic);
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&assembly)
            .viewport_state(&viewport)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .color_blend_state(&blend_state)
            .dynamic_state(&dynamic_state)
            .layout(self.pipeline_layout)
            .render_pass(render_pass)
            .subpass(0);
        let pipeline = match d.create_graphics_pipelines(vk::PipelineCache::null(), &[info], None) {
            Ok(p) => p[0],
            Err((_, e)) => {
                d.destroy_render_pass(render_pass, None);
                return Err(e);
            }
        };
        self.pipelines.insert(format, (render_pass, pipeline));
        Ok((render_pass, pipeline))
    }
}

unsafe fn destroy_texture(d: &ash::Device, t: Texture) {
    d.destroy_image_view(t.view, None);
    d.destroy_image(t.image, None);
    d.free_memory(t.memory, None);
}

fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

fn is_srgb(format: vk::Format) -> bool {
    matches!(
        format,
        vk::Format::B8G8R8A8_SRGB | vk::Format::R8G8B8A8_SRGB | vk::Format::A8B8G8R8_SRGB_PACK32 | vk::Format::R8G8B8_SRGB | vk::Format::B8G8R8_SRGB
    )
}

/// Screen size in points, sRGB flag, opacity: matches `Params` in overlay.wgsl.
fn push_constants(hud: &HudFrame, extent: vk::Extent2D, srgb: bool) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&(extent.width as f32 / hud.pixels_per_point).to_le_bytes());
    out[4..8].copy_from_slice(&(extent.height as f32 / hud.pixels_per_point).to_le_bytes());
    out[8..12].copy_from_slice(&(srgb as u32).to_le_bytes());
    out[12..16].copy_from_slice(&1.0f32.to_le_bytes());
    out
}

fn scissor(clip: epaint::Rect, ppp: f32, extent: vk::Extent2D) -> vk::Rect2D {
    let x0 = (clip.min.x * ppp).round().clamp(0.0, extent.width as f32) as u32;
    let y0 = (clip.min.y * ppp).round().clamp(0.0, extent.height as f32) as u32;
    let x1 = (clip.max.x * ppp).round().clamp(x0 as f32, extent.width as f32) as u32;
    let y1 = (clip.max.y * ppp).round().clamp(y0 as f32, extent.height as f32) as u32;
    vk::Rect2D {
        offset: vk::Offset2D { x: x0 as i32, y: y0 as i32 },
        extent: vk::Extent2D { width: x1 - x0, height: y1 - y0 },
    }
}
