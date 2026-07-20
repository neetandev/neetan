use std::marker::PhantomData;

use sdl3_sys::{
    iostream::{
        SDL_CloseIO, SDL_GetIOProperties, SDL_GetIOSize, SDL_IOFromConstMem, SDL_IOFromDynamicMem,
        SDL_IOStream, SDL_PROP_IOSTREAM_DYNAMIC_MEMORY_POINTER,
    },
    pixels::SDL_PixelFormat,
    properties::SDL_GetPointerProperty,
    surface as ffi,
};

use crate::Error;

/// The byte-order R, G, B, A 32-bit format the wrapper normalizes to.
const RGBA8: SDL_PixelFormat = SDL_PixelFormat::RGBA32;

/// An owned SDL surface. Calls `SDL_DestroySurface` on drop.
///
/// The wrapper only ever holds surfaces whose pixels SDL allocated and owns, so
/// the pixel buffer lives exactly as long as the surface.
pub struct Surface {
    ptr: *mut ffi::SDL_Surface,
    _marker: PhantomData<*mut ()>,
}

impl Surface {
    /// Wraps a non-null surface pointer, converting a null pointer to an error.
    fn from_ptr(ptr: *mut ffi::SDL_Surface) -> Result<Self, Error> {
        if ptr.is_null() {
            return Err(crate::get_error());
        }
        Ok(Self {
            ptr,
            _marker: PhantomData,
        })
    }

    /// Creates an RGBA8 surface of `width` by `height` and copies the tightly
    /// packed `width * height * 4` source bytes into it.
    pub fn from_rgba8(width: u32, height: u32, pixels: &[u8]) -> Result<Self, Error> {
        let expected = width as usize * height as usize * 4;
        if pixels.len() < expected {
            return Err(Error(format!(
                "from_rgba8: need {expected} bytes for {width}x{height}, got {}",
                pixels.len()
            )));
        }
        // Safety: dimensions are non-negative; a null pointer is rejected below.
        let ptr = unsafe { ffi::SDL_CreateSurface(width as i32, height as i32, RGBA8) };
        let surface = Self::from_ptr(ptr)?;
        surface.copy_in_rgba8(width, height, &pixels[..expected])?;
        Ok(surface)
    }

    /// Copies tightly packed RGBA8 rows into an owned surface, honoring its pitch.
    fn copy_in_rgba8(&self, width: u32, height: u32, pixels: &[u8]) -> Result<(), Error> {
        let row_bytes = width as usize * 4;
        // Safety: the surface pointer is valid and owned by this struct.
        let locked = unsafe { lock_surface_if_needed(self.ptr) }?;
        // Safety: the pointer is valid; fields are read-only scalars.
        let (destination, pitch) =
            unsafe { ((*self.ptr).pixels.cast::<u8>(), (*self.ptr).pitch as usize) };
        for row in 0..height as usize {
            let source = &pixels[row * row_bytes..row * row_bytes + row_bytes];
            // Safety: destination has at least `pitch` bytes per row for `height`
            // rows, and `row_bytes <= pitch` for an RGBA8 surface.
            unsafe {
                let target = destination.add(row * pitch);
                std::ptr::copy_nonoverlapping(source.as_ptr(), target, row_bytes);
            }
        }
        if locked {
            // Safety: the surface was locked above.
            unsafe { ffi::SDL_UnlockSurface(self.ptr) };
        }
        Ok(())
    }

    /// Decodes a PNG image from an in-memory buffer and normalizes it to RGBA8.
    pub fn load_png(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.is_empty() {
            return Err(Error("load_png: empty input".to_owned()));
        }
        // Safety: the buffer outlives the call; SDL_LoadPNG_IO closes the stream.
        let stream = unsafe { SDL_IOFromConstMem(bytes.as_ptr().cast(), bytes.len()) };
        if stream.is_null() {
            return Err(crate::get_error());
        }
        // Safety: the stream is valid; closeio = true frees it here.
        let decoded = unsafe { ffi::SDL_LoadPNG_IO(stream, true) };
        let decoded = Self::from_ptr(decoded)?;
        decoded.into_rgba8()
    }

    /// Returns an RGBA8 copy of this surface, converting the format if needed.
    fn into_rgba8(self) -> Result<Self, Error> {
        // Safety: the pointer is valid; format is a read-only scalar.
        let already_rgba8 = unsafe { (*self.ptr).format } == RGBA8;
        if already_rgba8 {
            return Ok(self);
        }
        // Safety: the source pointer is valid; a null result is rejected below.
        let converted = unsafe { ffi::SDL_ConvertSurface(self.ptr, RGBA8) };
        Self::from_ptr(converted)
    }

    /// Encodes this surface to a PNG image held in an in-memory buffer.
    pub fn save_png(&self) -> Result<Vec<u8>, Error> {
        // Safety: creates a growable in-memory stream owned until SDL_CloseIO.
        let stream = unsafe { SDL_IOFromDynamicMem() };
        if stream.is_null() {
            return Err(crate::get_error());
        }
        // Safety: both pointers are valid; closeio = false keeps the stream open
        // so its buffer can be read out before it is closed below.
        let saved = unsafe { ffi::SDL_SavePNG_IO(self.ptr, stream, false) };
        if !saved {
            let error = crate::get_error();
            // Safety: the stream is valid and owned here.
            unsafe { SDL_CloseIO(stream) };
            return Err(error);
        }
        let bytes = unsafe { read_dynamic_stream(stream) };
        // Safety: the stream is valid; closing frees its dynamic buffer.
        unsafe { SDL_CloseIO(stream) };
        bytes
    }

