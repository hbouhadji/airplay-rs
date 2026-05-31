use super::{Codec, VideoError};

#[cfg(feature = "macos-gstreamer")]
pub(super) struct SideBySideViews {
    parent: objc2::rc::Retained<objc2_app_kit::NSView>,
    native: objc2::rc::Retained<objc2_app_kit::NSView>,
    gstreamer: objc2::rc::Retained<objc2_app_kit::NSView>,
}

#[cfg(feature = "macos-gstreamer")]
impl SideBySideViews {
    pub(super) fn create(parent_handle: u64) -> Result<Self, VideoError> {
        use objc2::rc::Retained;
        use objc2_app_kit::{NSAutoresizingMaskOptions, NSView};
        use objc2_foundation::{CGPoint, CGRect, CGSize, MainThreadMarker};

        let mtm = MainThreadMarker::new()
            .ok_or_else(|| VideoError::new("side-by-side views must be created on main thread"))?;

        let parent = unsafe {
            Retained::retain(parent_handle as *mut NSView)
                .ok_or_else(|| VideoError::new("failed to retain parent NSView"))?
        };
        let bounds = parent.bounds();
        let half_width = bounds.size.width / 2.0;
        let left_frame = CGRect::new(bounds.origin, CGSize::new(half_width, bounds.size.height));
        let right_frame = CGRect::new(
            CGPoint::new(bounds.origin.x + half_width, bounds.origin.y),
            CGSize::new(bounds.size.width - half_width, bounds.size.height),
        );

        let native = unsafe { NSView::initWithFrame(mtm.alloc(), left_frame) };
        let gstreamer = unsafe { NSView::initWithFrame(mtm.alloc(), right_frame) };
        let resize_mask = NSAutoresizingMaskOptions::NSViewWidthSizable
            | NSAutoresizingMaskOptions::NSViewHeightSizable;

        unsafe {
            parent.setAutoresizesSubviews(false);
            native.setAutoresizingMask(resize_mask);
            gstreamer.setAutoresizingMask(resize_mask);
            parent.addSubview(&native);
            parent.addSubview(&gstreamer);
        }

        let views = Self {
            parent,
            native,
            gstreamer,
        };
        views.layout();
        Ok(views)
    }

    pub(super) fn native_handle(&self) -> u64 {
        (&*self.native as *const objc2_app_kit::NSView) as u64
    }

    pub(super) fn gstreamer_handle(&self) -> u64 {
        (&*self.gstreamer as *const objc2_app_kit::NSView) as u64
    }

    pub(super) fn layout(&self) {
        use objc2_foundation::{CGPoint, CGRect, CGSize};

        let bounds = self.parent.bounds();
        let half_width = bounds.size.width / 2.0;
        let left_frame = CGRect::new(bounds.origin, CGSize::new(half_width, bounds.size.height));
        let right_frame = CGRect::new(
            CGPoint::new(bounds.origin.x + half_width, bounds.origin.y),
            CGSize::new(bounds.size.width - half_width, bounds.size.height),
        );

        unsafe {
            self.native.setFrame(left_frame);
            self.gstreamer.setFrame(right_frame);
        }
    }

    pub(super) fn remove_from_parent(self) {
        unsafe {
            self.native.removeFromSuperview();
            self.gstreamer.removeFromSuperview();
        }
    }
}

pub(super) struct MacOsVideoPipeline {
    layer: objc2::rc::Retained<objc2::runtime::AnyObject>,
    format_description: CoreMediaRef<CMFormatDescription>,
    next_pts: i64,
}

impl MacOsVideoPipeline {
    pub(super) fn start(
        codec: Codec,
        codec_data: impl AsRef<[u8]> + Send + 'static,
        window_handle: Option<u64>,
    ) -> Result<Self, VideoError> {
        use objc2::{class, msg_send, msg_send_id, runtime::AnyObject};

        let ns_view = window_handle.ok_or_else(|| VideoError::new("missing AppKit view handle"))?
            as *mut AnyObject;
        let format_description = codec.create_format_description(codec_data.as_ref())?;

        let layer = unsafe {
            let layer: objc2::rc::Retained<AnyObject> =
                msg_send_id![class!(AVSampleBufferDisplayLayer), new];
            let _: () = msg_send![ns_view, setWantsLayer: objc2::runtime::Bool::YES];
            let _: () = msg_send![ns_view, setLayer: &*layer];
            layer
        };

        Ok(Self {
            layer,
            format_description,
            next_pts: 0,
        })
    }

