//! A floating panel: its texture, the thread that draws it on the CPU and the state of
//! its upload to the GPU. It serves any content (code, git output...).

use std::error::Error;
use std::sync::mpsc::{channel, Receiver, Sender};

use atlas_ui::Canvas;
use openxr as xr;

use crate::vk_ctx::{QuadTexture, VkContext, COLOR_FORMAT};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// `R` is the drawing request the thread receives (a copy of the state to paint).
pub struct PanelSlot<R: Send + 'static> {
    pub width: u32,
    pub height: u32,
    texture: Option<QuadTexture>,
    /// The layer is only sent after the first pixel upload.
    ready: bool,
    needs_upload: bool,
    dirty: bool,
    in_flight: bool,
    requests: Sender<R>,
    done: Receiver<Canvas>,
}

impl<R: Send + 'static> PanelSlot<R> {
    /// Creates the panel and launches its drawing thread. Drawing on the CPU takes tens of
    /// milliseconds on the Quest, so it cannot happen in the render loop.
    pub fn spawn(
        name: &str,
        width: u32,
        height: u32,
        mut render: impl FnMut(R) -> Canvas + Send + 'static,
    ) -> std::io::Result<Self> {
        let (requests, request_rx) = channel::<R>();
        let (done_tx, done) = channel::<Canvas>();
        std::thread::Builder::new().name(name.to_string()).spawn(move || {
            while let Ok(mut latest) = request_rx.recv() {
                // If changes piled up, only the most recent one is drawn.
                while let Ok(newer) = request_rx.try_recv() {
                    latest = newer;
                }
                if done_tx.send(render(latest)).is_err() {
                    break;
                }
            }
        })?;
        Ok(Self {
            width,
            height,
            texture: None,
            ready: false,
            needs_upload: false,
            dirty: true,
            in_flight: false,
            requests,
            done,
        })
    }

    /// The content changed: it has to be drawn again.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// There is at least one uploaded image, so the layer can be sent.
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub fn texture(&self) -> Option<&QuadTexture> {
        self.texture.as_ref()
    }

    /// Creates the texture the first time, collects a finished drawing and, if there are
    /// pending changes, requests another with the state `request` returns.
    pub fn pump(
        &mut self,
        session: &xr::Session<xr::Vulkan>,
        vk: &VkContext,
        request: impl FnOnce() -> R,
    ) -> Result<()> {
        if self.texture.is_none() {
            let handle = session.create_swapchain(&xr::SwapchainCreateInfo {
                create_flags: xr::SwapchainCreateFlags::EMPTY,
                usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT
                    | xr::SwapchainUsageFlags::SAMPLED
                    | xr::SwapchainUsageFlags::TRANSFER_DST,
                format: COLOR_FORMAT.as_raw() as _,
                sample_count: 1,
                width: self.width,
                height: self.height,
                face_count: 1,
                array_size: 1,
                mip_count: 1,
            })?;
            self.texture = Some(unsafe { vk.make_quad_texture(handle, self.width, self.height)? });
        }
        if let Ok(canvas) = self.done.try_recv() {
            let texture = self.texture.as_ref().expect("textura creada arriba");
            unsafe { vk.write_pixels(texture, &canvas.rgba)? };
            self.in_flight = false;
            self.needs_upload = true;
        }
        if self.dirty && !self.in_flight {
            self.requests.send(request())?;
            self.in_flight = true;
            self.dirty = false;
        }
        Ok(())
    }

    /// Records the pixel upload if there is a new drawing. Returns whether it did.
    pub fn record_upload(&mut self, vk: &VkContext, frame: usize, slot: usize) -> Result<bool> {
        if !self.needs_upload {
            return Ok(false);
        }
        let texture = self.texture.as_mut().expect("texture created in pump");
        // The Quad layer shows the last released image: one upload per change is enough.
        let image = texture.handle.acquire_image()?;
        texture.handle.wait_image(xr::Duration::INFINITE)?;
        unsafe { vk.record_upload(frame, slot, texture, image as usize)? };
        Ok(true)
    }

    /// Releases the image after sending the commands to the GPU.
    pub fn finish_upload(&mut self) -> Result<()> {
        if self.needs_upload {
            self.texture.as_mut().expect("texture created in pump").handle.release_image()?;
            self.needs_upload = false;
            self.ready = true;
        }
        Ok(())
    }

    /// Hands over the texture to destroy it together with the Vulkan context.
    pub fn into_texture(self) -> Option<QuadTexture> {
        self.texture
    }
}
