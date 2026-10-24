//! The user interface overlay of the modern backend.
//!
//! The overlay draws a [`gui::Scene`] with one instanced draw call of the
//! user interface pipeline. The instances and the three Slug arrays live in
//! storage buffers. Every frame uploads the instances. The Slug arrays upload
//! only when their content changed, and only the appended entries when the
//! same store grew.

use common::{Context, OptionContext};
use gui::{BLUE_NOISE, BLUE_NOISE_SIZE, Instance, SlugData};
use sdl3::gpu::{
    BufferRegion, GpuBuffer, GpuCopyPass, GpuDevice, GpuGraphicsPipeline, GpuRenderPass,
    GpuSampler, GpuTexture, GpuTransferBuffer, SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ,
    SDL_GPU_FILTER_NEAREST, SDL_GPU_SAMPLECOUNT_1, SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
    SDL_GPU_SAMPLERMIPMAPMODE_NEAREST, SDL_GPU_TEXTUREFORMAT_R8_UNORM, SDL_GPU_TEXTURETYPE_2D,
    SDL_GPU_TEXTUREUSAGE_SAMPLER, SDL_GPU_TRANSFERBUFFERUSAGE_UPLOAD, SDL_GPUBufferCreateInfo,
    SDL_GPUSamplerCreateInfo, SDL_GPUTextureCreateInfo, SDL_GPUTextureFormat,
    SDL_GPUTransferBufferCreateInfo, TextureRegion, TextureSamplerBinding, TextureTransferInfo,
    TransferBufferLocation,
};

use crate::{
    Overlay, Result,
    ui_pipeline::{self, UiUniforms, VERTICES_PER_INSTANCE},
};

/// The size in bytes of a new storage buffer.
const MINIMUM_BUFFER_BYTES: u32 = 4096;

/// The alignment in bytes of every upload in the transfer buffer.
const TRANSFER_ALIGNMENT: usize = 16;

/// The number of Slug arrays: curves, bands and glyphs.
const SLUG_ARRAYS: usize = 3;

/// The upload of one Slug array in one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ArrayUpload {
    /// The buffer holds the array already.
    Nothing,
    /// The entries from `start` to the end of the array.
    Tail {
        /// The index of the first entry to upload.
        start: usize,
    },
    /// The whole array.
    Whole,
}

/// The state of the Slug data the buffers hold.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SlugUpload {
    /// The [`SlugData::identity`] of the store.
    identity: u64,
    /// The [`SlugData::generation`] of the content.
    generation: u64,
    /// The entry counts of the curves, the bands and the glyphs.
    lengths: [usize; SLUG_ARRAYS],
}

impl SlugUpload {
    /// The state of `slug`.
    fn of(slug: &SlugData) -> Self {
        Self {
            identity: slug.identity(),
            generation: slug.generation(),
            lengths: [slug.curves().len(), slug.bands().len(), slug.glyphs().len()],
        }
    }
}

/// Chooses the upload of every Slug array. `last` is the state of the last
/// upload and `grown` tells which buffers were replaced by a larger one.
///
/// An unchanged generation needs no upload. The arrays only grow while the
/// identity stays the same, so a grown store uploads only its new tail. A new
/// store and a replaced buffer upload the whole array.
fn choose_slug_uploads(
    last: Option<SlugUpload>,
    current: SlugUpload,
    grown: [bool; SLUG_ARRAYS],
) -> [ArrayUpload; SLUG_ARRAYS] {
    let appended = last.filter(|last| {
        last.identity == current.identity
            && last
                .lengths
                .iter()
                .zip(&current.lengths)
                .all(|(last_length, length)| last_length <= length)
    });
    std::array::from_fn(|array| {
        if grown[array] {
            return ArrayUpload::Whole;
        }
        match appended {
            Some(last) if last.generation == current.generation => ArrayUpload::Nothing,
            Some(last) if last.lengths[array] == current.lengths[array] => ArrayUpload::Nothing,
            Some(last) => ArrayUpload::Tail {
                start: last.lengths[array],
            },
            None if last.is_some_and(|last| last.generation == current.generation) => {
                ArrayUpload::Nothing
            }
            None => ArrayUpload::Whole,
        }
    })
}

/// The size of a buffer that holds `bytes`: the next power of two, at least
/// [`MINIMUM_BUFFER_BYTES`].
fn buffer_capacity(bytes: usize) -> Result<u32> {
    let capacity = bytes
        .max(MINIMUM_BUFFER_BYTES as usize)
        .checked_next_power_of_two()
        .and_then(|capacity| u32::try_from(capacity).ok())
        .context("the overlay data exceeds the largest GPU buffer")?;
    Ok(capacity)
}

/// `offset` rounded up to [`TRANSFER_ALIGNMENT`].
fn align_transfer(offset: usize) -> usize {
    offset.next_multiple_of(TRANSFER_ALIGNMENT)
}