    pub(super) fn push_payload(&mut self, payload: &[u8]) -> Result<(), VideoError> {
        use objc2::msg_send;

        let sample_buffer =
            create_sample_buffer(payload, self.format_description.as_ptr(), self.next_pts)?;
        self.next_pts += 1;

        unsafe {
            let _: () = msg_send![&*self.layer, enqueueSampleBuffer: sample_buffer.as_ptr()];
        }

        Ok(())
    }

    pub(super) fn stop(self) {
        use objc2::msg_send;

        unsafe {
            let _: () = msg_send![&*self.layer, flushAndRemoveImage];
        }
    }
}

impl Codec {
    fn create_format_description(
        self,
        codec_data: &[u8],
    ) -> Result<CoreMediaRef<CMFormatDescription>, VideoError> {
        let normalized = self.normalize_codec_data(codec_data);
        let config = match self {
            Self::H264 => ParameterSets::from_avcc(normalized.as_ref())?,
            Self::H265 => ParameterSets::from_hvcc(normalized.as_ref())?,
        };
        let mut format_description = std::ptr::null_mut();
        let pointers = config.parameter_set_pointers();
        let sizes = config.parameter_set_sizes();

        let status = unsafe {
            match self {
                Self::H264 => CMVideoFormatDescriptionCreateFromH264ParameterSets(
                    std::ptr::null(),
                    pointers.len(),
                    pointers.as_ptr(),
                    sizes.as_ptr(),
                    config.nal_unit_header_length as i32,
                    &mut format_description,
                ),
                Self::H265 => CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                    std::ptr::null(),
                    pointers.len(),
                    pointers.as_ptr(),
                    sizes.as_ptr(),
                    config.nal_unit_header_length as i32,
                    std::ptr::null(),
                    &mut format_description,
                ),
            }
        };
        core_media_check(status, "create video format description")?;
        CoreMediaRef::new(format_description, "empty video format description")
    }
}

struct ParameterSets {
    nal_unit_header_length: u8,
    sets: Vec<Vec<u8>>,
}

impl ParameterSets {
    fn from_avcc(data: &[u8]) -> Result<Self, VideoError> {
        if data.len() < 7 {
            return Err(VideoError::new("invalid avcC codec data"));
        }

        let nal_unit_header_length = (data[4] & 0x03) + 1;
        let mut offset = 6;
        let mut sets = Vec::new();

        let sps_count = data[5] & 0x1f;
        for _ in 0..sps_count {
            sets.push(read_length_prefixed_parameter_set(data, &mut offset)?);
        }

        if offset >= data.len() {
            return Err(VideoError::new("avcC codec data is missing PPS entries"));
        }

        let pps_count = data[offset];
        offset += 1;
        for _ in 0..pps_count {
            sets.push(read_length_prefixed_parameter_set(data, &mut offset)?);
        }

        if sps_count == 0 || pps_count == 0 {
            return Err(VideoError::new("avcC codec data must include SPS and PPS"));
        }

        Ok(Self {
            nal_unit_header_length,
            sets,
        })
    }

    fn from_hvcc(data: &[u8]) -> Result<Self, VideoError> {
        if data.len() < 23 {
            return Err(VideoError::new("invalid hvcC codec data"));
        }

        let nal_unit_header_length = (data[21] & 0x03) + 1;
        let array_count = data[22];
        let mut offset = 23;
        let mut sets = Vec::new();
        let mut has_vps = false;
        let mut has_sps = false;
        let mut has_pps = false;

        for _ in 0..array_count {
            if offset + 3 > data.len() {
                return Err(VideoError::new("truncated hvcC parameter array"));
            }

            let nal_unit_type = data[offset] & 0x3f;
            offset += 1;
            let nal_count = u16::from_be_bytes([data[offset], data[offset + 1]]);
            offset += 2;

            for _ in 0..nal_count {
                let parameter_set = read_length_prefixed_parameter_set(data, &mut offset)?;
                match nal_unit_type {
                    32 => has_vps = true,
                    33 => has_sps = true,
                    34 => has_pps = true,
                    _ => {}
                }
                sets.push(parameter_set);
            }
        }

        if !has_vps || !has_sps || !has_pps {
            return Err(VideoError::new(
                "hvcC codec data must include VPS, SPS and PPS",
            ));
        }

        Ok(Self {
            nal_unit_header_length,
            sets,
        })
    }

    fn parameter_set_pointers(&self) -> Vec<*const u8> {
        self.sets
            .iter()
            .map(|parameter_set| parameter_set.as_ptr())
            .collect()
    }

