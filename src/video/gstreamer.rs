use super::{Codec, VideoError};

pub(super) struct GStreamerPipeline {
    appsrc: gstreamer_app::AppSrc,
    pipeline: gstreamer::Pipeline,
}

impl GStreamerPipeline {
    pub(super) fn start(
        codec: Codec,
        codec_data: impl AsRef<[u8]> + Send + 'static,
        window_handle: Option<u64>,
    ) -> Result<Self, VideoError> {
        use gstreamer as gst;
        use gstreamer::prelude::*;
        use gstreamer_video::prelude::VideoOverlayExtManual;

        init_gstreamer().expect("failed to initialize GStreamer");
        register_android_gstreamer_plugins();

        let pipeline = gst::parse::launch(&pipeline_description(codec))?
        .downcast::<gst::Pipeline>()
        .map_err(|_| VideoError::new("GStreamer launch did not create a pipeline"))?;

        let encoded_caps = codec.encoded_caps(codec_data.as_ref())?;
        let appsrc = pipeline
            .by_name("airplay_src")
            .ok_or_else(|| VideoError::new("missing AirPlay appsrc"))?
            .dynamic_cast::<gstreamer_app::AppSrc>()
            .map_err(|_| VideoError::new("airplay_src element has unexpected type"))?;
        appsrc.set_caps(Some(&encoded_caps));

        if let Some(window_handle) = window_handle {
            let overlay = pipeline
                .by_name("video_sink")
                .ok_or_else(|| VideoError::new("missing video sink"))?
                .dynamic_cast::<gstreamer_video::VideoOverlay>()
                .map_err(|_| VideoError::new("glimagesink does not implement GstVideoOverlay"))?;
            unsafe {
                overlay.set_window_handle(window_handle as _);
            }
        }

        pipeline.set_state(gst::State::Playing)?;

        Ok(Self { appsrc, pipeline })
    }

    pub(super) fn push_payload(&self, payload: &[u8]) -> Result<(), VideoError> {
        let buffer = gst_buffer_from_slice(payload)?;
        self.appsrc.push_buffer(buffer)?;
        Ok(())
    }

    pub(super) fn stop(self) {
        use gstreamer::prelude::*;

        let _ = self.appsrc.end_of_stream();
        let _ = self.pipeline.set_state(gstreamer::State::Null);
    }
}

impl Codec {
    fn parser_name(self) -> &'static str {
        match self {
            Self::H264 => "h264parse",
            Self::H265 => "h265parse",
        }
    }

    fn encoded_caps(self, codec_data: &[u8]) -> Result<gstreamer::Caps, VideoError> {
        let normalized = self.normalize_codec_data(codec_data);
        let codec_data = gst_buffer_from_slice(normalized.as_ref())?;

        Ok(match self {
            Self::H264 => gstreamer::Caps::builder("video/x-h264")
                .field("stream-format", "avc")
                .field("alignment", "au")
                .field("codec_data", codec_data)
                .build(),
            Self::H265 => gstreamer::Caps::builder("video/x-h265")
                .field("stream-format", "hvc1")
                .field("alignment", "au")
                .field("codec_data", codec_data)
                .build(),
        })
    }
}

fn gst_buffer_from_slice(data: &[u8]) -> Result<gstreamer::Buffer, VideoError> {
    let mut buffer = gstreamer::Buffer::with_size(data.len())?;
    let Some(buf) = buffer.get_mut() else {
        return Err(VideoError::new("failed to get writable GStreamer buffer"));
    };
    buf.copy_from_slice(0, data)
        .map_err(|_| VideoError::new("failed to copy data into GStreamer buffer"))?;
    Ok(buffer)
}

fn init_gstreamer() -> Result<(), VideoError> {
    gstreamer::init()?;
    scan_embedded_plugins()?;

    let mut elements = vec![
        "appsrc",
        "h264parse",
        "h265parse",
        "capsfilter",
        "glcolorconvert",
        "glimagesink",
    ];
    if cfg!(target_os = "macos") {
        let decoder = macos_decoder();
        elements.push(decoder.element_name());
        elements.extend(decoder.extra_element_names());
    } else {
        elements.push("vtdec_hw");
    }

    for element in elements {
        if gstreamer::ElementFactory::find(element).is_none() {
            return Err(VideoError::new(format!(
                "required GStreamer element '{element}' is missing"
            )));
        }
    }

    Ok(())
}

