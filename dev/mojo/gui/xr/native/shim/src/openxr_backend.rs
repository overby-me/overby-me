//! OpenXR backend — real XR session lifecycle, Vulkan graphics binding,
//! frame loop, input actions, and per-panel swapchain management.
//!
//! This module encapsulates all OpenXR and Vulkan complexity behind the
//! `OpenXrBackend` struct. The main `lib.rs` uses it optionally — when
//! the OpenXR runtime is available, `mxr_create_session` creates a real
//! backend; otherwise it falls back to headless mode.
//!
//! # Architecture
//!
//! ```text
//! OpenXrBackend
//!   ├── openxr::Entry (loaded via dlopen)
//!   ├── openxr::Instance
//!   ├── openxr::Session<Vulkan>
//!   │   ├── FrameWaiter  — xrWaitFrame
//!   │   ├── FrameStream  — xrBeginFrame / xrEndFrame
//!   │   └── per-panel Swapchain<Vulkan>
//!   ├── Reference spaces (stage, view, local)
//!   ├── Input actions (grip/aim poses, select/squeeze booleans)
//!   ├── ash Vulkan device (shared with wgpu via HAL)
//!   └── wgpu Device + vello Renderer (for panel texture rendering)
//! ```
//!
//! # Vulkan interop
//!
//! OpenXR requires a Vulkan instance and device that satisfy its
//! requirements (specific extensions, specific physical device). We:
//!
//! 1. Load the OpenXR entry via `Entry::load()` (dlopen)
//! 2. Create an OpenXR instance with `XR_KHR_vulkan_enable2`
//! 3. Query Vulkan requirements from OpenXR
//! 4. Create the Vulkan instance + device via `ash` per those requirements
//! 5. Pass the raw Vulkan handles to OpenXR to create the session
//! 6. Wrap the same Vulkan device in `wgpu` via HAL APIs for Vello rendering
//!
//! This ensures a single Vulkan device is shared between OpenXR compositing
//! and Vello rendering — no GPU→CPU→GPU copies needed.

use std::collections::HashMap;
use std::ffi::{CString, c_void};

use ash::vk::Handle;
use openxr as xr;

use crate::{MXR_HAND_HEAD, MXR_HAND_LEFT, MXR_HAND_RIGHT, MxrPose, Panel};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Swapchain format — sRGB for correct color rendering in the headset.
const SWAPCHAIN_FORMAT: u32 = ash::vk::Format::R8G8B8A8_SRGB.as_raw() as u32;

/// Fallback swapchain format if sRGB is not available.
const SWAPCHAIN_FORMAT_FALLBACK: u32 = ash::vk::Format::R8G8B8A8_UNORM.as_raw() as u32;

// ---------------------------------------------------------------------------
// Per-panel swapchain
// ---------------------------------------------------------------------------

/// Manages an OpenXR swapchain and its associated wgpu textures for a single
/// panel. Each panel gets its own swapchain so the compositor can reproject
/// them independently as quad layers.
pub(crate) struct PanelSwapchain {
    pub swapchain: xr::Swapchain<xr::Vulkan>,
    /// wgpu textures wrapping each swapchain image (created once, reused).
    pub textures: Vec<wgpu::Texture>,
    /// Texture views for each swapchain image.
    pub views: Vec<wgpu::TextureView>,
    pub width: u32,
    pub height: u32,
    /// Index of the currently acquired image (None if not acquired).
    pub acquired_index: Option<usize>,
}

// ---------------------------------------------------------------------------
// OpenXrBackend
// ---------------------------------------------------------------------------

/// All OpenXR + Vulkan state needed for a real XR session.
pub(crate) struct OpenXrBackend {
    // ── OpenXR core ─────────────────────────────────────────────────
    #[allow(dead_code)]
    entry: xr::Entry,
    instance: xr::Instance,
    system: xr::SystemId,
    session: xr::Session<xr::Vulkan>,
    frame_waiter: xr::FrameWaiter,
    frame_stream: xr::FrameStream<xr::Vulkan>,
    session_running: bool,
    /// Predicted display time from the most recent xrWaitFrame.
    predicted_display_time: xr::Time,
    /// Predicted display period from the most recent xrWaitFrame.
    #[allow(dead_code)]
    predicted_display_period: xr::Duration,
    /// Whether the compositor advised us to render this frame.
    should_render: bool,

    // ── Reference spaces ────────────────────────────────────────────
    stage_space: xr::Space,
    view_space: xr::Space,

    // ── View configuration ──────────────────────────────────────────
    #[allow(dead_code)]
    view_config_views: Vec<xr::ViewConfigurationView>,

    // ── Input ───────────────────────────────────────────────────────
    action_set: xr::ActionSet,
    left_grip_action: xr::Action<xr::Posef>,
    right_grip_action: xr::Action<xr::Posef>,
    left_aim_action: xr::Action<xr::Posef>,
    right_aim_action: xr::Action<xr::Posef>,
    select_action: xr::Action<bool>,
    squeeze_action: xr::Action<bool>,
    left_grip_space: xr::Space,
    right_grip_space: xr::Space,
    left_aim_space: xr::Space,
    right_aim_space: xr::Space,