    fn parameter_set_sizes(&self) -> Vec<usize> {
        self.sets
            .iter()
            .map(|parameter_set| parameter_set.len())
            .collect()
    }
}

fn read_length_prefixed_parameter_set(
    data: &[u8],
    offset: &mut usize,
) -> Result<Vec<u8>, VideoError> {
    if *offset + 2 > data.len() {
        return Err(VideoError::new("truncated codec parameter set length"));
    }

    let length = u16::from_be_bytes([data[*offset], data[*offset + 1]]) as usize;
    *offset += 2;

    let end = offset
        .checked_add(length)
        .ok_or_else(|| VideoError::new("codec parameter set length overflow"))?;
    if end > data.len() {
        return Err(VideoError::new("truncated codec parameter set"));
    }

    let parameter_set = data[*offset..end].to_vec();
    *offset = end;
    Ok(parameter_set)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

#[repr(C)]
struct CMSampleTimingInfo {
    duration: CMTime,
    presentation_time_stamp: CMTime,
    decode_time_stamp: CMTime,
}

#[repr(C)]
struct CMBlockBuffer {
    inner: [u8; 0],
    _p: std::cell::UnsafeCell<
        std::marker::PhantomData<(*const std::cell::UnsafeCell<()>, std::marker::PhantomPinned)>,
    >,
}

#[repr(C)]
struct CMFormatDescription {
    inner: [u8; 0],
    _p: std::cell::UnsafeCell<
        std::marker::PhantomData<(*const std::cell::UnsafeCell<()>, std::marker::PhantomPinned)>,
    >,
}

#[repr(C)]
struct CMSampleBuffer {
    inner: [u8; 0],
    _p: std::cell::UnsafeCell<
        std::marker::PhantomData<(*const std::cell::UnsafeCell<()>, std::marker::PhantomPinned)>,
    >,
}

unsafe impl objc2::RefEncode for CMBlockBuffer {
    const ENCODING_REF: objc2::encode::Encoding = objc2::encode::Encoding::Pointer(
        &objc2::encode::Encoding::Struct("OpaqueCMBlockBuffer", &[]),
    );
}

unsafe impl objc2::RefEncode for CMFormatDescription {
    const ENCODING_REF: objc2::encode::Encoding = objc2::encode::Encoding::Pointer(
        &objc2::encode::Encoding::Struct("opaqueCMFormatDescription", &[]),
    );
}

unsafe impl objc2::RefEncode for CMSampleBuffer {
    const ENCODING_REF: objc2::encode::Encoding = objc2::encode::Encoding::Pointer(
        &objc2::encode::Encoding::Struct("opaqueCMSampleBuffer", &[]),
    );
}

struct CoreMediaRef<T> {
    ptr: *mut T,
}

impl<T> CoreMediaRef<T> {
    fn new(ptr: *mut T, empty_message: &'static str) -> Result<Self, VideoError> {
        if ptr.is_null() {
            Err(VideoError::new(empty_message))
        } else {
            Ok(Self { ptr })
        }
    }

    fn as_ptr(&self) -> *mut T {
        self.ptr
    }
}

impl<T> Drop for CoreMediaRef<T> {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.ptr.cast());
        }
    }
}

fn create_sample_buffer(
    payload: &[u8],
    format_description: *mut CMFormatDescription,
    pts: i64,
) -> Result<CoreMediaRef<CMSampleBuffer>, VideoError> {
    if payload.is_empty() {
        return Err(VideoError::new("empty video payload"));
    }

    let mut block_buffer = std::ptr::null_mut();
    let status = unsafe {
        CMBlockBufferCreateWithMemoryBlock(
            std::ptr::null(),
            std::ptr::null_mut(),
            payload.len(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            payload.len(),
            K_CM_BLOCK_BUFFER_ASSURE_MEMORY_NOW_FLAG,
            &mut block_buffer,
        )
    };
    core_media_check(status, "create CMBlockBuffer")?;
    let block_buffer = CoreMediaRef::<CMBlockBuffer>::new(block_buffer, "empty CMBlockBuffer")?;

    let status = unsafe {
        CMBlockBufferReplaceDataBytes(
            payload.as_ptr().cast(),
            block_buffer.as_ptr().cast(),
            0,
            payload.len(),
        )
    };
    core_media_check(status, "copy video payload into CMBlockBuffer")?;

    let timing = CMSampleTimingInfo {
        duration: cm_time(1, 60),
        presentation_time_stamp: cm_time(pts, 60),
        decode_time_stamp: cm_time_invalid(),
    };
    let sample_size = payload.len();
    let mut sample_buffer = std::ptr::null_mut();
    let status = unsafe {
        CMSampleBufferCreateReady(
            std::ptr::null(),
            block_buffer.as_ptr().cast(),
            format_description.cast(),
            1,
            1,
            &timing,
            1,
            &sample_size,
            &mut sample_buffer,
        )
    };
    core_media_check(status, "create CMSampleBuffer")?;
    mark_sample_buffer_display_immediately(sample_buffer);
    CoreMediaRef::new(sample_buffer, "empty CMSampleBuffer")
}

fn mark_sample_buffer_display_immediately(sample_buffer: *mut CMSampleBuffer) {
    unsafe {
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sample_buffer, true);
        if attachments.is_null() {
            return;
        }

        let first_attachment = CFArrayGetValueAtIndex(attachments, 0);
        if first_attachment.is_null() {
            return;
        }

        CFDictionarySetValue(
            first_attachment,
            kCMSampleAttachmentKey_DisplayImmediately,
            kCFBooleanTrue,
        );
    }
}

