//! VA-API H.264 encoding for the host role.
//!
//! The captured frame arrives as packed BGRA in system memory, because the
//! ScreenCast portal only ever offers MemFd buffers (see `capture::pipewire`).
//! This encoder converts to NV12 and uploads it into a VA surface, then drives
//! the driver's `VAEntrypointEncSlice`.
//!
//! That still beats the software codec: the hardware path spends its time in
//! the colour conversion and the upload, while the motion search and entropy
//! coding -- the expensive part -- run on the GPU. Measured at 3342x2160 on a
//! Haswell i965, `h264_vaapi` runs about 1.4x the throughput of the software
//! encoder at roughly a third less CPU.
//!
//! The driver owns the bitstream syntax. We supply the sequence and picture
//! parameters; the driver emits SPS/PPS and slices.
use std::rc::Rc;
use std::sync::{Arc, Weak};

use cros_libva::{
    BufferType, Context, Display, EncCodedBuffer, EncMiscParameter,
    EncMiscParameterBufferMaxFrameSize, EncMiscParameterFrameRate, EncMiscParameterRateControl,
    EncPackedHeaderParameter, EncPictureParameterBufferH264, EncSequenceParameterBufferH264,
    EncSliceParameterBufferH264, H264EncPicFields, H264EncSeqFields, H264VuiFields,
    MappedCodedBuffer, Picture, PictureH264, RcFlags, Surface, UsageHint, VAConfigAttrib,
    VAConfigAttribType, VAEntrypoint, VAProfile, VASurfaceID,
};

use crate::media::encoding::{Encoded, Format, Rate};

/// NV12 (8-bit 4:2:0), the only layout this encoder feeds.
const RT_FORMAT_YUV420: u32 = cros_libva::VA_RT_FORMAT_YUV420;

/// Surfaces the encoder cycles through: one being filled, one being encoded,
/// and enough left for the driver to keep its reference frames.
const SURFACE_COUNT: usize = 4;

/// `VAEncMiscParameterRateControl` expresses the window in milliseconds.
const RC_WINDOW_MS: u32 = 1000;

/// H.264 level 5.1, which covers 4096x4096 at 30 fps.
const LEVEL_IDC: u8 = 51;

/// Flags for a picture the decoder may keep as a short-term reference.
const PICTURE_SHORT_TERM: u32 = cros_libva::VA_PICTURE_H264_SHORT_TERM_REFERENCE;

/// `log2_max_frame_num_minus4`. The parameter sets this encoder submits declare
/// 15-bit `frame_num`, and the driver's slice must agree with them.
const LOG2_MAX_FRAME_NUM_MINUS4: u32 = 11;

/// `log2_max_pic_order_cnt_lsb_minus4`, 16-bit POC in the same sets.
const LOG2_MAX_POC_LSB_MINUS4: u32 = 12;

/// Default quantiser. The rate controller moves it per frame.
const INITIAL_QP: u8 = 26;

#[derive(Debug, thiserror::Error)]
pub(crate) enum EncodeError {
    #[error("VA-API 编码驱动不可用：{0}")]
    Unsupported(String),
    #[error("VA-API 编码失败：{0}")]
    Backend(String),
}

impl From<cros_libva::VaError> for EncodeError {
    fn from(error: cros_libva::VaError) -> Self {
        Self::Backend(error.to_string())
    }
}

pub(crate) struct Encoder {
    context: Rc<Context>,
    /// Taken out for the frame being encoded and put back afterwards, because
    /// `Picture::new` takes ownership of the surface.
    surfaces: Vec<Option<Surface<()>>>,
    /// CPU capture frames are immutable Arc allocations. A weak identity per
    /// input slot lets repeated desktop frames reuse its converted pixels,
    /// without retaining large old frames or comparing their contents.
    uploaded: Vec<Option<Weak<Vec<u8>>>>,
    /// Reconstruction must never alias the raw input. These are the decoded
    /// pictures referenced by subsequent P slices, including DMA-BUF input.
    _reconstructed: Vec<Surface<()>>,
    /// Reconstructed surface ids used by picture parameters and P-slice DPB.
    ids: Vec<VASurfaceID>,
    /// Slot the next frame is uploaded into.
    next: usize,
    /// Slot holding the frame the driver may use as a reference.
    reference: Option<usize>,
    width: u32,
    height: u32,
    format: Format,
    rate: Rate,
    rate_control: crate::media::encoding::rate::Controller,
    pending_rate: Option<crate::media::encoding::rate::Update>,
    frame_num: u16,
    /// Alternates between consecutive IDRs.
    idr_pic_id: u16,
    /// Set whenever the rate changes, so the controller is re-sent.
    rc_dirty: bool,
    /// Resent before an IDR or when timing changes.
    sequence_dirty: bool,
    /// Annex B SPS and PPS, submitted as a raw packed header with the sequence
    /// parameter buffer. Built once, since the driver rejects generating them.
    parameter_sets: Vec<u8>,
    /// Time spent converting and uploading, and in the driver's encode.
    /// Only read by the throughput test.
    upload_time: std::time::Duration,
    encode_time: std::time::Duration,
    /// `vaDeriveImage` and the colour conversion, split out of `upload_time`.
    derive_time: std::time::Duration,
    convert_time: std::time::Duration,
    #[cfg(test)]
    uploads: u64,
}

impl Encoder {
    /// Whether this encoder can produce `format`.
    ///
    /// H.264 8-bit 4:2:0 only: that is what the host negotiates and what the
    /// i965 encode entrypoint accepts.
    pub fn accepts(format: Format) -> bool {
        format.valid()
            && format.codec == crate::media::encoding::Codec::H264
            && format.chroma == 1
            && format.depth == 8
    }