    // ── Vulkan resources (owned, shared with wgpu) ──────────────────
    #[allow(dead_code)]
    vk_entry: ash::Entry,
    #[allow(dead_code)]
    vk_instance: ash::Instance,
    #[allow(dead_code)]
    vk_physical_device: ash::vk::PhysicalDevice,
    #[allow(dead_code)]
    vk_device: ash::Device,
    #[allow(dead_code)]
    vk_queue_family_index: u32,

    // ── wgpu + Vello (for panel rendering) ──────────────────────────
    wgpu_device: wgpu::Device,
    wgpu_queue: wgpu::Queue,
    vello_renderer: vello::Renderer,

    // ── Per-panel swapchains ────────────────────────────────────────
    panel_swapchains: HashMap<u32, PanelSwapchain>,

    // ── Extension flags ─────────────────────────────────────────────
    has_hand_tracking: bool,
    has_passthrough: bool,
}

impl OpenXrBackend {
    /// Try to create a real OpenXR backend. Returns `None` if any step fails
    /// (no OpenXR runtime, no HMD, Vulkan setup failure, etc.).
    ///
    /// This performs the full initialization sequence:
    /// 1. Load OpenXR entry (dlopen)
    /// 2. Create OpenXR instance
    /// 3. Get system (HMD)
    /// 4. Create Vulkan instance + device per OpenXR requirements
    /// 5. Create OpenXR session with Vulkan binding
    /// 6. Create reference spaces
    /// 7. Set up input actions
    /// 8. Wrap Vulkan device in wgpu for Vello rendering
    pub fn try_new(app_name: &str) -> Option<Self> {
        // ── 1. Load OpenXR ──────────────────────────────────────────

        let entry = unsafe { xr::Entry::load().ok()? };

        // ── 2. Create instance ──────────────────────────────────────

        let available_exts = entry.enumerate_extensions().ok()?;

        // We need Vulkan graphics binding.
        if !available_exts.khr_vulkan_enable2 {
            eprintln!("OpenXR: XR_KHR_vulkan_enable2 not available");
            return None;
        }

        let mut enabled_exts = xr::ExtensionSet::default();
        enabled_exts.khr_vulkan_enable2 = true;

        let has_hand_tracking = available_exts.ext_hand_tracking;
        if has_hand_tracking {
            enabled_exts.ext_hand_tracking = true;
        }

        // Check for passthrough (Meta Quest / other runtimes)
        let has_passthrough = available_exts.fb_passthrough;
        if has_passthrough {
            enabled_exts.fb_passthrough = true;
        }

        let app_name_c =
            CString::new(app_name).unwrap_or_else(|_| CString::new("mojo-gui").unwrap());
        let app_name_bytes = app_name_c.as_bytes();
        let mut name_buf = [0u8; 128];
        let copy_len = app_name_bytes.len().min(127);
        name_buf[..copy_len].copy_from_slice(&app_name_bytes[..copy_len]);

        let instance = entry
            .create_instance(
                &xr::ApplicationInfo {
                    application_name: &String::from_utf8_lossy(&name_buf[..copy_len]),
                    application_version: 0,
                    engine_name: "mojo-gui",
                    engine_version: 0,
                    api_version: xr::Version::new(1, 0, 0),
                },
                &enabled_exts,
                &[],
            )
            .ok()?;

        // ── 3. Get system (HMD) ────────────────────────────────────

        let system = instance.system(xr::FormFactor::HEAD_MOUNTED_DISPLAY).ok()?;

        // ── 4. Vulkan setup ────────────────────────────────────────

        // Query Vulkan requirements from OpenXR.
        let _vk_reqs = instance.graphics_requirements::<xr::Vulkan>(system).ok()?;

        // Get required Vulkan instance extensions from OpenXR.
        let vk_instance_exts_raw = instance.vulkan_legacy_instance_extensions(system).ok()?;

        let vk_instance_ext_names: Vec<CString> = vk_instance_exts_raw
            .split(' ')
            .filter(|s| !s.is_empty())
            .filter_map(|s| CString::new(s).ok())
            .collect();

        let vk_instance_ext_ptrs: Vec<*const i8> =
            vk_instance_ext_names.iter().map(|s| s.as_ptr()).collect();

        // Create Vulkan entry and instance.
        let vk_entry = unsafe { ash::Entry::load().ok()? };

        let vk_app_info =
            ash::vk::ApplicationInfo::default().api_version(ash::vk::make_api_version(0, 1, 1, 0));

        let vk_instance_ci = ash::vk::InstanceCreateInfo::default()
            .application_info(&vk_app_info)
            .enabled_extension_names(&vk_instance_ext_ptrs);

        let vk_instance = unsafe { vk_entry.create_instance(&vk_instance_ci, None).ok()? };

        // Get the physical device OpenXR wants us to use.
        let vk_physical_device = ash::vk::PhysicalDevice::from_raw(unsafe {
            instance
                .vulkan_graphics_device(system, vk_instance.handle().as_raw() as *const c_void)
                .ok()? as u64
        });

        // Get required Vulkan device extensions from OpenXR.
        let vk_device_exts_raw = instance.vulkan_legacy_device_extensions(system).ok()?;

        let vk_device_ext_names: Vec<CString> = vk_device_exts_raw
            .split(' ')
            .filter(|s| !s.is_empty())
            .filter_map(|s| CString::new(s).ok())
            .collect();

        let vk_device_ext_ptrs: Vec<*const i8> =
            vk_device_ext_names.iter().map(|s| s.as_ptr()).collect();

        // Find a graphics queue family.
        let queue_family_index = unsafe {
            let families =
                vk_instance.get_physical_device_queue_family_properties(vk_physical_device);
            families
                .iter()
                .enumerate()
                .find(|(_, props)| props.queue_flags.contains(ash::vk::QueueFlags::GRAPHICS))
                .map(|(i, _)| i as u32)?
        };

        let queue_priority = [1.0f32];
        let queue_ci = ash::vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&queue_priority);

