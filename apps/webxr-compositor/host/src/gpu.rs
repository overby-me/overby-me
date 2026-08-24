//! dmabuf import and readback: hardware clients render into GPU buffers,
//! which are imported through EGL/GLES and copied back to RGBA for the
//! same streaming pipeline shm uses.

use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{Bind, ExportMem, Frame, ImportDma, Offscreen, Renderer, Texture};
use smithay::reexports::gbm;
use smithay::utils::{Physical, Rectangle, Size, Transform};
use webxr_compositor_protocol as protocol;

pub struct Gpu {
    renderer: GlesRenderer,
    /// The render node's device id, for dmabuf feedback.
    device_id: u64,
    /// One imported texture per client dmabuf: importing fresh every frame
    /// exhausts GL resources within seconds.
    textures: Vec<(Dmabuf, GlesTexture)>,
    /// Offscreen blit target, reused while the surface size holds.
    offscreen: Option<(Size<i32, smithay::utils::Buffer>, GlesTexture)>,
}

impl Gpu {
    /// The first render node that yields a working GLES context; None keeps
    /// dmabuf unadvertised and every client on shm.
    pub fn new() -> Option<Gpu> {
        let entries = std::fs::read_dir("/dev/dri").ok()?;
        let mut nodes: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("renderD"))
            })
            .collect();
        nodes.sort();
        for node in nodes {
            // smithay's lazy EGL loader panics rather than erring when the
            // runtime libraries are absent; a host without them must still
            // come up, just without dmabuf.
            let opened =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Self::open(&node)));
            match opened {
                Ok(Ok(gpu)) => {
                    tracing::info!(node = %node.display(), "dmabuf readback ready");
                    return Some(gpu);
                }
                Ok(Err(error)) => {
                    tracing::info!(node = %node.display(), error, "render node unusable");
                }
                Err(_) => {
                    tracing::info!("EGL runtime unavailable; dmabuf stays off");
                    return None;
                }
            }
        }
        None
    }

    fn open(node: &std::path::Path) -> Result<Gpu, String> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(node)
            .map_err(|error| error.to_string())?;
        let device_id = std::os::unix::fs::MetadataExt::rdev(
            &file.metadata().map_err(|error| error.to_string())?,
        );
        let device = gbm::Device::new(file).map_err(|error| error.to_string())?;
        // SAFETY: one display per device, created once and moved into the
        // renderer's context; nothing else touches this EGL display.
        let display = unsafe { EGLDisplay::new(device) }.map_err(|error| error.to_string())?;
        let context = EGLContext::new(&display).map_err(|error| error.to_string())?;
        // SAFETY: the context is only ever made current on the compositor
        // thread that owns this Gpu.
        let renderer = unsafe { GlesRenderer::new(context) }.map_err(|error| error.to_string())?;
        Ok(Gpu {
            renderer,
            device_id,
            textures: Vec::new(),
            offscreen: None,
        })
    }

    fn texture_for(&mut self, dmabuf: &Dmabuf) -> Result<usize, String> {
        if let Some(index) = self.textures.iter().position(|(known, _)| known == dmabuf) {
            return Ok(index);
        }
        let texture = self
            .renderer
            .import_dmabuf(dmabuf, None)
            .map_err(|error| format!("import failed: {error}"))?;
        self.textures.push((dmabuf.clone(), texture));
        Ok(self.textures.len() - 1)
    }

    /// The client destroyed this buffer; its texture goes with it.
    pub fn forget(&mut self, dmabuf: &Dmabuf) {
        self.textures.retain(|(known, _)| known != dmabuf);
    }

    pub fn device_id(&self) -> u64 {
        self.device_id
    }

    pub fn formats(&self) -> FormatSet {
        self.renderer.dmabuf_formats()
    }

    pub fn import_test(&mut self, dmabuf: &Dmabuf) -> bool {
        self.texture_for(dmabuf).is_ok()
    }

    /// Read a client dmabuf back as tightly packed RGBA. Imported dmabufs
    /// are external-only textures that cannot back an FBO, so the picture
    /// is first blitted into an ordinary offscreen texture.
    pub fn read_rgba(&mut self, dmabuf: &Dmabuf) -> Result<(protocol::Size, Vec<u8>), String> {
        let index = self.texture_for(dmabuf)?;
        let texture = self.textures[index].1.clone();
        let size = texture.size();

        if self.offscreen.as_ref().is_none_or(|(held, _)| *held != size) {
            let target = self
                .renderer
                .create_buffer(Fourcc::Abgr8888, size)
                .map_err(|error| format!("offscreen: {error}"))?;
            self.offscreen = Some((size, target));
        }
        let Some((_, target)) = self.offscreen.as_mut() else {
            return Err("no offscreen target".to_owned());
        };

        let physical: Size<i32, Physical> = Size::from((size.w, size.h));
        let mut framebuffer = self
            .renderer
            .bind(target)
            .map_err(|error| format!("bind: {error}"))?;
        {
            let mut frame = self
                .renderer
                .render(&mut framebuffer, physical, Transform::Normal)
                .map_err(|error| format!("render: {error}"))?;
            frame
                .render_texture_from_to(
                    &texture,
                    Rectangle::from_size(size).to_f64(),
                    Rectangle::from_size(physical),
                    &[Rectangle::from_size(physical)],
                    &[],
                    Transform::Normal,
                    1.0,
                    None,
                    &[],
                )
                .map_err(|error| format!("blit: {error}"))?;
            let _sync = frame
                .finish()
                .map_err(|error| format!("finish: {error}"))?;
        }
        let mapping = self
            .renderer
            .copy_framebuffer(&framebuffer, Rectangle::from_size(size), Fourcc::Abgr8888)
            .map_err(|error| format!("copy failed: {error}"))?;
        let bytes = self
            .renderer
            .map_texture(&mapping)
            .map_err(|error| format!("map failed: {error}"))?;
        Ok((
            protocol::Size {
                width: size.w.unsigned_abs(),
                height: size.h.unsigned_abs(),
            },
            bytes.to_vec(),
        ))
    }
}
