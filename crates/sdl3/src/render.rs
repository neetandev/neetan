use std::marker::PhantomData;

use sdl3_sys::{
    blendmode::{
        SDL_BLENDMODE_BLEND, SDL_BLENDMODE_BLEND_PREMULTIPLIED, SDL_BLENDMODE_NONE, SDL_BlendMode,
    },
    pixels::SDL_PixelFormat,
    properties::SDL_GetNumberProperty,
    rect::{SDL_FRect, SDL_Rect},
    render as ffi,
    surface::{SDL_SCALEMODE_LINEAR, SDL_SCALEMODE_NEAREST, SDL_SCALEMODE_PIXELART, SDL_ScaleMode},
    video::SDL_Window,
};

use crate::{Error, video::Window};

/// Texture scale mode used during sampling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScaleMode {
    /// Nearest-neighbor sampling.
    Nearest,
    /// Bilinear sampling.
    Linear,
    /// Pixelart sampling.
    Pixelart,
}

impl ScaleMode {
    fn to_ffi(self) -> SDL_ScaleMode {
        match self {
            Self::Nearest => SDL_SCALEMODE_NEAREST,
            Self::Linear => SDL_SCALEMODE_LINEAR,
            Self::Pixelart => SDL_SCALEMODE_PIXELART,
        }
    }
}

/// Blend mode used when a texture is drawn onto the rendering target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlendMode {
    /// No blending: the texture replaces the target.
    None,
    /// Alpha blending of straight alpha color.
    Blend,
    /// Alpha blending of premultiplied alpha color.
    BlendPremultiplied,
}

impl BlendMode {
    /// Returns the matching SDL blend mode.
    fn to_ffi(self) -> SDL_BlendMode {
        match self {
            Self::None => SDL_BLENDMODE_NONE,
            Self::Blend => SDL_BLENDMODE_BLEND,
            Self::BlendPremultiplied => SDL_BLENDMODE_BLEND_PREMULTIPLIED,
        }
    }
}

/// Pixel formats supported by the safe wrapper.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat {
    /// 32-bit byte-order R, G, B, A regardless of host endianness.
    Rgba32,
}

impl PixelFormat {
    fn to_ffi(self) -> SDL_PixelFormat {
        match self {
            Self::Rgba32 => SDL_PixelFormat::RGBA32,
        }
    }
}

/// Integer rectangle.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rect {
    /// Top-left x in pixels.
    pub x: i32,
    /// Top-left y in pixels.
    pub y: i32,
    /// Width in pixels.
    pub w: i32,
    /// Height in pixels.
    pub h: i32,
}

impl Rect {
    fn to_sdl_rect(self) -> SDL_Rect {
        SDL_Rect {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
        }
    }

    fn to_sdl_frect(self) -> SDL_FRect {
        SDL_FRect {
            x: self.x as f32,
            y: self.y as f32,
            w: self.w as f32,
            h: self.h as f32,
        }
    }
}

/// SDL 2D rendering context for a window. Calls `SDL_DestroyRenderer` on drop.
pub struct Renderer {
    ptr: *mut ffi::SDL_Renderer,
    _marker: PhantomData<*mut ()>,
}

impl Renderer {
    /// Creates a renderer for the given window, letting SDL pick the best driver.
    pub fn new(window: &Window) -> Result<Self, Error> {
        Self::new_for_raw_window(window.raw())
    }

    pub(crate) fn new_for_raw_window(window: *mut SDL_Window) -> Result<Self, Error> {
        // Safety: window pointer is valid for its lifetime; name is NULL to let SDL pick.
        let ptr = unsafe { ffi::SDL_CreateRenderer(window, std::ptr::null()) };
        if ptr.is_null() {
            return Err(crate::get_error());
        }
        Ok(Self {
            ptr,
            _marker: PhantomData,
        })
    }

