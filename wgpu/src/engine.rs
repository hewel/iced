use crate::graphics::{Antialiasing, Shell};
use crate::primitive;
use crate::quad;
use crate::text;
use crate::triangle;

use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct Engine {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) queue_synchronization: Option<Arc<dyn crate::QueueSynchronization>>,
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) backend: wgpu::Backend,

    pub(crate) quad_pipeline: quad::Pipeline,
    pub(crate) text_pipeline: text::Pipeline,
    pub(crate) triangle_pipeline: triangle::Pipeline,
    #[cfg(any(feature = "image", feature = "svg"))]
    pub(crate) image_pipeline: crate::image::Pipeline,
    pub(crate) primitive_storage: Arc<RwLock<primitive::Storage>>,
    _shell: Shell,
}

impl Engine {
    pub fn new(
        _adapter: &wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        antialiasing: Option<Antialiasing>, // TODO: Initialize AA pipelines lazily
        shell: Shell,
    ) -> Self {
        Self::with_queue_synchronization(
            _adapter, device, queue, format, antialiasing, shell, None,
        )
    }

    /// Creates an engine sharing external native queue synchronization.
    ///
    /// The hook is installed before renderers and image workers can clone it.
    /// See [`crate::QueueSynchronization`] for the non-reentry, callback, and
    /// custom queue/surface operation requirements.
    pub fn new_with_queue_synchronization(
        adapter: &wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        antialiasing: Option<Antialiasing>,
        shell: Shell,
        queue_synchronization: Arc<dyn crate::QueueSynchronization>,
    ) -> Self {
        Self::with_queue_synchronization(
            adapter, device, queue, format, antialiasing, shell,
            Some(queue_synchronization),
        )
    }

    fn with_queue_synchronization(
        _adapter: &wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        antialiasing: Option<Antialiasing>,
        shell: Shell,
        queue_synchronization: Option<Arc<dyn crate::QueueSynchronization>>,
    ) -> Self {
        Self {
            format,
            backend: _adapter.get_info().backend,

            quad_pipeline: quad::Pipeline::new(&device, format),
            text_pipeline: text::Pipeline::new(&device, &queue, format),
            triangle_pipeline: triangle::Pipeline::new(&device, format, antialiasing),

            #[cfg(any(feature = "image", feature = "svg"))]
            image_pipeline: {
                let backend = _adapter.get_info().backend;

                crate::image::Pipeline::new(&device, format, backend)
            },

            primitive_storage: Arc::new(RwLock::new(primitive::Storage::default())),

            device,
            queue,
            queue_synchronization,
            _shell: shell,
        }
    }

    #[cfg(any(feature = "image", feature = "svg"))]
    pub fn create_image_cache(&self) -> crate::image::Cache {
        self.image_pipeline.create_cache(
            &self.device,
            &self.queue,
            &self._shell,
            self.queue_synchronization.clone(),
        )
    }

    pub fn trim(&mut self) {
        self.text_pipeline.trim();

        self.primitive_storage
            .write()
            .expect("primitive storage should be writable")
            .trim();
    }
}