/// Creates a storage buffer of the fragment or the vertex stage with
/// `capacity` bytes.
fn create_storage_buffer(device: &GpuDevice, capacity: u32) -> Result<GpuBuffer> {
    let info = SDL_GPUBufferCreateInfo {
        usage: SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ,
        size: capacity,
        ..Default::default()
    };
    device
        .create_buffer(&info)
        .context("SDL_CreateGPUBuffer failed for the overlay")
        .map_err(Into::into)
}

/// Replaces `buffer` by a buffer of the next power of two when it holds
/// fewer than `bytes`. Returns whether it was replaced.
fn ensure_capacity(device: &GpuDevice, buffer: &mut GpuBuffer, bytes: usize) -> Result<bool> {
    if bytes <= buffer.size() as usize {
        return Ok(false);
    }
    *buffer = create_storage_buffer(device, buffer_capacity(bytes)?)?;
    Ok(true)
}

/// Creates an upload transfer buffer of `capacity` bytes.
fn create_transfer_buffer(device: &GpuDevice, capacity: u32) -> Result<GpuTransferBuffer> {
    let info = SDL_GPUTransferBufferCreateInfo {
        usage: SDL_GPU_TRANSFERBUFFERUSAGE_UPLOAD,
        size: capacity,
        ..Default::default()
    };
    device
        .create_transfer_buffer(&info)
        .context("SDL_CreateGPUTransferBuffer failed for the overlay")
        .map_err(Into::into)
}

/// Creates the blue noise tile of the gradient dither and uploads it.
fn create_blue_noise(device: &GpuDevice) -> Result<GpuTexture> {
    let info = SDL_GPUTextureCreateInfo {
        r#type: SDL_GPU_TEXTURETYPE_2D,
        format: SDL_GPU_TEXTUREFORMAT_R8_UNORM,
        usage: SDL_GPU_TEXTUREUSAGE_SAMPLER,
        width: BLUE_NOISE_SIZE as u32,
        height: BLUE_NOISE_SIZE as u32,
        layer_count_or_depth: 1,
        num_levels: 1,
        sample_count: SDL_GPU_SAMPLECOUNT_1,
        ..Default::default()
    };
    let texture = device
        .create_texture(&info)
        .context("SDL_CreateGPUTexture failed for the blue noise")?;

    let transfer_buffer = create_transfer_buffer(device, BLUE_NOISE.len() as u32)?;
    let mapped = device
        .map_transfer_buffer(&transfer_buffer, false)
        .context("SDL_MapGPUTransferBuffer failed for the blue noise")?;
    // Safety: the mapped transfer buffer holds `BLUE_NOISE.len()` bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(BLUE_NOISE.as_ptr(), mapped, BLUE_NOISE.len());
    }
    device.unmap_transfer_buffer(&transfer_buffer);

    let mut command_buffer = device
        .acquire_command_buffer()
        .context("SDL_AcquireGPUCommandBuffer failed for the blue noise")?;
    {
        let copy_pass = command_buffer.begin_copy_pass();
        copy_pass.upload_to_texture(
            &TextureTransferInfo {
                transfer_buffer: &transfer_buffer,
                offset: 0,
                pixels_per_row: BLUE_NOISE_SIZE as u32,
                rows_per_layer: BLUE_NOISE_SIZE as u32,
            },
            &TextureRegion {
                texture: &texture,
                mip_level: 0,
                layer: 0,
                x: 0,
                y: 0,
                z: 0,
                w: BLUE_NOISE_SIZE as u32,
                h: BLUE_NOISE_SIZE as u32,
                d: 1,
            },
            false,
        );
    }
    command_buffer
        .submit()
        .context("SDL_SubmitGPUCommandBuffer failed for the blue noise")?;
    Ok(texture)
}

/// Creates the nearest sampler of the blue noise tile.
fn create_nearest_sampler(device: &GpuDevice) -> Result<GpuSampler> {
    let info = SDL_GPUSamplerCreateInfo {
        min_filter: SDL_GPU_FILTER_NEAREST,
        mag_filter: SDL_GPU_FILTER_NEAREST,
        mipmap_mode: SDL_GPU_SAMPLERMIPMAPMODE_NEAREST,
        address_mode_u: SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
        address_mode_v: SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
        address_mode_w: SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE,
        ..Default::default()
    };
    device
        .create_sampler(&info)
        .context("SDL_CreateGPUSampler failed for the blue noise")
        .map_err(Into::into)
}

