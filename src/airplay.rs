use std::{
    error::Error,
    net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6},
    sync::Arc,
    thread::JoinHandle,
};

use rairplay::{
    ServiceFactory,
    config::{self, Config, DefaultKeychain, Features, Pairing},
    playback::{
        audio::{AudioPacket, AudioParams},
        null::NullDevice,
    },
    transport::DualStackListenerWithRtspRemap,
};

use crate::{discovery, video::AirPlayVideoDevice};

pub(crate) struct AirPlayServer {
    _mdns: mdns_sd::ServiceDaemon,
    _thread: JoinHandle<()>,
}

impl AirPlayServer {
    pub(crate) fn start(video_device: AirPlayVideoDevice) -> Self {
        let mut config = Config {
            mac_addr: Default::default(),
            features: Default::default(),
            manufacturer: env!("CARGO_PKG_AUTHORS").to_string(),
            model: env!("CARGO_PKG_NAME").to_string(),
            name: env!("CARGO_PKG_NAME").to_string(),
            fw_version: env!("CARGO_PKG_VERSION").to_string(),
            pin: None,
            pairing: Pairing::HomeKit,
            keychain: DefaultKeychain::default(),
            audio: config::Audio {
                device: NullDevice::<AudioParams, AudioPacket>::default(),
                ..Default::default()
            },
            video: config::Video {
                width: 1920,
                height: 1080,
                fps: 30,
                buf_size: 1024 * 1024,
                device: video_device,
            },
        };

        config.video.width = 3840;
        config.video.height = 2160;
        config.video.fps = 60;
        config.features |= Features::ScreenMultiCodec;

        match config.pairing {
            Pairing::Legacy => {
                config.features.insert(Features::LegacyPairing);
                config.features.remove(Features::HomeKitPairing);
            }
            Pairing::HomeKit => {
                config.features.insert(Features::HomeKitPairing);
                config.features.remove(Features::LegacyPairing);
            }
        }

        let config = Arc::new(config);
        let mdns = discovery::mdns_broadcast(&config, 7000);
        let thread_config = Arc::clone(&config);
        let thread = std::thread::spawn(move || {
            if let Err(err) = run_airplay_server(thread_config, 7000) {
                eprintln!("AirPlay server stopped: {err}");
            }
        });

        Self {
            _mdns: mdns,
            _thread: thread,
        }
    }
}

fn run_airplay_server(
    config: Arc<Config<NullDevice<AudioParams, AudioPacket>, AirPlayVideoDevice, DefaultKeychain>>,
    port: u16,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()?;

    rt.block_on(async move {
        let listener = DualStackListenerWithRtspRemap::bind(
            SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port),
            SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0),
        )?;

        axum::serve(listener, ServiceFactory::new(config)).await?;
        Ok(())
    })
}