fn cm_time(value: i64, timescale: i32) -> CMTime {
    CMTime {
        value,
        timescale,
        flags: K_CM_TIME_FLAGS_VALID,
        epoch: 0,
    }
}

fn cm_time_invalid() -> CMTime {
    CMTime {
        value: 0,
        timescale: 0,
        flags: 0,
        epoch: 0,
    }
}

fn core_media_check(status: i32, action: &'static str) -> Result<(), VideoError> {
    if status == 0 {
        Ok(())
    } else {
        Err(VideoError::new(format!(
            "{action} failed with OSStatus {status}"
        )))
    }
}

const K_CM_TIME_FLAGS_VALID: u32 = 1 << 0;
const K_CM_BLOCK_BUFFER_ASSURE_MEMORY_NOW_FLAG: u32 = 1 << 0;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: *const std::ffi::c_void;

    fn CFArrayGetValueAtIndex(
        the_array: *const std::ffi::c_void,
        idx: isize,
    ) -> *const std::ffi::c_void;
    fn CFDictionarySetValue(
        the_dict: *const std::ffi::c_void,
        key: *const std::ffi::c_void,
        value: *const std::ffi::c_void,
    );
    fn CFRelease(cf: *const std::ffi::c_void);
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    static kCMSampleAttachmentKey_DisplayImmediately: *const std::ffi::c_void;

    fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
        allocator: *const std::ffi::c_void,
        parameter_set_count: usize,
        parameter_set_pointers: *const *const u8,
        parameter_set_sizes: *const usize,
        nal_unit_header_length: i32,
        format_description_out: *mut *mut CMFormatDescription,
    ) -> i32;

    fn CMVideoFormatDescriptionCreateFromHEVCParameterSets(
        allocator: *const std::ffi::c_void,
        parameter_set_count: usize,
        parameter_set_pointers: *const *const u8,
        parameter_set_sizes: *const usize,
        nal_unit_header_length: i32,
        extensions: *const std::ffi::c_void,
        format_description_out: *mut *mut CMFormatDescription,
    ) -> i32;

    fn CMBlockBufferCreateWithMemoryBlock(
        structure_allocator: *const std::ffi::c_void,
        memory_block: *mut std::ffi::c_void,
        block_length: usize,
        block_allocator: *const std::ffi::c_void,
        custom_block_source: *const std::ffi::c_void,
        offset_to_data: usize,
        data_length: usize,
        flags: u32,
        block_buffer_out: *mut *mut CMBlockBuffer,
    ) -> i32;

    fn CMBlockBufferReplaceDataBytes(
        source_bytes: *const std::ffi::c_void,
        destination_buffer: *mut std::ffi::c_void,
        offset_into_destination: usize,
        data_length: usize,
    ) -> i32;

    fn CMSampleBufferCreateReady(
        allocator: *const std::ffi::c_void,
        data_buffer: *mut std::ffi::c_void,
        format_description: *mut std::ffi::c_void,
        num_samples: isize,
        num_sample_timing_entries: isize,
        sample_timing_array: *const CMSampleTimingInfo,
        num_sample_size_entries: isize,
        sample_size_array: *const usize,
        sample_buffer_out: *mut *mut CMSampleBuffer,
    ) -> i32;

    fn CMSampleBufferGetSampleAttachmentsArray(
        sbuf: *mut CMSampleBuffer,
        create_if_necessary: bool,
    ) -> *const std::ffi::c_void;
}