/// The instance that fills the target with the clear color of the scene, as
/// the CPU rasterizer starts every frame with it. `None` for a transparent
/// clear color.
fn clear_instance(overlay: &Overlay<'_>, width: u32, height: u32) -> Option<Instance> {
    let clear = overlay.scene.clear_color();
    if clear.a == 0 {
        return None;
    }
    let (width, height) = (width as f32, height as f32);
    Some(Instance {
        rect: [0.0, 0.0, width, height],
        clip: [0.0, 0.0, width, height],
        color: clear.to_linear(),
        kind: [Instance::KIND_RECTANGLE, 0, 0, 0],
        ..Instance::default()
    })
}

/// The storage buffer an upload writes.
#[derive(Clone, Copy)]
enum UploadTarget {
    /// The instance buffer.
    Instances,
    /// The Slug buffer of the curves, the bands or the glyphs at this index.
    Slug(usize),
}

/// One copy from the transfer buffer into a storage buffer.
#[derive(Clone, Copy)]
struct PendingUpload {
    /// The storage buffer the bytes go to.
    target: UploadTarget,
    /// The offset of the bytes in the transfer buffer.
    source_offset: usize,
    /// The offset of the bytes in the storage buffer.
    destination_offset: usize,
    /// The number of bytes.
    size: usize,
    /// Whether the upload replaces the whole content of the buffer.
    cycle: bool,
}

/// The draw of one frame that the upload prepared.
pub(crate) struct OverlayDraw {
    /// The number of instances in the instance buffer.
    instance_count: u32,
    /// The uniforms of both stages.
    uniforms: UiUniforms,
}

/// The GPU resources of the overlay.
pub(crate) struct OverlayRenderer {
    /// The user interface pipeline.
    pipeline: GpuGraphicsPipeline,
    /// The blue noise tile of the gradient dither.
    blue_noise: GpuTexture,
    /// The nearest sampler of the blue noise tile.
    sampler: GpuSampler,
    /// The instance buffer.
    instances: GpuBuffer,
    /// The buffers of the Slug curves, bands and glyphs.
    slug_buffers: [GpuBuffer; SLUG_ARRAYS],
    /// The upload transfer buffer. It grows to the largest upload of a frame.
    transfer_buffer: GpuTransferBuffer,
    /// The Slug data the buffers hold. `None` before the first upload.
    uploaded_slug: Option<SlugUpload>,
    /// The number of glyph records the glyph buffer holds.
    glyph_count: u32,
    /// Whether the shader encodes its output in sRGB.
    encode_srgb: bool,
}

impl OverlayRenderer {
    /// Creates the overlay resources for a target of `target_format` and
    /// uploads the blue noise tile. `encode_srgb` is set when the target
    /// stores the bytes of the color without a hardware sRGB encoding.
    pub(crate) fn new(
        device: &GpuDevice,
        target_format: SDL_GPUTextureFormat,
        encode_srgb: bool,
    ) -> Result<Self> {
        let pipeline = ui_pipeline::build(device, target_format)?;
        let blue_noise = create_blue_noise(device)?;
        let sampler = create_nearest_sampler(device)?;
        let instances = create_storage_buffer(device, MINIMUM_BUFFER_BYTES)?;
        let slug_buffers = [
            create_storage_buffer(device, MINIMUM_BUFFER_BYTES)?,
            create_storage_buffer(device, MINIMUM_BUFFER_BYTES)?,
            create_storage_buffer(device, MINIMUM_BUFFER_BYTES)?,
        ];
        let transfer_buffer = create_transfer_buffer(device, MINIMUM_BUFFER_BYTES)?;
        Ok(Self {
            pipeline,
            blue_noise,
            sampler,
            instances,
            slug_buffers,
            transfer_buffer,
            uploaded_slug: None,
            glyph_count: 0,
            encode_srgb,
        })
    }

