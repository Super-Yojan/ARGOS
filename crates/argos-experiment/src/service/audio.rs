use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

pub struct Recording {
    _stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    channels: u16,
    started: Instant,
    error: Arc<Mutex<Option<String>>>,
}

impl Recording {
    pub fn start() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or("No Mic found")?;
        let config = device
                    .default_input_config()
                    .map_err(|error| format!("Mic not usable:{error}"))?;
    }
}