    /// Enables or disables vsync. Uses the standard adaptive value when enabled.
    pub fn set_vsync(&self, enabled: bool) -> Result<(), Error> {
        let value = if enabled {
            1
        } else {
            ffi::SDL_RENDERER_VSYNC_DISABLED
        };
        // Safety: renderer pointer is valid.
        let ok = unsafe { ffi::SDL_SetRenderVSync(self.ptr, value) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }

    /// Returns the current output size of the renderer in pixels.
    pub fn output_size(&self) -> Result<(u32, u32), Error> {
        let mut w: i32 = 0;
        let mut h: i32 = 0;
        // Safety: renderer pointer is valid; w and h are valid pointers.
        let ok = unsafe { ffi::SDL_GetRenderOutputSize(self.ptr, &raw mut w, &raw mut h) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok((w as u32, h as u32))
    }

    /// Returns the largest texture width and height the renderer supports.
    /// `None` when the renderer does not report a limit.
    pub fn max_texture_size(&self) -> Option<u32> {
        // Safety: renderer pointer is valid.
        let properties = unsafe { ffi::SDL_GetRendererProperties(self.ptr) };
        if properties.0 == 0 {
            return None;
        }
        // Safety: the property id is valid; the name is a static C string.
        let size = unsafe {
            SDL_GetNumberProperty(
                properties,
                ffi::SDL_PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER,
                0,
            )
        };
        u32::try_from(size).ok().filter(|&size| size > 0)
    }

    /// Creates a streaming texture suitable for frequent CPU updates.
    ///
    /// The caller must ensure the [`Texture`] is dropped before this [`Renderer`].
    pub fn create_streaming_texture(
        &self,
        format: PixelFormat,
        width: u32,
        height: u32,
    ) -> Result<Texture, Error> {
        // Safety: renderer pointer is valid; dimensions are non-negative.
        let ptr = unsafe {
            ffi::SDL_CreateTexture(
                self.ptr,
                format.to_ffi(),
                ffi::SDL_TEXTUREACCESS_STREAMING,
                width as i32,
                height as i32,
            )
        };
        if ptr.is_null() {
            return Err(crate::get_error());
        }
        Ok(Texture {
            ptr,
            _marker: PhantomData,
        })
    }

    /// Sets the current draw color and clears the entire rendering target.
    pub fn clear_with_color(&self, r: u8, g: u8, b: u8, a: u8) -> Result<(), Error> {
        // Safety: renderer pointer is valid.
        let ok = unsafe { ffi::SDL_SetRenderDrawColor(self.ptr, r, g, b, a) };
        if !ok {
            return Err(crate::get_error());
        }
        // Safety: renderer pointer is valid.
        let ok = unsafe { ffi::SDL_RenderClear(self.ptr) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }

    /// Copies a portion of a texture to the rendering target.
    pub fn render_texture(
        &self,
        texture: &Texture,
        src: Option<Rect>,
        dst: Option<Rect>,
    ) -> Result<(), Error> {
        let src_frect = src.map(|r| r.to_sdl_frect());
        let dst_frect = dst.map(|r| r.to_sdl_frect());
        let src_ptr = src_frect
            .as_ref()
            .map_or(std::ptr::null(), |r| r as *const SDL_FRect);
        let dst_ptr = dst_frect
            .as_ref()
            .map_or(std::ptr::null(), |r| r as *const SDL_FRect);
        // Safety: renderer and texture pointers are valid; rect pointers are NULL or local refs.
        let ok = unsafe { ffi::SDL_RenderTexture(self.ptr, texture.ptr, src_ptr, dst_ptr) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }

    /// Presents the rendered frame to the window.
    pub fn present(&self) -> Result<(), Error> {
        // Safety: renderer pointer is valid.
        let ok = unsafe { ffi::SDL_RenderPresent(self.ptr) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // Safety: renderer pointer is valid and owned by this struct.
        unsafe { ffi::SDL_DestroyRenderer(self.ptr) }
    }
}

/// A renderer-owned texture. Calls `SDL_DestroyTexture` on drop.
///
/// The owning [`Renderer`] must outlive this texture.
pub struct Texture {
    ptr: *mut ffi::SDL_Texture,
    _marker: PhantomData<*mut ()>,
}

impl Texture {
    /// Uploads a contiguous block of pixels into the texture.
    pub fn update(&mut self, rect: Option<Rect>, pixels: &[u8], pitch: u32) -> Result<(), Error> {
        let sdl_rect = rect.map(|r| r.to_sdl_rect());
        let rect_ptr = sdl_rect
            .as_ref()
            .map_or(std::ptr::null(), |r| r as *const SDL_Rect);
        // Safety: texture pointer is valid; pixels slice covers at least pitch * height bytes.
        let ok = unsafe {
            ffi::SDL_UpdateTexture(
                self.ptr,
                rect_ptr,
                pixels.as_ptr() as *const std::ffi::c_void,
                pitch as i32,
            )
        };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }

    /// Sets the scale mode used when sampling this texture.
    pub fn set_scale_mode(&self, mode: ScaleMode) -> Result<(), Error> {
        // Safety: texture pointer is valid.
        let ok = unsafe { ffi::SDL_SetTextureScaleMode(self.ptr, mode.to_ffi()) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }

    /// Sets the blend mode used when drawing this texture.
    pub fn set_blend_mode(&self, mode: BlendMode) -> Result<(), Error> {
        // Safety: texture pointer is valid.
        let ok = unsafe { ffi::SDL_SetTextureBlendMode(self.ptr, mode.to_ffi()) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        // Safety: texture pointer is valid and owned by this struct.
        unsafe { ffi::SDL_DestroyTexture(self.ptr) }
    }
}