        let device_ci = ash::vk::DeviceCreateInfo::default()
            .queue_create_infos(std::slice::from_ref(&queue_ci))
            .enabled_extension_names(&vk_device_ext_ptrs);

        let vk_device = unsafe {
            vk_instance
                .create_device(vk_physical_device, &device_ci, None)
                .ok()?
        };

        let _vk_queue = unsafe { vk_device.get_device_queue(queue_family_index, 0) };

        // ── 5. Create OpenXR session ───────────────────────────────

        let (session, frame_waiter, frame_stream) = unsafe {
            instance
                .create_session::<xr::Vulkan>(
                    system,
                    &xr::vulkan::SessionCreateInfo {
                        instance: vk_instance.handle().as_raw() as *const c_void,
                        physical_device: vk_physical_device.as_raw() as *const c_void,
                        device: vk_device.handle().as_raw() as *const c_void,
                        queue_family_index,
                        queue_index: 0,
                    },
                )
                .ok()?
        };

        // ── 6. Reference spaces ────────────────────────────────────

        let stage_space = session
            .create_reference_space(xr::ReferenceSpaceType::STAGE, xr::Posef::IDENTITY)
            .or_else(|_| {
                session.create_reference_space(xr::ReferenceSpaceType::LOCAL, xr::Posef::IDENTITY)
            })
            .ok()?;

        let view_space = session
            .create_reference_space(xr::ReferenceSpaceType::VIEW, xr::Posef::IDENTITY)
            .ok()?;

        // ── 7. View configuration ──────────────────────────────────

        let view_config_views = instance
            .enumerate_view_configuration_views(system, xr::ViewConfigurationType::PRIMARY_STEREO)
            .ok()?;

        // ── 8. Input actions ───────────────────────────────────────

        let action_set = instance
            .create_action_set("mojo-gui", "mojo-gui input", 0)
            .ok()?;

        let left_grip_action = action_set
            .create_action::<xr::Posef>("left-grip", "Left Grip Pose", &[])
            .ok()?;
        let right_grip_action = action_set
            .create_action::<xr::Posef>("right-grip", "Right Grip Pose", &[])
            .ok()?;
        let left_aim_action = action_set
            .create_action::<xr::Posef>("left-aim", "Left Aim Pose", &[])
            .ok()?;
        let right_aim_action = action_set
            .create_action::<xr::Posef>("right-aim", "Right Aim Pose", &[])
            .ok()?;
        let select_action = action_set
            .create_action::<bool>("select", "Select (Trigger)", &[])
            .ok()?;
        let squeeze_action = action_set
            .create_action::<bool>("squeeze", "Squeeze (Grip)", &[])
            .ok()?;

        // Suggest bindings for common interaction profiles.
        let left_grip_path = instance
            .string_to_path("/user/hand/left/input/grip/pose")
            .ok()?;
        let right_grip_path = instance
            .string_to_path("/user/hand/right/input/grip/pose")
            .ok()?;
        let left_aim_path = instance
            .string_to_path("/user/hand/left/input/aim/pose")
            .ok()?;
        let right_aim_path = instance
            .string_to_path("/user/hand/right/input/aim/pose")
            .ok()?;
        let left_select_path = instance
            .string_to_path("/user/hand/left/input/select/click")
            .ok()?;
        let right_select_path = instance
            .string_to_path("/user/hand/right/input/select/click")
            .ok()?;
        let left_squeeze_path = instance
            .string_to_path("/user/hand/left/input/squeeze/click")
            .ok()?;
        let right_squeeze_path = instance
            .string_to_path("/user/hand/right/input/squeeze/click")
            .ok()?;

        // Khronos Simple Controller — universally supported.
        let simple_profile = instance
            .string_to_path("/interaction_profiles/khr/simple_controller")
            .ok()?;

