use std::ffi::CStr;

use common::{Context, info, warn};
#[cfg(not(debug_assertions))]
use sdl3::gpu::SDL_PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN;
use sdl3::{
    gpu::{
        GpuDevice, GpuSampler, GpuTexture, GpuTransferBuffer, SDL_GPU_FILTER_LINEAR,
        SDL_GPU_FILTER_NEAREST, SDL_GPU_PRESENTMODE_IMMEDIATE, SDL_GPU_PRESENTMODE_VSYNC,
        SDL_GPU_SAMPLECOUNT_1, SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
        SDL_GPU_SAMPLERMIPMAPMODE_NEAREST, SDL_GPU_SWAPCHAINCOMPOSITION_SDR,
        SDL_GPU_SWAPCHAINCOMPOSITION_SDR_LINEAR, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB,
        SDL_GPU_TEXTURETYPE_2D, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET, SDL_GPU_TEXTUREUSAGE_SAMPLER,
        SDL_GPU_TRANSFERBUFFERUSAGE_UPLOAD, SDL_GPUFilter, SDL_GPUPresentMode,
        SDL_GPUSamplerCreateInfo, SDL_GPUSwapchainComposition, SDL_GPUTextureCreateInfo,
        SDL_GPUTextureFormat, SDL_GPUTransferBufferCreateInfo,
        SDL_PROP_GPU_DEVICE_CREATE_D3D12_ALLOW_FEWER_RESOURCE_SLOTS_BOOLEAN,
        SDL_PROP_GPU_DEVICE_CREATE_FEATURE_ANISOTROPY_BOOLEAN,
        SDL_PROP_GPU_DEVICE_CREATE_FEATURE_CLIP_DISTANCE_BOOLEAN,
        SDL_PROP_GPU_DEVICE_CREATE_FEATURE_DEPTH_CLAMPING_BOOLEAN,
        SDL_PROP_GPU_DEVICE_CREATE_FEATURE_INDIRECT_DRAW_FIRST_INSTANCE_BOOLEAN,
    },
    properties::Properties,
    video::Window,
};

use crate::{Result, native_target_bytes, native_target_size};

/// Resources owned by an active `SdlGpuBackend` session, created in `on_resume`.
pub(crate) struct DeviceResources {
    pub(crate) swapchain_composition: SDL_GPUSwapchainComposition,
    pub(crate) swapchain_format: SDL_GPUTextureFormat,
    pub(crate) nearest_sampler: GpuSampler,
    pub(crate) linear_sampler: GpuSampler,
    pub(crate) native_target: GpuTexture,
    pub(crate) transfer_buffer: GpuTransferBuffer,
    pub(crate) device: GpuDevice,
}

/// Builds a [`DeviceResources`] bundle: device + samplers + native target + transfer buffer.
pub(crate) fn create(
    window: &Window,
    vsync_enabled: bool,
    large_native_target: bool,
) -> Result<DeviceResources> {
    let device = create_device().context("SDL_CreateGPUDeviceWithProperties failed")?;

    device
        .claim_window(window)
        .context("SDL_ClaimWindowForGPUDevice failed")?;

    let requested_present_mode = match vsync_enabled {
        true => SDL_GPU_PRESENTMODE_VSYNC,
        false => SDL_GPU_PRESENTMODE_IMMEDIATE,
    };

    // SDL guarantees VSYNC + SDR; everything else must be queried first.
    let present_mode = pick_present_mode(&device, window, requested_present_mode);
    let swapchain_composition = pick_composition(&device, window);

    device
        .set_swapchain_parameters(window, swapchain_composition, present_mode)
        .context("SDL_SetGPUSwapchainParameters failed")?;

    if let Err(error) = device.set_allowed_frames_in_flight(2) {
        warn!("SDL_SetGPUAllowedFramesInFlight(2) failed: {error}; keeping SDL default");
    }

    let swapchain_format = device.swapchain_texture_format(window);

    let nearest_sampler = create_sampler(&device, SDL_GPU_FILTER_NEAREST)
        .context("Failed to create nearest sampler")?;
    let linear_sampler = create_sampler(&device, SDL_GPU_FILTER_LINEAR)
        .context("Failed to create linear sampler")?;

    let native_target = create_native_target(&device, large_native_target)
        .context("Failed to create native target")?;
    let transfer_buffer = create_transfer_buffer(&device, large_native_target)
        .context("Failed to create framebuffer transfer buffer")?;

    Ok(DeviceResources {
        swapchain_format,
        swapchain_composition,
        device,
        nearest_sampler,
        linear_sampler,
        native_target,
        transfer_buffer,
    })
}

/// Picks the best supported present mode, falling back to VSYNC (always supported).
fn pick_present_mode(
    device: &GpuDevice,
    window: &Window,
    requested: SDL_GPUPresentMode,
) -> SDL_GPUPresentMode {
    if requested == SDL_GPU_PRESENTMODE_VSYNC {
        return SDL_GPU_PRESENTMODE_VSYNC;
    }
    if device.window_supports_present_mode(window, requested) {
        return requested;
    }
    warn!("Requested present mode not supported; falling back to VSYNC");
    SDL_GPU_PRESENTMODE_VSYNC
}

