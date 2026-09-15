use ash::vk;
use iced_wgpu::wgpu;
use std::error::Error;

/// Keep this context alive until `mpv_terminate_destroy` has returned. The feature
/// header and every node it points to have fixed heap addresses, even when this
/// context moves. They describe device creation, never a supported-feature query.
pub struct DeviceContext {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    enabled_features: Box<vk::PhysicalDeviceFeatures2<'static>>,
    _hal_features: Box<wgpu::hal::vulkan::PhysicalDeviceFeatures>,
    _host_query_reset: Box<vk::PhysicalDeviceHostQueryResetFeatures<'static>>,
}

impl DeviceContext {
    pub async fn new(
        instance: &wgpu::Instance,
        surface: &wgpu::Surface<'_>,
    ) -> Result<Self, Box<dyn Error>> {
        let adapter =
            wgpu::util::initialize_adapter_from_env_or_default(instance, Some(surface)).await?;
        let capabilities = surface.get_capabilities(&adapter);
        if !capabilities
            .formats
            .contains(&wgpu::TextureFormat::Rgb10a2Unorm)
        {
            return Err("the Vulkan surface does not support native Rgb10a2Unorm output".into());
        }
        let format = adapter.get_texture_format_features(wgpu::TextureFormat::Rgb10a2Unorm);
        if !format.allowed_usages.contains(
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST,
        ) || !format.flags.contains(
            wgpu::TextureFormatFeatureFlags::FILTERABLE
                | wgpu::TextureFormatFeatureFlags::BLENDABLE,
        ) {
            return Err(
                "Rgb10a2Unorm must support rendering, sampling, blending and transfer destination"
                    .into(),
            );
        }

        let desc = wgpu::DeviceDescriptor {
            label: Some("mpv shared Vulkan device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        };

        // HAL's feature builder includes private wgpu requirements (robustness,
        // shader capabilities, etc.) that cannot be reconstructed from Features.
        let (open_device, hal_features, host_query_reset, enabled_features) = unsafe {
            let hal = adapter
                .as_hal::<wgpu::hal::api::Vulkan>()
                .ok_or("mpv shared-device rendering requires a Vulkan adapter")?;
            let hal_surface = surface
                .as_hal::<wgpu::hal::api::Vulkan>()
                .ok_or("mpv shared-device rendering requires a Vulkan surface")?;
            let raw_surface = hal_surface
                .raw_native_handle()
                .ok_or("the surface has no native VkSurfaceKHR")?;
            let shared = hal.shared_instance();
            let raw_instance = shared.raw_instance();
            let physical_device = hal.raw_physical_device();
            let properties = raw_instance.get_physical_device_properties(physical_device);
            if shared.instance_api_version() < vk::API_VERSION_1_2
                || properties.api_version < vk::API_VERSION_1_2
            {
                return Err(
                    "mpv gpu-next requires Vulkan 1.2 or newer on instance and device".into(),
                );
            }

            let surface_api = ash::khr::surface::Instance::new(shared.entry(), raw_instance);
            let families =
                raw_instance.get_physical_device_queue_family_properties(physical_device);
            let mut queue_family = None;
            for (index, family) in families.iter().enumerate() {
                let index = u32::try_from(index)?;
                if family.queue_count > 0
                    && family
                        .queue_flags
                        .contains(vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE)
                    && surface_api.get_physical_device_surface_support(
                        physical_device,
                        index,
                        raw_surface,
                    )?
                {
                    queue_family = Some(index);
                    break;
                }
            }
            let queue_family = queue_family
                .ok_or("no graphics/compute queue family can present to this surface")?;

            // libplacebo 7 / API 360, src/vulkan/context.c:
            // pl_vulkan_required_features requires exactly hostQueryReset and
            // timelineSemaphore. Extension-specific structs avoid forbidden
            // duplication with HAL's Vulkan-1.2-promoted feature structs.
            let mut supported_reset = vk::PhysicalDeviceHostQueryResetFeatures::default();
            let mut supported_timeline = vk::PhysicalDeviceTimelineSemaphoreFeatures::default();
            let mut supported = vk::PhysicalDeviceFeatures2::default()
                .push_next(&mut supported_reset)
                .push_next(&mut supported_timeline);
            raw_instance.get_physical_device_features2(physical_device, &mut supported);
            if supported_reset.host_query_reset != vk::TRUE
                || supported_timeline.timeline_semaphore != vk::TRUE
            {
                return Err("libplacebo requires hostQueryReset and timelineSemaphore".into());
            }

            let mut extensions = hal.required_device_extensions(desc.required_features);
            let supported_extensions =
                raw_instance.enumerate_device_extension_properties(physical_device)?;
            // Enable optional DMA-BUF imports on the actual shared device. Vulkan
            // 1.2 supplies their core dependencies; unsupported extensions must
            // not prevent software decoding or copy-back playback.
            for extension in [
                ash::khr::external_memory_fd::NAME,
                ash::ext::external_memory_dma_buf::NAME,
                ash::ext::image_drm_format_modifier::NAME,
            ] {
                let supported = supported_extensions.iter().any(|properties| {
                    std::ffi::CStr::from_ptr(properties.extension_name.as_ptr()) == extension
                });
                if supported && !extensions.contains(&extension) {
                    extensions.push(extension);
                }
            }
            let extension_names: Vec<_> = extensions.iter().map(|name| name.as_ptr()).collect();
            let mut hal_features =
                Box::new(hal.physical_device_features(&extensions, desc.required_features));
            let mut host_query_reset = Box::new(
                vk::PhysicalDeviceHostQueryResetFeatures::default().host_query_reset(true),
            );
            let mut enabled_features = Box::new(vk::PhysicalDeviceFeatures2::default());
            let priorities = [1.0];
            let queues = [vk::DeviceQueueCreateInfo::default()
                .queue_family_index(queue_family)
                .queue_priorities(&priorities)];
            let info = vk::DeviceCreateInfo::default()
                .queue_create_infos(&queues)
                .enabled_extension_names(&extension_names)
                .push_next(&mut *host_query_reset);
            let info = hal_features.add_to_device_create(info);

            // HAL 29 emits this node for Vulkan >= 1.2. Enable the supported
            // required bit in that very node, rather than adding a duplicate.
            let mut node = info.p_next.cast::<vk::BaseOutStructure<'_>>().cast_mut();
            let mut timeline_found = false;
            while let Some(header) = node.as_mut() {
                if header.s_type == vk::StructureType::PHYSICAL_DEVICE_TIMELINE_SEMAPHORE_FEATURES {
                    (*node.cast::<vk::PhysicalDeviceTimelineSemaphoreFeatures<'_>>())
                        .timeline_semaphore = vk::TRUE;
                    timeline_found = true;
                    break;
                }
                node = header.p_next;
            }
            if !timeline_found {
                return Err("wgpu HAL did not provide its Vulkan 1.2 timeline feature node".into());
            }

            enabled_features.features = *info.p_enabled_features;
            enabled_features.p_next = info.p_next.cast_mut();
            let raw_device = raw_instance.create_device(physical_device, &info, None)?;

            // A callback owns destruction even if HAL wrapping fails. Retaining
            // an adapter here also keeps VkInstance alive through vkDestroyDevice;
            // HAL drops its own InstanceShared before invoking this callback.
            let destroy_device = raw_device.clone();
            let keep_instance = adapter.clone();
            let drop_callback = Box::new(move || {
                destroy_device.destroy_device(None);
                drop(keep_instance);
            });
            let open_device = hal.device_from_raw(
                raw_device,
                Some(drop_callback),
                &extensions,
                desc.required_features,
                &desc.required_limits,
                &desc.memory_hints,
                queue_family,
                0,
            )?;
            (
                open_device,
                hal_features,
                host_query_reset,
                enabled_features,
            )
        };

        // The HAL device came from this adapter, with exactly the requested
        // wgpu feature set and limits; additional Vulkan bits only serve mpv.
        let (device, queue) = unsafe { adapter.create_device_from_hal(open_device, &desc)? };
        Ok(Self {
            adapter,
            device,
            queue,
            enabled_features,
            _hal_features: hal_features,
            _host_query_reset: host_query_reset,
        })
    }

    pub fn features(&self) -> *const vk::PhysicalDeviceFeatures2<'static> {
        &*self.enabled_features
    }
}