fn pipeline_description(codec: Codec) -> String {
    let parser = codec.parser_name();
    if cfg!(target_os = "macos") {
        macos_decoder().pipeline_description(parser)
    } else {
        format!(
            "appsrc name=airplay_src is-live=true format=time do-timestamp=true block=true \
             ! {parser} disable-passthrough=true \
             ! vtdec_hw \
             ! video/x-raw(memory:GLMemory),format=NV12,texture-target=rectangle \
             ! glcolorconvert \
             ! glimagesink name=video_sink sync=false force-aspect-ratio=true"
        )
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
enum MacosDecoder {
    AirplayVtdecHw,
    VtdecHw,
    AvdecH265,
}

#[cfg(target_os = "macos")]
impl MacosDecoder {
    fn element_name(self) -> &'static str {
        match self {
            Self::AirplayVtdecHw => "airplayvtdec_hw",
            Self::VtdecHw => "vtdec_hw",
            Self::AvdecH265 => "avdec_h265",
        }
    }

    fn extra_element_names(self) -> &'static [&'static str] {
        match self {
            Self::AvdecH265 => &["videoconvert"],
            Self::AirplayVtdecHw | Self::VtdecHw => &[],
        }
    }

    fn pipeline_description(self, parser: &str) -> String {
        let decoder = self.element_name();
        match self {
            Self::AvdecH265 => format!(
                "appsrc name=airplay_src is-live=true format=time do-timestamp=true block=true \
                 ! {parser} disable-passthrough=true \
                 ! {decoder} \
                 ! videoconvert \
                 ! glimagesink name=video_sink sync=false force-aspect-ratio=true"
            ),
            Self::AirplayVtdecHw | Self::VtdecHw => format!(
                "appsrc name=airplay_src is-live=true format=time do-timestamp=true block=true \
                 ! {parser} disable-passthrough=true \
                 ! {decoder} \
                 ! video/x-raw(memory:GLMemory),format=NV12,texture-target=rectangle \
                 ! glcolorconvert \
                 ! glimagesink name=video_sink sync=false force-aspect-ratio=true"
            ),
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_decoder() -> MacosDecoder {
    match std::env::var("AIRPLAY_GST_DECODER").as_deref() {
        Ok("vtdec_hw") => MacosDecoder::VtdecHw,
        Ok("avdec_h265") => MacosDecoder::AvdecH265,
        Ok("airplayvtdec_hw") | Err(_) => MacosDecoder::AirplayVtdecHw,
        Ok(value) => panic!(
            "unsupported AIRPLAY_GST_DECODER='{value}'; expected airplayvtdec_hw, vtdec_hw, or avdec_h265"
        ),
    }
}

fn scan_embedded_plugins() -> Result<(), VideoError> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let plugin_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("vendor/gstreamer/plugins/macos-aarch64");
        if !plugin_dir.exists() {
            return Err(VideoError::new(format!(
                "missing embedded GStreamer plugin directory '{}'; run 'just build-airplayvtdec'",
                plugin_dir.display()
            )));
        }

        if !gstreamer::Registry::get().scan_path(&plugin_dir)
            && gstreamer::ElementFactory::find("airplayvtdec_hw").is_none()
        {
            return Err(VideoError::new(format!(
                "failed to scan embedded GStreamer plugin directory '{}'",
                plugin_dir.display()
            )));
        }
    }

    Ok(())
}

#[cfg(target_os = "android")]
fn register_android_gstreamer_plugins() {
    unsafe extern "C" {
        fn gst_plugin_opengl_register();
    }

    unsafe {
        gst_plugin_opengl_register();
    }
}

#[cfg(not(target_os = "android"))]
fn register_android_gstreamer_plugins() {}

impl From<gstreamer::glib::BoolError> for VideoError {
    fn from(error: gstreamer::glib::BoolError) -> Self {
        Self::new(error.to_string())
    }
}

impl From<gstreamer::glib::Error> for VideoError {
    fn from(error: gstreamer::glib::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<gstreamer::StateChangeError> for VideoError {
    fn from(error: gstreamer::StateChangeError) -> Self {
        Self::new(error.to_string())
    }
}

impl From<gstreamer::FlowError> for VideoError {
    fn from(error: gstreamer::FlowError) -> Self {
        Self::new(error.to_string())
    }
}