    /// Returns the `(width, height)` of the surface in pixels.
    #[must_use]
    pub fn dimensions(&self) -> (u32, u32) {
        // Safety: the pointer is valid; w and h are read-only scalars.
        unsafe { ((*self.ptr).w as u32, (*self.ptr).h as u32) }
    }

    /// Returns tightly packed `width * height * 4` RGBA8 bytes, dropping any pitch
    /// padding. Converts the format first when the surface is not already RGBA8.
    pub fn to_rgba8(&self) -> Result<Vec<u8>, Error> {
        // Safety: the pointer is valid; format is a read-only scalar.
        let already_rgba8 = unsafe { (*self.ptr).format } == RGBA8;
        if already_rgba8 {
            return self.read_rgba8_rows();
        }
        // Safety: the source pointer is valid; a null result is rejected below.
        let converted = unsafe { ffi::SDL_ConvertSurface(self.ptr, RGBA8) };
        Self::from_ptr(converted)?.read_rgba8_rows()
    }

    /// Copies the RGBA8 rows out of an already-RGBA8 surface into a packed vector.
    fn read_rgba8_rows(&self) -> Result<Vec<u8>, Error> {
        let (width, height) = self.dimensions();
        let row_bytes = width as usize * 4;
        let mut out = vec![0u8; row_bytes * height as usize];
        // Safety: the surface pointer is valid and owned by this struct.
        let locked = unsafe { lock_surface_if_needed(self.ptr) }?;
        // Safety: the pointer is valid; fields are read-only scalars.
        let (source, pitch) =
            unsafe { ((*self.ptr).pixels.cast::<u8>(), (*self.ptr).pitch as usize) };
        for row in 0..height as usize {
            // Safety: source has at least `pitch` bytes per row for `height` rows,
            // and `row_bytes <= pitch` for an RGBA8 surface.
            unsafe {
                let start = source.add(row * pitch);
                std::ptr::copy_nonoverlapping(
                    start,
                    out.as_mut_ptr().add(row * row_bytes),
                    row_bytes,
                );
            }
        }
        if locked {
            // Safety: the surface was locked above.
            unsafe { ffi::SDL_UnlockSurface(self.ptr) };
        }
        Ok(out)
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // Safety: the pointer is valid and owned by this struct.
        unsafe { ffi::SDL_DestroySurface(self.ptr) }
    }
}

/// Locks a surface only when it declares that pixel access needs a lock.
///
/// Returns whether a lock was taken so the caller unlocks symmetrically.
///
/// # Safety
///
/// `surface` must be a valid, non-null surface pointer.
unsafe fn lock_surface_if_needed(surface: *mut ffi::SDL_Surface) -> Result<bool, Error> {
    // Safety: the caller guarantees the pointer is valid; flags is read-only.
    let needs_lock = unsafe { (*surface).flags.0 & ffi::SDL_SURFACE_LOCK_NEEDED.0 }
        == ffi::SDL_SURFACE_LOCK_NEEDED.0;
    if !needs_lock {
        return Ok(false);
    }
    // Safety: the caller guarantees the pointer is valid.
    if unsafe { ffi::SDL_LockSurface(surface) } {
        Ok(true)
    } else {
        Err(crate::get_error())
    }
}

/// Copies the bytes written to a dynamic-memory IO stream into a vector.
///
/// # Safety
///
/// `stream` must be a valid dynamic-memory stream created by
/// `SDL_IOFromDynamicMem`.
unsafe fn read_dynamic_stream(stream: *mut SDL_IOStream) -> Result<Vec<u8>, Error> {
    // Safety: the caller guarantees the stream is valid.
    let size = unsafe { SDL_GetIOSize(stream) };
    if size < 0 {
        return Err(crate::get_error());
    }
    let size = size as usize;
    // Safety: the caller guarantees the stream is valid.
    let properties = unsafe { SDL_GetIOProperties(stream) };
    if properties.0 == 0 {
        return Err(crate::get_error());
    }
    // Safety: the property id is valid; a null default surfaces as an error.
    let pointer = unsafe {
        SDL_GetPointerProperty(
            properties,
            SDL_PROP_IOSTREAM_DYNAMIC_MEMORY_POINTER,
            std::ptr::null_mut(),
        )
    };
    if pointer.is_null() {
        return Err(Error(
            "save_png: dynamic memory pointer was null".to_owned(),
        ));
    }
    let mut out = vec![0u8; size];
    // Safety: the pointer addresses at least `size` bytes of the stream buffer.
    unsafe {
        std::ptr::copy_nonoverlapping(pointer.cast::<u8>(), out.as_mut_ptr(), size);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::Surface;

    #[test]
    fn png_round_trips_rgba8_pixels() {
        let width = 3;
        let height = 2;
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                pixels.push((x * 40) as u8);
                pixels.push((y * 80) as u8);
                pixels.push(((x + y) * 30) as u8);
                pixels.push(0xFF);
            }
        }

        let surface = Surface::from_rgba8(width, height, &pixels).expect("create surface");
        assert_eq!(surface.dimensions(), (width, height));
        assert_eq!(surface.to_rgba8().expect("read pixels"), pixels);

        let encoded = surface.save_png().expect("encode png");
        let decoded = Surface::load_png(&encoded).expect("decode png");
        assert_eq!(decoded.dimensions(), (width, height));
        assert_eq!(decoded.to_rgba8().expect("read decoded"), pixels);
    }
}