        let _ = instance.suggest_interaction_profile_bindings(
            simple_profile,
            &[
                xr::Binding::new(&left_grip_action, left_grip_path),
                xr::Binding::new(&right_grip_action, right_grip_path),
                xr::Binding::new(&left_aim_action, left_aim_path),
                xr::Binding::new(&right_aim_action, right_aim_path),
                xr::Binding::new(&select_action, left_select_path),
                xr::Binding::new(&select_action, right_select_path),
                xr::Binding::new(&squeeze_action, left_squeeze_path),
                xr::Binding::new(&squeeze_action, right_squeeze_path),
            ],
        );

        // Also try Oculus Touch for Meta Quest.
        if let Ok(touch_profile) =
            instance.string_to_path("/interaction_profiles/oculus/touch_controller")
        {
            let left_trigger = instance
                .string_to_path("/user/hand/left/input/trigger/value")
                .ok()?;
            let right_trigger = instance
                .string_to_path("/user/hand/right/input/trigger/value")
                .ok()?;

            let _ = instance.suggest_interaction_profile_bindings(
                touch_profile,
                &[
                    xr::Binding::new(&left_grip_action, left_grip_path),
                    xr::Binding::new(&right_grip_action, right_grip_path),
                    xr::Binding::new(&left_aim_action, left_aim_path),
                    xr::Binding::new(&right_aim_action, right_aim_path),
                    xr::Binding::new(&select_action, left_trigger),
                    xr::Binding::new(&select_action, right_trigger),
                    xr::Binding::new(&squeeze_action, left_squeeze_path),
                    xr::Binding::new(&squeeze_action, right_squeeze_path),
                ],
            );
        }

        session.attach_action_sets(&[&action_set]).ok()?;

        // Create action spaces for pose tracking.
        let left_grip_space = left_grip_action
            .create_space(session.clone(), xr::Path::NULL, xr::Posef::IDENTITY)
            .ok()?;
        let right_grip_space = right_grip_action
            .create_space(session.clone(), xr::Path::NULL, xr::Posef::IDENTITY)
            .ok()?;
        let left_aim_space = left_aim_action
            .create_space(session.clone(), xr::Path::NULL, xr::Posef::IDENTITY)
            .ok()?;
        let right_aim_space = right_aim_action
            .create_space(session.clone(), xr::Path::NULL, xr::Posef::IDENTITY)
            .ok()?;

        // ── 9. Wrap Vulkan in wgpu for Vello rendering ─────────────

        let (wgpu_device, wgpu_queue, vello_renderer) = Self::create_wgpu_from_vulkan(
            &vk_entry,
            &vk_instance,
            vk_physical_device,
            &vk_device,
            queue_family_index,
        )?;