    /// Opens an encode session, or reports why the driver cannot provide one.
    pub fn new(
        display: &Rc<Display>,
        size: (u32, u32),
        format: Format,
        rate: Rate,
    ) -> Result<Self, EncodeError> {
        if !Self::accepts(format) {
            return Err(EncodeError::Unsupported(format!("不支持 {format:?}")));
        }
        if size.0 == 0 || size.1 == 0 || size.0 > 4096 || size.1 > 4096 {
            return Err(EncodeError::Unsupported(format!(
                "编码尺寸超出驱动上限：{}x{}",
                size.0, size.1
            )));
        }
        let config = display
            .create_config(
                vec![
                    // Without an explicit mode i965 defaults to CQP and ignores
                    // the congestion controller's requested bit rate.
                    VAConfigAttrib {
                        type_: VAConfigAttribType::VAConfigAttribRateControl,
                        value: cros_libva::VA_RC_CBR,
                    },
                    VAConfigAttrib {
                        type_: VAConfigAttribType::VAConfigAttribRTFormat,
                        value: RT_FORMAT_YUV420,
                    },
                    // The driver writes SPS/PPS only if the client echoes back
                    // the packed-header modes it advertises. Without this the
                    // bitstream starts at the IDR slice and no decoder can pick
                    // it up.
                    VAConfigAttrib {
                        type_: VAConfigAttribType::VAConfigAttribEncPackedHeaders,
                        value: packed_headers(display)?,
                    },
                ],
                VAProfile::VAProfileH264High,
                VAEntrypoint::VAEntrypointEncSlice,
            )
            .map_err(|error| EncodeError::Unsupported(error.to_string()))?;
        let mut surfaces = display
            .create_surfaces(
                RT_FORMAT_YUV420,
                None,
                size.0,
                size.1,
                Some(UsageHint::USAGE_HINT_ENCODER),
                vec![(); SURFACE_COUNT * 2],
            )
            .map_err(|error| EncodeError::Backend(error.to_string()))?;
        let context = display
            .create_context(&config, size.0, size.1, Some(&surfaces), true)
            .map_err(|error| EncodeError::Backend(error.to_string()))?;
        let reconstructed = surfaces.split_off(SURFACE_COUNT);
        let parameter_sets =
            openuuyc_h264::encoder::parameter_sets(openuuyc_h264::encoder::Config {
                width: size.0,
                height: size.1,
                fps: rate.fps,
                bitrate: rate.target,
            })
            .map_err(|error| EncodeError::Unsupported(error.to_string()))?;
        Ok(Self {
            ids: reconstructed.iter().map(Surface::id).collect(),
            _reconstructed: reconstructed,
            context,
            surfaces: surfaces.into_iter().map(Some).collect(),
            uploaded: vec![None; SURFACE_COUNT],
            next: 0,
            reference: None,
            width: size.0,
            height: size.1,
            format,
            rate,
            rate_control: crate::media::encoding::rate::Controller::new(rate),
            pending_rate: None,
            frame_num: 0,
            idr_pic_id: 0,
            rc_dirty: true,
            sequence_dirty: true,
            parameter_sets,
            upload_time: std::time::Duration::ZERO,
            encode_time: std::time::Duration::ZERO,
            derive_time: std::time::Duration::ZERO,
            convert_time: std::time::Duration::ZERO,
            #[cfg(test)]
            uploads: 0,
        })
    }