    /// Uploads the instances and the changed Slug data of `overlay` for a
    /// target of `width` x `height` pixels in `copy_pass`. Returns `None`
    /// when the overlay draws nothing.
    pub(crate) fn upload(
        &mut self,
        device: &GpuDevice,
        copy_pass: &GpuCopyPass<'_>,
        overlay: Overlay<'_>,
        width: u32,
        height: u32,
    ) -> Result<Option<OverlayDraw>> {
        let clear = clear_instance(&overlay, width, height);
        if overlay.scene.is_empty() && clear.is_none() {
            return Ok(None);
        }
        let clear_bytes = clear.as_ref().map_or(&[][..], Instance::as_bytes);
        let scene_bytes = overlay.scene.instance_bytes();
        let instance_bytes = clear_bytes.len() + scene_bytes.len();
        let instance_count = u32::try_from(instance_bytes / size_of::<Instance>())
            .context("the overlay holds too many instances")?;
        ensure_capacity(device, &mut self.instances, instance_bytes)?;

        let slug = overlay.slug;
        let slug_bytes = [slug.curve_bytes(), slug.band_bytes(), slug.glyph_bytes()];
        let element_sizes = [
            size_of::<[f32; 4]>(),
            size_of::<u32>(),
            size_of::<gui::SlugGlyph>(),
        ];
        let mut grown = [false; SLUG_ARRAYS];
        for (array, buffer) in self.slug_buffers.iter_mut().enumerate() {
            grown[array] = ensure_capacity(device, buffer, slug_bytes[array].len())?;
        }
        let current = SlugUpload::of(slug);
        let choices = choose_slug_uploads(self.uploaded_slug, current, grown);

        let mut uploads = [None; 1 + SLUG_ARRAYS];
        uploads[0] = Some(PendingUpload {
            target: UploadTarget::Instances,
            source_offset: 0,
            destination_offset: 0,
            size: instance_bytes,
            cycle: true,
        });
        let mut transfer_size = align_transfer(instance_bytes);
        for (array, choice) in choices.into_iter().enumerate() {
            let (start, cycle) = match choice {
                ArrayUpload::Nothing => continue,
                ArrayUpload::Tail { start } => (start * element_sizes[array], false),
                ArrayUpload::Whole => (0, true),
            };
            let size = slug_bytes[array].len() - start;
            if size == 0 {
                continue;
            }
            uploads[1 + array] = Some(PendingUpload {
                target: UploadTarget::Slug(array),
                source_offset: transfer_size,
                destination_offset: start,
                size,
                cycle,
            });
            transfer_size = align_transfer(transfer_size + size);
        }

        if transfer_size > self.transfer_buffer.size() as usize {
            self.transfer_buffer = create_transfer_buffer(device, buffer_capacity(transfer_size)?)?;
        }
        let mapped = device
            .map_transfer_buffer(&self.transfer_buffer, true)
            .context("SDL_MapGPUTransferBuffer failed for the overlay")?;
        for upload in uploads.iter().flatten() {
            let pieces: [&[u8]; 2] = match upload.target {
                UploadTarget::Instances => [clear_bytes, scene_bytes],
                UploadTarget::Slug(array) => [&slug_bytes[array][upload.destination_offset..], &[]],
            };
            let mut offset = upload.source_offset;
            for piece in pieces {
                // Safety: the transfer buffer holds `transfer_size` bytes, and
                // every piece lies inside the range the layout above reserved.
                unsafe {
                    std::ptr::copy_nonoverlapping(piece.as_ptr(), mapped.add(offset), piece.len());
                }
                offset += piece.len();
            }
        }
        device.unmap_transfer_buffer(&self.transfer_buffer);

        for upload in uploads.iter().flatten() {
            let buffer = match upload.target {
                UploadTarget::Instances => &self.instances,
                UploadTarget::Slug(array) => &self.slug_buffers[array],
            };
            copy_pass.upload_to_buffer(
                &TransferBufferLocation {
                    transfer_buffer: &self.transfer_buffer,
                    offset: upload.source_offset as u32,
                },
                &BufferRegion {
                    buffer,
                    offset: upload.destination_offset as u32,
                    size: upload.size as u32,
                },
                upload.cycle,
            );
        }
        self.uploaded_slug = Some(current);
        self.glyph_count = current.lengths[2] as u32;

        Ok(Some(OverlayDraw {
            instance_count,
            uniforms: UiUniforms {
                size: [width as f32, height as f32],
                encode_srgb: u32::from(self.encode_srgb),
                glyph_count: self.glyph_count,
            },
        }))
    }

    /// Draws the uploaded overlay over the whole target of `render_pass`.
    pub(crate) fn draw(&self, render_pass: &GpuRenderPass<'_>, draw: &OverlayDraw) {
        render_pass.bind_graphics_pipeline(&self.pipeline);
        render_pass.bind_vertex_storage_buffers(0, &[&self.instances]);
        render_pass.bind_fragment_samplers(
            0,
            &[TextureSamplerBinding {
                texture: &self.blue_noise,
                sampler: &self.sampler,
            }],
        );
        let [curves, bands, glyphs] = &self.slug_buffers;
        render_pass.bind_fragment_storage_buffers(0, &[curves, bands, glyphs]);
        render_pass.push_vertex_uniform_data(0, draw.uniforms.as_bytes());
        render_pass.push_fragment_uniform_data(0, draw.uniforms.as_bytes());
        render_pass.draw_primitives(VERTICES_PER_INSTANCE, draw.instance_count, 0, 0);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        f32::consts::{PI, TAU},
        path::PathBuf,
    };

    use gui::{Color, Fonts, Path, Rasterizer, Rect, RectStyle, Scene, linear_to_srgb};
    use sdl3::gpu::{
        ColorTargetDescriptor, SDL_FColor, SDL_GPU_LOADOP_CLEAR, SDL_GPU_STOREOP_STORE,
        SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB,
        SDL_GPU_TEXTUREUSAGE_COLOR_TARGET, SDL_GPU_TRANSFERBUFFERUSAGE_DOWNLOAD,
    };