        Some(Self {
            entry,
            instance,
            system,
            session,
            frame_waiter,
            frame_stream,
            session_running: false,
            predicted_display_time: xr::Time::from_nanos(0),
            predicted_display_period: xr::Duration::from_nanos(0),
            should_render: false,
            stage_space,
            view_space,
            view_config_views,
            action_set,
            left_grip_action,
            right_grip_action,
            left_aim_action,
            right_aim_action,
            select_action,
            squeeze_action,
            left_grip_space,
            right_grip_space,
            left_aim_space,
            right_aim_space,
            vk_entry,
            vk_instance,
            vk_physical_device,
            vk_device,
            vk_queue_family_index: queue_family_index,
            wgpu_device,
            wgpu_queue,
            vello_renderer,
            panel_swapchains: HashMap::new(),
            has_hand_tracking,
            has_passthrough,
        })
    }

    // ── wgpu creation from raw Vulkan handles ───────────────────────

    /// Create a wgpu Device + Queue + Vello Renderer from existing Vulkan
    /// handles. Returns None if any step fails.
    fn create_wgpu_from_vulkan(
        vk_entry: &ash::Entry,
        vk_instance: &ash::Instance,
        vk_physical_device: ash::vk::PhysicalDevice,
        vk_device: &ash::Device,
        queue_family_index: u32,
    ) -> Option<(wgpu::Device, wgpu::Queue, vello::Renderer)> {
        use wgpu::hal::api::Vulkan as VulkanApi;

        // Collect instance extensions that wgpu wants.
        let instance_extensions = <VulkanApi as wgpu::hal::Api>::Instance::desired_extensions(
            vk_entry,
            ash::vk::make_api_version(0, 1, 1, 0),
            wgpu::InstanceFlags::empty(),
        )
        .ok()?;

        // Create the HAL instance wrapping our raw Vulkan instance.
        let hal_instance = unsafe {
            <VulkanApi as wgpu::hal::Api>::Instance::from_raw(
                vk_entry.clone(),
                vk_instance.clone(),
                ash::vk::make_api_version(0, 1, 1, 0),
                0,    // android_sdk_version
                None, // debug_utils_create_info
                instance_extensions,
                wgpu::InstanceFlags::empty(),
                wgpu::MemoryBudgetThresholds::default(),
                false, // has_nv_optimus
                None,  // drop_callback
            )
            .ok()?
        };

        // Expose the adapter for our physical device.
        let hal_exposed_adapter = hal_instance.expose_adapter(vk_physical_device)?;

        // Create the wgpu instance from the HAL instance.
        let wgpu_instance = unsafe { wgpu::Instance::from_hal::<VulkanApi>(hal_instance) };

        // Create the wgpu adapter from the HAL exposed adapter.
        let wgpu_adapter = unsafe { wgpu_instance.create_adapter_from_hal(hal_exposed_adapter) };

        // Create a HAL open device wrapping our raw Vulkan device.
        let hal_open_device = unsafe {
            let adapter_guard = wgpu_adapter.as_hal::<VulkanApi>()?;
            adapter_guard
                .device_from_raw(
                    vk_device.clone(),
                    None, // drop_callback
                    &[],  // enabled_extensions
                    wgpu::Features::empty(),
                    &wgpu::MemoryHints::Performance,
                    queue_family_index,
                    0, // queue_index
                )
                .ok()?
        };

        // Create the wgpu device + queue from the HAL device.
        let (wgpu_device, wgpu_queue) = unsafe {
            wgpu_adapter
                .create_device_from_hal(
                    hal_open_device,
                    &wgpu::DeviceDescriptor {
                        label: Some("mojo-gui-xr"),
                        required_features: wgpu::Features::empty(),
                        required_limits: wgpu::Limits::default(),
                        memory_hints: wgpu::MemoryHints::Performance,
                        trace: wgpu::Trace::Off,
                    },
                )
                .ok()?
        };

        // Create the Vello renderer.
        let vello_renderer = vello::Renderer::new(
            &wgpu_device,
            RendererOptions {
                use_cpu: false,
                antialiasing_support: AaSupport::area_only(),
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .ok()?;

        Some((wgpu_device, wgpu_queue, vello_renderer))
    }

    // ── Session event polling ───────────────────────────────────────

    /// Poll OpenXR session events and update session state.
    /// Returns the new session state as an MXR_STATE_* constant.
    pub fn poll_session_events(&mut self) -> i32 {
        let mut buf = xr::EventDataBuffer::new();
        while let Some(event) = self.instance.poll_event(&mut buf).ok().flatten() {
            match event {
                xr::Event::SessionStateChanged(e) => {
                    let new_state = e.state();
                    match new_state {
                        xr::SessionState::READY => {
                            // Begin the session.
                            let _ = self
                                .session
                                .begin(xr::ViewConfigurationType::PRIMARY_STEREO);
                            self.session_running = true;
                            return crate::MXR_STATE_READY;
                        }
                        xr::SessionState::FOCUSED => {
                            return crate::MXR_STATE_FOCUSED;
                        }
                        xr::SessionState::VISIBLE => {
                            return crate::MXR_STATE_VISIBLE;
                        }
                        xr::SessionState::STOPPING => {
                            let _ = self.session.end();
                            self.session_running = false;
                            return crate::MXR_STATE_STOPPING;
                        }
                        xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => {
                            self.session_running = false;
                            return crate::MXR_STATE_EXITING;
                        }
                        _ => {}
                    }
                }
                xr::Event::InstanceLossPending(_) => {
                    self.session_running = false;
                    return crate::MXR_STATE_EXITING;
                }
                _ => {}
            }
        }
        // No state change.
        -1
    }

    // ── Frame loop ──────────────────────────────────────────────────

    /// Wait for the next frame from the OpenXR runtime.
    /// Returns predicted display time in nanoseconds, or 0 if the session
    /// is not running.
    pub fn wait_frame(&mut self) -> i64 {
        if !self.session_running {
            return 0;
        }
        match self.frame_waiter.wait() {
            Ok(state) => {
                self.predicted_display_time = state.predicted_display_time;
                self.predicted_display_period = state.predicted_display_period;
                self.should_render = state.should_render;
                state.predicted_display_time.as_nanos()
            }
            Err(e) => {
                eprintln!("OpenXR: xrWaitFrame failed: {e}");
                0
            }
        }
    }

    /// Begin a new frame. Returns 1 if we should render, 0 otherwise.
    pub fn begin_frame(&mut self) -> i32 {
        if !self.session_running {
            return 0;
        }
        match self.frame_stream.begin() {
            Ok(()) => {
                if self.should_render {
                    1
                } else {
                    0
                }
            }
            Err(e) => {
                eprintln!("OpenXR: xrBeginFrame failed: {e}");
                0
            }
        }
    }

    /// End the frame, submitting composition layers for all visible panels
    /// as quad layers.
    pub fn end_frame(&mut self, panels: &HashMap<u32, Panel>, panel_order: &[u32]) {
        if !self.session_running {
            return;
        }

        // Build composition layers for visible panels that have swapchains.
        let mut quad_layers: Vec<xr::CompositionLayerQuad<xr::Vulkan>> = Vec::new();

        for &panel_id in panel_order {
            let Some(panel) = panels.get(&panel_id) else {
                continue;
            };
            if !panel.visible {
                continue;
            }
            let Some(swapchain) = self.panel_swapchains.get(&panel_id) else {
                continue;
            };

            let t = &panel.transform;

            // Build the pose from panel transform.
            let pose = xr::Posef {
                orientation: xr::Quaternionf {
                    x: t.rotation[0],
                    y: t.rotation[1],
                    z: t.rotation[2],
                    w: t.rotation[3],
                },
                position: xr::Vector3f {
                    x: t.position[0],
                    y: t.position[1],
                    z: t.position[2],
                },
            };

            let layer = xr::CompositionLayerQuad::new()
                .space(&self.stage_space)
                .eye_visibility(xr::EyeVisibility::BOTH)
                .sub_image(
                    xr::SwapchainSubImage::new()
                        .swapchain(&swapchain.swapchain)
                        .image_rect(xr::Rect2Di {
                            offset: xr::Offset2Di { x: 0, y: 0 },
                            extent: xr::Extent2Di {
                                width: swapchain.width as i32,
                                height: swapchain.height as i32,
                            },
                        })
                        .image_array_index(0),
                )
                .pose(pose)
                .size(xr::Extent2Df {
                    width: t.size_m[0],
                    height: t.size_m[1],
                });

            quad_layers.push(layer);
        }

        // Convert to trait object references for xrEndFrame.
        let layer_refs: Vec<&xr::CompositionLayerBase<xr::Vulkan>> = quad_layers
            .iter()
            .map(|l| l as &xr::CompositionLayerBase<xr::Vulkan>)
            .collect();

        let _ = self.frame_stream.end(
            self.predicted_display_time,
            xr::EnvironmentBlendMode::OPAQUE,
            &layer_refs,
        );
    }

    // ── Swapchain management ────────────────────────────────────────

    /// Create a swapchain for a panel. Returns true on success.
    pub fn create_panel_swapchain(&mut self, panel_id: u32, width: u32, height: u32) -> bool {
        if self.panel_swapchains.contains_key(&panel_id) {
            return true; // Already exists.
        }

        // Enumerate supported formats and pick our preferred one.
        let formats: Vec<u32> = match self.session.enumerate_swapchain_formats() {
            Ok(f) => f,
            Err(e) => {
                eprintln!("OpenXR: enumerate_swapchain_formats failed: {e}");
                return false;
            }
        };

        let format: u32 = if formats.contains(&SWAPCHAIN_FORMAT) {
            SWAPCHAIN_FORMAT
        } else if formats.contains(&SWAPCHAIN_FORMAT_FALLBACK) {
            SWAPCHAIN_FORMAT_FALLBACK
        } else if let Some(&f) = formats.first() {
            eprintln!("OpenXR: preferred swapchain formats not available, using format {f}");
            f
        } else {
            eprintln!("OpenXR: no swapchain formats available");
            return false;
        };

        let swapchain = match self.session.create_swapchain(&xr::SwapchainCreateInfo {
            create_flags: xr::SwapchainCreateFlags::EMPTY,
            usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT
                | xr::SwapchainUsageFlags::SAMPLED
                | xr::SwapchainUsageFlags::TRANSFER_DST,
            format,
            sample_count: 1,
            width,
            height,
            face_count: 1,
            array_size: 1,
            mip_count: 1,
        }) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("OpenXR: create_swapchain failed: {e}");
                return false;
            }
        };

        // Get the swapchain images (Vulkan images).
        let images = match swapchain.enumerate_images() {
            Ok(imgs) => imgs,
            Err(e) => {
                eprintln!("OpenXR: enumerate_images failed: {e}");
                return false;
            }
        };

        // Determine the wgpu texture format from the Vulkan format.
        let wgpu_format = if format == SWAPCHAIN_FORMAT {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };

        // Wrap each Vulkan image in a wgpu texture.
        let mut textures = Vec::with_capacity(images.len());
        let mut views = Vec::with_capacity(images.len());

        for vk_image in &images {
            let hal_texture = unsafe {
                let device_guard = self
                    .wgpu_device
                    .as_hal::<wgpu::hal::api::Vulkan>()
                    .expect("XR backend must use Vulkan wgpu device");
                device_guard.texture_from_raw(
                    ash::vk::Image::from_raw(*vk_image),
                    &wgpu::hal::TextureDescriptor {
                        label: Some("xr-swapchain"),
                        size: wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu_format,
                        usage: wgpu::TextureUses::COLOR_TARGET | wgpu::TextureUses::COPY_DST,
                        memory_flags: wgpu::hal::MemoryFlags::empty(),
                        view_formats: vec![],
                    },
                    None, // No drop guard — OpenXR owns the image.
                )
            };

            let texture = unsafe {
                self.wgpu_device
                    .create_texture_from_hal::<wgpu::hal::api::Vulkan>(
                        hal_texture,
                        &wgpu::TextureDescriptor {
                            label: Some("xr-swapchain"),
                            size: wgpu::Extent3d {
                                width,
                                height,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu_format,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                                | wgpu::TextureUsages::COPY_DST,
                            view_formats: &[],
                        },
                    )
            };

            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            textures.push(texture);
            views.push(view);
        }

        self.panel_swapchains.insert(
            panel_id,
            PanelSwapchain {
                swapchain,
                textures,
                views,
                width,
                height,
                acquired_index: None,
            },
        );

        true
    }

    /// Destroy a panel's swapchain.
    pub fn destroy_panel_swapchain(&mut self, panel_id: u32) {
        self.panel_swapchains.remove(&panel_id);
    }

    /// Acquire a swapchain image for a panel. Returns the wgpu TextureView
    /// to render into, or None if acquisition fails.
    fn acquire_swapchain_image(&mut self, panel_id: u32) -> Option<&wgpu::TextureView> {
        let sc = self.panel_swapchains.get_mut(&panel_id)?;

        if sc.acquired_index.is_some() {
            // Already acquired — return the current view.
            return sc.views.get(sc.acquired_index.unwrap());
        }

        let index = sc.swapchain.acquire_image().ok()? as usize;
        sc.swapchain
            .wait_image(xr::Duration::from_nanos(100_000_000)) // 100ms timeout
            .ok()?;
        sc.acquired_index = Some(index);
        sc.views.get(index)
    }

    /// Release the currently acquired swapchain image for a panel.
    fn release_swapchain_image(&mut self, panel_id: u32) {
        if let Some(sc) = self.panel_swapchains.get_mut(&panel_id) {
            if sc.acquired_index.is_some() {
                let _ = sc.swapchain.release_image();
                sc.acquired_index = None;
            }
        }
    }

    // ── Panel rendering ─────────────────────────────────────────────

    /// Render a dirty panel to its swapchain image using Vello.
    /// Creates the swapchain on first render if it doesn't exist.
    /// Returns true if the panel was rendered.
    pub fn render_panel(&mut self, panel: &mut Panel) -> bool {
        let panel_id = panel.panel_id;
        let width = panel.texture_width;
        let height = panel.texture_height;

        // Ensure swapchain exists.
        if !self.panel_swapchains.contains_key(&panel_id) {
            if !self.create_panel_swapchain(panel_id, width, height) {
                return false;
            }
        }

        // Acquire swapchain image.
        // We need to work around borrow checker: acquire first, then render.
        let sc = self.panel_swapchains.get_mut(&panel_id).unwrap();
        if sc.acquired_index.is_none() {
            let index = match sc.swapchain.acquire_image() {
                Ok(i) => i as usize,
                Err(_) => return false,
            };
            if sc
                .swapchain
                .wait_image(xr::Duration::from_nanos(100_000_000))
                .is_err()
            {
                return false;
            }
            sc.acquired_index = Some(index);
        }
        let image_index = sc.acquired_index.unwrap();
        let view = &sc.views[image_index];

        // Paint the panel's Blitz DOM to a Vello scene.
        let mut scene = vello::Scene::new();
        let viewport = blitz_traits::shell::Viewport {
            window_size: (width, height),
            ..Default::default()
        };

        let mut painter = anyrender_vello::VelloScenePainter::new(&mut scene);
        blitz_paint::paint_scene(
            &mut painter,
            &panel.doc,
            viewport.scale() as f64,
            width,
            height,
        );

        // Render the scene to the swapchain texture.
        let params = vello::RenderParams {
            base_color: vello::peniko::Color::from_rgba8(26, 26, 46, 255), // Match UA dark bg
            width,
            height,
            antialiasing_method: vello::AaConfig::Area,
        };

        let _ = self.vello_renderer.render_to_texture(
            &self.wgpu_device,
            &self.wgpu_queue,
            &scene,
            view,
            &params,
        );

        // Release the swapchain image.
        let sc = self.panel_swapchains.get_mut(&panel_id).unwrap();
        let _ = sc.swapchain.release_image();
        sc.acquired_index = None;

        true
    }

    // ── Input ───────────────────────────────────────────────────────

    /// Sync input actions. Call once per frame before querying poses.
    pub fn sync_actions(&self) {
        if !self.session_running {
            return;
        }
        let _ = self
            .session
            .sync_actions(&[xr::ActiveActionSet::new(&self.action_set)]);
    }

    /// Get a hand/head pose relative to stage space.
    pub fn get_pose(&self, hand: u8) -> MxrPose {
        if !self.session_running {
            return MxrPose::default();
        }

        let space = match hand {
            MXR_HAND_LEFT => &self.left_grip_space,
            MXR_HAND_RIGHT => &self.right_grip_space,
            MXR_HAND_HEAD => &self.view_space,
            _ => return MxrPose::default(),
        };

        match space.locate(&self.stage_space, self.predicted_display_time) {
            Ok(location) => {
                let flags = location.location_flags;
                let valid = flags.contains(
                    xr::SpaceLocationFlags::ORIENTATION_VALID
                        | xr::SpaceLocationFlags::POSITION_VALID,
                );
                if valid {
                    let p = location.pose.position;
                    let q = location.pose.orientation;
                    MxrPose {
                        px: p.x,
                        py: p.y,
                        pz: p.z,
                        qx: q.x,
                        qy: q.y,
                        qz: q.z,
                        qw: q.w,
                        valid: 1,
                    }
                } else {
                    MxrPose::default()
                }
            }
            Err(_) => MxrPose::default(),
        }
    }

    /// Get the aim ray (origin + direction) for a controller hand.
    /// Returns (origin_x, origin_y, origin_z, dir_x, dir_y, dir_z, valid).
    pub fn get_aim_ray(&self, hand: u8) -> ([f32; 3], [f32; 3], bool) {
        if !self.session_running {
            return ([0.0; 3], [0.0; 3], false);
        }

        let space = match hand {
            MXR_HAND_LEFT => &self.left_aim_space,
            MXR_HAND_RIGHT => &self.right_aim_space,
            _ => return ([0.0; 3], [0.0; 3], false),
        };

        match space.locate(&self.stage_space, self.predicted_display_time) {
            Ok(location) => {
                let flags = location.location_flags;
                let valid = flags.contains(
                    xr::SpaceLocationFlags::ORIENTATION_VALID
                        | xr::SpaceLocationFlags::POSITION_VALID,
                );
                if !valid {
                    return ([0.0; 3], [0.0; 3], false);
                }

                let p = location.pose.position;
                let q = location.pose.orientation;

                // The aim direction is the forward vector (-Z) rotated by the
                // controller's orientation quaternion.
                let dx = -2.0 * (q.x * q.z + q.w * q.y);
                let dy = -2.0 * (q.y * q.z - q.w * q.x);
                let dz = -(1.0 - 2.0 * (q.x * q.x + q.y * q.y));

                ([p.x, p.y, p.z], [dx, dy, dz], true)
            }
            Err(_) => ([0.0; 3], [0.0; 3], false),
        }
    }

    /// Check if the select (trigger) action is active for either hand.
    /// Returns (left_active, right_active).
    pub fn get_select_state(&self) -> (bool, bool) {
        if !self.session_running {
            return (false, false);
        }

        let left = self
            .select_action
            .state(
                &self.session,
                self.instance
                    .string_to_path("/user/hand/left")
                    .unwrap_or(xr::Path::NULL),
            )
            .map(|s| s.current_state)
            .unwrap_or(false);

        let right = self
            .select_action
            .state(
                &self.session,
                self.instance
                    .string_to_path("/user/hand/right")
                    .unwrap_or(xr::Path::NULL),
            )
            .map(|s| s.current_state)
            .unwrap_or(false);

        (left, right)
    }

    /// Check if the squeeze (grip) action is active for either hand.
    /// Returns (left_active, right_active).
    pub fn get_squeeze_state(&self) -> (bool, bool) {
        if !self.session_running {
            return (false, false);
        }

        let left = self
            .squeeze_action
            .state(
                &self.session,
                self.instance
                    .string_to_path("/user/hand/left")
                    .unwrap_or(xr::Path::NULL),
            )
            .map(|s| s.current_state)
            .unwrap_or(false);

        let right = self
            .squeeze_action
            .state(
                &self.session,
                self.instance
                    .string_to_path("/user/hand/right")
                    .unwrap_or(xr::Path::NULL),
            )
            .map(|s| s.current_state)
            .unwrap_or(false);

        (left, right)
    }

    // ── Query ───────────────────────────────────────────────────────

    /// Whether the XR session is currently running (between begin and end).
    pub fn is_session_running(&self) -> bool {
        self.session_running
    }

    /// Whether the runtime supports hand tracking.
    pub fn has_hand_tracking(&self) -> bool {
        self.has_hand_tracking
    }

    /// Whether the runtime supports passthrough (AR).
    pub fn has_passthrough(&self) -> bool {
        self.has_passthrough
    }

    /// Check if a named extension is available on the instance.
    pub fn has_extension(&self, name: &str) -> bool {
        if let Ok(exts) = self.entry.enumerate_extensions() {
            // Check common extensions by name.
            match name {
                "XR_KHR_vulkan_enable2" => exts.khr_vulkan_enable2,
                "XR_EXT_hand_tracking" => exts.ext_hand_tracking,
                "XR_FB_passthrough" => exts.fb_passthrough,
                _ => false,
            }
        } else {
            false
        }
    }

    /// Get the wgpu device (for use by OffscreenRenderer integration).
    pub fn wgpu_device(&self) -> &wgpu::Device {
        &self.wgpu_device
    }

    /// Get the wgpu queue.
    pub fn wgpu_queue(&self) -> &wgpu::Queue {
        &self.wgpu_queue
    }
}

impl Drop for OpenXrBackend {
    fn drop(&mut self) {
        // Destroy swapchains before the session.
        self.panel_swapchains.clear();

        // The openxr crate handles cleanup of session, instance, etc.
        // via Drop impls. Vulkan cleanup is handled by ash Drop impls.
    }
}

// Re-export the Vello types used in rendering.
use vello::{AaSupport, RendererOptions};
