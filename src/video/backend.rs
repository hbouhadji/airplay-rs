use std::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoBackend {
    Auto,
    Native,
    GStreamer,
}

impl Default for VideoBackend {
    fn default() -> Self {
        Self::Auto
    }
}

impl VideoBackend {
    pub fn from_env_and_args<I, S>(args: I) -> Result<Self, Box<dyn Error>>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut backend = match std::env::var("AIRPLAY_RS_VIDEO_BACKEND") {
            Ok(value) => Self::parse_name(&value)?,
            Err(std::env::VarError::NotPresent) => Self::Auto,
            Err(error) => return Err(Box::new(error)),
        };

        let mut args = args.into_iter().map(Into::into);
        while let Some(arg) = args.next() {
            if let Some(value) = arg.strip_prefix("--video-backend=") {
                backend = Self::parse_name(value)?;
            } else if arg == "--video-backend" {
                let Some(value) = args.next() else {
                    return Err(invalid_input("--video-backend requires a value"));
                };
                backend = Self::parse_name(&value)?;
            } else if arg == "--native-video" {
                backend = Self::Native;
            } else if arg == "--gstreamer-video" {
                backend = Self::GStreamer;
            } else {
                return Err(invalid_input(format!(
                    "unknown argument '{arg}'. Use --video-backend auto|native|gstreamer"
                )));
            }
        }

        backend.ensure_available()?;
        Ok(backend)
    }

    pub(crate) fn resolved(self) -> Self {
        match self {
            Self::Auto => default_video_backend(),
            other => other,
        }
    }

    fn parse_name(value: &str) -> Result<Self, Box<dyn Error>> {
        match value {
            "auto" => Ok(Self::Auto),
            "native" | "macos" | "avfoundation" => Ok(Self::Native),
            "gstreamer" | "gst" => Ok(Self::GStreamer),
            other => Err(invalid_input(format!(
                "unknown video backend '{other}'. Expected auto, native, or gstreamer"
            ))),
        }
    }

    fn ensure_available(self) -> Result<(), Box<dyn Error>> {
        match self.resolved() {
            Self::Auto => unreachable!("video backend must be resolved before availability check"),
            Self::Native => {
                #[cfg(target_os = "macos")]
                {
                    Ok(())
                }

                #[cfg(not(target_os = "macos"))]
                {
                    Err(invalid_input(
                        "native video backend is only available on macOS",
                    ))
                }
            }
            Self::GStreamer => {
                #[cfg(any(not(target_os = "macos"), feature = "macos-gstreamer"))]
                {
                    Ok(())
                }

                #[cfg(all(target_os = "macos", not(feature = "macos-gstreamer")))]
                {
                    Err(invalid_input(
                        "GStreamer backend requested, but this binary was built without the 'macos-gstreamer' feature",
                    ))
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn default_video_backend() -> VideoBackend {
    VideoBackend::Native
}

#[cfg(not(target_os = "macos"))]
fn default_video_backend() -> VideoBackend {
    VideoBackend::GStreamer
}

fn invalid_input(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message.into(),
    ))
}