    use super::*;

    /// The width of the comparison target in pixels.
    const WIDTH: u32 = 960;

    /// The height of the comparison target in pixels.
    const HEIGHT: u32 = 600;

    /// The largest difference of a channel between the GPU and the CPU on a
    /// target with hardware sRGB encoding, in 8 bit steps. Every blend of the
    /// GPU rounds to 8 bits, and the GPU evaluates the transcendental
    /// functions and the sRGB curves with other rounding than the CPU.
    const MAXIMUM_DIFFERENCE: u8 = 4;

    /// The largest mean difference of a channel on a target with hardware
    /// sRGB encoding, in 8 bit steps. The clear color covers every pixel, and
    /// its 8 bit sRGB storage rounds differently from the CPU output.
    const MAXIMUM_MEAN_DIFFERENCE: f64 = 0.25;

    /// The environment variable that names the directory of the screenshots.
    const SHOTS_VARIABLE: &str = "GUI_SHOTS";

    /// A state with the given identity, generation and lengths.
    fn state(identity: u64, generation: u64, lengths: [usize; 3]) -> SlugUpload {
        SlugUpload {
            identity,
            generation,
            lengths,
        }
    }

    #[test]
    fn the_slug_upload_follows_identity_generation_and_lengths() {
        use ArrayUpload::{Nothing, Tail, Whole};
        let none = [false; 3];
        let first = state(1, 10, [4, 8, 1]);
        assert_eq!(choose_slug_uploads(None, first, none), [Whole; 3]);
        assert_eq!(choose_slug_uploads(Some(first), first, none), [Nothing; 3]);

        let grown = state(1, 11, [6, 8, 2]);
        assert_eq!(
            choose_slug_uploads(Some(first), grown, none),
            [Tail { start: 4 }, Nothing, Tail { start: 1 }]
        );
        assert_eq!(
            choose_slug_uploads(Some(first), grown, [false, false, true]),
            [Tail { start: 4 }, Nothing, Whole]
        );

        let replaced = state(2, 12, [6, 8, 2]);
        assert_eq!(choose_slug_uploads(Some(first), replaced, none), [Whole; 3]);
        let same_lengths = state(2, 13, [4, 8, 1]);
        assert_eq!(
            choose_slug_uploads(Some(first), same_lengths, none),
            [Whole; 3]
        );

        let clone = state(3, 10, [4, 8, 1]);
        assert_eq!(choose_slug_uploads(Some(first), clone, none), [Nothing; 3]);

        let shrunk = state(1, 14, [2, 8, 1]);
        assert_eq!(choose_slug_uploads(Some(first), shrunk, none), [Whole; 3]);
    }

    #[test]
    fn buffers_grow_to_the_next_power_of_two() {
        assert_eq!(buffer_capacity(0).unwrap(), MINIMUM_BUFFER_BYTES);
        assert_eq!(buffer_capacity(4097).unwrap(), 8192);
        assert_eq!(buffer_capacity(65536).unwrap(), 65536);
        assert!(buffer_capacity(usize::MAX).is_err());
        assert_eq!(align_transfer(17), 32);
    }