/// Picks the best supported swapchain composition. Prefers `SDR_LINEAR`
/// (hardware-managed sRGB encoding) and falls back to `SDR` (always supported).
fn pick_composition(device: &GpuDevice, window: &Window) -> SDL_GPUSwapchainComposition {
    if device.window_supports_swapchain_composition(window, SDL_GPU_SWAPCHAINCOMPOSITION_SDR_LINEAR)
    {
        info!("Using sRGB swapchain");
        return SDL_GPU_SWAPCHAINCOMPOSITION_SDR_LINEAR;
    }
    warn!("sRGB swapchain not supported; falling back to shader based sRGB handling");
    SDL_GPU_SWAPCHAINCOMPOSITION_SDR
}

fn create_device() -> Result<GpuDevice> {
    let properties = Properties::new().context("SDL_CreateProperties failed")?;

    #[cfg(not(debug_assertions))]
    set_bool_property(
        &properties,
        SDL_PROP_GPU_DEVICE_CREATE_DEBUGMODE_BOOLEAN,
        false,
    )?;

    // Disable features that we don't use. This increased device compatibility
    // especially with mobile chips and very old graphic cards.
    set_bool_property(
        &properties,
        SDL_PROP_GPU_DEVICE_CREATE_FEATURE_CLIP_DISTANCE_BOOLEAN,
        false,
    )?;
    set_bool_property(
        &properties,
        SDL_PROP_GPU_DEVICE_CREATE_FEATURE_DEPTH_CLAMPING_BOOLEAN,
        false,
    )?;
    set_bool_property(
        &properties,
        SDL_PROP_GPU_DEVICE_CREATE_FEATURE_INDIRECT_DRAW_FIRST_INSTANCE_BOOLEAN,
        false,
    )?;
    set_bool_property(
        &properties,
        SDL_PROP_GPU_DEVICE_CREATE_FEATURE_ANISOTROPY_BOOLEAN,
        false,
    )?;
    set_bool_property(
        &properties,
        SDL_PROP_GPU_DEVICE_CREATE_D3D12_ALLOW_FEWER_RESOURCE_SLOTS_BOOLEAN,
        true,
    )?;

    // Shader-format hints matching what build.rs emitted for this target.
    #[cfg(target_os = "windows")]
    {
        set_bool_property(
            &properties,
            sdl3::gpu::SDL_PROP_GPU_DEVICE_CREATE_SHADERS_DXIL_BOOLEAN,
            true,
        )?;
        set_bool_property(
            &properties,
            sdl3::gpu::SDL_PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN,
            true,
        )?;
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        set_bool_property(
            &properties,
            sdl3::gpu::SDL_PROP_GPU_DEVICE_CREATE_SHADERS_METALLIB_BOOLEAN,
            true,
        )?;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "ios")))]
    {
        set_bool_property(
            &properties,
            sdl3::gpu::SDL_PROP_GPU_DEVICE_CREATE_SHADERS_SPIRV_BOOLEAN,
            true,
        )?;
    }

    GpuDevice::with_properties(&properties)
        .context("SDL_CreateGPUDeviceWithProperties failed")
        .map_err(Into::into)
}

fn set_bool_property(
    properties: &Properties,
    name: *const std::os::raw::c_char,
    value: bool,
) -> Result<()> {
    // Safety: the SDL_PROP_* constants are pointers to static C strings owned by SDL.
    let name = unsafe { CStr::from_ptr(name) };
    properties
        .set_bool_cstr(name, value)
        .context("SDL_SetBooleanProperty failed")
        .map_err(Into::into)
}

fn create_sampler(device: &GpuDevice, filter: SDL_GPUFilter) -> Result<GpuSampler> {
    let info = SDL_GPUSamplerCreateInfo {
        min_filter: filter,
        mag_filter: filter,
        mipmap_mode: SDL_GPU_SAMPLERMIPMAPMODE_NEAREST,
        address_mode_u: SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
        address_mode_v: SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
        address_mode_w: SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
        enable_anisotropy: false,
        enable_compare: false,
        ..Default::default()
    };
    device
        .create_sampler(&info)
        .context("SDL_CreateGPUSampler failed")
        .map_err(Into::into)
}

fn create_native_target(device: &GpuDevice, large_native_target: bool) -> Result<GpuTexture> {
    let (width, height) = native_target_size(large_native_target);
    let info = SDL_GPUTextureCreateInfo {
        r#type: SDL_GPU_TEXTURETYPE_2D,
        format: SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB,
        usage: sdl3::gpu::SDL_GPUTextureUsageFlags(
            SDL_GPU_TEXTUREUSAGE_SAMPLER.0 | SDL_GPU_TEXTUREUSAGE_COLOR_TARGET.0,
        ),
        width,
        height,
        layer_count_or_depth: 1,
        num_levels: 1,
        sample_count: SDL_GPU_SAMPLECOUNT_1,
        ..Default::default()
    };
    device
        .create_texture(&info)
        .context("SDL_CreateGPUTexture failed")
        .map_err(Into::into)
}

fn create_transfer_buffer(
    device: &GpuDevice,
    large_native_target: bool,
) -> Result<GpuTransferBuffer> {
    let info = SDL_GPUTransferBufferCreateInfo {
        usage: SDL_GPU_TRANSFERBUFFERUSAGE_UPLOAD,
        size: native_target_bytes(large_native_target) as u32,
        ..Default::default()
    };
    device
        .create_transfer_buffer(&info)
        .context("SDL_CreateGPUTransferBuffer failed")
        .map_err(Into::into)
}