    pub fn maximum_size(&self) -> (u32, u32) {
        (4096, 4096)
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Applies a new rate. Returns whether a keyframe should follow.
    pub fn configure_rate(&mut self, rate: Rate) -> Result<bool, EncodeError> {
        if !(1..=144).contains(&rate.fps) || rate.target == 0 {
            return Err(EncodeError::Backend("无效 VA-API 码率或帧率".into()));
        }
        let Some(update) = self.rate_control.decide(rate) else {
            return Ok(false);
        };
        let fps_changed = self.rate.fps != update.rate.fps;
        let key = update.control_changed || fps_changed;
        if fps_changed {
            self.parameter_sets =
                openuuyc_h264::encoder::parameter_sets(openuuyc_h264::encoder::Config {
                    width: self.width,
                    height: self.height,
                    fps: update.rate.fps,
                    bitrate: update.rate.target,
                })
                .map_err(|error| EncodeError::Backend(error.to_string()))?;
        }
        tracing::debug!(
            previous_bps = self.rate.target,
            target_bps = update.rate.target,
            fps = update.rate.fps,
            quality = update.rate.quality,
            keyframe = key,
            "VA-API rate update"
        );
        self.rate = update.rate;
        self.pending_rate = Some(update);
        self.rc_dirty = true;
        self.sequence_dirty |= fps_changed;
        Ok(key)
    }

    /// Encodes one BGRA frame and returns the driver's bitstream.
    pub fn encode(
        &mut self,
        pixels: &[u8],
        pitch: usize,
        timestamp: i64,
        keyframe: bool,
    ) -> Result<Vec<Encoded>, EncodeError> {
        self.encode_cpu(pixels, pitch, None, timestamp, keyframe)
    }

    pub fn encode_shared(
        &mut self,
        pixels: &Arc<Vec<u8>>,
        timestamp: i64,
        keyframe: bool,
    ) -> Result<Vec<Encoded>, EncodeError> {
        self.encode_cpu(
            pixels,
            self.width as usize * 4,
            Some(pixels),
            timestamp,
            keyframe,
        )
    }

    fn encode_cpu(
        &mut self,
        pixels: &[u8],
        pitch: usize,
        identity: Option<&Arc<Vec<u8>>>,
        timestamp: i64,
        keyframe: bool,
    ) -> Result<Vec<Encoded>, EncodeError> {
        self.rate_control.input(timestamp);
        let needed = pitch
            .checked_mul(self.height as usize)
            .ok_or_else(|| EncodeError::Backend("帧尺寸溢出".into()))?;
        if pixels.len() < needed {
            return Err(EncodeError::Backend("采集帧小于编码尺寸".into()));
        }
        // The first frame has no reference, so it must be an IDR whatever the
        // caller asked for.
        let idr = keyframe || self.reference.is_none();
        // An IDR starts a new H.264 reference sequence; frame_num must be zero.
        if idr {
            self.frame_num = 0;
        }
        let index = self.next;
        self.next = (self.next + 1) % self.surfaces.len();

        let started = std::time::Instant::now();
        let reused = identity.is_some_and(|pixels| {
            self.uploaded[index]
                .as_ref()
                .is_some_and(|old| old.ptr_eq(&Arc::downgrade(pixels)))
        });
        if !reused {
            // Invalidate before writing: a partial failed upload must never
            // make the old allocation eligible for reuse.
            self.uploaded[index] = None;
            self.upload(index, pixels, pitch)?;
            self.uploaded[index] = identity.map(Arc::downgrade);
        }
        self.upload_time += started.elapsed();
        let started = std::time::Instant::now();
        let encoded = self.encode_surface(index, idr, timestamp)?;
        self.encode_time += started.elapsed();
        self.finish_encode(index, idr, encoded, timestamp)
    }

    /// Raw input and reconstructed reference pictures have distinct ownership.
    /// The imported allocation stays alive through synchronous encode completion.
    pub fn encode_nv12(
        &mut self,
        frame: &std::sync::Arc<super::gpu_convert::OwnedNv12>,
        timestamp: i64,
        keyframe: bool,
    ) -> Result<Vec<Encoded>, EncodeError> {
        self.rate_control.input(timestamp);
        if frame.size() != (self.width, self.height) {
            return Err(EncodeError::Backend("NV12 尺寸不匹配".into()));
        }
        let idr = keyframe || self.reference.is_none();
        if idr {
            self.frame_num = 0;
        }
        let index = self.next;
        self.next = (self.next + 1) % self.surfaces.len();
        let started = std::time::Instant::now();
        let surface = frame
            .import(self.context.display())
            .map_err(|e| EncodeError::Backend(e.to_string()))?;
        self.upload_time += started.elapsed();
        let started = std::time::Instant::now();
        let (encoded, surface) = self.encode_picture(index, idr, timestamp, surface)?;
        self.encode_time += started.elapsed();
        drop(surface);
        self.finish_encode(index, idr, encoded, timestamp)
    }

    fn finish_encode(
        &mut self,
        index: usize,
        idr: bool,
        encoded: Vec<u8>,
        timestamp: i64,
    ) -> Result<Vec<Encoded>, EncodeError> {
        self.reference = Some(index);
        // Buffer construction alone does not mean the driver applied them.
        // Keep dirty state and the controller decision retryable on failure.
        self.rc_dirty = false;
        self.sequence_dirty = false;
        if let Some(update) = self.pending_rate.take() {
            self.rate_control.commit(update);
        }
        if idr {
            self.idr_pic_id = self.idr_pic_id.wrapping_add(1);
        }
        self.frame_num = self.frame_num.wrapping_add(1) & 0x7fff;
        Ok(vec![Encoded {
            data: encoded,
            keyframe: idr,
            // The capture worker and RTP sender already use 100 ns units.
            timestamp_100ns: timestamp,
            is_new: true,
            timing: None,
            format: self.format,
            color: crate::media::encoding::Color::Sdr.space(None),
        }])
    }

    /// The accumulated upload and encode time, and resets them.
    #[cfg(test)]
    fn take_stage_totals(&mut self) -> (std::time::Duration, std::time::Duration) {
        (
            std::mem::take(&mut self.upload_time),
            std::mem::take(&mut self.encode_time),
        )
    }

    /// The `vaDeriveImage` and conversion time, and resets them.
    #[cfg(test)]
    fn take_upload_breakdown(&mut self) -> (std::time::Duration, std::time::Duration) {
        (
            std::mem::take(&mut self.derive_time),
            std::mem::take(&mut self.convert_time),
        )
    }

    /// Converts BGRA to NV12 and copies it into the surface.
    ///
    /// `vaDeriveImage` maps the surface directly, so this is the only copy of
    /// the frame; `vaCreateImage` + `vaGetImage` would read the surface back
    /// first, which is pure waste when every pixel is about to be overwritten.
    fn upload(&mut self, index: usize, pixels: &[u8], pitch: usize) -> Result<(), EncodeError> {
        #[cfg(test)]
        {
            self.uploads += 1;
        }
        let derive_started = std::time::Instant::now();
        let surface = self.surfaces[index]
            .as_ref()
            .ok_or_else(|| EncodeError::Backend("编码表面已借出".into()))?;
        let mut image = cros_libva::Image::derive_from(surface, (self.width, self.height))
            .map_err(|error| EncodeError::Backend(format!("derive_from: {error}")))?;
        let info = image.image();
        if info.format.fourcc != cros_libva::VA_FOURCC_NV12 {
            return Err(EncodeError::Backend(format!(
                "驱动未提供 NV12 图像，而是 {:#x}",
                info.format.fourcc
            )));
        }
        let (width, height) = (self.width as usize, self.height as usize);
        let (luma_offset, luma_pitch) = (info.offsets[0] as usize, info.pitches[0] as usize);
        let (chroma_offset, chroma_pitch) = (info.offsets[1] as usize, info.pitches[1] as usize);
        let derive_done = std::time::Instant::now();
        self.derive_time += derive_done - derive_started;
        let bytes = image.as_mut();
        bgra_to_nv12(
            pixels,
            pitch,
            width,
            height,
            bytes,
            luma_offset,
            luma_pitch,
            chroma_offset,
            chroma_pitch,
        )?;
        self.convert_time += derive_done.elapsed();
        // Dropping unmaps the derived view; the writes above went straight into
        // the surface, so there is nothing to copy back.
        drop(image);
        Ok(())
    }

    /// Drives `vaBeginPicture`/`vaRenderPicture`/`vaEndPicture` for one frame.
    fn encode_surface(
        &mut self,
        index: usize,
        idr: bool,
        timestamp: i64,
    ) -> Result<Vec<u8>, EncodeError> {
        let surface = self.surfaces[index]
            .take()
            .ok_or_else(|| EncodeError::Backend("编码表面已借出".into()))?;
        let (data, surface) = self.encode_picture(index, idr, timestamp, surface)?;
        self.surfaces[index] = Some(surface);
        Ok(data)
    }

    fn encode_picture<D: cros_libva::SurfaceMemoryDescriptor>(
        &mut self,
        index: usize,
        idr: bool,
        timestamp: i64,
        surface: Surface<D>,
    ) -> Result<(Vec<u8>, Surface<D>), EncodeError> {
        let coded = self
            .context
            .create_enc_coded(coded_buffer_size(self.width, self.height))
            .map_err(|error| EncodeError::Backend(format!("create_enc_coded: {error}")))?;
        let buffers = self.parameter_buffers(index, &coded, idr)?;
        let mut picture = Picture::new(timestamp.max(0) as u64, Rc::clone(&self.context), surface);
        for buffer in buffers {
            let buffer = self
                .context
                .create_buffer(buffer)
                .map_err(|error| EncodeError::Backend(format!("create_buffer: {error}")))?;
            picture.add_buffer(buffer);
        }
        let picture = picture
            .begin()
            .map_err(|error| EncodeError::Backend(format!("begin: {error}")))?
            .render()
            .map_err(|error| EncodeError::Backend(format!("render: {error}")))?
            .end()
            .map_err(|error| EncodeError::Backend(format!("end: {error}")))?;
        let picture = picture
            .sync()
            .map_err(|(error, _)| EncodeError::Backend(error.to_string()))?;
        // `vaEndPicture` filled the coded buffer, so the picture is done with.
        // Reclaim the surface so the next frame can reuse the slot.
        let surface = picture
            .take_surface()
            .unwrap_or_else(|_| unreachable!("the picture owns its surface alone"));
        let mapped = MappedCodedBuffer::new(&coded)?;
        let mut data = Vec::new();
        for segment in mapped.iter() {
            tracing::debug!(
                bit_offset = segment.bit_offset,
                status = segment.status,
                len = segment.buf.len(),
                "coded segment"
            );
            data.extend_from_slice(segment.buf);
        }
        if data.is_empty() {
            return Err(EncodeError::Backend("编码器未输出码流".into()));
        }
        Ok((data, surface))
    }

    /// The parameter buffers for one picture, in submission order.
    fn parameter_buffers(
        &mut self,
        index: usize,
        coded: &EncCodedBuffer,
        idr: bool,
    ) -> Result<Vec<BufferType>, EncodeError> {
        let mut buffers = Vec::new();

        if self.sequence_dirty || idr {
            buffers.push(BufferType::EncSequenceParameter(
                cros_libva::EncSequenceParameter::H264(self.sequence_parameter()),
            ));
            // The driver writes the slice but not the parameter sets. Submit the
            // SPS/PPS as raw packed headers, which is the only form i965 takes:
            // asking it to generate them yields "Invalid packed header data".
            buffers.push(BufferType::EncPackedHeaderParameter(
                EncPackedHeaderParameter::new(
                    cros_libva::VAEncPackedHeaderTypeH264::VAEncPackedHeaderH264_SPS,
                    (self.parameter_sets.len() * 8) as u32,
                    true,
                ),
            ));
            buffers.push(BufferType::EncPackedHeaderData(self.parameter_sets.clone()));
        }
        if self.rc_dirty {
            buffers.push(BufferType::EncMiscParameter(EncMiscParameter::FrameRate(
                EncMiscParameterFrameRate::new(u32::from(self.rate.fps), 0),
            )));
            buffers.push(BufferType::EncMiscParameter(
                EncMiscParameter::MaxFrameSize(EncMiscParameterBufferMaxFrameSize::new(
                    // libva specifies bits, while our allocation is in bytes.
                    (coded_buffer_size(self.width, self.height) * 8) as u32,
                )),
            ));
            buffers.push(BufferType::EncMiscParameter(EncMiscParameter::RateControl(
                EncMiscParameterRateControl::new(
                    self.rate.target,
                    100,
                    RC_WINDOW_MS,
                    u32::from(INITIAL_QP),
                    1,
                    0,
                    RcFlags::new(0, 1, 0, 0, 0, 0, 0, 0, 0),
                    0,
                    51,
                    0,
                    0,
                ),
            )));
        }

        let current = PictureH264::new(
            self.ids[index],
            u32::from(self.frame_num),
            PICTURE_SHORT_TERM,
            i32::from(self.frame_num) * 2,
            i32::from(self.frame_num) * 2,
        );
        // Every unused reference slot must be explicitly invalid; a zero id
        // reads as "surface 0", which the driver rejects.
        let mut references = std::array::from_fn(|_| invalid_picture());
        let mut list0 = std::array::from_fn(|_| invalid_picture());
        if let Some(reference) = self.reference.filter(|_| !idr) {
            let previous = self.frame_num.wrapping_sub(1) & 0x7fff;
            references[0] = PictureH264::new(
                self.ids[reference],
                u32::from(previous),
                PICTURE_SHORT_TERM,
                i32::from(previous) * 2,
                i32::from(previous) * 2,
            );
            list0[0] = PictureH264::new(
                self.ids[reference],
                u32::from(previous),
                PICTURE_SHORT_TERM,
                i32::from(previous) * 2,
                i32::from(previous) * 2,
            );
        }
        let picture_parameter = EncPictureParameterBufferH264::new(
            current,
            references,
            coded.id(),
            0,
            0,
            0,
            self.frame_num,
            INITIAL_QP,
            0,
            0,
            0,
            0,
            &H264EncPicFields::new(u32::from(idr), 1, 0, 0, 0, 0, 0, 1, 0, 0, 0),
        );
        buffers.push(BufferType::EncPictureParameter(
            cros_libva::EncPictureParameter::H264(picture_parameter),
        ));

        let slice = EncSliceParameterBufferH264::new(
            0,
            self.width.div_ceil(16) * self.height.div_ceil(16),
            0,
            if idr { 2 } else { 0 },
            0,
            self.idr_pic_id,
            0,
            0,
            [0, 0],
            1,
            0,
            0,
            0,
            list0,
            std::array::from_fn(|_| invalid_picture()),
            0,
            0,
            0,
            [0; 32],
            [0; 32],
            0,
            [[0; 2]; 32],
            [[0; 2]; 32],
            0,
            [0; 32],
            [0; 32],
            0,
            [[0; 2]; 32],
            [[0; 2]; 32],
            0,
            0,
            0,
            0,
            0,
        );
        buffers.push(BufferType::EncSliceParameter(
            cros_libva::EncSliceParameter::H264(slice),
        ));
        Ok(buffers)
    }

    fn sequence_parameter(&self) -> EncSequenceParameterBufferH264 {
        // 4:2:0, progressive, no scaling matrices. The frame_num and POC widths
        // must match the SPS in `parameter_sets`, or the driver's slice header
        // disagrees with what the decoder reads.
        let seq_fields = H264EncSeqFields::new(
            1,
            1,
            0,
            0,
            1,
            LOG2_MAX_FRAME_NUM_MINUS4,
            0,
            LOG2_MAX_POC_LSB_MINUS4,
            0,
        );
        // Timing info lets the receiver derive the frame rate from the VUI.
        let vui = H264VuiFields::new(0, 1, 0, 0, 0, 0, 0, 0);
        EncSequenceParameterBufferH264::new(
            0,
            LEVEL_IDC,
            0,
            u32::from(self.rate.fps) * 2,
            1,
            self.rate.target,
            1,
            self.width.div_ceil(16) as u16,
            self.height.div_ceil(16) as u16,
            &seq_fields,
            0,
            0,
            0,
            0,
            0,
            [0; 256],
            None,
            Some(vui),
            1,
            1,
            1,
            1,
            u32::from(self.rate.fps) * 2,
        )
    }
}

/// A coded buffer must hold the worst case for the frame. A quarter of the raw
/// size is far above what a rate-controlled stream reaches.
fn coded_buffer_size(width: u32, height: u32) -> usize {
    (width as usize * height as usize * 3 / 2 / 4).max(1 << 20)
}

/// Converts packed BGRA to NV12 straight into the driver's image.
///
/// The planes are written through their real strides, so the driver's padding
/// stays untouched. BT.601 limited range matches the software encoder and the
/// VUI in the parameter sets.
#[allow(clippy::too_many_arguments)]
fn bgra_to_nv12(
    pixels: &[u8],
    pitch: usize,
    width: usize,
    height: usize,
    out: &mut [u8],
    luma_offset: usize,
    luma_pitch: usize,
    chroma_offset: usize,
    chroma_pitch: usize,
) -> Result<(), EncodeError> {
    let (luma_end, chroma_end) = (
        luma_offset + luma_pitch * height,
        chroma_offset + chroma_pitch * height.div_ceil(2),
    );
    let end = luma_end.max(chroma_end);
    if out.len() < end {
        return Err(EncodeError::Backend(format!(
            "NV12 图像太小：{} < {end}",
            out.len()
        )));
    }
    // The planes are written through the whole slice; the conversion only
    // touches the first `height` rows of each, which is all the driver reads.
    let (luma, chroma) = out.split_at_mut(chroma_offset.min(out.len()));
    let luma_start = luma_offset.min(luma.len());
    let luma = &mut luma[luma_start..];
    yuv::bgra_to_yuv_nv12(
        &mut yuv::YuvBiPlanarImageMut {
            y_plane: yuv::BufferStoreMut::Borrowed(luma),
            y_stride: luma_pitch as u32,
            uv_plane: yuv::BufferStoreMut::Borrowed(chroma),
            uv_stride: chroma_pitch as u32,
            width: width as u32,
            height: height as u32,
        },
        pixels,
        u32::try_from(pitch).map_err(|_| EncodeError::Backend("行距过大".into()))?,
        yuv::YuvRange::Limited,
        yuv::YuvStandardMatrix::Bt601,
        yuv::YuvConversionMode::Balanced,
    )
    .map_err(|error| EncodeError::Backend(error.to_string()))
}

/// An unused picture slot. VA-API wants `VA_INVALID_ID` plus the invalid flag
/// rather than a zeroed entry, which would name surface 0.
fn invalid_picture() -> PictureH264 {
    PictureH264::new(
        cros_libva::VA_INVALID_ID,
        0,
        cros_libva::VA_PICTURE_H264_INVALID,
        0,
        0,
    )
}

/// The packed-header modes the driver advertises for H.264 encode.
///
/// `vaCreateConfig` must be told which of them to enable; the sequence mode is
/// what makes the driver emit SPS/PPS of its own.
fn packed_headers(display: &Rc<Display>) -> Result<u32, EncodeError> {
    let mut attributes = [VAConfigAttrib {
        type_: VAConfigAttribType::VAConfigAttribEncPackedHeaders,
        value: 0,
    }];
    display
        .get_config_attributes(
            VAProfile::VAProfileH264High,
            VAEntrypoint::VAEntrypointEncSlice,
            &mut attributes,
        )
        .map_err(|error| EncodeError::Unsupported(error.to_string()))?;
    let supported = attributes[0].value;
    if supported == cros_libva::VA_ATTRIB_NOT_SUPPORTED {
        return Err(EncodeError::Unsupported("驱动不支持 packed header".into()));
    }
    // Sequence headers carry SPS, picture headers carry PPS.
    Ok(supported
        & (cros_libva::VA_ENC_PACKED_HEADER_SEQUENCE | cros_libva::VA_ENC_PACKED_HEADER_PICTURE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn vaapi_cached_inputs_survive_content_rate_and_keyframe_changes() {
        let display = crate::platform::linux::decoder::shared_display().unwrap();
        let (width, height) = (768, 480);
        let rate = Rate {
            target: 4_000_000,
            peak: 4_000_000,
            fps: 30,
            quality: 4,
            quality_target: crate::media::encoding::QualityTarget {
                bitrate: 4_000_000,
                fps: 30,
            },
        };
        let mut encoder = Encoder::new(&display, (width, height), Format::AVC, rate).unwrap();
        let mut pixels = Arc::new(vec![80; (width * height * 4) as usize]);
        let borrowed = vec![40; pixels.len()];
        let mut decoder = openuuyc_h264::stream::Decoder::new();
        let mut decoded = Vec::new();
        for index in 0..90 {
            if index == 25 {
                Arc::make_mut(&mut pixels).fill(180);
            }
            let target = if (30..60).contains(&index) {
                300_000
            } else {
                4_000_000
            };
            let key = encoder
                .configure_rate(Rate {
                    target,
                    peak: target,
                    ..rate
                })
                .unwrap();
            let timestamp = i64::from(index) * 333_333;
            let packets = if (50..60).contains(&index) {
                encoder.encode(&borrowed, width as usize * 4, timestamp, key)
            } else {
                encoder.encode_shared(&pixels, timestamp, key || index == 20)
            }
            .unwrap();
            for packet in packets {
                decoded.extend(decoder.submit(&packet.data, index as u64).unwrap());
            }
        }
        decoded.extend(decoder.finish().unwrap());
        assert_eq!(decoded.len(), 90);
        assert_eq!(
            encoder.uploads, 22,
            "four slots per Arc, plus uncached writes and restoration"
        );
        for (index, picture) in decoded.iter().enumerate() {
            let value = if index < 25 {
                80
            } else if (50..60).contains(&index) {
                40
            } else {
                180
            };
            let expected = ((220 * value + 128) / 256 + 16) as u8;
            let mut nv12 = Vec::new();
            picture.picture.pack_into(&mut nv12).unwrap();
            let error = nv12[..(width * height) as usize]
                .iter()
                .map(|v| u64::from(v.abs_diff(expected)))
                .sum::<u64>() as f64
                / f64::from(width * height);
            assert!(
                error < 5.,
                "frame {index}: stale/corrupt cached input MAE={error}"
            );
        }
        println!(
            "cached input: 90 decoded frames, {} uploads; mutation/rate/IDR/uncached overwrite passed",
            encoder.uploads
        );
    }

    #[test]
    #[ignore]
    fn vaapi_native_repeated_input_throughput() {
        let display = crate::platform::linux::decoder::shared_display().unwrap();
        let (width, height) = (2560, 1600);
        let rate = Rate {
            target: 4_000_000,
            peak: 4_000_000,
            fps: 30,
            quality: 4,
            quality_target: crate::media::encoding::QualityTarget {
                bitrate: 4_000_000,
                fps: 30,
            },
        };
        let pixels = Arc::new(
            (0..width * height)
                .flat_map(|p| {
                    let x = p % width;
                    let y = p / width;
                    [(x % 251) as u8, (y % 241) as u8, ((x ^ y) % 239) as u8, 255]
                })
                .collect::<Vec<_>>(),
        );
        for cached in [false, true] {
            let mut encoder = Encoder::new(&display, (width, height), Format::AVC, rate).unwrap();
            for index in 0..4 {
                if cached {
                    encoder.encode_shared(&pixels, index * 333_333, false)
                } else {
                    encoder.encode(&pixels, width as usize * 4, index * 333_333, false)
                }
                .unwrap();
            }
            encoder.take_stage_totals();
            encoder.take_upload_breakdown();
            encoder.uploads = 0;
            let start = std::time::Instant::now();
            for index in 4..64 {
                if cached {
                    encoder.encode_shared(&pixels, index * 333_333, false)
                } else {
                    encoder.encode(&pixels, width as usize * 4, index * 333_333, false)
                }
                .unwrap();
            }
            let (upload, encode) = encoder.take_stage_totals();
            println!(
                "native repeated input cached={cached}: {:.2} ms/frame, upload={:.2} encode={:.2} uploads={}",
                start.elapsed().as_secs_f64() * 1000. / 60.,
                upload.as_secs_f64() * 1000. / 60.,
                encode.as_secs_f64() * 1000. / 60.,
                encoder.uploads
            );
            assert_eq!(encoder.uploads, if cached { 0 } else { 60 });
        }
    }

    /// Exercise prediction and rate recovery, not only the first solid IDR.
    /// Run on the deployed GPU with `--ignored vaapi_rate_recovery --nocapture`.
    #[test]
    #[ignore]
    fn vaapi_rate_recovery_preserves_predicted_pictures() {
        let display = crate::platform::linux::decoder::shared_display().unwrap();
        let (width, height) = (768u32, 480u32);
        let rate = Rate {
            target: 4_000_000,
            peak: 4_000_000,
            fps: 30,
            quality: 4,
            quality_target: crate::media::encoding::QualityTarget {
                bitrate: 4_000_000,
                fps: 30,
            },
        };
        let mut encoder = Encoder::new(&display, (width, height), Format::AVC, rate).unwrap();
        let mut decoder = openuuyc_h264::stream::Decoder::new();
        let mut expected = Vec::new();
        let mut decoded = Vec::new();
        let mut bytes = [0usize; 3];
        for index in 0..90 {
            let target = if (30..60).contains(&index) {
                300_000
            } else {
                4_000_000
            };
            let key = encoder
                .configure_rate(Rate {
                    target,
                    peak: target,
                    ..rate
                })
                .unwrap();
            let mut pixels = Vec::with_capacity((width * height * 4) as usize);
            let mut luma = Vec::with_capacity((width * height) as usize);
            for y in 0..height {
                for x in 0..width {
                    // Moving fine details expose reference drift and macroblock damage.
                    let value = (((x + index * 3) / 5 + y / 5) % 2 * 160 + 40) as u8;
                    pixels.extend_from_slice(&[value, value, value, 255]);
                    luma.push(((220 * u32::from(value) + 128) / 256 + 16) as u8);
                }
            }
            expected.push(luma);
            for output in encoder
                .encode(&pixels, width as usize * 4, i64::from(index) * 333_333, key)
                .unwrap()
            {
                bytes[index as usize / 30] += output.data.len();
                decoded.extend(decoder.submit(&output.data, u64::from(index)).unwrap());
            }
        }
        decoded.extend(decoder.finish().unwrap());
        assert_eq!(decoded.len(), expected.len());
        let mut errors = [0.0f64; 3];
        for (index, picture) in decoded.iter().enumerate() {
            assert_eq!(
                (picture.picture.crop.width, picture.picture.crop.height),
                (width as usize, height as usize)
            );
            let mut nv12 = Vec::new();
            picture.picture.pack_into(&mut nv12).unwrap();
            let error = nv12
                .iter()
                .zip(&expected[index])
                .map(|(a, b)| a.abs_diff(*b) as u64)
                .sum::<u64>() as f64
                / f64::from(width * height);
            errors[index / 30] += error / 30.;
        }
        println!("VAAPI recovery: size={width}x{height} bytes={bytes:?} luma_mae={errors:?}");
        assert!(
            errors[0] < 12. && errors[2] < 12.,
            "prediction/recovery corrupted the high-rate picture: {errors:?}"
        );
    }

    /// Real desktop detail and native geometry, without any network loss.
    /// Keep the same captured input across a rate collapse and recovery so a
    /// damaged static reference cannot hide behind replacement desktop frames.
    #[test]
    #[ignore]
    fn vaapi_native_desktop_recovers_without_a_new_capture_or_idr() {
        use crate::platform::linux::capture::{Desktop, Image, screens};
        let screens = screens().unwrap();
        let screen = screens.iter().find(|s| s.primary).unwrap_or(&screens[0]);
        let mut desktop = Desktop::open_selected(screen).unwrap();
        let frame = (0..100)
            .find_map(|_| desktop.next(50, 4, false, false, (3840, 2160)).unwrap())
            .expect("合成器未交付画面");
        let Image::Cpu { pixels, .. } = &frame.image else {
            panic!("此探针要求 wlr 的 BGRA CPU 画面");
        };
        let (width, height) = (frame.width as usize, frame.height as usize);
        let luma = width * height;
        let mut expected = vec![0; luma * 3 / 2];
        bgra_to_nv12(
            pixels,
            width * 4,
            width,
            height,
            &mut expected,
            0,
            width,
            luma,
            width,
        )
        .unwrap();
        let rate = Rate {
            target: 4_000_000,
            peak: 4_000_000,
            fps: 30,
            quality: 4,
            quality_target: crate::media::encoding::QualityTarget {
                bitrate: 4_000_000,
                fps: 30,
            },
        };
        let display = crate::platform::linux::decoder::shared_display().unwrap();
        let mut encoder =
            Encoder::new(&display, (frame.width, frame.height), Format::AVC, rate).unwrap();
        let mut decoder = openuuyc_h264::stream::Decoder::new();
        let mut errors = Vec::new();
        let mut bytes = [0; 3];
        let mut measure = |picture: openuuyc_h264::stream::Output| {
            let mut nv12 = Vec::new();
            picture.picture.pack_into(&mut nv12).unwrap();
            assert_eq!(nv12.len(), expected.len());
            let mae = |range: std::ops::Range<usize>| {
                range
                    .clone()
                    .map(|i| nv12[i].abs_diff(expected[i]) as u64)
                    .sum::<u64>() as f64
                    / range.len() as f64
            };
            errors.push((mae(0..luma), mae(luma..expected.len())));
        };
        for index in 0..90u64 {
            let target = if (30..60).contains(&index) {
                200_000
            } else {
                4_000_000
            };
            let key = encoder
                .configure_rate(Rate {
                    target,
                    peak: target,
                    ..rate
                })
                .unwrap();
            assert!(!key, "仅码率变化不应靠 IDR 掩盖恢复问题");
            for output in encoder
                .encode_shared(pixels, index as i64 * 333_333, key)
                .unwrap()
            {
                bytes[index as usize / 30] += output.data.len();
                for picture in decoder.submit(&output.data, index).unwrap() {
                    measure(picture);
                }
            }
        }
        for picture in decoder.finish().unwrap() {
            measure(picture);
        }
        assert_eq!(errors.len(), 90);
        let settled: Vec<(f64, f64)> = (0..3)
            .map(|stage| {
                let samples = &errors[stage * 30 + 20..stage * 30 + 30];
                (
                    samples.iter().map(|e| e.0).sum::<f64>() / 10.,
                    samples.iter().map(|e| e.1).sum::<f64>() / 10.,
                )
            })
            .collect();
        println!(
            "native static recovery: {width}x{height} bytes={bytes:?} settled_y_uv_mae={settled:?}"
        );
        assert!(
            settled[2].0 < 8. && settled[2].1 < 8.,
            "恢复后仍严重失真：{settled:?}"
        );
        assert!(
            settled[2].0 <= settled[0].0 + 1. && settled[2].1 <= settled[0].1 + 1.,
            "恢复后参考画面未恢复到原始质量：{settled:?}"
        );
    }

    /// Needs an Intel GPU with the encode entrypoint:
    /// `cargo test -- --ignored vaapi_encodes_a_decodable_stream`.
    ///
    /// The point is not the bitrate but that the driver's output parses back
    /// into the picture we handed it, and that the colour matrix is the BT.601
    /// limited range the stream's VUI declares.
    #[test]
    #[ignore]
    fn vaapi_encodes_a_decodable_stream() {
        let display = crate::platform::linux::decoder::shared_display().unwrap();
        let (width, height) = (320u32, 240u32);
        let rate = Rate {
            target: 4_000_000,
            peak: 4_000_000,
            fps: 30,
            quality: 4,
            quality_target: crate::media::encoding::QualityTarget {
                bitrate: 4_000_000,
                fps: 30,
            },
        };
        for (name, bgr, bt601) in [
            ("red", [0u8, 0, 255], (81, 90, 240)),
            ("green", [0, 255, 0], (145, 54, 34)),
            ("blue", [255, 0, 0], (41, 240, 110)),
            ("white", [255, 255, 255], (235, 128, 128)),
        ] {
            let mut pixels = Vec::with_capacity((width * height * 4) as usize);
            for _ in 0..width * height {
                pixels.extend_from_slice(&[bgr[0], bgr[1], bgr[2], 255]);
            }
            let mut encoder = Encoder::new(
                &display,
                (width, height),
                crate::media::encoding::Format::AVC,
                rate,
            )
            .unwrap();
            let encoded = encoder
                .encode(&pixels, width as usize * 4, 10_000_000, true)
                .unwrap();
            assert_eq!(encoded[0].timestamp_100ns, 10_000_000);
            assert!(!encoded.is_empty() && !encoded[0].data.is_empty());
            // The driver emits SPS and PPS itself only when asked; a stream that
            // starts at the IDR slice cannot be decoded.
            assert_eq!(
                &encoded[0].data[..5],
                &[0, 0, 0, 1, 0x67],
                "{name}: 码流缺少 SPS"
            );
            let mut decoder = openuuyc_h264::stream::Decoder::new();
            let outputs = decoder.submit(&encoded[0].data, 0).unwrap();
            let outputs = if outputs.is_empty() {
                decoder.finish().unwrap()
            } else {
                outputs
            };
            assert!(!outputs.is_empty(), "{name}: 码流未能解出画面");
            let mut nv12 = Vec::new();
            outputs[0].picture.pack_into(&mut nv12).unwrap();
            let luma = (width * height) as usize;
            let (y, u, v) = (nv12[luma / 2 + 128], nv12[luma + 1000], nv12[luma + 1001]);
            println!("{name}: Y {y} U {u} V {v}   BT.601 {bt601:?}");
            let near = |a: u8, b: i32| (i32::from(a) - b).abs() <= 4;
            assert!(
                near(y, bt601.0) && near(u, bt601.1) && near(v, bt601.2),
                "{name} 不是 BT.601 有限范围"
            );
        }
    }

    /// Needs an Intel GPU: `cargo test -- --ignored vaapi_throughput`.
    ///
    /// Reports frames per second at the size a 4K desktop negotiates, which is
    /// what decides whether hardware encoding is worth the conversion and
    /// upload it costs when the capture side cannot hand over a dmabuf.
    #[test]
    #[ignore]
    fn vaapi_throughput_at_4k() {
        let display = crate::platform::linux::decoder::shared_display().unwrap();
        let (width, height) = (3342u32, 2160u32);
        let rate = Rate {
            target: 20_000_000,
            peak: 20_000_000,
            fps: 30,
            quality: 4,
            quality_target: crate::media::encoding::QualityTarget {
                bitrate: 20_000_000,
                fps: 30,
            },
        };
        let mut encoder = Encoder::new(
            &display,
            (width, height),
            crate::media::encoding::Format::AVC,
            rate,
        )
        .unwrap();
        // A flat colour compresses to nothing and hides how long the encode
        // really takes, so fill the frame with a moving pattern.
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let offset = (y * width as usize + x) * 4;
                pixels[offset] = (x % 251) as u8;
                pixels[offset + 1] = (y % 241) as u8;
                pixels[offset + 2] = ((x ^ y) % 239) as u8;
                pixels[offset + 3] = 0xff;
            }
        }
        // One warm-up frame so the driver's first-call setup stays out of the
        // measurement.
        encoder
            .encode(&pixels, width as usize * 4, 0, true)
            .unwrap();
        let frames = 30;
        let started = std::time::Instant::now();
        let mut bytes = 0usize;
        for index in 0..frames {
            let out = encoder
                .encode(
                    &pixels,
                    width as usize * 4,
                    i64::from(index) * 33_333,
                    false,
                )
                .unwrap();
            bytes += out.iter().map(|packet| packet.data.len()).sum::<usize>();
        }
        let elapsed = started.elapsed();
        println!(
            "vaapi {width}x{height}: {frames} frames in {elapsed:?} ({:.1} fps), {:.1} ms/frame, {} KiB",
            f64::from(frames) / elapsed.as_secs_f64(),
            elapsed.as_secs_f64() * 1000.0 / f64::from(frames),
            bytes / 1024
        );
        let (upload, encode) = encoder.take_stage_totals();
        let (derive, convert) = encoder.take_upload_breakdown();
        println!(
            "  upload {:.1} ms/frame (derive {:.1}, convert {:.1}), encode {:.1} ms/frame",
            upload.as_secs_f64() * 1000.0 / f64::from(frames),
            derive.as_secs_f64() * 1000.0 / f64::from(frames),
            convert.as_secs_f64() * 1000.0 / f64::from(frames),
            encode.as_secs_f64() * 1000.0 / f64::from(frames),
        );
    }
}