    /// Writes RGBA8 pixels with premultiplied sRGB color as `<name>.ppm` over
    /// black into the directory of the `GUI_SHOTS` environment variable.
    fn write_ppm(name: &str, rgba: &[u8]) {
        let Some(directory) = std::env::var_os(SHOTS_VARIABLE) else {
            return;
        };
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).expect("the screenshot directory can be created");
        let mut bytes = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
        for pixel in rgba.as_chunks::<4>().0 {
            bytes.extend_from_slice(&pixel[..3]);
        }
        let path = directory.join(format!("{name}.ppm"));
        std::fs::write(&path, bytes).expect("the screenshot can be written");
        eprintln!("wrote {}", path.display());
    }

    /// A circle of four cubic curves around `(x, y)`.
    fn circle(path: Path, x: f32, y: f32, radius: f32) -> Path {
        let handle = radius * 0.552_284_8;
        path.move_to(x + radius, y)
            .cubic_to(
                x + radius,
                y + handle,
                x + handle,
                y + radius,
                x,
                y + radius,
            )
            .cubic_to(
                x - handle,
                y + radius,
                x - radius,
                y + handle,
                x - radius,
                y,
            )
            .cubic_to(
                x - radius,
                y - handle,
                x - handle,
                y - radius,
                x,
                y - radius,
            )
            .cubic_to(
                x + handle,
                y - radius,
                x + radius,
                y - handle,
                x + radius,
                y,
            )
            .close()
    }

    /// The path of the font file below the repository root.
    const REPOSITORY_FONT_PATH: &str = "font/SarasaUiJ-Regular.ttf";

    /// Loads the font of the user interface from the repository.
    fn load_fonts() -> (Fonts, gui::FontId) {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(REPOSITORY_FONT_PATH);
        let data = std::fs::read(&path).expect("the font of the repository can be read");
        let mut fonts = Fonts::new();
        let font = fonts.add(data).expect("the font parses");
        (fonts, font)
    }

    /// A dialog with every kind of item, glyphs of Latin and Japanese text and
    /// paths. With `more_text` it shapes text that adds glyphs to the store.
    fn sample_scene(fonts: &mut Fonts, font: gui::FontId, more_text: bool) -> Scene {
        let ring = fonts
            .add_path(&circle(circle(Path::new(), 0.5, 0.5, 0.45), 0.5, 0.5, 0.25))
            .unwrap();
        let star = fonts
            .add_path(&Path::new().polygon(&[
                [0.5, 0.0],
                [0.62, 0.38],
                [1.0, 0.38],
                [0.69, 0.62],
                [0.81, 1.0],
                [0.5, 0.76],
                [0.19, 1.0],
                [0.31, 0.62],
                [0.0, 0.38],
                [0.38, 0.38],
            ]))
            .unwrap();
        let scale = 1.25;
        let mut scene = Scene::new();
        scene.begin(scale, Color::hex(0x101820).with_alpha(0.35));

        let panel = Rect::new(40.0, 30.0, 680.0, 410.0);
        scene.shadow(panel, 14.0, 18.0, (0.0, 6.0), Color::BLACK.with_alpha(0.6));
        scene.rect(
            panel,
            &RectStyle::filled(Color::hex(0x2A2D3A), 14.0)
                .with_gradient(Color::hex(0x1E2029))
                .with_stroke(1.0, Color::hex(0x4A4F66)),
        );
        scene.rect(
            Rect::new(panel.x, panel.y, panel.width, 52.0),
            &RectStyle {
                radii: [14.0, 14.0, 0.0, 0.0],
                fill: Color::hex(0x3B4160),
                fill_bottom: Some(Color::hex(0x30354D)),
                ..RectStyle::default()
            },
        );
        let title = fonts.shape(font, "Disk images ディスクイメージ");
        scene.run((panel.x + 20.0, panel.y + 34.0), 20.0, &title, Color::WHITE);

        let list = Rect::new(panel.x + 16.0, panel.y + 64.0, 440.0, 260.0);
        scene.fill(list, 8.0, Color::hex(0x16181F));
        scene.push_clip(list);
        let names = [
            "The quick brown fox jumps over the lazy dog.hdi",
            "日本語のファイル名、テスト。.d88",
            "カタカナとひらがなの名前.fdi",
            "Sorcerian (Disk A).d88",
            "0123456789 ABCDEFGHIJ abcdefghij.nfd",
            "漢字のディスク「第二巻」.hdm",
            "Ys II - Ancient Ys Vanished.d88",
            "ねこぷろじぇくと.fdi",
            "MS-DOS 6.20 起動ディスク.hdi",
            "last row that the clip cuts.d88",
        ];
        for (row, name) in names.iter().enumerate() {
            let row_y = list.y + row as f32 * 28.0 - 6.0;
            if row == 2 {
                scene.fill(
                    Rect::new(list.x + 4.0, row_y, list.width - 8.0, 28.0),
                    6.0,
                    Color::hex(0x3E5BC2),
                );
            }
            scene.path(
                (list.x + 10.0, row_y + 6.0),
                16.0,
                if row % 2 == 0 { ring } else { star },
                Color::hex(0x8890A8),
            );
            let run = fonts.shape(font, name);
            scene.run(
                (list.x + 34.0, row_y + 19.0),
                14.0,
                &run,
                Color::hex(0xD8DCE8),
            );
        }
        scene.pop_clip();

        let light = Rect::new(panel.x + 472.0, panel.y + 64.0, 192.0, 120.0);
        scene.fill(light, 6.0, Color::hex(0xF4F4F0));
        for (line, size) in [13.0f32, 16.0, 24.0].into_iter().enumerate() {
            let run = fonts.shape(font, "Dark text 黒");
            scene.run(
                (light.x + 10.0, light.y + 24.0 + line as f32 * 32.0),
                size,
                &run,
                Color::hex(0x202028),
            );
        }
        if more_text {
            let run = fonts.shape(font, "追加の文字列 Qwz@#%&");
            scene.run(
                (panel.x + 472.0, panel.y + 214.0),
                18.0,
                &run,
                Color::hex(0xFFC060),
            );
        }

        for index in 0..4 {
            let center = (panel.x + 500.0 + index as f32 * 44.0, panel.y + 260.0);
            scene.arc(center, 15.0, 4.0, (0.0, TAU), Color::hex(0x30354D));
            scene.arc(
                center,
                15.0,
                4.0,
                (-PI * 0.5, TAU * (index as f32 + 1.0) / 5.0),
                Color::hex(0x7FA0FF),
            );
        }
        scene.line(
            (panel.x + 16.0, panel.y + 340.0),
            (panel.x + panel.width - 16.0, panel.y + 340.0),
            1.0,
            Color::hex(0x4A4F66),
        );
        scene.line(
            (panel.x + 480.0, panel.y + 300.0),
            (panel.x + 660.0, panel.y + 330.0),
            3.0,
            Color::hex(0xFF8040).with_alpha(0.8),
        );
        for button in 0..2 {
            let button_rect = Rect::new(
                panel.x + panel.width - 300.0 + button as f32 * 146.0,
                panel.y + panel.height - 56.0,
                132.0,
                40.0,
            );
            scene.shadow(
                button_rect,
                8.0,
                4.0,
                (0.0, 2.0),
                Color::BLACK.with_alpha(0.4),
            );
            scene.rect(
                button_rect,
                &RectStyle::filled(Color::hex(0x5870D0), 8.0).with_gradient(Color::hex(0x4058B0)),
            );
            let label = fonts.shape(font, if button == 0 { "Insert" } else { "取消" });
            scene.run(
                (button_rect.x + 40.0, button_rect.y + 26.0),
                16.0,
                &label,
                Color::WHITE,
            );
        }
        scene.fill(
            Rect::new(600.0, 360.0, 140.0, 80.0),
            20.0,
            Color::hex(0x40C080).with_alpha(0.5),
        );
        scene
    }

    /// The GPU objects of the comparison.
    struct GpuTarget {
        /// The texture the overlay draws into.
        texture: GpuTexture,
        /// The buffer the texture downloads into.
        download: GpuTransferBuffer,
    }

    impl GpuTarget {
        /// A target of [`WIDTH`] x [`HEIGHT`] pixels in `format`.
        fn new(device: &GpuDevice, format: SDL_GPUTextureFormat) -> Self {
            let texture = device
                .create_texture(&SDL_GPUTextureCreateInfo {
                    r#type: SDL_GPU_TEXTURETYPE_2D,
                    format,
                    usage: SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
                    width: WIDTH,
                    height: HEIGHT,
                    layer_count_or_depth: 1,
                    num_levels: 1,
                    sample_count: SDL_GPU_SAMPLECOUNT_1,
                    ..Default::default()
                })
                .expect("the target texture can be created");
            let download = device
                .create_transfer_buffer(&SDL_GPUTransferBufferCreateInfo {
                    usage: SDL_GPU_TRANSFERBUFFERUSAGE_DOWNLOAD,
                    size: WIDTH * HEIGHT * 4,
                    ..Default::default()
                })
                .expect("the download buffer can be created");
            Self { texture, download }
        }

        /// Draws `overlay` over a transparent target and returns its bytes.
        fn render(
            &self,
            device: &GpuDevice,
            renderer: &mut OverlayRenderer,
            overlay: Overlay<'_>,
        ) -> Vec<u8> {
            let mut command_buffer = device.acquire_command_buffer().unwrap();
            let draw = {
                let copy_pass = command_buffer.begin_copy_pass();
                renderer
                    .upload(device, &copy_pass, overlay, WIDTH, HEIGHT)
                    .unwrap()
            };
            {
                let render_pass = command_buffer.begin_render_pass(&ColorTargetDescriptor {
                    texture: &self.texture,
                    clear_color: SDL_FColor {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    },
                    load_op: SDL_GPU_LOADOP_CLEAR,
                    store_op: SDL_GPU_STOREOP_STORE,
                });
                if let Some(draw) = &draw {
                    renderer.draw(&render_pass, draw);
                }
            }
            {
                let copy_pass = command_buffer.begin_copy_pass();
                copy_pass.download_from_texture(
                    &TextureRegion {
                        texture: &self.texture,
                        mip_level: 0,
                        layer: 0,
                        x: 0,
                        y: 0,
                        z: 0,
                        w: WIDTH,
                        h: HEIGHT,
                        d: 1,
                    },
                    &TextureTransferInfo {
                        transfer_buffer: &self.download,
                        offset: 0,
                        pixels_per_row: WIDTH,
                        rows_per_layer: HEIGHT,
                    },
                );
            }
            command_buffer.submit().unwrap();
            device.wait_for_idle().unwrap();
            let length = (WIDTH * HEIGHT * 4) as usize;
            let mapped = device.map_transfer_buffer(&self.download, false).unwrap();
            // Safety: the download buffer holds `length` bytes and stays
            // mapped until the copy finished.
            let bytes = unsafe { std::slice::from_raw_parts(mapped, length) }.to_vec();
            device.unmap_transfer_buffer(&self.download);
            bytes
        }
    }

    /// The bytes of a target with hardware sRGB encoding in the output format
    /// of the CPU rasterizer: sRGB encoded, premultiplied color.
    fn hardware_srgb_to_premultiplied(bytes: &[u8]) -> Vec<u8> {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|pixel| {
                let alpha = f32::from(pixel[3]) / 255.0;
                let channel = |value: u8| {
                    if alpha <= 0.0 {
                        return 0;
                    }
                    let linear = gui::srgb_to_linear(f32::from(value) / 255.0);
                    (linear_to_srgb((linear / alpha).min(1.0)) * alpha * 255.0 + 0.5) as u8
                };
                [
                    channel(pixel[0]),
                    channel(pixel[1]),
                    channel(pixel[2]),
                    pixel[3],
                ]
            })
            .collect()
    }

    /// The largest and the mean difference of the channels of two images.
    fn difference(first: &[u8], second: &[u8]) -> (u8, f64) {
        let maximum = first
            .iter()
            .zip(second)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        let sum: u64 = first
            .iter()
            .zip(second)
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum();
        (maximum, sum as f64 / first.len() as f64)
    }

    /// The differences of two images, scaled by 32 for the eye.
    fn difference_image(first: &[u8], second: &[u8]) -> Vec<u8> {
        first
            .as_chunks::<4>()
            .0
            .iter()
            .zip(second.as_chunks::<4>().0)
            .flat_map(|(a, b)| {
                let channel = |index: usize| a[index].abs_diff(b[index]).saturating_mul(32);
                [channel(0), channel(1), channel(2), 255]
            })
            .collect()
    }

    /// Draws `scene` with `slug` on the CPU and on the GPU and returns the
    /// largest and the mean difference.
    fn compare(
        device: &GpuDevice,
        target: &GpuTarget,
        renderer: &mut OverlayRenderer,
        scene: &Scene,
        slug: &SlugData,
        name: &str,
    ) -> (u8, f64) {
        let mut cpu = vec![0; (WIDTH * HEIGHT * 4) as usize];
        Rasterizer::new().rasterize(scene, slug, WIDTH, HEIGHT, &mut cpu);
        let bytes = target.render(device, renderer, Overlay { scene, slug });
        let gpu = if renderer.encode_srgb {
            bytes
        } else {
            hardware_srgb_to_premultiplied(&bytes)
        };
        let (maximum, mean) = difference(&gpu, &cpu);
        eprintln!("{name}: maximum difference {maximum}, mean {mean:.4}");
        write_ppm(&format!("overlay_cpu_{name}"), &cpu);
        write_ppm(&format!("overlay_gpu_{name}"), &gpu);
        write_ppm(
            &format!("overlay_difference_{name}"),
            &difference_image(&gpu, &cpu),
        );
        (maximum, mean)
    }

    #[test]
    #[ignore = "needs a GPU, compares the GPU overlay with the CPU rasterizer"]
    fn the_gpu_overlay_matches_the_cpu_rasterizer() {
        let sdl = sdl3::init().expect("SDL initializes");
        let _video = sdl.video().expect("the video subsystem initializes");
        let device = crate::device::create_device().expect("a GPU device exists");

        for (format, encode_srgb, name) in [
            (SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB, false, "srgb"),
            (SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, true, "unorm"),
        ] {
            let target = GpuTarget::new(&device, format);
            let mut renderer = OverlayRenderer::new(&device, format, encode_srgb)
                .expect("the overlay renderer can be created");
            let (mut fonts, font) = load_fonts();

            let scene = sample_scene(&mut fonts, font, false);
            let first = compare(
                &device,
                &target,
                &mut renderer,
                &scene,
                fonts.slug(),
                &format!("{name}_first"),
            );

            let scene = sample_scene(&mut fonts, font, true);
            let slug = fonts.slug();
            let grown = std::array::from_fn(|array| {
                let bytes = [slug.curve_bytes(), slug.band_bytes(), slug.glyph_bytes()][array];
                bytes.len() > renderer.slug_buffers[array].size() as usize
            });
            let choices = choose_slug_uploads(renderer.uploaded_slug, SlugUpload::of(slug), grown);
            assert!(
                choices
                    .iter()
                    .any(|choice| matches!(choice, ArrayUpload::Tail { .. })),
                "the second frame uploads a tail: {choices:?}"
            );
            let second = compare(
                &device,
                &target,
                &mut renderer,
                &scene,
                slug,
                &format!("{name}_appended"),
            );

            // A target without hardware sRGB encoding blends translucent overlaps in encoded
            // space and differs from the CPU. Its result is only reported.
            if !encode_srgb {
                for (maximum, mean) in [first, second] {
                    assert!(
                        maximum <= MAXIMUM_DIFFERENCE && mean <= MAXIMUM_MEAN_DIFFERENCE,
                        "maximum difference {maximum}, mean {mean}"
                    );
                }
            }
        }
    }
}
