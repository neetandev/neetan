use std::{ffi::CString, marker::PhantomData};

use sdl3_sys::{init, keyboard as keyboard_ffi, mouse as mouse_ffi, rect::SDL_Rect, video as ffi};

use crate::{Error, render::Renderer};

/// Manages the SDL3 video subsystem. Calls `SDL_QuitSubSystem(VIDEO)` on drop.
pub struct VideoSubsystem {
    _marker: PhantomData<*mut ()>,
}

impl VideoSubsystem {
    pub(crate) fn new() -> Result<Self, Error> {
        // Safety: Called from the main thread after SDL_Init.
        let ok = unsafe { init::SDL_InitSubSystem(init::SDL_INIT_VIDEO) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(Self {
            _marker: PhantomData,
        })
    }

    /// Returns the usable size of the primary display in screen coordinates:
    /// the display without the task bars and docks of the desktop. `None`
    /// when SDL does not know it.
    pub fn primary_display_usable_size(&self) -> Option<(u32, u32)> {
        // Safety: The video subsystem is initialized.
        let display = unsafe { ffi::SDL_GetPrimaryDisplay() };
        display_usable_size(display)
    }

    /// Creates a [`WindowBuilder`] with the given title and dimensions.
    pub fn window(&self, title: &str, width: u32, height: u32) -> WindowBuilder {
        WindowBuilder {
            title: CString::new(title).unwrap_or_default(),
            width: width as i32,
            height: height as i32,
            flags: ffi::SDL_WindowFlags(0),
            centered: false,
        }
    }
}

impl Drop for VideoSubsystem {
    fn drop(&mut self) {
        // Safety: Matches the SDL_InitSubSystem call in new().
        unsafe { init::SDL_QuitSubSystem(init::SDL_INIT_VIDEO) }
    }
}

/// Returns the usable size of `display` in screen coordinates, or `None`
/// when SDL does not know it.
fn display_usable_size(display: ffi::SDL_DisplayID) -> Option<(u32, u32)> {
    if display.0 == 0 {
        return None;
    }
    let mut bounds = SDL_Rect {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    };
    // Safety: The display id came from SDL and `bounds` is a valid rectangle.
    let ok = unsafe { ffi::SDL_GetDisplayUsableBounds(display, &mut bounds) };
    if !ok || bounds.w <= 0 || bounds.h <= 0 {
        return None;
    }
    Some((bounds.w as u32, bounds.h as u32))
}

/// Builder for creating an SDL3 window.
pub struct WindowBuilder {
    title: CString,
    width: i32,
    height: i32,
    flags: ffi::SDL_WindowFlags,
    centered: bool,
}

impl WindowBuilder {
    /// Enables high-DPI / high pixel density rendering.
    pub fn high_pixel_density(mut self) -> Self {
        self.flags.0 |= ffi::SDL_WINDOW_HIGH_PIXEL_DENSITY.0;
        self
    }

    /// Makes the window resizable.
    pub fn resizable(mut self) -> Self {
        self.flags.0 |= ffi::SDL_WINDOW_RESIZABLE.0;
        self
    }

    /// Centers the window on the display.
    pub fn position_centered(mut self) -> Self {
        self.centered = true;
        self
    }

    /// Creates the window initially hidden.
    pub fn hidden(mut self) -> Self {
        self.flags.0 |= ffi::SDL_WINDOW_HIDDEN.0;
        self
    }

    /// Creates the window in fullscreen (borderless desktop) mode.
    pub fn fullscreen(mut self) -> Self {
        self.flags.0 |= ffi::SDL_WINDOW_FULLSCREEN.0;
        self
    }

    /// Builds and returns the window.
    pub fn build(self) -> Result<Window, Error> {
        // Safety: We pass valid title, dimensions and flags.
        let ptr = unsafe {
            ffi::SDL_CreateWindow(self.title.as_ptr(), self.width, self.height, self.flags)
        };
        if ptr.is_null() {
            return Err(crate::get_error());
        }

        if self.centered {
            // Safety: Window pointer is valid; we just created it.
            unsafe {
                ffi::SDL_SetWindowPosition(
                    ptr,
                    ffi::SDL_WINDOWPOS_CENTERED,
                    ffi::SDL_WINDOWPOS_CENTERED,
                );
            }
        }

        Ok(Window {
            ptr,
            _marker: PhantomData,
        })
    }
}

/// An SDL3 window. Destroyed on drop.
pub struct Window {
    ptr: *mut ffi::SDL_Window,
    _marker: PhantomData<*mut ()>,
}

impl Window {
    /// Returns the raw SDL3 window pointer.
    pub fn raw(&self) -> *mut ffi::SDL_Window {
        self.ptr
    }

    /// Shows a previously hidden window.
    pub fn show(&mut self) {
        // Safety: Window pointer is valid.
        unsafe {
            ffi::SDL_ShowWindow(self.ptr);
        }
    }

    /// Returns the window's logical size in screen coordinates.
    pub fn size(&self) -> (u32, u32) {
        let mut w: i32 = 0;
        let mut h: i32 = 0;
        // Safety: Window pointer is valid; w and h are valid pointers.
        unsafe {
            ffi::SDL_GetWindowSize(self.ptr, &raw mut w, &raw mut h);
        }
        (w as u32, h as u32)
    }

    /// Returns the window's size in physical pixels.
    pub fn size_in_pixels(&self) -> (u32, u32) {
        let mut w: i32 = 0;
        let mut h: i32 = 0;
        // Safety: Window pointer is valid; w and h are valid pointers.
        unsafe {
            ffi::SDL_GetWindowSizeInPixels(self.ptr, &raw mut w, &raw mut h);
        }
        (w as u32, h as u32)
    }

    /// Returns the display scale factor for this window.
    pub fn display_scale(&self) -> f32 {
        // Safety: Window pointer is valid.
        unsafe { ffi::SDL_GetWindowDisplayScale(self.ptr) }
    }

    /// Returns the physical pixels for each window coordinate.
    pub fn pixel_density(&self) -> f32 {
        // Safety: Window pointer is valid.
        unsafe { ffi::SDL_GetWindowPixelDensity(self.ptr) }
    }

    /// Starts the text input and input method events for this window.
    pub fn start_text_input(&self) -> Result<(), Error> {
        // Safety: Window pointer is valid.
        let ok = unsafe { keyboard_ffi::SDL_StartTextInput(self.ptr) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }

    /// Stops the text input and input method events for this window.
    pub fn stop_text_input(&self) -> Result<(), Error> {
        // Safety: Window pointer is valid.
        let ok = unsafe { keyboard_ffi::SDL_StopTextInput(self.ptr) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }

    /// Sets the area of the text being edited in window coordinates, for the
    /// candidate window of an input method. `cursor` is the offset of the
    /// text cursor from the left edge of the area.
    pub fn set_text_input_area(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        cursor: i32,
    ) -> Result<(), Error> {
        let rect = SDL_Rect {
            x,
            y,
            w: width,
            h: height,
        };
        // Safety: Window pointer is valid; rect is a valid pointer for the call.
        let ok = unsafe { keyboard_ffi::SDL_SetTextInputArea(self.ptr, &raw const rect, cursor) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }

    /// Locks the window's aspect ratio to the given value for both min and max.
    pub fn set_aspect_ratio(&self, ratio: f32) -> Result<(), Error> {
        // Safety: Window pointer is valid.
        let ok = unsafe { ffi::SDL_SetWindowAspectRatio(self.ptr, ratio, ratio) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }

    /// Returns the usable size of the display of the window in screen
    /// coordinates, or `None` when SDL does not know it.
    pub fn display_usable_size(&self) -> Option<(u32, u32)> {
        // Safety: Window pointer is valid.
        let display = unsafe { ffi::SDL_GetDisplayForWindow(self.ptr) };
        display_usable_size(display)
    }

    /// Waits until the window system applied the pending size, position
    /// and fullscreen changes of the window. Returns `false` when the wait
    /// timed out first.
    pub fn sync(&self) -> bool {
        // Safety: Window pointer is valid.
        unsafe { ffi::SDL_SyncWindow(self.ptr) }
    }

    /// Removes the aspect ratio lock of the window.
    pub fn clear_aspect_ratio(&self) -> Result<(), Error> {
        // Safety: Window pointer is valid. 0.0 means no limit for both bounds.
        let ok = unsafe { ffi::SDL_SetWindowAspectRatio(self.ptr, 0.0, 0.0) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }

    /// Requests the size of the client area in screen coordinates. The
    /// window system may deny the request or apply a different size.
    pub fn set_size(&self, width: u32, height: u32) -> Result<(), Error> {
        let width = i32::try_from(width).unwrap_or(i32::MAX);
        let height = i32::try_from(height).unwrap_or(i32::MAX);
        // Safety: Window pointer is valid.
        let ok = unsafe { ffi::SDL_SetWindowSize(self.ptr, width, height) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }

    /// Requests the position that centers the window on its display.
    pub fn center(&self) -> Result<(), Error> {
        // Safety: Window pointer is valid.
        let ok = unsafe {
            ffi::SDL_SetWindowPosition(
                self.ptr,
                ffi::SDL_WINDOWPOS_CENTERED,
                ffi::SDL_WINDOWPOS_CENTERED,
            )
        };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }

    /// Sets the minimum size of the client area in screen coordinates. A zero
    /// dimension means no limit.
    pub fn set_minimum_size(&self, width: u32, height: u32) -> Result<(), Error> {
        let width = i32::try_from(width).unwrap_or(i32::MAX);
        let height = i32::try_from(height).unwrap_or(i32::MAX);
        // Safety: Window pointer is valid.
        let ok = unsafe { ffi::SDL_SetWindowMinimumSize(self.ptr, width, height) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }

    /// Enables or disables fullscreen (borderless desktop) mode on this window.
    pub fn set_fullscreen(&self, fullscreen: bool) -> Result<(), Error> {
        let ok = unsafe { ffi::SDL_SetWindowFullscreen(self.ptr, fullscreen) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }

    /// Sets the window title.
    pub fn set_title(&self, title: &str) {
        let c_title = CString::new(title).unwrap_or_default();
        // Safety: Window pointer is valid; c_title is a valid C string.
        unsafe {
            ffi::SDL_SetWindowTitle(self.ptr, c_title.as_ptr());
        }
    }

    /// Enables or disables relative mouse mode on this window.
    pub fn set_relative_mouse_mode(&self, enabled: bool) -> Result<(), Error> {
        // Safety: Window pointer is valid.
        let ok = unsafe { mouse_ffi::SDL_SetWindowRelativeMouseMode(self.ptr, enabled) };
        if !ok {
            return Err(crate::get_error());
        }
        Ok(())
    }

    /// Creates a 2D rendering context for this window. SDL picks the best driver.
    pub fn create_renderer(&self) -> Result<Renderer, Error> {
        Renderer::new_for_raw_window(self.ptr)
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // Safety: Window pointer is valid and we own it.
        unsafe { ffi::SDL_DestroyWindow(self.ptr) }
    }
}
